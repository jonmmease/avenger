//! Expressions for Vega value rules and native DataFusion aggregates.

use crate::udf::Function;
use datafusion::{
    common::ScalarValue,
    functions::core::expr_fn::get_field,
    functions_aggregate::expr_fn as agg,
    logical_expr::{lit, when, Expr},
};

/// Convert Boolean, numeric, or string values to non-null Vega truthiness.
pub fn truthy(value: Expr) -> Expr {
    Function::Truthy.call(vec![value])
}

/// Inclusive start of the bin containing a numeric value.
pub fn bin_start(value: Expr, parameters: Expr) -> Expr {
    get_field(Function::BinBounds.call(vec![value, parameters]), "start")
}

/// End of the bin containing a numeric value. The final bin includes its upper boundary.
pub fn bin_end(value: Expr, parameters: Expr) -> Expr {
    get_field(Function::BinBounds.call(vec![value, parameters]), "end")
}

/// Count all rows, including rows with missing values.
pub fn count() -> Expr {
    agg::count(lit(1_i64))
}

/// Count values other than null, NaN, and the empty string.
pub fn valid(value: Expr) -> Expr {
    count_matching(Function::Valid.call(vec![value]))
}

/// Count null and empty strings. NaN is neither valid nor missing.
pub fn missing(value: Expr) -> Expr {
    count_matching(Function::Missing.call(vec![value]))
}

fn count_matching(predicate: Expr) -> Expr {
    agg::count(
        when(predicate, lit(1_i64))
            .otherwise(lit(ScalarValue::Int64(None)))
            .expect("Boolean CASE has one branch"),
    )
}

/// Sum numeric values as Float64, excluding null and NaN.
pub fn sum(value: Expr) -> Expr {
    agg::sum(Function::Numeric.call(vec![value]))
}

/// Average numeric values as Float64, excluding null and NaN.
pub fn mean(value: Expr) -> Expr {
    agg::avg(Function::Numeric.call(vec![value]))
}

/// Minimum numeric or string value, excluding null, NaN, and empty strings.
pub fn min(value: Expr) -> Expr {
    agg::min(Function::Clean.call(vec![value]))
}

/// Maximum numeric or string value, excluding null, NaN, and empty strings.
pub fn max(value: Expr) -> Expr {
    agg::max(Function::Clean.call(vec![value]))
}

fn moment(value: Expr, operation: fn(Expr) -> Expr) -> Expr {
    let value = Function::Numeric.call(vec![value]);
    when(agg::count(value.clone()).gt(lit(1_i64)), operation(value))
        .otherwise(lit(ScalarValue::Float64(None)))
        .expect("Boolean CASE has one branch")
}

/// Sample variance, null when fewer than two valid numeric values exist.
pub fn variance(value: Expr) -> Expr {
    moment(value, agg::var_sample)
}

/// Population variance, null when fewer than two valid numeric values exist.
pub fn variancep(value: Expr) -> Expr {
    moment(value, agg::var_pop)
}

/// Sample standard deviation, null when fewer than two valid numeric values exist.
pub fn stdev(value: Expr) -> Expr {
    moment(value, agg::stddev)
}

/// Population standard deviation, null when fewer than two valid numeric values exist.
pub fn stdevp(value: Expr) -> Expr {
    moment(value, agg::stddev_pop)
}
