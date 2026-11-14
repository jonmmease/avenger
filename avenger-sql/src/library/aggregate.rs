//! The aggregate functions and the window functions.

use std::sync::Arc;

use datafusion::{
    arrow::datatypes::DataType,
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

use super::{lit_str, CallError, Function, FunctionKind, Param, Planned};

pub(super) fn functions() -> Vec<Function> {
    vec![
        Function::new(
            "COUNT",
            FunctionKind::Aggregate,
            vec![Param::optional("value")],
            |call| {
                let arg = match (call.star, call.value(0)) {
                    (true, _) => Expr::Literal(ScalarValue::Int64(Some(1)), None),
                    (false, Some(value)) => value.clone(),
                    (false, None) => return Err(CallError::call("COUNT takes a value or `*`")),
                };
                Ok(aggregate(count_udaf(), vec![arg]))
            },
        ),
        Function::new(
            "COUNTIF",
            FunctionKind::Aggregate,
            vec![Param::required("condition")],
            |call| {
                Ok(Planned::Aggregate {
                    func: count_udaf(),
                    args: vec![Expr::Literal(ScalarValue::Int64(Some(1)), None)],
                    filter: Some(call.arg(0)),
                    result: None,
                })
            },
        ),
        unary("SUM", sum_udaf),
        unary("AVG", avg_udaf),
        unary("MIN", min_udaf),
        unary("MAX", max_udaf),
        unary("ANY_VALUE", any_value_udaf),
        unary("ARRAY_AGG", array_agg_udaf),
        unary("LOGICAL_AND", bool_and_udaf),
        unary("LOGICAL_OR", bool_or_udaf),
        Function::new(
            "STRING_AGG",
            FunctionKind::Aggregate,
            vec![Param::required("value"), Param::optional("delimiter")],
            |call| {
                let delimiter = call.value(1).cloned().unwrap_or_else(|| lit_str(","));
                Ok(Planned::Aggregate {
                    func: string_agg_udaf(),
                    args: vec![call.arg(0), delimiter],
                    filter: None,
                    result: Some(DataType::Utf8),
                })
            },
        ),
    ]
}

pub(super) fn window_functions() -> Vec<Function> {
    vec![
        counting(nullary("ROW_NUMBER", row_number_udwf)),
        counting(nullary("RANK", rank_udwf)),
        counting(nullary("DENSE_RANK", dense_rank_udwf)),
        nullary("PERCENT_RANK", percent_rank_udwf),
        nullary("CUME_DIST", cume_dist_udwf),
        counting(window(
            "NTILE",
            vec![Param::required("buckets")],
            ntile_udwf,
        )),
        lag_lead("LAG", lag_udwf),
        lag_lead("LEAD", lead_udwf),
        window(
            "FIRST_VALUE",
            vec![Param::required("value")],
            first_value_udwf,
        ),
        window(
            "LAST_VALUE",
            vec![Param::required("value")],
            last_value_udwf,
        ),
    ]
}

/// `LAG(value [, offset [, default]])` or `LEAD`, whose offset is 1 when a call gives only a
/// default.
fn lag_lead(name: &str, udwf: fn() -> Arc<WindowUDF>) -> Function {
    let params = vec![
        Param::required("value"),
        Param::optional("offset"),
        Param::optional("default_value"),
    ];
    Function::new(name, FunctionKind::Window, params, move |call| {
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
        Ok(Planned::Window {
            func: WindowFunctionDefinition::WindowUDF(udwf()),
            args,
            result: None,
        })
    })
}

fn aggregate(func: Arc<AggregateUDF>, args: Vec<Expr>) -> Planned {
    Planned::Aggregate {
        func,
        args,
        filter: None,
        result: None,
    }
}

fn unary(name: &str, udaf: fn() -> Arc<AggregateUDF>) -> Function {
    Function::new(
        name,
        FunctionKind::Aggregate,
        vec![Param::required("value")],
        move |call| Ok(aggregate(udaf(), vec![call.arg(0)])),
    )
}

fn nullary(name: &str, udwf: fn() -> Arc<WindowUDF>) -> Function {
    window(name, vec![], udwf)
}

/// A window function whose arguments are the values of its parameters, in order, up to the last
/// that the call gives.
fn window(name: &str, params: Vec<Param>, udwf: fn() -> Arc<WindowUDF>) -> Function {
    Function::new(name, FunctionKind::Window, params, move |call| {
        let args: Vec<Expr> = call.values().into_iter().map_while(|value| value).collect();
        Ok(Planned::Window {
            func: WindowFunctionDefinition::WindowUDF(udwf()),
            args,
            result: None,
        })
    })
}

/// A window function that counts rows, whose `UInt64` result is an `INT64` in GoogleSQL.
fn counting(function: Function) -> Function {
    let Function {
        name,
        kind,
        params,
        safe,
        doc,
        mapping,
    } = function;
    Function {
        name,
        kind,
        params,
        safe,
        doc,
        mapping: Box::new(move |call| match mapping(call)? {
            Planned::Window { func, args, .. } => Ok(Planned::Window {
                func,
                args,
                result: Some(DataType::Int64),
            }),
            planned => Ok(planned),
        }),
    }
}
