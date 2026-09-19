use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, DFSchema, Result, ScalarValue,
    },
    logical_expr::{expr::ScalarFunction, Expr, ExprSchemable, Operator, Volatility},
    optimizer::analyzer::type_coercion::TypeCoercionRewriter,
};
use std::{fmt::Debug, sync::Arc};

/// Trusted properties of configured scalar functions. Child expressions and
/// volatility are checked independently. Unknown functions should return defaults.
pub trait ExpressionProperties: Debug + Send + Sync {
    /// Describe this resolved invocation without evaluating it.
    fn scalar_function(
        &self,
        function: &ScalarFunction,
        input_schema: &DFSchema,
    ) -> ScalarFunctionProperties;
}

/// Scalar properties needed when evaluation moves before a changing filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScalarFunctionProperties {
    /// Evaluation cannot fail for any value admitted by the input types.
    pub total: bool,
    /// Grouping-equal arguments produce grouping-equal results, including NaNs and signed zero.
    pub respects_grouping_equality: bool,
}
#[derive(Debug, Default)]
pub(crate) struct ConservativeProperties;
impl ExpressionProperties for ConservativeProperties {
    fn scalar_function(&self, _: &ScalarFunction, _: &DFSchema) -> ScalarFunctionProperties {
        ScalarFunctionProperties::default()
    }
}

