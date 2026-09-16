//! DataFusion 54.1 derives scalar-subquery nullability from its table field.
//! Widen that field before analysis because an empty table produces a null scalar.
use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::datatypes::{DataType, FieldRef},
    common::{
        tree_node::{Transformed, TreeNode},
        Result,
    },
    logical_expr::{
        expr_rewriter::NamePreserver, ColumnarValue, Expr, LogicalPlan, LogicalPlanBuilder,
        ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

#[derive(Debug, PartialEq, Eq, Hash)]
struct NullableField;

impl ScalarUDFImpl for NullableField {
    fn name(&self) -> &str {
        "__avenger_nullable_field"
    }
    fn signature(&self) -> &Signature {
        static SIGNATURE: LazyLock<Signature> =
            LazyLock::new(|| Signature::any(1, Volatility::Immutable));
        &SIGNATURE
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs<'_>) -> Result<FieldRef> {
        Ok(Arc::new(
            args.arg_fields[0].as_ref().clone().with_nullable(true),
        ))
    }
    fn return_type(&self, args: &[DataType]) -> Result<DataType> {
        Ok(args[0].clone())
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        Ok(args.args[0].clone())
    }
}

pub(crate) fn normalize_plan(plan: LogicalPlan) -> Result<LogicalPlan> {
    plan.transform_up_with_subqueries(|plan| {
        let names = NamePreserver::new(&plan);
        plan.map_expressions(|expr| {
            let original = names.save(&expr);
            expr.transform_up(widen_scalar_subquery)
                .map(|result| result.update_data(|expr| original.restore(expr)))
        })?
        .map_data(LogicalPlan::recompute_schema)
    })
    .map(|result| result.data)
}

pub(crate) fn normalize_expr(expr: Expr) -> Result<Expr> {
    // Wrapping also lets the native plan traversal visit all nested subqueries.
    let plan = LogicalPlanBuilder::empty(true)
        .project(vec![expr])?
        .build()?;
    let LogicalPlan::Projection(projection) = normalize_plan(plan)? else {
        unreachable!()
    };
    Ok(projection.expr.into_iter().next().expect("one expression"))
}

fn widen_scalar_subquery(expr: Expr) -> Result<Transformed<Expr>> {
    let Expr::ScalarSubquery(mut subquery) = expr else {
        return Ok(Transformed::no(expr));
    };
    let schema = subquery.subquery.schema();
    if schema.fields().len() != 1 || schema.field(0).is_nullable() {
        return Ok(Transformed::no(Expr::ScalarSubquery(subquery)));
    }
    let (qualifier, field) = schema.qualified_field(0);
    let value = ScalarUDF::from(NullableField)
        .call(vec![Expr::Column(schema.columns()[0].clone())])
        .alias_qualified(qualifier.cloned(), field.name());
    subquery.subquery = Arc::new(
        LogicalPlanBuilder::from(subquery.subquery.as_ref().clone())
            .project(vec![value])?
            .build()?,
    );
    Ok(Transformed::yes(Expr::ScalarSubquery(subquery)))
}
