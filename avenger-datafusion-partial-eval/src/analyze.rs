//! Foldability analysis.

use datafusion::logical_expr::{Expr, LogicalPlan, Subquery};
use datafusion_common::tree_node::{TreeNode, TreeNodeRecursion};

use crate::PartialEvalPolicy;

const TEMPORAL_FUNCTION_NAMES: &[&str] =
    &["now", "current_date", "current_timestamp", "current_time"];

#[derive(Clone, Copy)]
enum ExprNeedle {
    Placeholder,
    Volatile,
    TemporalFunction,
}

pub(crate) fn expr_contains_placeholder(expr: &Expr) -> bool {
    expr_or_subquery_plan_matches(expr, ExprNeedle::Placeholder)
}

/// Test-support wrapper: production code composes the expr-level detectors
/// per node instead of whole-plan scans.
#[cfg(test)]
pub(crate) fn plan_contains_placeholder(plan: &LogicalPlan) -> bool {
    plan_or_subquery_plan_matches(plan, ExprNeedle::Placeholder)
}

pub(crate) fn expr_is_volatile_deep(expr: &Expr) -> bool {
    expr_or_subquery_plan_matches(expr, ExprNeedle::Volatile)
}

/// Test-support wrapper; see [`plan_contains_placeholder`].
#[cfg(test)]
pub(crate) fn plan_is_volatile_deep(plan: &LogicalPlan) -> bool {
    plan_or_subquery_plan_matches(plan, ExprNeedle::Volatile)
}

pub(crate) fn expr_contains_temporal_function(expr: &Expr) -> bool {
    expr_or_subquery_plan_matches(expr, ExprNeedle::TemporalFunction)
}

/// Test-support wrapper; see [`plan_contains_placeholder`].
#[cfg(test)]
pub(crate) fn plan_contains_temporal_function(plan: &LogicalPlan) -> bool {
    plan_or_subquery_plan_matches(plan, ExprNeedle::TemporalFunction)
}

pub(crate) fn node_local_foldable(plan: &LogicalPlan, policy: &PartialEvalPolicy) -> bool {
    if matches!(plan, LogicalPlan::Extension(_)) {
        return false;
    }

    if let LogicalPlan::TableScan(scan) = plan
        && table_is_unfoldable(&scan.table_name, policy)
    {
        return false;
    }

    !plan.expressions().iter().any(|expr| {
        expr_contains_placeholder(expr)
            || expr_is_volatile_deep(expr)
            || expr_contains_temporal_function(expr)
    })
}

