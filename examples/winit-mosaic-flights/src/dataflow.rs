use crate::{
    config::Config,
    selection::{PLOTS, Plot, Selections},
};
use anyhow::{Context, Result};
use arrow::{
    array::AsArray,
    datatypes::{DataType, Int32Type, Int64Type},
};
use avenger_datafusion_dataflow::*;
use avenger_datafusion_preaggregate::{
    FilterQuery, PreaggregatePlanner, PreparedQuery,
    runtime::{ParameterExpressions, ParameterizedFamily},
};
use datafusion::{
    functions::math::expr_fn::floor,
    functions_aggregate::expr_fn::count,
    logical_expr::{LogicalPlan, LogicalPlanBuilder as LP, cast, col, lit},
    prelude::{ParquetReadOptions, SessionContext},
};
use std::{sync::Arc, time::Instant};

struct HistogramQuery {
    filter: ExprInput,
    output: TableOutput,
}
struct CrossFilterQuery {
    focus: usize,
    target: usize,
    fixed: ExprInput,
    cells: ExprInput,
    prepared: PreparedQuery,
    templates: ParameterizedFamily,
    states: TableOutput,
    rollup: TableOutput,
}
pub struct Engine {
    prepared: PreparedDataflow,
    defaults: Inputs,
    histograms: Vec<HistogramQuery>,
    crossfilters: Vec<CrossFilterQuery>,
    diagnostics: bool,
}
pub struct Evaluation {
    pub bins: [Vec<(i32, i64)>; 3],
    pub elapsed_ms: f64,
    pub preaggregated: usize,
}
fn histogram(rows: LogicalPlan, plot: &Plot) -> datafusion::common::Result<LogicalPlan> {
    LP::from(rows)
        .aggregate(
            vec![
                cast(
                    floor((col(plot.name) - lit(plot.domain[0] as f64)) / lit(plot.step as f64)),
                    DataType::Int32,
                )
                .alias("bin"),
            ],
            vec![count(lit(1)).alias("count")],
        )?
        .build()
}
fn output(
    b: &mut DataflowBuilder,
    name: &str,
    plan: LogicalPlan,
) -> Result<(PlanNode, TableOutput)> {
    let node = b.add_plan(name, plan)?;
    let output = b.table_output(name, &node)?;
    Ok((node, output))
}
impl Engine {
    pub async fn load(config: &Config, selections: &Selections) -> Result<Arc<Self>> {
        let context = SessionContext::new();
        context
            .register_parquet(
                "source",
                config.data.to_str().context("UTF-8 data path")?,
                ParquetReadOptions::default(),
            )
            .await?;
        let source = context
            .sql(
                r#"
            SELECT GREATEST(-60, LEAST("ARR_DELAY", 180))::DOUBLE AS delay,
                   "DEP_TIME"::DOUBLE AS time, "DISTANCE"::DOUBLE AS distance
            FROM source
        "#,
            )
            .await?
            .into_unoptimized_plan();
        let mut b = DataflowBuilder::new();
        let rows = b.add_plan("flights", source)?;
        let mut histograms = vec![];
        let mut crossfilters = vec![];
        for (target, plot) in PLOTS.iter().enumerate() {
            let filter = b.expr_input(format!("{}_filter", plot.name), DataType::Boolean)?;
            let (_, direct) = output(
                &mut b,
                &format!("{}_direct", plot.name),
                histogram(
                    LP::from(rows.plan_ref())
                        .filter(filter.expr_ref())?
                        .build()?,
                    plot,
                )?,
            )?;
            histograms.push(HistogramQuery {
                filter,
                output: direct,
            });
            if !config.preaggregate {
                continue;
            }
            for (focus, producer) in selections.producers.iter().enumerate() {
                if focus == target {
                    continue;
                }
                let name = format!("{}_to_{}", PLOTS[focus].name, plot.name);
                let fixed = b.expr_input(format!("{name}_fixed"), DataType::Boolean)?;
                let cells = b.expr_input(format!("{name}_cells"), DataType::Boolean)?;
                let predicates = selections
                    .consumer(target)?
                    .predicates(&selections.state, producer)?;
                let split = predicates.split().map_err(|e| anyhow::anyhow!(e))?;
                // Other brushes are ordinary inputs upstream of the aggregate states.
                // Dataflow invalidates those states when their fixed predicate changes.
                let source = LP::from(rows.plan_ref())
                    .filter(fixed.expr_ref())?
                    .build()?;
                let query = FilterQuery::new(source, |rows| histogram(rows, plot))?;
                let prepared =
                    PreaggregatePlanner::default().prepare(query, split.dimensions().to_vec())?;
                let templates = prepared.parameterize(ParameterExpressions {
                    source: lit(true),
                    retained: cells.expr_ref(),
                })?;
                let plans = templates.preaggregated.as_ref().with_context(|| {
                    format!(
                        "histogram preaggregation: {:?}",
                        prepared.explain().direct_reason
                    )
                })?;
                let (node, states) = output(
                    &mut b,
                    &format!("{name}_states"),
                    plans.materialization.clone(),
                )?;
                let (_, rollup) = output(
                    &mut b,
                    &format!("{name}_rollup"),
                    plans.rollup.with_materialization(node.plan_ref())?,
                )?;
                crossfilters.push(CrossFilterQuery {
                    focus,
                    target,
                    fixed,
                    cells,
                    prepared,
                    templates,
                    states,
                    rollup,
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
        let prepared = runtime.prepare(&b.finish()?).await?;
        let mut defaults = prepared.inputs();
        for h in &histograms {
            defaults = defaults.expr(&h.filter, lit(true))?;
        }
        for pair in &crossfilters {
            defaults = defaults
                .expr(&pair.fixed, lit(true))?
                .expr(&pair.cells, lit(true))?;
        }
        Ok(Arc::new(Self {
            defaults: defaults.finish()?,
            prepared,
            histograms,
            crossfilters,
            diagnostics: config.diagnostics,
        }))
    }
    pub async fn query(
        &self,
        selections: &Selections,
        focus: Option<usize>,
        warm: bool,
    ) -> Result<Evaluation> {
        let start = Instant::now();
        let mut inputs = self.defaults.edit();
        let mut outputs = self.histograms.iter().map(|h| h.output).collect::<Vec<_>>();
        for (i, h) in self.histograms.iter().enumerate() {
            inputs = inputs.expr(
                &h.filter,
                selections.consumer(i)?.predicate(&selections.state)?,
            )?;
        }
        let mut states = vec![];
        let mut preaggregated = 0;
        for pair in self.crossfilters.iter().filter(|p| Some(p.focus) == focus) {
            let predicates = selections
                .consumer(pair.target)?
                .predicates(&selections.state, &selections.producers[pair.focus])?;
            let Ok(split) = predicates.split() else {
                continue;
            };
            let bound = pair.prepared.bind(split.changing().clone())?;
            pair.templates.check_binding(bound.predicates())?;
            if let Some(cells) = bound.predicates().retained() {
                inputs = inputs
                    .expr(&pair.fixed, split.fixed().clone())?
                    .expr(&pair.cells, cells.clone())?;
                outputs[pair.target] = pair.rollup;
                states.push(pair.states);
                preaggregated += 1;
            }
        }
        // The focused histogram stays unchanged during its own drag.
        if warm && let Some(focus) = focus {
            states.push(self.histograms[focus].output);
        }
        let requested = if warm { &states } else { &outputs };
        let result = self
            .prepared
            .query(requested, &[], &inputs.finish()?)
            .await?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
        if self.diagnostics {
            let r = result.report();
            println!(
                "{}: {elapsed_ms:.1} ms, {preaggregated}/3 preaggregated, cache hits {}, executed [{}]",
                if warm { "Warm-up" } else { "Update" },
                r.cache_hits,
                r.executed_nodes.join(", ")
            );
        }
        let mut bins: [Vec<(i32, i64)>; 3] = Default::default();
        if !warm {
            for (i, output) in outputs.iter().enumerate() {
                for batch in result.table(output)?.batches() {
                    let keys = batch
                        .column_by_name("bin")
                        .context("bin column")?
                        .as_primitive::<Int32Type>();
                    let counts = batch
                        .column_by_name("count")
                        .context("count column")?
                        .as_primitive::<Int64Type>();
                    bins[i].extend(keys.iter().zip(counts.iter()).filter_map(|(k, n)| k.zip(n)));
                }
                bins[i].sort_unstable_by_key(|&(bin, _)| bin);
            }
        }
        Ok(Evaluation {
            bins,
            elapsed_ms,
            preaggregated,
        })
    }
}
