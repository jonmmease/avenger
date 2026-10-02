use crate::selection::{PLOTS, Plot, Selections};
use anyhow::{Context, Result};
use arrow::{
    array::{Array, AsArray},
    datatypes::{DataType, Float64Type, Int32Type, Int64Type, SchemaRef},
};
use avenger_datafusion_dataflow::*;
use avenger_datafusion_preaggregate::{FilterQuery, PreaggregatePlanner, dataflow::Query};
use avenger_transform::{self as transform, BinOptions, expr_fn};
use datafusion::{
    functions::core::expr_fn::{greatest, least, named_struct},
    functions_aggregate::expr_fn::{avg, stddev},
    logical_expr::{LogicalPlan, LogicalPlanBuilder as LP, cast, col, lit, try_cast},
    prelude::{SessionConfig, SessionContext},
};
use std::{sync::Arc, time::Instant};

struct DirectQuery {
    filter: ExprInput,
    output: TableOutput,
    target: CacheNode,
}
struct CrossFilterQuery {
    focus: usize,
    target: usize,
    fixed: ExprInput,
    query: Query,
    cache_target: CacheNode,
}

pub struct Engine {
    prepared: PreparedDataflow,
    source: TableInput,
    defaults: Inputs,
    direct: Vec<DirectQuery>,
    crossfilters: Vec<CrossFilterQuery>,
    diagnostics: bool,
}

#[derive(Debug)]
pub struct Carrier {
    pub name: String,
    pub count: i64,
    pub mean: Option<f64>,
    pub stddev: Option<f64>,
}

pub struct Evaluation {
    pub bins: [Vec<(i32, i64)>; 3],
    pub carriers: Vec<Carrier>,
    pub snapshot: TableSnapshot,
    pub elapsed_ms: f64,
    pub cache_hits: usize,
    pub executed: Vec<String>,
}

impl Evaluation {
    pub fn empty(snapshot: TableSnapshot) -> Self {
        Self {
            bins: Default::default(),
            carriers: vec![],
            snapshot,
            elapsed_ms: 0.,
            cache_hits: 0,
            executed: vec![],
        }
    }
}

pub struct Warming {
    query: CacheAwareQuery,
    outputs: Vec<TableOutput>,
    pub snapshot: TableSnapshot,
    started: Instant,
}

