pub(crate) mod predicates;

use crate::{query::selection_site, DirectReason, Result};
#[cfg(feature = "dataflow")]
use avenger_datafusion_preaggregate::runtime::{ParameterExpressions, ParameterizedFamily};
use avenger_datafusion_preaggregate::{
    BoundQuery, ExpressionProperties, FilterQuery, PreaggregatePlanner, PreparedQuery, RollupQuery,
    ScalarFunctionProperties,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{Transformed, TreeNode},
        DFSchema, DFSchemaRef,
    },
    logical_expr::{
        create_udf, expr::ScalarFunction, Expr, LogicalPlan, LogicalPlanBuilder, ScalarUDF,
        Volatility,
    },
};
use std::sync::{Arc, LazyLock};

pub(crate) type Eligibility<T> = std::result::Result<T, DirectReason>;

#[derive(Clone, Debug)]
pub(crate) struct Preaggregation {
    pub prepared: PreparedQuery,
    schema: DFSchemaRef,
    pub interaction_keys: Vec<Expr>,
}
impl Preaggregation {
    pub fn analyze(plan: &LogicalPlan, site: u64, keys: Vec<Expr>) -> Result<Eligibility<Self>> {
        let mut source = None;
        plan.apply_with_subqueries(|p| {
            if let Some(s) = selection_site(p).filter(|s| s.id == site) {
                source = Some(s.input.clone());
            }
            Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
        })?;
        let Some(source) = source else {
            return Ok(Err(DirectReason::UnsupportedQueryShape));
        };
        let schema = source.schema().clone();
        let builder = FilterQuery::builder(source);
        // Selection's site owns the complete predicate for direct fallback. The
        // generic marker represents only the changing part after factorization.
        // The fixed wrapper carries a contract checked on every selection binding.
        // It becomes an ordinary filter dependency when installed in a dataflow.
        let rows = LogicalPlanBuilder::from(builder.rows())
            .filter(fixed_udf().call(vec![]))?
            .build()?;
        let plan = plan
            .clone()
            .transform_up_with_subqueries(|p| {
                if selection_site(&p).is_some_and(|s| s.id == site) {
                    Ok(Transformed::yes(rows.clone()))
                } else {
                    Ok(Transformed::no(p))
                }
            })?
            .data;
        let query = builder.finish(plan)?;
        let prepared = planner().prepare(query, keys.clone())?;
        if let Some(reason) = prepared.explain().direct_reason {
            return Ok(Err(reason.into()));
        }
        Ok(Ok(Self {
            prepared,
            schema,
            interaction_keys: keys,
        }))
    }
    pub fn materialization(&self, fixed: Expr) -> Result<LogicalPlan> {
        with_fixed(
            self.prepared
                .materialization_plan()
                .expect("eligible preparation"),
            fixed,
        )
    }
    pub fn aggregate(&self, changing: Expr) -> Result<AggregateStep> {
        let BoundQuery::Preaggregated { rollup, .. } = self.prepared.bind(changing)? else {
            return Err(crate::Error::InvalidQuery(
                "preaggregation requires an eligible predicate binding".into(),
            ));
        };
        Ok(AggregateStep(rollup))
    }
    pub fn validate(&self, split: &predicates::PredicateSplit) -> Result<Eligibility<()>> {
        if !planner()
            .expression_properties(split.fixed.clone(), &self.schema)?
            .total
        {
            return Ok(Err(DirectReason::UnsafeMovedExpression));
        }
        let binding = self.prepared.bind(split.changing.clone())?;
        Ok(match binding.diagnostics().direct_reason {
            Some(reason) => Err(reason.into()),
            None => Ok(()),
        })
    }
    #[cfg(feature = "dataflow")]
    pub fn parameterize(&self, parameters: ParameterExpressions) -> Result<ParameterizedFamily> {
        Ok(self.prepared.parameterize(parameters)?)
    }
}

/// The final filter, aggregate-state merge, and original query suffix.
#[derive(Clone, Debug)]
pub struct AggregateStep(pub(crate) RollupQuery);
impl AggregateStep {
    /// Return the expected materialization schema, including retained keys and state.
    pub fn schema(&self) -> &DFSchemaRef {
        self.0.materialization_schema()
    }
    /// Substitute a relation with matching Arrow fields and metadata. Qualifiers
    /// may differ. No data is read and no functions are evaluated.
    pub fn over(&self, materialized: LogicalPlan) -> Result<LogicalPlan> {
        Ok(self.0.with_materialization(materialized)?)
    }
}

#[derive(Debug)]
struct SelectionProperties;
impl ExpressionProperties for SelectionProperties {
    fn scalar_function(&self, function: &ScalarFunction, _: &DFSchema) -> ScalarFunctionProperties {
        let supported = function.func.inner().is::<crate::pixels::PixelCell>()
            || function.func.inner().is::<crate::predicate::NumberClass>()
            || function.func == *fixed_udf();
        ScalarFunctionProperties {
            total: supported,
            respects_grouping_equality: supported,
        }
    }
}
fn planner() -> PreaggregatePlanner {
    PreaggregatePlanner::default().with_expression_properties(Arc::new(SelectionProperties))
}
fn fixed_udf() -> &'static Arc<ScalarUDF> {
    static FIXED: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| {
        Arc::new(create_udf(
            "avenger_selection_fixed",
            vec![],
            DataType::Boolean,
            Volatility::Immutable,
            Arc::new(|_| {
                datafusion::common::internal_err!(
                    "selection fixed predicate must be bound before execution"
                )
            }),
        ))
    });
    &FIXED
}
pub(crate) fn with_fixed(plan: &LogicalPlan, fixed: Expr) -> Result<LogicalPlan> {
    Ok(plan
        .clone()
        .transform_up_with_subqueries(|plan| {
            plan.map_expressions(|expr| {
                expr.transform_up(|e| {
                    if matches!(&e, Expr::ScalarFunction(f) if f.func == *fixed_udf()) {
                        Ok(Transformed::yes(fixed.clone()))
                    } else {
                        Ok(Transformed::no(e))
                    }
                })
            })
        })?
        .data)
}
impl From<avenger_datafusion_preaggregate::DirectReason> for DirectReason {
    fn from(reason: avenger_datafusion_preaggregate::DirectReason) -> Self {
        use avenger_datafusion_preaggregate::DirectReason as R;
        match reason {
            R::Forced => Self::Forced,
            R::UnsupportedAggregate => Self::UnsupportedAggregate,
            R::UnsupportedGroupingExpression => Self::UnsupportedGroupingExpression,
            R::NonImmutableQuery => Self::NonImmutableQuery,
            R::UnsafeMovedExpression => Self::UnsafeMovedExpression,
            R::PredicateNeedsUnretainedExpression | R::UnsupportedPredicate => {
                Self::UnsupportedInteraction
            }
            _ => Self::UnsupportedQueryShape,
        }
    }
}
