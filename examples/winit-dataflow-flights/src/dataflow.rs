use crate::{
    config::Config,
    queries::Target,
    selection::{Focus, Selections},
};
use anyhow::{Context, Result, ensure};
use arrow::{
    array::{Array, ArrayRef, Int32Array, StringArray},
    compute::concat_batches,
    datatypes::DataType,
    record_batch::RecordBatch,
};
use avenger_datafusion_dataflow::*;
use avenger_scales_datafusion::{BuiltinScale, list_literal, options_literal, scale_expr};
use avenger_selection::{ConsumerFilter, PredicateSplit};
use datafusion::{
    common::ScalarValue,
    functions_aggregate::expr_fn::{avg, count, max, min},
    functions_nested::expr_fn::make_array,
    logical_expr::{
        Expr, JoinType, LogicalPlan, LogicalPlanBuilder as LP, col, lit, scalar_subquery,
    },
    prelude::{ParquetReadOptions, SessionContext},
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub struct Metadata {
    pub source_rows: usize,
    pub eligible_rows: usize,
    pub carriers: Vec<String>,
    pub destinations: Vec<String>,
    pub domains: [[f32; 2]; 2],
}
#[derive(Clone)]
pub struct Request {
    pub selections: Selections,
    pub scatter_size: [f32; 2],
    pub airline_size: [f32; 2],
    pub panel_size: [f32; 2],
    pub bins: BTreeMap<String, i32>,
    pub focus: Option<Focus>,
}
struct Base {
    prepared: PreparedDataflow,
    source: TableOutput,
    scatter: TableOutput,
    predicate: ExprInput,
    width: ScalarInput,
    height: ScalarInput,
}
struct View {
    definition: Dataflow,
    prepared: PreparedExtension,
    targets: [Target; 3],
    full: [ExprInput; 2],
    retained: [ExprInput; 3],
    summary_scalars: Vec<[ScalarOutput; 2]>,
    scatter: TableOutput,
    scope: ScopeHandle,
    bin: ScalarInput,
    width: ScalarInput,
    height: ScalarInput,
    panel_width: ScalarInput,
    panel_height: ScalarInput,
}
struct Slot {
    key: Vec<Expr>,
    view: Arc<View>,
}
pub struct Engine {
    pub metadata: Metadata,
    pub config: Config,
    base: Base,
    direct: Arc<View>,
    slots: [Option<Slot>; 2],
}

pub struct Job {
    view: Arc<View>,
    base_inputs: Inputs,
    inputs: Inputs,
    outputs: Vec<TableOutput>,
    scalars: Vec<ScalarOutput>,
    warm: bool,
    pub strategy: String,
}
pub struct Evaluation {
    pub result: DataflowResult,
    view: Arc<View>,
    outputs: Vec<TableOutput>,
    scalars: Vec<ScalarOutput>,
    pub strategy: String,
    pub elapsed: std::time::Duration,
    pub warm: bool,
}
#[derive(Clone)]
pub struct Tables {
    pub scatter: TableSnapshot,
    pub airlines: TableSnapshot,
    pub summary: TableSnapshot,
    pub panels: BTreeMap<String, TableSnapshot>,
}

pub fn batch(table: &TableSnapshot) -> Result<RecordBatch> {
    Ok(concat_batches(table.schema(), table.batches())?)
}
pub fn number(batch: &RecordBatch, col_name: &str, row: usize) -> Result<Option<f64>> {
    let array = batch
        .column_by_name(col_name)
        .with_context(|| format!("missing column {col_name}"))?;
    if array.is_null(row) {
        return Ok(None);
    }
    let cast = arrow::compute::cast(array, &DataType::Float64)?;
    Ok(Some(
        cast.as_any()
            .downcast_ref::<arrow::array::Float64Array>()
            .unwrap()
            .value(row),
    ))
}
pub fn strings(table: &TableSnapshot, name: &str) -> Result<Vec<String>> {
    let b = batch(table)?;
    let array = arrow::compute::cast(
        b.column_by_name(name).context("string column")?,
        &DataType::Utf8,
    )?;
    Ok(array
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .iter()
        .flatten()
        .map(str::to_owned)
        .collect())
}
fn all_columns() -> Vec<Expr> {
    [
        "flight_id",
        "dep_delay",
        "arr_delay",
        "carrier",
        "dest",
        "scheduled_minute",
    ]
    .map(col)
    .to_vec()
}
fn scale(domain: Expr, range: Expr, values: Expr) -> datafusion::common::Result<Expr> {
    scale_expr(
        BuiltinScale::Linear,
        domain,
        range,
        options_literal(&Default::default())?,
        values,
    )
}
fn pair(a: Expr, b: Expr) -> Expr {
    make_array(vec![a, b])
}
fn scalar_column(
    b: &mut DataflowBuilder,
    name: &str,
    table: &PlanNode,
    column: &str,
) -> avenger_datafusion_dataflow::Result<ScalarNode> {
    let plan = LP::from(table.plan_ref())
        .project(vec![col(column)])?
        .build()?;
    b.add_scalar(name, scalar_subquery(Arc::new(plan)))
}
fn snapshot(columns: Vec<(&str, ArrayRef)>) -> Result<TableSnapshot> {
    let batch = RecordBatch::try_from_iter(columns)?;
    Ok(TableSnapshot::from_batches(batch.schema(), vec![batch])?)
}
impl Engine {
    pub async fn load(config: Config) -> Result<Self> {
        let context = SessionContext::new();
        let scan = context
            .read_parquet(
                config.data.to_str().context("data path must be UTF-8")?,
                ParquetReadOptions::default(),
            )
            .await?
            .into_unoptimized_plan();
        Self::from_plan(config, context, scan).await
    }
    pub async fn from_plan(
        config: Config,
        context: SessionContext,
        scan: LogicalPlan,
    ) -> Result<Self> {
        let mut b = DataflowBuilder::new();
        let scan = b.add_plan(
            "scan",
            LP::from(scan)
                .project(all_columns().into_iter().map(|e| {
                    if e == col("carrier") {
                        datafusion::logical_expr::cast(e, DataType::Utf8).alias("carrier")
                    } else if e == col("dest") {
                        datafusion::logical_expr::cast(e, DataType::Utf8).alias("dest")
                    } else {
                        e
                    }
                }))?
                .build()?,
        )?;
        let eligible = b.add_plan(
            "eligible",
            LP::from(scan.plan_ref())
                .filter(
                    col("dep_delay")
                        .is_not_null()
                        .and(col("arr_delay").is_not_null()),
                )?
                .build()?,
        )?;
        let source = b.table_output("eligible", &eligible)?;
        let totals = b.add_plan(
            "source_count",
            LP::from(scan.plan_ref())
                .aggregate(Vec::<Expr>::new(), vec![count(lit(1)).alias("n")])?
                .build()?,
        )?;
        let source_count = b.table_output("source_count", &totals)?;
        let domains = b.add_plan(
            "metadata",
            LP::from(eligible.plan_ref())
                .aggregate(
                    Vec::<Expr>::new(),
                    vec![
                        count(lit(1)).alias("n"),
                        min(col("dep_delay")).alias("x0"),
                        max(col("dep_delay")).alias("x1"),
                        min(col("arr_delay")).alias("y0"),
                        max(col("arr_delay")).alias("y1"),
                    ],
                )?
                .build()?,
        )?;
        let metadata_output = b.table_output("metadata", &domains)?;
        let mut domain_refs = vec![];
        for name in ["x0", "x1", "y0", "y1"] {
            domain_refs.push(scalar_column(&mut b, name, &domains, name)?);
        }
        let carriers = b.add_plan(
            "carrier_catalog",
            LP::from(eligible.plan_ref())
                .aggregate(vec![col("carrier")], vec![count(lit(1)).alias("count")])?
                .sort(vec![col("carrier").sort(true, false)])?
                .build()?,
        )?;
        let carrier_output = b.table_output("carrier_catalog", &carriers)?;
        let destinations = b.add_plan(
            "destination_catalog",
            LP::from(eligible.plan_ref())
                .aggregate(vec![col("dest")], vec![count(lit(1)).alias("count")])?
                .sort(vec![
                    col("count").sort(false, false),
                    col("dest").sort(true, false),
                ])?
                .limit(0, Some(6))?
                .build()?,
        )?;
        let destination_output = b.table_output("destination_catalog", &destinations)?;
        let predicate = b.expr_input("scatter_filter", DataType::Boolean)?;
        let width = b.scalar_input("scatter_width", DataType::Float32)?;
        let height = b.scalar_input("scatter_height", DataType::Float32)?;
        let rows = b.add_plan(
            "scatter_rows",
            LP::from(eligible.plan_ref())
                .filter(predicate.expr_ref())?
                .build()?,
        )?;
        // Padding and the nondegenerate domain are independent of selections.
        let pad = |lo: Expr, hi: Expr| {
            datafusion::functions::core::expr_fn::greatest(vec![hi - lo, lit(1_f32)])
                * lit(0.015_f32)
        };
        let domain = |i: usize| {
            let lo = datafusion::functions::core::expr_fn::least(vec![
                domain_refs[i].expr_ref(),
                lit(0_f32),
            ]);
            let hi = datafusion::functions::core::expr_fn::greatest(vec![
                domain_refs[i + 1].expr_ref(),
                lit(0_f32),
            ]);
            pair(
                lo.clone() - pad(lo.clone(), hi.clone()),
                hi.clone() + pad(lo, hi),
            )
        };
        let mut cols = all_columns();
        cols.push(
            scale(
                domain(0),
                pair(lit(0_f32), width.expr_ref()),
                col("dep_delay"),
            )?
            .alias("x"),
        );
        cols.push(
            scale(
                domain(2),
                pair(height.expr_ref(), lit(0_f32)),
                col("arr_delay"),
            )?
            .alias("y"),
        );
        let marks = b.add_plan(
            "scatter_marks",
            LP::from(rows.plan_ref())
                .project(cols)?
                .sort(vec![col("flight_id").sort(true, false)])?
                .build()?,
        )?;
        let scatter = b.table_output("scatter_marks", &marks)?;
        let runtime = Runtime::with_session_state(
            context.state(),
            RuntimeConfig {
                cache: if config.no_cache {
                    CachePolicy::Disabled
                } else {
                    CachePolicy::Lru(CacheConfig {
                        max_bytes: config
                            .cache_mib
                            .checked_mul(1024 * 1024)
                            .context("cache size exceeds the supported byte range")?,
                        ..Default::default()
                    })
                },
                execution: ExecutionConfig {
                    max_materialized_bytes: config
                        .materialization_mib
                        .checked_mul(1024 * 1024)
                        .context("materialization size exceeds the supported byte range")?,
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
        let definition = b.finish()?;
        if config.sql {
            println!("Scatter SQL:\n{}", definition.sql().table_output(&scatter)?);
        }
        let prepared = runtime.prepare(&definition).await?;
        let inputs = prepared
            .inputs()
            .expr(&predicate, lit(true))?
            .scalar(&width, 600_f32.into())?
            .scalar(&height, 350_f32.into())?
            .finish()?;
        let initial = prepared
            .query(
                &[
                    source_count,
                    metadata_output,
                    carrier_output,
                    destination_output,
                ],
                &[],
                &inputs,
            )
            .await?;
        let counts = batch(initial.table(&metadata_output)?)?;
        let source_rows = number(&batch(initial.table(&source_count)?)?, "n", 0)?.unwrap() as usize;
        let eligible_rows = number(&counts, "n", 0)?.unwrap() as usize;
        ensure!(eligible_rows > 0, "no flights have both delay values");
        let domains = [("x0", "x1"), ("y0", "y1")].map(|(a, b)| {
            let lo = (number(&counts, a, 0).unwrap().unwrap() as f32).min(0.);
            let hi = (number(&counts, b, 0).unwrap().unwrap() as f32).max(0.);
            let padding = (hi - lo).max(1.) * 0.015;
            [lo - padding, hi + padding]
        });
        let metadata = Metadata {
            source_rows,
            eligible_rows,
            carriers: strings(initial.table(&carrier_output)?, "carrier")?,
            destinations: strings(initial.table(&destination_output)?, "dest")?,
            domains,
        };
        let selections = Selections::new(&metadata.carriers, domains, [600., 350.], config.exact)?;
        let base = Base {
            prepared,
            source,
            scatter,
            predicate,
            width,
            height,
        };
        let direct = Arc::new(build_view(&base, &metadata, &selections, None).await?);
        println!(
            "Loaded {} source flights; {} eligible; {} excluded for missing delays.",
            source_rows,
            eligible_rows,
            source_rows - eligible_rows
        );
        Ok(Self {
            metadata,
            config,
            base,
            direct,
            slots: [None, None],
        })
    }
    pub async fn job(&mut self, request: &Request, warm: bool) -> Result<Option<Job>> {
        let mut view = self.direct.clone();
        let focus = request.focus.filter(|_| self.config.preaggregate);
        if warm && focus.is_none() {
            return Ok(None);
        }
        if let Some(focus) = focus {
            let index = if focus == Focus::Scatter { 0 } else { 1 };
            let key = context_key(&request.selections, focus)?;
            if self.slots[index]
                .as_ref()
                .is_none_or(|slot| slot.key != key)
            {
                let next = Arc::new(
                    build_view(&self.base, &self.metadata, &request.selections, Some(focus))
                        .await?,
                );
                if self.config.sql {
                    for target in &next.targets {
                        if let Some(out) = target.materialization {
                            println!(
                                "Warm-up SQL:\n{}",
                                next.definition.sql().table_output(&out)?
                            );
                        }
                    }
                }
                self.slots[index] = Some(Slot { key, view: next });
            }
            view = self.slots[index].as_ref().unwrap().view.clone();
        }
        let selections = &request.selections;
        let base_inputs = self
            .base
            .prepared
            .inputs()
            .expr(
                &self.base.predicate,
                selections
                    .consumer("scatter")?
                    .predicate(&selections.state)?,
            )?
            .scalar(&self.base.width, request.scatter_size[0].into())?
            .scalar(&self.base.height, request.scatter_size[1].into())?
            .finish()?;
        let filters = [
            selections.consumer("airline")?,
            selections.consumer("summary")?,
        ];
        let mut b = view
            .prepared
            .inputs()
            .expr(&view.full[0], filters[0].predicate(&selections.state)?)?
            .expr(&view.full[1], filters[1].predicate(&selections.state)?)?;
        let mut outputs = vec![view.scatter];
        let mut optimized = 0;
        for (i, target) in view.targets.iter().enumerate() {
            let predicates = focus
                .map(|f| {
                    filters[usize::from(i != 0)]
                        .predicates(&selections.state, selections.producer(f))
                })
                .transpose()?;
            let split = predicates.as_ref().and_then(|p| p.split().ok());
            let (output, expr) = target.bind(split)?;
            optimized += usize::from(Some(output) == target.optimized);
            outputs.push(output);
            b = b.expr(&view.retained[i], expr)?;
        }
        b = b
            .scalar(&view.width, request.airline_size[0].into())?
            .scalar(&view.height, request.airline_size[1].into())?
            .scope_defaults(&view.scope, |b| {
                b.scalar(&view.bin, 30_i32.into())?
                    .scalar(&view.panel_width, request.panel_size[0].into())?
                    .scalar(&view.panel_height, request.panel_size[1].into())
            })?;
        for (dest, width) in &request.bins {
            let instance = view.scope.instance([ScalarValue::from(dest.as_str())])?;
            b = b.at(&instance, |b| b.scalar(&view.bin, (*width).into()))?;
        }
        let inputs = b.finish()?;
        let summary_optimized = Some(outputs[2]) == view.targets[1].optimized;
        let scalars = if warm {
            vec![]
        } else {
            view.summary_scalars[usize::from(summary_optimized)].to_vec()
        };
        if warm {
            outputs = view
                .targets
                .iter()
                .filter_map(|t| t.materialization)
                .collect();
        }
        if outputs.is_empty() {
            return Ok(None);
        }
        Ok(Some(Job {
            view,
            base_inputs,
            inputs,
            outputs,
            scalars,
            warm,
            strategy: format!("{optimized}/3 aggregate targets preaggregated"),
        }))
    }
}
fn context_key(s: &Selections, focus: Focus) -> Result<Vec<Expr>> {
    let mut key = vec![];
    for name in ["airline", "summary"] {
        let p = s.consumer(name)?.predicates(&s.state, s.producer(focus))?;
        if let Ok(split) = p.split() {
            key.push(split.fixed().clone());
            key.extend(split.dimensions().iter().cloned());
        }
    }
    Ok(key)
}
async fn build_view(
    base: &Base,
    metadata: &Metadata,
    selections: &Selections,
    focus: Option<Focus>,
) -> Result<View> {
    let mut b = DataflowBuilder::with_base(&base.prepared.interface());
    let rows = b.import_table("flights", &base.source)?;
    let scatter = b.import_table("scatter", &base.scatter)?;
    let scatter = b.table_output("scatter", &scatter)?;
    let full = [
        b.expr_input("airline_filter", DataType::Boolean)?,
        b.expr_input("summary_filter", DataType::Boolean)?,
    ];
    let retained = [
        b.expr_input("airline_cells", DataType::Boolean)?,
        b.expr_input("summary_cells", DataType::Boolean)?,
        b.expr_input("histogram_cells", DataType::Boolean)?,
    ];
    let width = b.scalar_input("airline_width", DataType::Float32)?;
    let height = b.scalar_input("airline_height", DataType::Float32)?;
    let split = |filter: ConsumerFilter| -> Result<Option<PredicateSplit>> {
        Ok(focus
            .map(|f| filter.predicates(&selections.state, selections.producer(f)))
            .transpose()?
            .and_then(|p| p.split().ok().cloned()))
    };
    let airline_split = split(selections.consumer("airline")?)?;
    let summary_split = split(selections.consumer("summary")?)?;
    let catalog = b.table_snapshot(
        "carriers",
        snapshot(vec![(
            "carrier",
            Arc::new(StringArray::from(metadata.carriers.clone())),
        )])?,
    )?;
    let airlines = Target::install(
        |name, plan| {
            let node = b.add_plan(format!("airline_{name}"), plan)?;
            let final_node = if name == "states" {
                node.clone()
            } else {
                let completed = b.add_plan(
                    format!("airline_{name}_complete"),
                    complete(node.plan_ref(), catalog.plan_ref(), "carrier")?,
                )?;
                let max_table = b.add_plan(
                    format!("airline_{name}_max"),
                    LP::from(completed.plan_ref())
                        .aggregate(Vec::<Expr>::new(), vec![max(col("count")).alias("maximum")])?
                        .build()?,
                )?;
                let maximum = scalar_column(
                    &mut b,
                    &format!("airline_{name}_xmax"),
                    &max_table,
                    "maximum",
                )?;
                let domain = list_literal(Arc::new(StringArray::from(metadata.carriers.clone())))?;
                let y = scale_expr(
                    BuiltinScale::Band,
                    domain.clone(),
                    pair(lit(0_f32), height.expr_ref()),
                    options_literal(&Default::default())?,
                    col("carrier"),
                )?;
                let color = scale_expr(
                    BuiltinScale::Ordinal,
                    domain,
                    list_literal(Arc::new(Int32Array::from(
                        (0..metadata.carriers.len() as i32).collect::<Vec<_>>(),
                    )))?,
                    options_literal(&Default::default())?,
                    col("carrier"),
                )?;
                let x = scale(
                    pair(
                        lit(0_f32),
                        datafusion::functions::core::expr_fn::greatest(vec![
                            maximum.expr_ref(),
                            lit(1_f32),
                        ]),
                    ),
                    pair(lit(0_f32), width.expr_ref() - lit(52_f32)),
                    col("count"),
                )?;
                b.add_plan(
                    format!("airline_{name}_marks"),
                    LP::from(completed.plan_ref())
                        .project(vec![
                            col("carrier"),
                            col("count"),
                            x.alias("width"),
                            y.alias("y"),
                            color.alias("color"),
                        ])?
                        .build()?,
                )?
            };
            let output = b.table_output(format!("airline_{name}"), &final_node)?;
            Ok((node, output))
        },
        rows.plan_ref(),
        full[0].expr_ref(),
        retained[0].expr_ref(),
        airline_split.as_ref(),
        |rows| {
            LP::from(rows)
                .aggregate(vec![col("carrier")], vec![count(lit(1)).alias("count")])?
                .build()
        },
    )?;
    let summary = Target::install(
        |name, plan| {
            let node = b.add_plan(format!("summary_{name}"), plan)?;
            let out = b.table_output(format!("summary_{name}"), &node)?;
            Ok((node, out))
        },
        rows.plan_ref(),
        full[1].expr_ref(),
        retained[1].expr_ref(),
        summary_split.as_ref(),
        |rows| {
            LP::from(rows)
                .aggregate(
                    Vec::<Expr>::new(),
                    vec![
                        count(lit(1)).alias("count"),
                        avg(col("arr_delay")).alias("mean"),
                    ],
                )?
                .build()
        },
    )?;
    let mut summary_scalars = Vec::new();
    for (i, node) in std::iter::once(&summary.direct_node)
        .chain(summary.optimized_node.iter())
        .enumerate()
    {
        let count = scalar_column(&mut b, &format!("selected_{i}"), node, "count")?;
        let mean = scalar_column(&mut b, &format!("mean_{i}"), node, "mean")?;
        summary_scalars.push([
            b.scalar_output(format!("selected_{i}"), &count)?,
            b.scalar_output(format!("mean_{i}"), &mean)?,
        ]);
    }
    let panel_rows = b.add_plan(
        "panel_rows",
        LP::from(rows.plan_ref())
            .filter(
                col("dest").in_list(
                    metadata
                        .destinations
                        .iter()
                        .map(|s| lit(s.as_str()))
                        .collect(),
                    false,
                ),
            )?
            .build()?,
    )?;
    let (scope, (histogram, bin, panel_width, panel_height)) = b.partition_by(
        "destinations",
        panel_rows.plan_ref(),
        vec![col("dest")],
        |scope| {
            let bin = scope.scalar_input("bin_minutes", DataType::Int32)?;
            let panel_width = scope.scalar_input("width", DataType::Float32)?;
            let panel_height = scope.scalar_input("height", DataType::Float32)?;
            let mut cols = all_columns();
            cols.push((col("scheduled_minute") / bin.expr_ref()).alias("display_bin"));
            let bin_rows = scope.add_plan(
                "bin_rows",
                LP::from(scope.rows().plan_ref()).project(cols)?.build()?,
            )?;
            let catalog_data = snapshot(vec![(
                "minute",
                Arc::new(Int32Array::from((0..1440).collect::<Vec<_>>())),
            )])
            .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            let catalog = scope.table_snapshot("minutes", catalog_data)?;
            let catalog = scope.add_plan(
                "bins",
                LP::from(catalog.plan_ref())
                    .project(vec![(col("minute") / bin.expr_ref()).alias("display_bin")])?
                    .distinct()?
                    .build()?,
            )?;
            let histogram = Target::install(
                |name, plan| {
                    let node = scope.add_plan(name, plan)?;
                    let final_node = if name == "states" {
                        node.clone()
                    } else {
                        let completed = scope.add_plan(
                            format!("{name}_complete"),
                            complete(node.plan_ref(), catalog.plan_ref(), "display_bin")?,
                        )?;
                        let maximum = scope.add_plan(
                            format!("{name}_max"),
                            LP::from(completed.plan_ref())
                                .aggregate(
                                    Vec::<Expr>::new(),
                                    vec![max(col("count")).alias("maximum")],
                                )?
                                .build()?,
                        )?;
                        let ymax = scope.add_scalar(
                            format!("{name}_ymax"),
                            scalar_subquery(Arc::new(maximum.plan_ref())),
                        )?;
                        let ydomain = pair(
                            lit(0_f32),
                            datafusion::functions::core::expr_fn::greatest(vec![
                                lit(1_f32),
                                ymax.expr_ref(),
                            ]),
                        );
                        let xdomain = pair(lit(0_f32), lit(1440_f32));
                        let x0 = scale(
                            xdomain.clone(),
                            pair(lit(0_f32), panel_width.expr_ref()),
                            col("display_bin") * bin.expr_ref(),
                        )?;
                        let x1 = scale(
                            xdomain,
                            pair(lit(0_f32), panel_width.expr_ref()),
                            (col("display_bin") + lit(1_i32)) * bin.expr_ref(),
                        )?;
                        let y = scale(
                            ydomain,
                            pair(panel_height.expr_ref(), lit(0_f32)),
                            col("count"),
                        )?;
                        scope.add_plan(
                            format!("{name}_marks"),
                            LP::from(completed.plan_ref())
                                .project(vec![
                                    col("display_bin"),
                                    col("count"),
                                    x0.alias("x0"),
                                    x1.alias("x1"),
                                    y.alias("y"),
                                ])?
                                .build()?,
                        )?
                    };
                    let out = scope.table_output(name, &final_node)?;
                    Ok((node, out))
                },
                bin_rows.plan_ref(),
                full[1].expr_ref(),
                retained[2].expr_ref(),
                summary_split.as_ref(),
                |rows| {
                    LP::from(rows)
                        .aggregate(vec![col("display_bin")], vec![count(lit(1)).alias("count")])?
                        .build()
                },
            )
            .map_err(|e| Error::InvalidConfig(format!("histogram: {e:#}")))?;
            Ok((histogram, bin, panel_width, panel_height))
        },
    )?;
    let definition = b.finish()?;
    let prepared = base.prepared.prepare_extension(&definition).await?;
    Ok(View {
        definition,
        prepared,
        targets: [airlines, summary, histogram],
        full,
        retained,
        summary_scalars,
        scatter,
        scope,
        bin,
        width,
        height,
        panel_width,
        panel_height,
    })
}
fn complete(
    aggregate: LogicalPlan,
    catalog: LogicalPlan,
    key: &str,
) -> datafusion::common::Result<LogicalPlan> {
    LP::from(LP::from(catalog).alias("catalog")?.build()?)
        .join(
            LP::from(aggregate).alias("values")?.build()?,
            JoinType::Left,
            (vec![key], vec![key]),
            None,
        )?
        .project(vec![
            col(format!("catalog.{key}")).alias(key),
            datafusion::functions::core::expr_fn::coalesce(vec![col("values.count"), lit(0_i64)])
                .alias("count"),
        ])?
        .sort(vec![col(key).sort(true, false)])?
        .build()
}
impl Job {
    pub async fn run(self) -> Result<Evaluation> {
        let started = std::time::Instant::now();
        let result = self
            .view
            .prepared
            .query(
                &self.outputs,
                &self.scalars,
                &self.base_inputs,
                &self.inputs,
            )
            .await?;
        Ok(Evaluation {
            result,
            view: self.view,
            outputs: self.outputs,
            scalars: self.scalars,
            strategy: self.strategy,
            elapsed: started.elapsed(),
            warm: self.warm,
        })
    }
}
impl Evaluation {
    pub fn summary(&self) -> Result<(usize, Option<f64>)> {
        ensure!(!self.warm, "warm-up has no summary scalars");
        let count = match self.result.scalar(&self.scalars[0])? {
            ScalarValue::Int64(Some(n)) => *n as usize,
            value => anyhow::bail!("unexpected count: {value:?}"),
        };
        let mean = match self.result.scalar(&self.scalars[1])? {
            ScalarValue::Float64(v) => *v,
            value => anyhow::bail!("unexpected mean: {value:?}"),
        };
        Ok((count, mean))
    }

    pub fn tables_for(&self, metadata: &Metadata) -> Result<Tables> {
        ensure!(!self.warm, "warm-up contains states, not chart results");
        let mut t = Tables {
            scatter: self.result.table(&self.outputs[0])?.clone(),
            airlines: self.result.table(&self.outputs[1])?.clone(),
            summary: self.result.table(&self.outputs[2])?.clone(),
            panels: BTreeMap::new(),
        };
        let scope = self.result.scope(&self.view.scope)?;
        for dest in &metadata.destinations {
            let key = self.view.scope.key([ScalarValue::from(dest.as_str())])?;
            if let Some(panel) = scope.get(&key) {
                t.panels
                    .insert(dest.clone(), panel.table(&self.outputs[3])?.clone());
            }
        }
        Ok(t)
    }
    pub fn print(&self, label: &str) {
        let r = self.result.report();
        println!(
            "{label}: {} | {:.1} ms | cache {} | in-flight {}",
            self.strategy,
            self.elapsed.as_secs_f64() * 1000.,
            r.cache_hits,
            r.in_flight_hits
        );
        let mut nodes = BTreeMap::new();
        for name in &r.executed_nodes {
            *nodes.entry(name).or_insert(0) += 1;
        }
        if nodes.is_empty() {
            println!("  Executed: none");
        }
        for (node, count) in nodes {
            if count == 1 {
                println!("  {node}");
            } else {
                println!("  {node} ({count} instances)");
            }
        }
        if self.warm {
            let size = |t: &TableSnapshot| {
                (
                    t.num_rows(),
                    t.batches()
                        .iter()
                        .map(|b| b.get_array_memory_size())
                        .sum::<usize>(),
                )
            };
            for (i, name) in ["airlines", "summary"].iter().enumerate() {
                if let Some(output) = self.view.targets[i].materialization
                    && let Ok(table) = self.result.table(&output)
                {
                    let (rows, bytes) = size(table);
                    println!("  {name} states: {rows} rows, {bytes} bytes");
                }
            }
            if let Some(output) = self.view.targets[2].materialization
                && let Ok(scope) = self.result.scope(&self.view.scope)
            {
                let (mut rows, mut bytes) = (0, 0);
                for (_, panel) in scope.iter() {
                    if let Ok(table) = panel.table(&output) {
                        let (r, b) = size(table);
                        rows += r;
                        bytes += b;
                    }
                }
                println!("  Destination states (all instances): {rows} rows, {bytes} bytes");
            }
        }
    }
}
