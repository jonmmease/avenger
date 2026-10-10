use datafusion::{
    common::Result,
    functions_aggregate::expr_fn::{max, min},
    logical_expr::{col, Expr, LogicalPlan, LogicalPlanBuilder},
};

use crate::udf::Function;

/// Compute one row containing an `extent` struct with nullable Float64 min/max.
/// NaN values are ignored. Empty data or an infinite endpoint makes both null.
pub fn extent(input: LogicalPlan, value: Expr) -> Result<LogicalPlan> {
    let numeric = Function::Numeric.call(vec![value]);
    LogicalPlanBuilder::from(input)
        .aggregate(
            Vec::<Expr>::new(),
            vec![
                min(numeric.clone()).alias("__min"),
                max(numeric).alias("__max"),
            ],
        )?
        .project(vec![Function::Extent
            .call(vec![col("__min"), col("__max")])
            .alias("extent")])?
        .build()
}