impl Warming {
    /// Observe this exact captured version, even when ingestion has moved ahead.
    pub async fn probe(&self) -> Result<Option<usize>> {
        match self.query.read(CacheRead::CachedOnly).await {
            Ok(result) => Ok(Some(self.outputs.iter().try_fold(0, |rows, output| {
                Ok::<_, Error>(rows + result.result().table(output)?.num_rows())
            })?)),
            Err(Error::CacheMiss { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.
    }
}

struct Bound {
    inputs: Inputs,
    outputs: Vec<TableOutput>,
    targets: Vec<CacheNode>,
    warm_outputs: Vec<TableOutput>,
}

fn histogram(rows: LogicalPlan, plot: &Plot) -> datafusion::common::Result<LogicalPlan> {
    let parameters = transform::bin_parameters(
        named_struct(vec![
            lit("min"),
            lit(plot.domain[0] as f64),
            lit("max"),
            lit(plot.domain[1] as f64),
        ]),
        BinOptions {
            step: Some(lit(plot.step as f64)),
            nice: Some(lit(false)),
            ..Default::default()
        },
    )?;
    let start = expr_fn::bin_start(col(plot.name), parameters);
    transform::aggregate(
        rows,
        vec![
            // Binning emits infinity outside the plot domain. Null bin keys are
            // omitted by decoding, without removing rows from the other charts.
            try_cast(
                (start - lit(plot.domain[0] as f64)) / lit(plot.step as f64),
                DataType::Int32,
            )
            .alias("bin"),
        ],
        vec![expr_fn::count().alias("count")],
    )
}

fn chart_query(rows: LogicalPlan, target: usize) -> datafusion::common::Result<LogicalPlan> {
    if let Some(plot) = PLOTS.get(target) {
        histogram(rows, plot)
    } else {
        transform::aggregate(
            rows,
            vec![col("carrier")],
            vec![
                expr_fn::count().alias("count"),
                avg(col("arrival_delay")).alias("mean"),
                stddev(col("arrival_delay")).alias("stddev"),
            ],
        )
    }
}

impl Engine {
    pub async fn new(
        schema: SchemaRef,
        selections: &Selections,
        diagnostics: bool,
    ) -> Result<Arc<Self>> {
        let context =
            SessionContext::new_with_config(SessionConfig::new().with_target_partitions(4));
        let mut builder = DataflowBuilder::new();
        let source = builder.table_input("flights", schema.clone())?;
        // Keep transforms inside each materialization plan so a later incremental
        // implementation can bind its raw input to a suffix of this same table.
        let rows = LP::from(source.plan_ref())
            .project(vec![
                cast(
                    greatest(vec![lit(-60), least(vec![col("arr_delay"), lit(180)])]),
                    DataType::Float64,
                )
                .alias("delay"),
                (cast(col("scheduled_minute"), DataType::Float64) / lit(60.)).alias("time"),
                cast(col("distance"), DataType::Float64).alias("distance"),
                cast(col("arr_delay"), DataType::Float64).alias("arrival_delay"),
                col("carrier"),
            ])?
            .build()?;
        let mut direct = vec![];
        let mut crossfilters = vec![];
        for target in 0..4 {
            let name = PLOTS.get(target).map_or("carriers", |p| p.name);
            let filter = builder.expr_input(format!("{name}_filter"), DataType::Boolean)?;
            let node = builder.add_plan(
                format!("{name}_direct"),
                chart_query(transform::filter(rows.clone(), filter.expr_ref())?, target)?,
            )?;
            let output = builder.table_output(format!("{name}_direct"), &node)?;
            direct.push(DirectQuery {
                filter,
                output,
                target: CacheNode::Plan(node),
            });
            for (focus, producer) in selections.producers.iter().enumerate() {
                if focus == target {
                    continue;
                }
                let name = format!("{}_to_{name}", PLOTS[focus].name);
                let fixed = builder.expr_input(format!("{name}_fixed"), DataType::Boolean)?;
                let predicates = selections
                    .consumer(target)?
                    .predicates(&selections.state, producer)?;
                let split = predicates.split().map_err(|e| anyhow::anyhow!(e))?;
                let query =
                    FilterQuery::new(transform::filter(rows.clone(), fixed.expr_ref())?, |rows| {
                        chart_query(rows, target)
                    })?;
                let prepared =
                    PreaggregatePlanner::default().prepare(query, split.dimensions().to_vec())?;
                let query = Query::install(&mut builder, &name, prepared)?;
                query.materialization_output().with_context(|| {
                    format!("preaggregation {name}: {:?}", query.explain().direct_reason)
                })?;
                crossfilters.push(CrossFilterQuery {
                    focus,
                    target,
                    fixed,
                    query,
                    cache_target: CacheNode::Named(Reference {
                        scope: vec![],
                        name: format!("{name}_states"),
                    }),
                });
            }
        }
        let runtime = Runtime::with_session_state(
            context.state(),
            RuntimeConfig {
                cache: CachePolicy::Lru(CacheConfig {
                    max_bytes: 2 * 1024 * 1024 * 1024,
                    ..Default::default()
                }),
                execution: ExecutionConfig {
                    max_materialized_bytes: 4 * 1024 * 1024 * 1024,
                    ..Default::default()
                },
                ..Default::default()
            },
        )?;
        let prepared = runtime.prepare(&builder.finish()?).await?;
        let mut defaults = prepared
            .inputs()
            .table(&source, TableSnapshot::empty(schema))?;
        for query in &direct {
            defaults = defaults.expr(&query.filter, lit(true))?;
        }
        for pair in &crossfilters {
            defaults = pair
                .query
                .bind(lit(true))?
                .apply(defaults.expr(&pair.fixed, lit(true))?)?;
        }
        Ok(Arc::new(Self {
            prepared,
            source,
            defaults: defaults.finish()?,
            direct,
            crossfilters,
            diagnostics,
        }))
    }

    fn bind(
        &self,
        selections: &Selections,
        focus: usize,
        snapshot: TableSnapshot,
    ) -> Result<Bound> {
        let mut inputs = self.defaults.edit().table(&self.source, snapshot)?;
        let mut outputs = self.direct.iter().map(|q| q.output).collect::<Vec<_>>();
        let mut targets = self
            .direct
            .iter()
            .map(|q| q.target.clone())
            .collect::<Vec<_>>();
        let mut warm_outputs = outputs.clone();
        for (i, query) in self.direct.iter().enumerate() {
            inputs = inputs.expr(
                &query.filter,
                selections.consumer(i)?.predicate(&selections.state)?,
            )?;
        }
        for pair in self.crossfilters.iter().filter(|p| p.focus == focus) {
            let predicates = selections
                .consumer(pair.target)?
                .predicates(&selections.state, &selections.producers[focus])?;
            let Ok(split) = predicates.split() else {
                continue;
            };
            let bound = pair.query.bind(split.changing().clone())?;
            if let Some(materialization) = bound.materialization_output() {
                inputs = bound.apply(inputs.expr(&pair.fixed, split.fixed().clone())?)?;
                outputs[pair.target] = bound.output();
                targets[pair.target] = pair.cache_target.clone();
                warm_outputs[pair.target] = materialization;
            }
        }
        // The focused histogram and any direct fallback remain required targets.
        // A cached state on another branch must never authorize a raw foreground scan.
        Ok(Bound {
            inputs: inputs.finish()?,
            outputs,
            targets,
            warm_outputs,
        })
    }

    pub fn warm(
        &self,
        selections: &Selections,
        focus: usize,
        snapshot: TableSnapshot,
    ) -> Result<Warming> {
        let started = Instant::now();
        let bound = self.bind(selections, focus, snapshot.clone())?;
        let query = self.prepared.cache_aware_query(
            &bound.warm_outputs,
            &[],
            QueryInputs::new(bound.inputs),
            CacheAwareOptions {
                start_latest: true,
                ..Default::default()
            },
        )?;
        Ok(Warming {
            query,
            outputs: bound.warm_outputs,
            snapshot,
            started,
        })
    }

    pub async fn read(
        &self,
        selections: &Selections,
        focus: usize,
        latest: TableSnapshot,
        fallbacks: &[TableSnapshot],
    ) -> Result<Option<Evaluation>> {
        let started = Instant::now();
        let bound = self.bind(selections, focus, latest.clone())?;
        let mut ids = vec![latest.id()];
        let mut overrides = vec![];
        for snapshot in fallbacks {
            if ids.contains(&snapshot.id()) {
                continue;
            }
            ids.push(snapshot.id());
            overrides.push(
                self.prepared
                    .inputs()
                    .table(&self.source, snapshot.clone())?
                    .finish_overrides()?,
            );
        }
        let query = self.prepared.cache_aware_query(
            &bound.outputs,
            &[],
            QueryInputs::new(bound.inputs).fallbacks(overrides)?,
            CacheAwareOptions {
                targets: CacheTargets::Nodes(bound.targets),
                start_latest: false,
            },
        )?;
        let result = match query.read(CacheRead::FromCachedTargets).await {
            Ok(result) => result,
            Err(Error::CacheMiss { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let snapshot = result.inputs().table_value(&self.source)?.clone();
        let evaluation = decode(result.result(), &bound.outputs, snapshot, started)?;
        if self.diagnostics {
            eprintln!(
                "Brush: {:.2} ms, {} displayed rows, candidate {}, cache hits {}, executed [{}]",
                evaluation.elapsed_ms,
                evaluation.snapshot.num_rows(),
                result.candidate_index(),
                evaluation.cache_hits,
                evaluation.executed.join(", ")
            );
        }
        Ok(Some(evaluation))
    }

    pub fn clear(&self) {
        self.prepared.clear_results();
    }

    #[cfg(test)]
    pub async fn direct(
        &self,
        selections: &Selections,
        snapshot: TableSnapshot,
    ) -> Result<Evaluation> {
        let started = Instant::now();
        let bound = self.bind(selections, 0, snapshot.clone())?;
        let outputs = self.direct.iter().map(|q| q.output).collect::<Vec<_>>();
        let result = self.prepared.query(&outputs, &[], &bound.inputs).await?;
        decode(&result, &outputs, snapshot, started)
    }
}

fn decode(
    result: &DataflowResult,
    outputs: &[TableOutput],
    snapshot: TableSnapshot,
    started: Instant,
) -> Result<Evaluation> {
    let mut evaluation = Evaluation::empty(snapshot);
    for (i, output) in outputs[..3].iter().enumerate() {
        for batch in result.table(output)?.batch_iter() {
            let keys = batch
                .column_by_name("bin")
                .context("bin column")?
                .as_primitive::<Int32Type>();
            let counts = batch
                .column_by_name("count")
                .context("count column")?
                .as_primitive::<Int64Type>();
            evaluation.bins[i].extend(keys.iter().zip(counts.iter()).filter_map(|(k, n)| k.zip(n)));
        }
        evaluation.bins[i].sort_unstable_by_key(|&(bin, _)| bin);
    }
    for batch in result.table(&outputs[3])?.batch_iter() {
        let names = batch
            .column_by_name("carrier")
            .context("carrier column")?
            .as_string::<i32>();
        let counts = batch
            .column_by_name("count")
            .context("count column")?
            .as_primitive::<Int64Type>();
        let means = batch
            .column_by_name("mean")
            .context("mean column")?
            .as_primitive::<Float64Type>();
        let stddevs = batch
            .column_by_name("stddev")
            .context("stddev column")?
            .as_primitive::<Float64Type>();
        for row in 0..batch.num_rows() {
            evaluation.carriers.push(Carrier {
                name: names.value(row).into(),
                count: counts.value(row),
                mean: (!means.is_null(row)).then(|| means.value(row)),
                stddev: (!stddevs.is_null(row)).then(|| stddevs.value(row)),
            });
        }
    }
    evaluation.carriers.sort_by(|a, b| a.name.cmp(&b.name));
    evaluation.cache_hits = result.report().cache_hits;
    evaluation.executed = result.report().executed_nodes.clone();
    evaluation.elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
    Ok(evaluation)
}
