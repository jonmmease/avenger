//! Private chart-side composition shared by the study example and boundary tests.
use avenger_datafusion_dataflow::{DataflowBuilder, ExprInput, TableOutput};
use avenger_datafusion_preaggregate::{
    runtime::{ParameterExpressions, ParameterizedFamily},
    FilterQuery, PreaggregatePlanner, PreparedQuery,
};
use avenger_selection::PredicateSplit;
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{TreeNode, TreeNodeRecursion},
        Result as DFResult,
    },
    logical_expr::{lit, Expr, LogicalPlan, LogicalPlanBuilder},
};

pub type ExampleResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn prepare(
    source: LogicalPlan,
    split: &PredicateSplit,
    target: impl FnOnce(LogicalPlan) -> DFResult<LogicalPlan>,
) -> DFResult<PreparedQuery> {
    let query = FilterQuery::new(source, |rows| {
        target(
            LogicalPlanBuilder::from(rows)
                .filter(split.fixed().clone())?
                .build()?,
        )
    })?;
    PreaggregatePlanner::default().prepare(query, split.dimensions().to_vec())
}

/// Graph handles for one target and concrete fixed predicate. The base owns fallback.
pub struct OptimizedQuery {
    pub prepared: PreparedQuery,
    pub predicate: ExprInput,
    pub materialization: TableOutput,
    pub output: TableOutput,
    fixed: Expr,
    dimensions: Vec<Expr>,
    templates: ParameterizedFamily,
}
impl OptimizedQuery {
    pub fn install(
        builder: &mut DataflowBuilder,
        name: &str,
        prepared: PreparedQuery,
        split: &PredicateSplit,
    ) -> ExampleResult<Option<Self>> {
        if prepared
            .bind(split.changing().clone())?
            .predicates()
            .retained()
            .is_none()
        {
            return Ok(None);
        }
        let predicate = builder.expr_input(format!("{name}_cells"), DataType::Boolean)?;
        let templates = prepared.parameterize(ParameterExpressions {
            // Only the rollup is installed. The base query binds complete membership.
            source: lit(true),
            retained: predicate.expr_ref(),
        })?;
        let plans = templates
            .preaggregated
            .as_ref()
            .expect("eligible preparation");
        let states = builder.add_plan(format!("{name}_states"), plans.materialization.clone())?;
        let materialization = builder.table_output(format!("{name}_states"), &states)?;
        let rollup = builder.add_plan(
            format!("{name}_rollup"),
            plans.rollup.with_materialization(states.plan_ref())?,
        )?;
        let output = builder.table_output(name, &rollup)?;
        Ok(Some(Self {
            prepared,
            predicate,
            materialization,
            output,
            fixed: split.fixed().clone(),
            dimensions: split.dimensions().to_vec(),
            templates,
        }))
    }

    /// None selects the base direct output. Fixed changes require a new preparation.
    pub fn bind(&self, split: &PredicateSplit) -> DFResult<Option<Expr>> {
        if !same_expr(&self.fixed, split.fixed())
            || self.dimensions.len() != split.dimensions().len()
            || !self
                .dimensions
                .iter()
                .zip(split.dimensions())
                .all(|(a, b)| same_expr(a, b))
        {
            return Ok(None);
        }
        let bound = self.prepared.bind(split.changing().clone())?;
        self.templates.check_binding(bound.predicates())?;
        Ok(bound.predicates().retained().cloned())
    }
}

fn same_expr(a: &Expr, b: &Expr) -> bool {
    fn types(expr: &Expr) -> Vec<DataType> {
        let mut types = Vec::new();
        expr.apply(|e| {
            if let Expr::Literal(value, _) = e {
                types.push(value.data_type());
            }
            Ok(TreeNodeRecursion::Continue)
        })
        .expect("infallible literal traversal");
        types
    }
    // DataFusion scalar equality alone ignores timestamp-zone metadata.
    a == b && types(a) == types(b)
}
