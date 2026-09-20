use anyhow::Result;
use avenger_datafusion_dataflow::{PlanNode, TableOutput};
use avenger_datafusion_preaggregate::{
    FilterQuery, PreaggregatePlanner, PreparedQuery,
    runtime::{ParameterExpressions, ParameterizedFamily},
};
use avenger_selection::PredicateSplit;
use datafusion::logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder, lit};

pub struct Target {
    pub direct: TableOutput,
    pub direct_node: PlanNode,
    pub optimized_node: Option<PlanNode>,
    pub optimized: Option<TableOutput>,
    pub materialization: Option<TableOutput>,
    prepared: Option<PreparedQuery>,
    templates: Option<ParameterizedFamily>,
}
impl Target {
    pub fn install(
        mut add: impl FnMut(
            &str,
            LogicalPlan,
        ) -> avenger_datafusion_dataflow::Result<(PlanNode, TableOutput)>,
        source: LogicalPlan,
        full: Expr,
        retained: Expr,
        split: Option<&PredicateSplit>,
        aggregate: impl Fn(LogicalPlan) -> datafusion::common::Result<LogicalPlan>,
    ) -> Result<Self> {
        let (direct_node, direct) = add(
            "direct",
            aggregate(
                LogicalPlanBuilder::from(source.clone())
                    .filter(full)?
                    .build()?,
            )?,
        )?;
        let mut target = Self {
            direct,
            direct_node,
            optimized_node: None,
            optimized: None,
            materialization: None,
            prepared: None,
            templates: None,
        };
        if let Some(split) = split {
            let query = FilterQuery::new(source, |rows| {
                aggregate(
                    LogicalPlanBuilder::from(rows)
                        .filter(split.fixed().clone())?
                        .build()?,
                )
            })?;
            let prepared =
                PreaggregatePlanner::default().prepare(query, split.dimensions().to_vec())?;
            let bound = prepared.bind(split.changing().clone())?;
            if bound.predicates().retained().is_some() {
                let templates = prepared.parameterize(ParameterExpressions {
                    source: lit(true),
                    retained,
                })?;
                let plans = templates.preaggregated.as_ref().expect("eligible family");
                let (states, output) = add("states", plans.materialization.clone())?;
                let (rollup_node, rollup) = add(
                    "rollup",
                    plans.rollup.with_materialization(states.plan_ref())?,
                )?;
                target.materialization = Some(output);
                target.optimized = Some(rollup);
                target.optimized_node = Some(rollup_node);
                target.prepared = Some(prepared);
                target.templates = Some(templates);
            }
        }
        Ok(target)
    }
    pub fn bind(&self, split: Option<&PredicateSplit>) -> Result<(TableOutput, Expr)> {
        if let (Some(prepared), Some(split)) = (&self.prepared, split) {
            let bound = prepared.bind(split.changing().clone())?;
            self.templates
                .as_ref()
                .expect("paired templates")
                .check_binding(bound.predicates())?;
            if let Some(expr) = bound.predicates().retained() {
                return Ok((self.optimized.expect("eligible output"), expr.clone()));
            }
        }
        Ok((self.direct, lit(true)))
    }
}
