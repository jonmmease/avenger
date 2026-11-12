//! avenger's own functions: the marks, constructors, `scaled`, and `direct` of avenger-chart, and
//! the window transforms `bin`, `stack`, and `timeunit`.
//!
//! A mark is an aggregate that builds a chart of the rows of its group, as in
//! `|> AGGREGATE bar(x => species, y => n) AS chart`, and takes each channel as a parameter. A
//! constructor returns the fragment of its message. The transforms take their options as named
//! arguments, as in `bin(x, maxbins => 30)`, which their window functions take as pairs of a name
//! and a value.
//!
//! A parameter whose name is a reserved word of GoogleSQL takes the name with a trailing
//! underscore, as `range` of `scale` is `range_`, so a call can name it: GoogleSQL takes no
//! reserved word as the name of an argument. Each binding of avenger-chart's functions follows
//! this rule for the reserved words of its language.

use std::sync::Arc;

use avenger_chart::functions::{constructors, encoding_options, marks, Kind, COORD};
use avenger_datafusion_transforms::{bin::BinWindow, stack::StackWindow, timeunit::TimeUnitWindow};
use datafusion::{
    common::ScalarValue,
    logical_expr::{
        expr::ScalarFunction, AggregateUDF, Expr, ScalarUDF, WindowFunctionDefinition, WindowUDF,
    },
};

use super::{lit_str, Function, FunctionKind, Param, Planned};
use crate::functions::{
    constructor::ConstructorFunction, declaration::MarkDeclaration, kind::KindFunction,
};

/// The options of `bin`, `timeunit`, and `stack`.
const BIN_OPTIONS: &[&str] = &[
    "maxbins", "base", "divide", "extent", "span", "step", "steps", "minstep", "nice", "anchor",
];
const TIMEUNIT_OPTIONS: &[&str] = &[
    "unit",
    "min_unit",
    "maxbins",
    "week_start",
    "fiscal_year_start",
];
const STACK_OPTIONS: &[&str] = &["offset"];

pub(super) fn functions() -> Vec<Function> {
    let mut functions = Vec::new();
    for mark in marks() {
        let udaf = Arc::new(AggregateUDF::new_from_impl(MarkDeclaration::new(mark)));
        let params = (mark.channels.iter())
            .map(|channel| match channel.required {
                true => Param::required(&sql_name(channel.name)),
                false => Param::optional(&sql_name(channel.name)),
            })
            .chain(
                mark.properties
                    .iter()
                    .map(|property| Param::optional(&sql_name(property.name))),
            )
            .chain([Param::optional(&sql_name(COORD))])
            .collect();
        let function = Function::new(mark.name, FunctionKind::Aggregate, params, move |call| {
            Ok(Planned::Aggregate {
                func: udaf.clone(),
                args: positional(call.values()),
                filter: None,
                result: None,
            })
        });
        functions.push(function.with_doc(mark.doc));
    }
    for constructor in constructors() {
        let udf = Arc::new(ScalarUDF::new_from_impl(ConstructorFunction::new(
            constructor,
        )));
        let params = (constructor.params.iter())
            .map(|param| match param.required {
                true => Param::required(&sql_name(param.name)),
                false => Param::optional(&sql_name(param.name)),
            })
            .collect();
        let function = Function::scalar(constructor.name, params, move |call| {
            Ok(call_udf(&udf, positional(call.values())))
        });
        functions.push(function.with_doc(constructor.doc));
    }
    for kind in [Kind::Scaled, Kind::Direct] {
        let udf = Arc::new(ScalarUDF::new_from_shared_impl(Arc::new(
            KindFunction::new(kind),
        )));
        let params = match kind {
            Kind::Scaled => std::iter::once(Param::required("value"))
                .chain(
                    encoding_options()
                        .iter()
                        .map(|option| Param::optional(&sql_name(option.name)).named()),
                )
                .collect(),
            Kind::Direct => vec![Param::required("value")],
        };
        functions.push(Function::scalar(kind.name(), params, move |call| {
            Ok(call_udf(&udf, positional(call.values())))
        }));
    }
    functions.push(transform("bin", BIN_OPTIONS, || {
        WindowUDF::new_from_impl(BinWindow::default())
    }));
    functions.push(transform("timeunit", TIMEUNIT_OPTIONS, || {
        WindowUDF::new_from_impl(TimeUnitWindow::default())
    }));
    functions.push(transform("stack", STACK_OPTIONS, || {
        WindowUDF::new_from_impl(StackWindow::default())
    }));
    functions
}

/// The name of the parameter `name` in SQL: `name`, or with a trailing underscore when it is a
/// reserved word of GoogleSQL.
fn sql_name(name: &str) -> String {
    match avenger_sql_syntax::is_reserved(name) {
        true => format!("{name}_"),
        false => name.to_string(),
    }
}

/// The values of the parameters in their order, with `NULL` for an omitted one before the last
/// that the call gives, as the functions of avenger-chart take them.
fn positional(values: Vec<Option<Expr>>) -> Vec<Expr> {
    let given = values
        .iter()
        .rposition(|value| value.is_some())
        .map_or(0, |last| last + 1);
    values
        .into_iter()
        .take(given)
        .map(|value| value.unwrap_or(Expr::Literal(ScalarValue::Null, None)))
        .collect()
}

fn call_udf(udf: &Arc<ScalarUDF>, args: Vec<Expr>) -> Expr {
    Expr::ScalarFunction(ScalarFunction::new_udf(udf.clone(), args))
}

/// A window transform whose options are named arguments after its value, and which its window
/// function takes as pairs of a name and a value.
fn transform(name: &str, options: &'static [&'static str], udwf: fn() -> WindowUDF) -> Function {
    let params = std::iter::once(Param::required("value"))
        .chain(
            options
                .iter()
                .map(|option| Param::optional(&sql_name(option)).named()),
        )
        .collect();
    let udwf = Arc::new(udwf());
    Function::new(name, FunctionKind::Window, params, move |call| {
        let mut args = vec![call.arg(0)];
        for (index, option) in options.iter().enumerate() {
            if let Some(value) = call.value(index + 1) {
                args.extend([lit_str(option), value.clone()]);
            }
        }
        Ok(Planned::Window {
            func: WindowFunctionDefinition::WindowUDF(udwf.clone()),
            args,
            result: None,
        })
    })
}
