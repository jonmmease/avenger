use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, DFSchema, Result,
    },
    logical_expr::{Expr, ExprSchemable, Volatility},
    optimizer::analyzer::type_coercion::TypeCoercionRewriter,
};
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

// Subqueries, unresolved parameters, and non-scalar operators need separate lineage analysis.
pub(crate) fn row_expression(expr: &Expr) -> Result<bool> {
    Ok(!expr.exists(|e| {
        Ok(!matches!(
            e,
            Expr::Column(_)
                | Expr::Literal(..)
                | Expr::Alias(_)
                | Expr::BinaryExpr(_)
                | Expr::Like(_)
                | Expr::SimilarTo(_)
                | Expr::Not(_)
                | Expr::IsNull(_)
                | Expr::IsNotNull(_)
                | Expr::IsTrue(_)
                | Expr::IsFalse(_)
                | Expr::IsUnknown(_)
                | Expr::IsNotTrue(_)
                | Expr::IsNotFalse(_)
                | Expr::IsNotUnknown(_)
                | Expr::Negative(_)
                | Expr::Between(_)
                | Expr::Case(_)
                | Expr::Cast(_)
                | Expr::TryCast(_)
                | Expr::ScalarFunction(_)
                | Expr::InList(_)
        ))
    })?)
}
