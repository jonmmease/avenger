//! The aggregate functions and the window functions.

use std::sync::Arc;

use datafusion::{
    common::ScalarValue,
    functions_aggregate::{
        any_value::any_value_udaf,
        array_agg::array_agg_udaf,
        average::avg_udaf,
        bool_and_or::{bool_and_udaf, bool_or_udaf},
        count::count_udaf,
        min_max::{max_udaf, min_udaf},
        string_agg::string_agg_udaf,
        sum::sum_udaf,
    },
    functions_window::{
        cume_dist::cume_dist_udwf,
        lead_lag::{lag_udwf, lead_udwf},
        nth_value::{first_value_udwf, last_value_udwf},
        ntile::ntile_udwf,
        rank::{dense_rank_udwf, percent_rank_udwf, rank_udwf},
        row_number::row_number_udwf,
    },
    logical_expr::{AggregateUDF, Expr, WindowFunctionDefinition, WindowUDF},
};

use super::{lit_str, CallError, Function, Need, Planned};

/// The example table of the aggregates, `x` of `[1, 2, 2, 3, NULL]`.
const NUMBERS: &str = "FROM UNNEST([1, 2, 2, 3, NULL]) AS x";

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::aggregate("COUNT", "Returns the number of rows, or of the rows whose value is not `NULL`.")
            .description("`COUNT(*)` counts every row of the group, and `COUNT(value)` the rows whose value is not `NULL`. With `DISTINCT`, it counts the distinct values.")
            .param("value", "The value whose rows to count.")
            .form("(value: T) -> INT64 where T: ANY", |call| {
                let arg = match (call.star, call.value(0)) {
                    (true, _) => Expr::Literal(ScalarValue::Int64(Some(1)), None),
                    (false, Some(value)) => value.clone(),
                    (false, None) => return Err(CallError::call("COUNT takes a value or `*`")),
                };
                Ok(aggregate(count_udaf(), vec![arg]))
            })
            .distinct()
            .example(&format!("{NUMBERS} |> AGGREGATE COUNT(*) AS row_count, COUNT(x) AS value_count, COUNT(DISTINCT x) AS distinct_count"), "row_count | value_count | distinct_count\n5 | 4 | 3"),
        Function::aggregate("COUNTIF", "Returns the number of rows whose condition is `TRUE`.")
            .param("condition", "The condition of each row. `FALSE` and `NULL` don't count.")
            .form("(condition: BOOL) -> INT64", |call| {
                Ok(Planned::Aggregate {
                    func: count_udaf(),
                    args: vec![Expr::Literal(ScalarValue::Int64(Some(1)), None)],
                    filter: Some(call.arg(0)),
                    result: None,
                })
            })
            .distinct()
            .example(&format!("{NUMBERS} |> AGGREGATE COUNTIF(x > 1) AS n"), "n\n3"),
        unary("SUM", "Returns the sum of the values that are not `NULL`.", "(value: T) -> T where T: INT64 | NUMERIC | BIGNUMERIC | FLOAT64", sum_udaf)
            .description("Returns `NULL` when the group has no value that isn't `NULL`.")
            .distinct()
            .example(&format!("{NUMBERS} |> AGGREGATE SUM(x) AS total"), "total\n8"),
        unary("AVG", "Returns the mean of the values that are not `NULL`.", "(value: INT64) -> FLOAT64", avg_udaf)
            .form("(value: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC", |call| {
                Ok(aggregate(avg_udaf(), vec![call.arg(0)]))
            })
            .description("The mean of `INT64` values is a `FLOAT64`. Returns `NULL` when the group has no value that isn't `NULL`.")
            .distinct()
            .example(&format!("{NUMBERS} |> AGGREGATE AVG(x) AS mean"), "mean\n2.0"),
        unary("MIN", "Returns the least of the values that are not `NULL`.", "(value: T) -> T where T: ANY", min_udaf)
            .distinct()
            .example(&format!("{NUMBERS} |> AGGREGATE MIN(x) AS least"), "least\n1"),
        unary("MAX", "Returns the greatest of the values that are not `NULL`.", "(value: T) -> T where T: ANY", max_udaf)
            .distinct()
            .example(&format!("{NUMBERS} |> AGGREGATE MAX(x) AS greatest"), "greatest\n3"),
        unary("ANY_VALUE", "Returns a value of the group, which can be any of them.", "(value: T) -> T where T: ANY", any_value_udaf)
            .example("FROM UNNEST([7, 7]) AS x |> AGGREGATE ANY_VALUE(x) AS one", "one\n7"),
        unary("ARRAY_AGG", "Returns an array of the values of the group.", "(value: T) -> ARRAY<T> where T: ANY", array_agg_udaf)
            .description("`ORDER BY` orders the elements, and `IGNORE NULLS` leaves out the `NULL` values.")
            .distinct()
            .null_treatment()
            .order_by()
            .limit()
            .example(&format!("{NUMBERS} |> AGGREGATE ARRAY_AGG(x IGNORE NULLS ORDER BY x DESC) AS values"), "values\n[3, 2, 2, 1]"),
        Function::aggregate("GROUPING", "Returns whether a group expression is aggregated over in a row of grouping sets.")
            .description("In an aggregate with `ROLLUP`, `CUBE` or `GROUPING SETS`, returns 1 for a row whose grouping set leaves the expression out, so its value is `NULL` for all its values, and 0 for a row grouped by it.")
            .param("groupby_expression", "An expression of the `GROUP BY`.")
            .form("(groupby_expression: T) -> INT64 where T: ANY", |call| {
                Ok(aggregate(
                    datafusion::functions_aggregate::grouping::grouping_udaf(),
                    vec![call.arg(0)],
                ))
            })
            .example(&format!("{NUMBERS} |> AGGREGATE COUNT(*) AS n, GROUPING(x) AS g GROUP BY ROLLUP(x) |> WHERE g = 1"), "x | n | g\nNULL | 5 | 1")
            .without_window(),
        unary("LOGICAL_AND", "Returns whether every value that is not `NULL` is `TRUE`.", "(value: BOOL) -> BOOL", bool_and_udaf)
            .example(&format!("{NUMBERS} |> AGGREGATE LOGICAL_AND(x > 0) AS all_positive"), "all_positive\nTRUE"),
        unary("LOGICAL_OR", "Returns whether any value is `TRUE`.", "(value: BOOL) -> BOOL", bool_or_udaf)
            .example(&format!("{NUMBERS} |> AGGREGATE LOGICAL_OR(x > 2) AS any_above_2"), "any_above_2\nTRUE"),
        Function::aggregate("STRING_AGG", "Joins the strings of the group into one, with a delimiter between them.")
            .description("`ORDER BY` orders the strings. Returns `NULL` when the group has no string that isn't `NULL`.")
            .param("value", "The string of each row. A `NULL` value is left out.")
            .param("delimiter", "The string between two values, a constant. When omitted, `','`.")
            .form("(value: STRING[, delimiter: CONST STRING]) -> STRING", |call| {
                let delimiter = call.value(1).cloned().unwrap_or_else(|| lit_str(","));
                Ok(aggregate(string_agg_udaf(), vec![call.arg(0), delimiter]))
            })
            .distinct()
            .order_by()
            .limit()
            .example("FROM UNNEST(['b', 'a', 'c']) AS s |> AGGREGATE STRING_AGG(s, '-' ORDER BY s) AS joined", "joined\n'a-b-c'"),
    ]
}