fn expr_or_subquery_plan_matches(expr: &Expr, needle: ExprNeedle) -> bool {
    let mut found = false;
    let _ = expr.apply(|candidate| {
        if expr_matches(candidate, needle) || subquery_expr_matches(candidate, needle) {
            found = true;
            return Ok(TreeNodeRecursion::Stop);
        }
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

fn expr_matches(expr: &Expr, needle: ExprNeedle) -> bool {
    match needle {
        ExprNeedle::Placeholder => matches!(expr, Expr::Placeholder(_)),
        ExprNeedle::Volatile => expr.is_volatile_node(),
        ExprNeedle::TemporalFunction => {
            let Expr::ScalarFunction(function) = expr else {
                return false;
            };
            TEMPORAL_FUNCTION_NAMES
                .iter()
                .any(|name| function.name().eq_ignore_ascii_case(name))
        }
    }
}

fn subquery_expr_matches(expr: &Expr, needle: ExprNeedle) -> bool {
    match expr {
        Expr::ScalarSubquery(subquery) => subquery_matches(subquery, needle),
        Expr::Exists(exists) => subquery_matches(&exists.subquery, needle),
        Expr::InSubquery(in_subquery) => subquery_matches(&in_subquery.subquery, needle),
        Expr::SetComparison(set_comparison) => subquery_matches(&set_comparison.subquery, needle),
        _ => false,
    }
}

fn subquery_matches(subquery: &Subquery, needle: ExprNeedle) -> bool {
    plan_or_subquery_plan_matches(&subquery.subquery, needle)
        || subquery
            .outer_ref_columns
            .iter()
            .any(|expr| expr_or_subquery_plan_matches(expr, needle))
}

fn plan_or_subquery_plan_matches(plan: &LogicalPlan, needle: ExprNeedle) -> bool {
    let mut found = false;
    let _ = plan.apply(|node| {
        if node
            .expressions()
            .iter()
            .any(|expr| expr_or_subquery_plan_matches(expr, needle))
        {
            found = true;
            return Ok(TreeNodeRecursion::Stop);
        }
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

fn table_is_unfoldable(
    table_name: &datafusion_common::TableReference,
    policy: &PartialEvalPolicy,
) -> bool {
    policy.unfoldable_tables.contains(&table_name.to_string())
        || policy.unfoldable_tables.contains(table_name.table())
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::fmt;
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use arrow::array::{Float64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use datafusion::logical_expr::expr::Placeholder;
    use datafusion::logical_expr::{Extension, LogicalPlan, UserDefinedLogicalNodeCore, col, lit};
    use datafusion::prelude::SessionContext;
    use datafusion_common::{DFSchema, DFSchemaRef};

    use super::*;

    fn placeholder(id: &str) -> Expr {
        Expr::Placeholder(Placeholder::new_with_field(id.to_string(), None))
    }

    fn table_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("x", DataType::Float64, false),
                Field::new("region", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as _,
                Arc::new(StringArray::from(vec!["EU", "NA", "EU"])) as _,
            ],
        )
        .unwrap()
    }

    async fn sql_plan(sql: &str) -> LogicalPlan {
        let ctx = SessionContext::new();
        ctx.register_batch("t", table_batch()).unwrap();
        ctx.sql(sql).await.unwrap().logical_plan().clone()
    }

    #[test]
    fn placeholder_detector_finds_and_ignores_placeholders() {
        assert!(expr_contains_placeholder(&col("x").lt(placeholder("$p"))));
        assert!(!expr_contains_placeholder(&col("x").lt(lit(10.0))));
    }

    #[tokio::test]
    async fn placeholder_inside_scalar_subquery_is_detected() {
        let plan = sql_plan("SELECT * FROM t WHERE x < (SELECT $limit)").await;
        assert!(plan_contains_placeholder(&plan));
    }

    #[tokio::test]
    async fn volatile_inside_in_subquery_is_detected() {
        let plan = sql_plan("SELECT * FROM t WHERE x IN (SELECT random())").await;
        assert!(plan_is_volatile_deep(&plan));
    }

    #[tokio::test]
    async fn temporal_function_detector_finds_planned_names() {
        let now_plan = sql_plan("SELECT * FROM t WHERE now() IS NOT NULL").await;
        let current_date_plan = sql_plan("SELECT * FROM t WHERE current_date() IS NOT NULL").await;

        assert!(plan_contains_temporal_function(&now_plan));
        assert!(plan_contains_temporal_function(&current_date_plan));
        assert!(!expr_contains_temporal_function(&col("x").lt(lit(10.0))));
    }

    #[tokio::test]
    async fn now_inside_subquery_is_detected() {
        let plan = sql_plan("SELECT * FROM t WHERE x < (SELECT EXTRACT(second FROM now()))").await;
        assert!(plan_contains_temporal_function(&plan));
    }

    #[tokio::test]
    async fn node_local_foldability_handles_common_nodes_and_excluded_tables() {
        let plan = sql_plan("SELECT region, SUM(x) AS total FROM t GROUP BY region").await;
        assert!(all_nodes_local_foldable(
            &plan,
            &PartialEvalPolicy::default()
        ));

        let filtered = sql_plan("SELECT * FROM t WHERE x < $limit").await;
        let filter = first_node(&filtered, |node| matches!(node, LogicalPlan::Filter(_)))
            .expect("plan should contain filter");
        assert!(!node_local_foldable(filter, &PartialEvalPolicy::default()));

        let mut policy = PartialEvalPolicy::default();
        policy.unfoldable_tables.insert("t".to_string());
        let scan = scan_node(&plan).expect("plan should contain scan");
        assert!(!node_local_foldable(scan, &policy));
    }

    #[test]
    fn extension_node_is_unfoldable() {
        let extension = LogicalPlan::Extension(Extension {
            node: Arc::new(TestExtensionNode {
                schema: DFSchemaRef::new(DFSchema::empty()),
            }),
        });
        assert!(!node_local_foldable(
            &extension,
            &PartialEvalPolicy::default()
        ));
    }

    fn all_nodes_local_foldable(plan: &LogicalPlan, policy: &PartialEvalPolicy) -> bool {
        let mut all_foldable = true;
        let _ = plan.apply(|node| {
            all_foldable &= node_local_foldable(node, policy);
            Ok(TreeNodeRecursion::Continue)
        });
        all_foldable
    }

    fn scan_node(plan: &LogicalPlan) -> Option<&LogicalPlan> {
        first_node(plan, |node| matches!(node, LogicalPlan::TableScan(_)))
    }

    fn first_node(
        plan: &LogicalPlan,
        mut predicate: impl FnMut(&LogicalPlan) -> bool,
    ) -> Option<&LogicalPlan> {
        let mut found = None;
        let _ = plan.apply(|node| {
            if predicate(node) {
                found = Some(node);
                return Ok(TreeNodeRecursion::Stop);
            }
            Ok(TreeNodeRecursion::Continue)
        });
        found
    }

    #[derive(Clone, Debug)]
    struct TestExtensionNode {
        schema: DFSchemaRef,
    }

    impl PartialEq for TestExtensionNode {
        fn eq(&self, _other: &Self) -> bool {
            true
        }
    }

    impl Eq for TestExtensionNode {}

    impl PartialOrd for TestExtensionNode {
        fn partial_cmp(&self, _other: &Self) -> Option<Ordering> {
            Some(Ordering::Equal)
        }
    }

    impl Hash for TestExtensionNode {
        fn hash<H: Hasher>(&self, state: &mut H) {
            "TestExtensionNode".hash(state);
        }
    }

    impl UserDefinedLogicalNodeCore for TestExtensionNode {
        fn name(&self) -> &str {
            "test_extension"
        }

        fn inputs(&self) -> Vec<&LogicalPlan> {
            Vec::new()
        }

        fn schema(&self) -> &DFSchemaRef {
            &self.schema
        }

        fn expressions(&self) -> Vec<Expr> {
            Vec::new()
        }

        fn fmt_for_explain(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "TestExtensionNode")
        }

        fn with_exprs_and_inputs(
            &self,
            _exprs: Vec<Expr>,
            _inputs: Vec<LogicalPlan>,
        ) -> datafusion_common::Result<Self> {
            Ok(self.clone())
        }
    }
}
