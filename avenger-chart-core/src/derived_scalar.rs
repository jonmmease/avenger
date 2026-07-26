use std::{collections::HashMap, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, Field},
    logical_expr::{
        Expr,
        expr::{Placeholder, ScalarFunction},
    },
};
use datafusion_common::tree_node::{Transformed, TreeNode};
use indexmap::IndexMap;

use crate::AvengerChartError;

const DERIVED_SCALAR_PLACEHOLDER_PREFIX: &str = "$__derived_scalar_";

/// Runtime expressions for scalar values derived from the current chart data scope.
pub type DerivedScalarMap = IndexMap<String, Expr>;

/// Runtime-derived scalar expressions grouped by owning channel.
pub type DerivedScalarsByChannel = HashMap<String, DerivedScalarMap>;

/// Reference a runtime-derived scalar expression.
///
/// The placeholder is resolved by chart runtime code before DataFusion evaluates
/// the surrounding expression.
pub fn derived_scalar(id: impl AsRef<str>, data_type: Option<DataType>) -> Expr {
    Expr::Placeholder(Placeholder::new_with_field(
        derived_scalar_placeholder_id(id.as_ref()),
        data_type.map(|data_type| Arc::new(Field::new("", data_type, true))),
    ))
}

fn derived_scalar_placeholder_id(id: &str) -> String {
    format!("{DERIVED_SCALAR_PLACEHOLDER_PREFIX}{id}")
}

/// Extract a derived scalar id from a DataFusion placeholder id.
pub fn derived_scalar_id_from_placeholder(placeholder_id: &str) -> Option<&str> {
    placeholder_id.strip_prefix(DERIVED_SCALAR_PLACEHOLDER_PREFIX)
}

/// Resolve all derived-scalar placeholders in an expression.
pub fn resolve_derived_scalars(
    expr: Expr,
    derived_scalars: &DerivedScalarMap,
) -> Result<Expr, AvengerChartError> {
    expr.transform(|candidate| {
        if let Expr::Placeholder(placeholder) = &candidate
            && let Some(id) = derived_scalar_id_from_placeholder(&placeholder.id)
        {
            let Some(replacement) = derived_scalars.get(id) else {
                return Err(datafusion::error::DataFusionError::Plan(format!(
                    "Derived scalar '{id}' was referenced but not produced in this data scope"
                )));
            };
            return Ok(Transformed::yes(replacement.clone()));
        }

        Ok(Transformed::no(candidate))
    })
    .map(|transformed| transformed.data)
    .map_err(AvengerChartError::DataFusionError)
}

/// Resolve derived-scalar placeholders whose ids are present in
/// `derived_scalars`, leaving unknown derived-scalar placeholders untouched.
///
/// Transform-chain executors use this to substitute scalars produced by
/// earlier stages into later stage expressions. An id missing from the map is
/// not an error here: it may be produced by a later stage and consumed by
/// channels, where the strict [`resolve_derived_scalars`] runs during channel
/// finalization and reports genuinely unproduced references.
pub fn resolve_known_derived_scalars(
    expr: Expr,
    derived_scalars: &DerivedScalarMap,
) -> Result<Expr, AvengerChartError> {
    if derived_scalars.is_empty() {
        return Ok(expr);
    }
    expr.transform(|candidate| {
        if let Expr::Placeholder(placeholder) = &candidate
            && let Some(id) = derived_scalar_id_from_placeholder(&placeholder.id)
            && let Some(replacement) = derived_scalars.get(id)
        {
            return Ok(Transformed::yes(replacement.clone()));
        }

        Ok(Transformed::no(candidate))
    })
    .map(|transformed| transformed.data)
    .map_err(AvengerChartError::DataFusionError)
}

/// Resolve known derived-scalar placeholders inside a channel value's data
/// expressions (`Scaled`/`Value` expressions and `Conditional`
/// conditions/branches). Scale, axis, and legend configuration expressions
/// are left untouched: those resolve later through the scale builder's own
/// derived-scalar handling.
///
/// Channel expressions are stored in protobuf form, so a resolved expression
/// containing a scalar subquery fails to re-encode on DataFusion versions
/// without subquery serialization support; callers surface that error with
/// actionable guidance.
pub fn resolve_known_derived_scalars_in_channel_value(
    value: crate::ChannelValue,
    derived_scalars: &DerivedScalarMap,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<crate::ChannelValue, AvengerChartError> {
    use crate::{ChannelValue, ConditionalValue, DefaultLogicalExprNodeExt};
    use datafusion_proto::protobuf::LogicalExprNode;

    if derived_scalars.is_empty() {
        return Ok(value);
    }

    let resolve_node = |node: LogicalExprNode| -> Result<LogicalExprNode, AvengerChartError> {
        let expr = node.to_expr(ctx)?;
        if collect_derived_scalar_ids(&expr)?.is_empty() {
            return Ok(node);
        }
        let resolved = resolve_known_derived_scalars(expr, derived_scalars)?;
        LogicalExprNode::from_default_expr(resolved)
    };
    let resolve_conditional =
        |value: ConditionalValue| -> Result<ConditionalValue, AvengerChartError> {
            Ok(match value {
                ConditionalValue::Scaled { expr } => ConditionalValue::Scaled {
                    expr: resolve_node(expr)?,
                },
                ConditionalValue::Value { expr } => ConditionalValue::Value {
                    expr: resolve_node(expr)?,
                },
            })
        };

    Ok(match value {
        ChannelValue::Scaled {
            expr,
            scale_name,
            position_boundary,
            scale_config,
            nested_band_config,
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
            scale_domain_inference,
        } => ChannelValue::Scaled {
            expr: resolve_node(expr)?,
            scale_name,
            position_boundary,
            scale_config,
            nested_band_config,
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
            scale_domain_inference,
        },
        ChannelValue::Value { expr } => ChannelValue::Value {
            expr: resolve_node(expr)?,
        },
        ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config,
            nested_band_config,
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
            scale_domain_inference,
        } => ChannelValue::Conditional {
            conditions: conditions
                .into_iter()
                .map(|(condition, branch)| {
                    Ok((resolve_node(condition)?, resolve_conditional(branch)?))
                })
                .collect::<Result<Vec<_>, AvengerChartError>>()?,
            otherwise: resolve_conditional(otherwise)?,
            scale_config,
            nested_band_config,
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
            scale_domain_inference,
        },
    })
}