/// The example of the window functions, `x` of `[1, 2, 2, 3]`, in the order of `x`.
const TIES: &str = "FROM UNNEST([1, 2, 2, 3]) AS x";
const DISTINCT: &str = "FROM UNNEST([1, 2, 3, 4]) AS x";

pub(super) fn window_functions() -> Vec<Function> {
    vec![
        nullary("ROW_NUMBER", "Returns the number of each row in its window, from 1.", Need::Allowed, "INT64", row_number_udwf)
            .description("Rows that tie in the window's order take distinct numbers, in an order that isn't fixed.")
            .example(&format!("{TIES} |> EXTEND ROW_NUMBER() OVER (ORDER BY x) AS n"), "x | n\n1 | 1\n2 | 2\n2 | 3\n3 | 4"),
        nullary("RANK", "Returns the rank of each row in its window, from 1, with gaps after ties.", Need::Required, "INT64", rank_udwf)
            .description("Rows that tie take one rank, and the next rank skips the tied rows.")
            .example(&format!("{TIES} |> EXTEND RANK() OVER (ORDER BY x) AS rank"), "x | rank\n1 | 1\n2 | 2\n2 | 2\n3 | 4"),
        nullary("DENSE_RANK", "Returns the rank of each row in its window, from 1, without gaps after ties.", Need::Required, "INT64", dense_rank_udwf)
            .example(&format!("{TIES} |> EXTEND DENSE_RANK() OVER (ORDER BY x) AS rank"), "x | rank\n1 | 1\n2 | 2\n2 | 2\n3 | 3"),
        nullary("PERCENT_RANK", "Returns the rank of each row as a fraction of its window, from 0 to 1.", Need::Required, "FLOAT64", percent_rank_udwf)
            .description("The fraction is `(rank - 1) / (rows - 1)`, and 0 for a window of one row.")
            .example(&format!("{TIES} |> EXTEND PERCENT_RANK() OVER (ORDER BY x) AS fraction"), "x | fraction\n1 | 0.0\n2 | 0.3333333333333333\n2 | 0.3333333333333333\n3 | 1.0"),
        nullary("CUME_DIST", "Returns the fraction of the rows of its window that come before each row or tie with it.", Need::Required, "FLOAT64", cume_dist_udwf)
            .example(&format!("{TIES} |> EXTEND CUME_DIST() OVER (ORDER BY x) AS fraction"), "x | fraction\n1 | 0.25\n2 | 0.75\n2 | 0.75\n3 | 1.0"),
        Function::window("NTILE", "Divides the rows of each window into buckets, and returns the bucket of each row, from 1.", Need::Required, false)
            .description("The buckets differ in size by at most one row, and the first buckets take the extra rows.")
            .param("buckets", "The number of buckets, which is positive.")
            .form("(buckets: INT64) -> INT64", |call| Ok(window(ntile_udwf(), vec![call.arg(0)])))
            .example(&format!("{TIES} |> EXTEND NTILE(2) OVER (ORDER BY x) AS bucket"), "x | bucket\n1 | 1\n2 | 1\n2 | 2\n3 | 2"),
        lag_lead("LAG", "Returns the value of a row before each row in its window.", "before", lag_udwf)
            .example(&format!("{DISTINCT} |> EXTEND LAG(x) OVER (ORDER BY x) AS previous"), "x | previous\n1 | NULL\n2 | 1\n3 | 2\n4 | 3")
            .example(&format!("{DISTINCT} |> EXTEND LAG(x, 2, 0) OVER (ORDER BY x) AS two_before"), "x | two_before\n1 | 0\n2 | 0\n3 | 1\n4 | 2"),
        lag_lead("LEAD", "Returns the value of a row after each row in its window.", "after", lead_udwf)
            .example(&format!("{DISTINCT} |> EXTEND LEAD(x, 1, 0) OVER (ORDER BY x) AS next"), "x | next\n1 | 2\n2 | 3\n3 | 4\n4 | 0"),
        Function::window("FIRST_VALUE", "Returns the value of the first row of each row's window frame.", Need::Required, true)
            .description("`IGNORE NULLS` returns the first value that isn't `NULL`.")
            .param("value", "The value of each row.")
            .form("(value: T) -> T where T: ANY", |call| Ok(window(first_value_udwf(), vec![call.arg(0)])))
            .null_treatment()
            .example(&format!("{DISTINCT} |> EXTEND FIRST_VALUE(x) OVER (ORDER BY x) AS first"), "x | first\n1 | 1\n2 | 1\n3 | 1\n4 | 1"),
        Function::window("LAST_VALUE", "Returns the value of the last row of each row's window frame.", Need::Required, true)
            .description("With an `ORDER BY` and no frame, the frame ends at the row and its ties, so the last value is the row's own. `IGNORE NULLS` returns the last value that isn't `NULL`.")
            .param("value", "The value of each row.")
            .form("(value: T) -> T where T: ANY", |call| Ok(window(last_value_udwf(), vec![call.arg(0)])))
            .null_treatment()
            .example(&format!("{DISTINCT} |> EXTEND LAST_VALUE(x) OVER (ORDER BY x) AS last"), "x | last\n1 | 1\n2 | 2\n3 | 3\n4 | 4"),
    ]
}