pub(crate) fn coerce(expr: Expr, schema: &DFSchema) -> Result<Expr> {
    // Some Boolean expressions have a known return type without inspecting
    // their children. Resolve columns explicitly, including ambiguity checks.
    expr.apply(|e| {
        if let Expr::Column(c) = e {
            schema.qualified_field_from_column(c)?;
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(expr.rewrite(&mut TypeCoercionRewriter::new(schema))?.data)
}
pub(crate) fn boolean(expr: Expr, schema: &DFSchema, concrete: bool) -> Result<Expr> {
    let expr = coerce(expr, schema)?;
    if concrete {
        expr.apply(|e| {
            if matches!(
                e,
                Expr::Placeholder(_) | Expr::OuterReferenceColumn(..) | Expr::ScalarVariable(..)
            ) {
                return datafusion::common::plan_err!(
                    "a concrete predicate cannot contain unresolved parameters or outer references"
                );
            }
            let subquery = match e {
                Expr::ScalarSubquery(s) => Some(&s.subquery),
                Expr::InSubquery(s) => Some(&s.subquery.subquery),
                Expr::Exists(s) => Some(&s.subquery.subquery),
                _ => None,
            };
            if let Some(plan) = subquery {
                if !plan.get_parameter_names()?.is_empty() {
                    return datafusion::common::plan_err!(
                        "a concrete predicate cannot contain unresolved subquery parameters"
                    );
                }
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
    }
    if expr.get_type(schema)? != DataType::Boolean {
        return datafusion::common::plan_err!("changing predicate must have Boolean type");
    }
    Ok(expr)
}

// Ordinals make alias lineage explicit without relying on displayed SQL names.
pub(crate) fn canonical(expr: Expr, schema: &DFSchema) -> Result<Expr> {
    Ok(expr
        .transform_up(|e| {
            Ok(match e {
                Expr::Alias(a) => Transformed::yes(*a.expr),
                Expr::Column(c) => Transformed::yes(Expr::Column(Column::from_name(format!(
                    "__preagg_source_{}",
                    column_index(schema, &c)?
                )))),
                e => Transformed::no(e),
            })
        })?
        .data)
}
pub(crate) fn requalify(expr: Expr, from: &DFSchema, to: &DFSchema) -> Result<Expr> {
    Ok(expr
        .transform_up(|e| {
            Ok(match e {
                Expr::Column(c) => {
                    Transformed::yes(Expr::Column(to.columns()[column_index(from, &c)?].clone()))
                }
                e => Transformed::no(e),
            })
        })?
        .data)
}

fn column_index(schema: &DFSchema, column: &Column) -> Result<usize> {
    let (qualifier, field) = schema.qualified_field_from_column(column)?;
    // Native resolution can prefer an unqualified field over same-named
    // qualified fields. Preserve that choice when converting it to an ordinal.
    Ok(schema
        .iter()
        .position(|(q, f)| q == qualifier && f.name() == field.name())
        .expect("resolved field belongs to the schema"))
}

pub(crate) fn immutable(expr: &Expr) -> Result<bool> {
    let mut result = true;
    expr.apply(|e| {
        let volatility = match e {
            Expr::ScalarFunction(f) => Some(f.func.signature().volatility),
            Expr::AggregateFunction(f) => Some(f.func.signature().volatility),
            Expr::WindowFunction(f) => Some(f.fun.signature().volatility),
            Expr::HigherOrderFunction(f) => Some(f.func.signature().volatility),
            _ => None,
        };
        result &= volatility.is_none_or(|v| v == Volatility::Immutable);
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(result)
}

pub(crate) fn properties(
    expr: &Expr,
    schema: &DFSchema,
    policy: &Arc<dyn ExpressionProperties>,
) -> ScalarFunctionProperties {
    let yes = ScalarFunctionProperties {
        total: true,
        respects_grouping_equality: true,
    };
    let no = ScalarFunctionProperties::default();
    let child = |e: &Expr| properties(e, schema, policy);
    let combine = |xs: Vec<ScalarFunctionProperties>| ScalarFunctionProperties {
        total: xs.iter().all(|p| p.total),
        respects_grouping_equality: xs.iter().all(|p| p.respects_grouping_equality),
    };
    match expr {
        Expr::Column(_) | Expr::Literal(..) => yes,
        Expr::Alias(a) => child(&a.expr),
        Expr::ScalarFunction(f) if f.func.signature().volatility == Volatility::Immutable => {
            let mut ps = f.args.iter().map(child).collect::<Vec<_>>();
            ps.push(policy.scalar_function(f, schema));
            combine(ps)
        }
        Expr::Cast(c)
            if c.expr
                .get_type(schema)
                .is_ok_and(|from| total_cast(&from, c.field.data_type())) =>
        {
            child(&c.expr)
        }
        Expr::TryCast(c) => {
            let mut p = child(&c.expr);
            p.respects_grouping_equality &= c
                .expr
                .get_type(schema)
                .is_ok_and(|t| !t.is_floating() || c.field.data_type().is_numeric());
            p
        }
        Expr::BinaryExpr(b) => {
            let supported = match b.op {
                Operator::Eq
                | Operator::NotEq
                | Operator::Lt
                | Operator::LtEq
                | Operator::Gt
                | Operator::GtEq
                | Operator::And
                | Operator::Or
                | Operator::IsDistinctFrom
                | Operator::IsNotDistinctFrom => true,
                Operator::Plus | Operator::Minus | Operator::Multiply => expr
                    .get_type(schema)
                    .is_ok_and(|t| matches!(t, DataType::Float32 | DataType::Float64)),
                Operator::Divide => {
                    positive_literal(&b.right)
                        && b.left.get_type(schema).ok() == b.right.get_type(schema).ok()
                }
                _ => false,
            };
            if supported {
                combine(vec![child(&b.left), child(&b.right)])
            } else {
                no
            }
        }
        Expr::Not(e)
        | Expr::IsNull(e)
        | Expr::IsNotNull(e)
        | Expr::IsTrue(e)
        | Expr::IsFalse(e)
        | Expr::IsUnknown(e)
        | Expr::IsNotTrue(e)
        | Expr::IsNotFalse(e)
        | Expr::IsNotUnknown(e) => child(e),
        Expr::Between(b) => combine(vec![child(&b.expr), child(&b.low), child(&b.high)]),
        Expr::InList(l) => combine(
            std::iter::once(child(&l.expr))
                .chain(l.list.iter().map(child))
                .collect(),
        ),
        Expr::Case(c) => combine(
            c.expr
                .iter()
                .map(|e| child(e))
                .chain(
                    c.when_then_expr
                        .iter()
                        .flat_map(|(w, t)| [child(w), child(t)]),
                )
                .chain(c.else_expr.iter().map(|e| child(e)))
                .collect(),
        ),
        _ => no,
    }
}
fn total_cast(from: &DataType, to: &DataType) -> bool {
    if from == to || *from == DataType::Null {
        return true;
    }
    // Native moment aggregates coerce decimal inputs to Float64. All Arrow
    // decimal magnitudes fit its exponent range, although precision can be lost.
    if matches!(
        from,
        DataType::Decimal32(..)
            | DataType::Decimal64(..)
            | DataType::Decimal128(..)
            | DataType::Decimal256(..)
    ) && *to == DataType::Float64
    {
        return true;
    }
    matches!(
        (from, to),
        (
            DataType::Int8,
            DataType::Int16 | DataType::Int32 | DataType::Int64
        ) | (DataType::Int16, DataType::Int32 | DataType::Int64)
            | (DataType::Int32, DataType::Int64)
            | (
                DataType::UInt8,
                DataType::UInt16 | DataType::UInt32 | DataType::UInt64
            )
            | (DataType::UInt16, DataType::UInt32 | DataType::UInt64)
            | (DataType::UInt32, DataType::UInt64)
            | (DataType::Float32, DataType::Float64)
    ) || (from.is_integer() && matches!(to, DataType::Float32 | DataType::Float64))
}
fn positive_literal(expr: &Expr) -> bool {
    let Expr::Literal(value, _) = expr else {
        return false;
    };
    match value {
        ScalarValue::Int8(Some(v)) => *v > 0,
        ScalarValue::Int16(Some(v)) => *v > 0,
        ScalarValue::Int32(Some(v)) => *v > 0,
        ScalarValue::Int64(Some(v)) => *v > 0,
        ScalarValue::UInt8(Some(v)) => *v > 0,
        ScalarValue::UInt16(Some(v)) => *v > 0,
        ScalarValue::UInt32(Some(v)) => *v > 0,
        ScalarValue::UInt64(Some(v)) => *v > 0,
        ScalarValue::Float32(Some(v)) => *v > 0.0 && v.is_finite(),
        ScalarValue::Float64(Some(v)) => *v > 0.0 && v.is_finite(),
        _ => false,
    }
}
