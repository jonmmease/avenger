//! Private chart-side composition shared by the study example and boundary tests.
use avenger_datafusion_dataflow::DataflowBuilder;
use avenger_datafusion_preaggregate::{
    dataflow::{Binding, Query},
    FilterQuery, PreaggregatePlanner, PreparedQuery,
};
use avenger_selection::PredicateSplit;
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{TreeNode, TreeNodeRecursion},
        Result as DFResult,
    },
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder},
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
    pub query: Query,
    fixed: Expr,
    dimensions: Vec<Expr>,
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
        Ok(Some(Self {
            query: Query::install(builder, name, prepared)?,
            fixed: split.fixed().clone(),
            dimensions: split.dimensions().to_vec(),
        }))
    }

    /// None selects the base direct output. Fixed changes require a new preparation.
    pub fn bind(&self, split: &PredicateSplit) -> DFResult<Option<Binding>> {
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
        let bound = self.query.bind(split.changing().clone())?;
        Ok(bound.materialization_output().map(|_| bound))
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