/// `LAG(value [, offset [, default_value]])` or `LEAD`.
fn lag_lead(name: &str, summary: &str, side: &str, udwf: fn() -> Arc<WindowUDF>) -> Function {
    Function::window(name, summary, Need::Required, false)
        .param("value", "The value of the row.")
        .param(
            "offset",
            &format!(
                "How many rows {side} the row to read, which is not negative. When omitted, 1."
            ),
        )
        .param(
            "default_value",
            "The result when no row is that far from the row. When omitted, `NULL`.",
        )
        .form(
            "(value: T[, offset: INT64][, default_value: T]) -> T where T: ANY",
            move |call| {
                let mut args = vec![call.arg(0)];
                match (call.value(1), call.value(2)) {
                    (Some(offset), Some(default)) => args.extend([offset.clone(), default.clone()]),
                    (Some(offset), None) => args.push(offset.clone()),
                    (None, Some(default)) => args.extend([
                        Expr::Literal(ScalarValue::Int64(Some(1)), None),
                        default.clone(),
                    ]),
                    (None, None) => {}
                }
                Ok(window(udwf(), args))
            },
        )
}

fn aggregate(func: Arc<AggregateUDF>, args: Vec<Expr>) -> Planned {
    Planned::Aggregate {
        func,
        args,
        filter: None,
        result: None,
    }
}

fn window(func: Arc<WindowUDF>, args: Vec<Expr>) -> Planned {
    Planned::Window {
        func: WindowFunctionDefinition::WindowUDF(func),
        args,
        result: None,
    }
}

/// An aggregate of one value, of the form `declaration`.
fn unary(
    name: &str,
    summary: &str,
    declaration: &str,
    udaf: fn() -> Arc<AggregateUDF>,
) -> Function {
    Function::aggregate(name, summary)
        .param("value", "The value of each row.")
        .form(declaration, move |call| {
            Ok(aggregate(udaf(), vec![call.arg(0)]))
        })
}

/// A window function of no arguments and the result type `result`, whose `OVER` takes an
/// `ORDER BY` as `order_by` says. The ranks are `INT64` values in GoogleSQL, where DataFusion's
/// are `UInt64`, and the type of the form's result gives the cast.
fn nullary(
    name: &str,
    summary: &str,
    order_by: Need,
    result: &str,
    udwf: fn() -> Arc<WindowUDF>,
) -> Function {
    Function::window(name, summary, order_by, false).form(&format!("() -> {result}"), move |_| {
        Ok(window(udwf(), vec![]))
    })
}
