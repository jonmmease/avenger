//! Fixed-parameter binding and held-predicate markers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use arrow::datatypes::DataType;
use datafusion::logical_expr::expr::Placeholder;
use datafusion::logical_expr::utils::{conjunction, split_conjunction_owned};
use datafusion::logical_expr::{
    ColumnarValue, Expr, LogicalPlan, ScalarUDF, Volatility, create_udf, lit,
};
use datafusion_common::tree_node::{Transformed, TreeNode, TreeNodeRecursion};
use datafusion_common::{Result, ScalarValue, internal_err, plan_datafusion_err};

use crate::analyze::{expr_contains_placeholder, expr_contains_temporal_function};

pub(crate) const PE_HOLD_UDF_NAME: &str = "__pe_hold";

pub(crate) fn bind_fixed_params(
    plan: LogicalPlan,
    fixed_params: &[(String, ScalarValue)],
) -> Result<(LogicalPlan, BTreeSet<String>)> {
    let fixed = fixed_params
        .iter()
        .map(|(name, value)| (normalize_param_name(name), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut applied = BTreeSet::new();
    let rewritten = plan.transform_up_with_subqueries(|node| {
        node.map_expressions(|expr| bind_expr(expr, &fixed, &mut applied))
    })?;
    Ok((rewritten.data, applied))
}

pub(crate) fn collect_placeholder_ids(plan: &LogicalPlan) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let _ = plan.apply_with_subqueries(|node| {
        for expr in node.expressions() {
            let _ = expr.apply(|candidate| {
                if let Expr::Placeholder(placeholder) = candidate {
                    ids.insert(placeholder.id.clone());
                }
                Ok(TreeNodeRecursion::Continue)
            });
        }
        Ok(TreeNodeRecursion::Continue)
    });
    ids
}

pub(crate) fn pe_hold_udf() -> Arc<ScalarUDF> {
    Arc::new(create_udf(
        PE_HOLD_UDF_NAME,
        vec![DataType::Boolean],
        DataType::Boolean,
        Volatility::Volatile,
        Arc::new(|args: &[ColumnarValue]| {
            let Some(value) = args.first() else {
                return internal_err!("{PE_HOLD_UDF_NAME} expects one argument");
            };
            Ok(value.clone())
        }),
    ))
}

pub(crate) fn wrap_held_predicates(plan: LogicalPlan) -> Result<LogicalPlan> {
    Ok(plan
        .transform_up_with_subqueries(|node| match node {
            LogicalPlan::Filter(mut filter) => {
                filter.predicate = wrap_predicate(filter.predicate)?;
                Ok(Transformed::yes(LogicalPlan::Filter(filter)))
            }
            LogicalPlan::Join(mut join) => {
                if let Some(filter) = join.filter {
                    join.filter = Some(wrap_predicate(filter)?);
                }
                Ok(Transformed::yes(LogicalPlan::Join(join)))
            }
            other => Ok(Transformed::no(other)),
        })?
        .data)
}

pub(crate) fn unwrap_held_predicates(plan: LogicalPlan) -> Result<LogicalPlan> {
    let unwrapped = plan.transform_up_with_subqueries(|node| {
        node.map_expressions(|expr| {
            expr.transform_up(|candidate| {
                if let Some(arg) = pe_hold_arg(&candidate) {
                    return Ok(Transformed::yes(arg));
                }
                Ok(Transformed::no(candidate))
            })
        })
    })?;
    Ok(unwrapped.data)
}

pub(crate) fn assert_no_markers(plan: &LogicalPlan) {
    debug_assert!(
        !plan_contains_hold_marker(plan),
        "{PE_HOLD_UDF_NAME} marker leaked into residual plan"
    );
}

fn bind_expr(
    expr: Expr,
    fixed: &BTreeMap<String, ScalarValue>,
    applied: &mut BTreeSet<String>,
) -> Result<Transformed<Expr>> {
    expr.transform_up(|candidate| {
        let Expr::Placeholder(placeholder) = candidate else {
            return Ok(Transformed::no(candidate));
        };
        let normalized = normalize_param_name(&placeholder.id);
        let Some(value) = fixed.get(&normalized) else {
            return Ok(Transformed::no(Expr::Placeholder(placeholder)));
        };
        applied.insert(normalized);
        Ok(Transformed::yes(lit(value_for_placeholder(
            &placeholder,
            value,
        )?)))
    })
}

fn value_for_placeholder(placeholder: &Placeholder, value: &ScalarValue) -> Result<ScalarValue> {
    let Some(field) = &placeholder.field else {
        return Ok(value.clone());
    };
    value.cast_to(field.data_type()).map_err(|err| {
        plan_datafusion_err!(
            "Fixed param '{}' with value type {:?} cannot be cast to inferred placeholder type {:?}: {err}",
            placeholder.id,
            value.data_type(),
            field.data_type()
        )
    })
}

fn wrap_predicate(predicate: Expr) -> Result<Expr> {
    let conjuncts = split_conjunction_owned(predicate)
        .into_iter()
        .map(|conjunct| {
            if expr_contains_placeholder(&conjunct) || expr_contains_temporal_function(&conjunct) {
                pe_hold_udf().call(vec![conjunct])
            } else {
                conjunct
            }
        })
        .collect::<Vec<_>>();
    conjunction(conjuncts)
        .ok_or_else(|| plan_datafusion_err!("Cannot wrap an empty predicate conjunction"))
}

fn pe_hold_arg(expr: &Expr) -> Option<Expr> {
    let Expr::ScalarFunction(function) = expr else {
        return None;
    };
    if !function.name().eq_ignore_ascii_case(PE_HOLD_UDF_NAME) {
        return None;
    }
    match function.args.as_slice() {
        [arg] => Some(arg.clone()),
        _ => None,
    }
}

fn plan_contains_hold_marker(plan: &LogicalPlan) -> bool {
    let mut found = false;
    let _ = plan.apply_with_subqueries(|node| {
        for expr in node.expressions() {
            let _ = expr.apply(|candidate| {
                if pe_hold_arg(candidate).is_some() {
                    found = true;
                    return Ok(TreeNodeRecursion::Stop);
                }
                Ok(TreeNodeRecursion::Continue)
            });
        }
        if found {
            Ok(TreeNodeRecursion::Stop)
        } else {
            Ok(TreeNodeRecursion::Continue)
        }
    });
    found
}

fn normalize_param_name(name: &str) -> String {
    name.trim_start_matches('$').to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Float64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use datafusion::logical_expr::{Expr, Join, LogicalPlan};
    use datafusion::prelude::SessionContext;

    use super::*;

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

    #[tokio::test]
    async fn bind_fixed_params_rewrites_subset() {
        let plan = sql_plan("SELECT * FROM t WHERE x < $max AND region = $region").await;
        let (bound, applied) = bind_fixed_params(
            plan,
            &[
                ("max".to_string(), ScalarValue::Float64(Some(2.5))),
                (
                    "unused".to_string(),
                    ScalarValue::Utf8(Some("x".to_string())),
                ),
            ],
        )
        .unwrap();

        assert_eq!(applied, BTreeSet::from(["max".to_string()]));
        let remaining = collect_placeholder_ids(&bound);
        assert_eq!(remaining, BTreeSet::from(["$region".to_string()]));
    }

    #[tokio::test]
    async fn bind_fixed_params_normalizes_dollar_prefix() {
        let plan = sql_plan("SELECT * FROM t WHERE region = $region").await;
        let (bound, applied) = bind_fixed_params(
            plan,
            &[(
                "$region".to_string(),
                ScalarValue::Utf8(Some("EU".to_string())),
            )],
        )
        .unwrap();

        assert_eq!(applied, BTreeSet::from(["region".to_string()]));
        assert!(collect_placeholder_ids(&bound).is_empty());
    }

    #[tokio::test]
    async fn bind_fixed_params_reports_type_mismatch() {
        let plan = sql_plan("SELECT * FROM t WHERE x < $max").await;
        let err = bind_fixed_params(
            plan,
            &[(
                "max".to_string(),
                ScalarValue::Utf8(Some("not a float".to_string())),
            )],
        )
        .unwrap_err();
        assert!(err.to_string().contains("Fixed param '$max'"));
    }

    #[tokio::test]
    async fn wrap_splits_and_holds_only_param_conjuncts() {
        let plan = sql_plan("SELECT * FROM t WHERE x < $max AND region = 'EU'").await;
        let wrapped = wrap_held_predicates(plan.clone()).unwrap();
        let predicate = first_filter_predicate(&wrapped).expect("filter predicate");
        let conjuncts = split_conjunction_owned(predicate.clone());

        assert_eq!(
            conjuncts
                .iter()
                .filter(|expr| pe_hold_arg(expr).is_some())
                .count(),
            1
        );
        assert!(
            conjuncts
                .iter()
                .any(|expr| expr.to_string().contains("region"))
        );

        let unwrapped = unwrap_held_predicates(wrapped).unwrap();
        assert_eq!(unwrapped, plan);
        assert_no_markers(&unwrapped);
    }

    #[tokio::test]
    async fn wrap_holds_temporal_conjuncts() {
        let plan = sql_plan("SELECT * FROM t WHERE now() IS NOT NULL AND region = 'EU'").await;
        let wrapped = wrap_held_predicates(plan).unwrap();
        let predicate = first_filter_predicate(&wrapped).expect("filter predicate");
        assert!(
            split_conjunction_owned(predicate)
                .iter()
                .any(|expr| pe_hold_arg(expr).is_some())
        );
    }

    #[tokio::test]
    async fn join_filter_wrapping() {
        let plan =
            sql_plan("SELECT * FROM t a JOIN t b ON a.region = b.region AND a.x < $max").await;
        let wrapped = wrap_held_predicates(plan.clone()).unwrap();
        let join = first_join(&wrapped).expect("join");
        let filter = join.filter.as_ref().expect("join filter");
        assert!(
            split_conjunction_owned(filter.clone())
                .iter()
                .any(|expr| pe_hold_arg(expr).is_some())
        );

        let unwrapped = unwrap_held_predicates(wrapped).unwrap();
        assert_eq!(unwrapped, plan);
    }

    fn first_filter_predicate(plan: &LogicalPlan) -> Option<Expr> {
        let mut predicate = None;
        let _ = plan.apply_with_subqueries(|node| {
            if let LogicalPlan::Filter(filter) = node {
                predicate = Some(filter.predicate.clone());
                return Ok(TreeNodeRecursion::Stop);
            }
            Ok(TreeNodeRecursion::Continue)
        });
        predicate
    }

    fn first_join(plan: &LogicalPlan) -> Option<Join> {
        let mut join = None;
        let _ = plan.apply_with_subqueries(|node| {
            if let LogicalPlan::Join(candidate) = node {
                join = Some(candidate.clone());
                return Ok(TreeNodeRecursion::Stop);
            }
            Ok(TreeNodeRecursion::Continue)
        });
        join
    }
}