/// Collect derived scalar ids referenced by an expression.
pub fn collect_derived_scalar_ids(expr: &Expr) -> Result<Vec<String>, AvengerChartError> {
    let mut ids = IndexMap::<String, ()>::new();
    expr.apply(|candidate| {
        if let Expr::Placeholder(placeholder) = candidate
            && let Some(id) = derived_scalar_id_from_placeholder(&placeholder.id)
        {
            ids.insert(id.to_string(), ());
        }
        Ok(datafusion_common::tree_node::TreeNodeRecursion::Continue)
    })
    .map_err(AvengerChartError::DataFusionError)?;
    Ok(ids.keys().cloned().collect())
}

/// Collect derived scalar ids referenced by an expression tree that may contain
/// scalar function arguments.
pub fn collect_derived_scalar_ids_from_scalar_function(
    function: &ScalarFunction,
) -> Result<Vec<String>, AvengerChartError> {
    let mut ids = IndexMap::<String, ()>::new();
    for arg in &function.args {
        for id in collect_derived_scalar_ids(arg)? {
            ids.insert(id, ());
        }
    }
    Ok(ids.keys().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        arrow::datatypes::DataType,
        logical_expr::{expr_fn::scalar_subquery, lit},
        prelude::{SessionContext, col, named_struct},
    };
    use datafusion_proto::protobuf::LogicalExprNode;
    use std::sync::Arc;

    use crate::DefaultLogicalExprNodeExt;

    #[test]
    fn derived_scalar_placeholder_serializes_and_is_recognized() {
        let expr = derived_scalar("tick_spacing", Some(DataType::Float64));
        LogicalExprNode::from_default_expr(expr.clone())
            .expect("derived scalar placeholder serializes");
        let Expr::Placeholder(placeholder) = expr else {
            panic!("expected placeholder expression");
        };
        assert_eq!(
            derived_scalar_id_from_placeholder(&placeholder.id),
            Some("tick_spacing")
        );
        assert_eq!(
            placeholder.field.as_ref().map(|field| field.data_type()),
            Some(&DataType::Float64)
        );
    }

    #[test]
    fn resolve_derived_scalars_replaces_placeholders() {
        let mut derived = DerivedScalarMap::new();
        derived.insert("offset".to_string(), lit(2.0));

        let resolved = resolve_derived_scalars(
            col("value") + derived_scalar("offset", Some(DataType::Float64)),
            &derived,
        )
        .expect("resolve derived scalar");

        assert_eq!(resolved.to_string(), "value + Float64(2)");
    }

    #[test]
    fn resolve_derived_scalars_errors_on_missing_placeholder() {
        let err = resolve_derived_scalars(
            derived_scalar("missing", Some(DataType::Float64)),
            &DerivedScalarMap::new(),
        )
        .expect_err("missing derived scalar should error");
        assert!(
            err.to_string().contains("Derived scalar 'missing'"),
            "{err}"
        );
    }

    #[test]
    fn resolve_derived_scalars_replaces_named_struct_expression() {
        let mut derived = DerivedScalarMap::new();
        derived.insert(
            "tick_spacing".to_string(),
            named_struct(vec![lit("start"), lit(0.0), lit("step"), lit(2.5)]),
        );

        let expr = resolve_derived_scalars(derived_scalar("tick_spacing", None), &derived)
            .expect("resolve named struct expression");

        let expr_text = expr.to_string();
        assert!(expr_text.starts_with("named_struct("), "{expr_text}");
        assert!(expr_text.contains("start"), "{expr_text}");
        assert!(expr_text.contains("step"), "{expr_text}");
    }

    #[tokio::test]
    async fn resolve_derived_scalars_replaces_scalar_subquery_expression() {
        let ctx = SessionContext::new();
        let subquery = ctx
            .sql("select 2.5 as value")
            .await
            .expect("build subquery")
            .into_unoptimized_plan();
        let mut derived = DerivedScalarMap::new();
        derived.insert(
            "subquery_value".to_string(),
            scalar_subquery(Arc::new(subquery)),
        );

        let expr = resolve_derived_scalars(
            derived_scalar("subquery_value", Some(DataType::Float64)),
            &derived,
        )
        .expect("resolve scalar subquery");

        assert!(matches!(expr, Expr::ScalarSubquery(_)));
    }
}
