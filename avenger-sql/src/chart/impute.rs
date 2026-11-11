//! Stage 3: the `impute` option of a mark call, which adds the rows that each group lacks at each
//! key value, after the aggregates of stage 2 and before the stacks and the windows of stage 4
//! (`statistics/imputation.md` of the spec).
//!
//! `impute => 0` is short for `impute => impute(value => 0)`, and `impute(...)` takes `value` or
//! `method`, with `frame`, `keys`, and `extra_keys`. The option is a parameter of the mark calls
//! of `|> CHART` only, which the lifting reads as syntax, so it does not collide with the table
//! function `impute`, and the chart has no imputation: stage 3 is the pipe
//! `CALL impute(<key>, [<groups>], <measure>, ...)` of the table function, and stage 5 does not
//! read the option.

use std::ops::ControlFlow;

use datafusion::{
    common::Result,
    sql::sqlparser::{
        ast::{
            Array, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, Ident, PipeOperator,
            Spanned, UnaryOperator, Value, Visit, VisitMut, Visitor, VisitorMut, WindowSpec,
            WindowType,
        },
        tokenizer::Span,
    },
};

use super::{
    call::{MarkCall, IMPUTE},
    classify::{self, call_kind, first_argument, CallKind, Discretizer, Level},
    lifting::{column, function, named, replaced, Names},
    roles::{domain_order, DomainOrder, Roles},
    stage4,
};
use crate::{error::at, registry::Registry, syntax::function_name};

/// The marks that impute: the marks with a key channel and a measure channel (`r[impute.channel]`).
const MARKS: [&str; 5] = ["bar", "area", "line", "symbol", "text"];

/// The `impute` option of a mark call, as written.
pub(super) struct Imputation {
    /// The name of the option, where its errors of the roles point.
    option: Ident,
    fill: Fill,
    /// The `frame` of a statistic, with its name as written.
    frame: Option<(Ident, Expr)>,
    keys: Option<Expr>,
    extra_keys: Option<Expr>,
}

/// The value of an added row.
enum Fill {
    /// The constant `value`.
    Value(Box<Expr>),
    /// The statistic of the table function, such as `'avg'` for `method => 'mean'`, with the span
    /// of the method.
    Stat(&'static str, Span),
    /// `method => 'gap'`: `NULL`, which breaks a line (`r[impute.method.gap]`).
    Gap,
}

/// The `impute` option of `call`, checked as V10 checks its syntax: a plan-time constant (V8),
/// on a mark that imputes, with exactly one of `value` and `method`, a method of the spec, and a
/// `frame` of two bounds with a method of a statistic.
pub(super) fn read(call: &MarkCall) -> Result<Option<Imputation>> {
    let Some((option, value)) = &call.impute else {
        return Ok(None);
    };
    let mark = call.mark.name;
    if !MARKS.contains(&mark) {
        return Err(at(
            option.span,
            format!(
                "`{IMPUTE}` is valid only on bar(), area(), line(), symbol(), and text(); found on {mark}()"
            ),
        ));
    }
    classify::constant(value, &format!("option `{IMPUTE}` of {mark}()"))?;
    let constructor = match value {
        Expr::Function(function) if function_name(function).as_deref() == Some(IMPUTE) => function,
        _ => {
            return Ok(Some(Imputation {
                option: option.clone(),
                fill: Fill::Value(Box::new(value.clone())),
                frame: None,
                keys: None,
                extra_keys: None,
            }))
        }
    };
    let mut fill_value = None;
    let mut method = None;
    let mut frame = None;
    let mut keys = None;
    let mut extra_keys = None;
    let args: &[FunctionArg] = match &constructor.args {
        FunctionArguments::List(list) => &list.args,
        _ => &[],
    };
    for argument in args {
        let FunctionArg::Named {
            name,
            arg: FunctionArgExpr::Expr(value),
            ..
        } = argument
        else {
            return Err(at(
                argument.span(),
                "impute() takes named arguments: value, method, frame, keys, and extra_keys",
            ));
        };
        let slot = match name.value.to_lowercase().as_str() {
            "value" => &mut fill_value,
            "method" => &mut method,
            "frame" => &mut frame,
            "keys" => &mut keys,
            "extra_keys" => &mut extra_keys,
            _ => {
                return Err(at(
                    name.span,
                    format!(
                        "impute() has no argument `{}`; its arguments are value, method, frame, keys, and extra_keys",
                        name.value
                    ),
                ))
            }
        };
        if slot.is_some() {
            return Err(at(
                name.span,
                format!("impute() takes `{}` once", name.value),
            ));
        }
        *slot = Some((name.clone(), value.clone()));
    }
    let fill = match (fill_value, method) {
        (Some((_, value)), None) => Fill::Value(Box::new(value)),
        (None, Some((_, method))) => read_method(&method)?,
        _ => {
            return Err(at(
                constructor.name.span(),
                "an impute() option needs exactly one of `value` and `method`",
            ))
        }
    };
    if let Some((name, frame)) = &frame {
        match &fill {
            Fill::Stat(..) => check_frame(name, frame)?,
            Fill::Value(_) => {
                return Err(at(
                    name.span,
                    "`frame` needs a method of 'mean', 'median', 'min', or 'max'; found `value`",
                ))
            }
            Fill::Gap => {
                return Err(at(
                    name.span,
                    "`frame` needs a method of 'mean', 'median', 'min', or 'max'; found 'gap'",
                ))
            }
        }
    }
    Ok(Some(Imputation {
        option: option.clone(),
        fill,
        frame,
        keys: keys.map(|(_, keys)| keys),
        extra_keys: extra_keys.map(|(_, keys)| keys),
    }))
}

/// The fill of `method`, one of the methods of the spec (`r[impute.method.invalid]`).
fn read_method(method: &Expr) -> Result<Fill> {
    let text = match method {
        Expr::Value(value) => match &value.value {
            Value::SingleQuotedString(text) => Some(text.to_lowercase()),
            _ => None,
        },
        _ => None,
    };
    let span = method.span();
    match text.as_deref() {
        Some("mean") => Ok(Fill::Stat("avg", span)),
        Some("median") => Ok(Fill::Stat("median", span)),
        Some("min") => Ok(Fill::Stat("min", span)),
        Some("max") => Ok(Fill::Stat("max", span)),
        Some("gap") => Ok(Fill::Gap),
        _ => Err(at(
            span,
            format!("`method` must be 'mean', 'median', 'min', 'max', or 'gap'; found {method}"),
        )),
    }
}

/// `frame` is a list of two bounds, each an integer or `NULL`, whose start is not after its end
/// (`r[impute.method.frame.valid]`). `name` is the name of the argument as written.
fn check_frame(name: &Ident, frame: &Expr) -> Result<()> {
    const FORM: &str = "`frame` is a list of two bounds, each an integer or NULL, such as [-2, 2]";
    let Expr::Array(Array { elem, .. }) = frame else {
        return Err(at(name.span, FORM));
    };
    let [lo, hi] = elem.as_slice() else {
        return Err(at(name.span, FORM));
    };
    let bound = |expr: &Expr| -> Result<Option<i64>> {
        let (negative, literal) = match expr {
            Expr::UnaryOp {
                op: UnaryOperator::Minus,
                expr,
            } => (true, expr.as_ref()),
            other => (false, other),
        };
        let parsed = match literal {
            Expr::Value(value) => match &value.value {
                Value::Null if !negative => return Ok(None),
                Value::Number(text, _) => text.parse::<i64>().ok(),
                _ => None,
            },
            _ => None,
        };
        match parsed {
            Some(n) => Ok(Some(if negative { -n } else { n })),
            None => Err(at(expr.span(), FORM)),
        }
    };
    if let (Some(lo), Some(hi)) = (bound(lo)?, bound(hi)?) {
        if lo > hi {
            return Err(at(
                name.span,
                format!("the start of `frame`, {lo}, is after its end, {hi}"),
            ));
        }
    }
    Ok(())
}

/// The channels of an imputation: the key channel, along which it adds rows, and the value of the
/// measure channel without its `stack`, which takes the imputed value.
pub(super) struct Target {
    /// The index of the key channel in the channels of the call.
    pub(super) key: usize,
    measure: Expr,
}

/// The target of `imputation`, checked as V10 checks the roles: the call has a key channel and a
/// measure channel (`r[impute.key]`), and the measure, without its `stack`, is one aggregate in a
/// grouping call or a column in a call that does not group (`r[impute.value]`).
pub(super) fn target(
    call: &MarkCall,
    imputation: &Imputation,
    roles: Option<&Roles>,
    grouping: bool,
    registry: &dyn Registry,
) -> Result<Target> {
    let mark = call.mark.name;
    let span = imputation.option.span;
    let Some(roles) = roles else {
        return Err(at(
            span,
            format!("`{IMPUTE}` needs a key channel and a measure channel, and {mark}() has none"),
        ));
    };
    let (key_channel, measure_channel) = match roles.vertical {
        true => ("x", "y"),
        false => ("y", "x"),
    };
    let Some(measure) = roles.measure else {
        return Err(at(
            span,
            format!("`{IMPUTE}` needs a measure channel; this {mark} has no {measure_channel}"),
        ));
    };
    let Some(key) = roles.key else {
        return Err(at(
            span,
            format!("`{IMPUTE}` needs a key channel; this {mark} has no {key_channel}"),
        ));
    };
    let mut value = &call.channels[measure].value;
    if let Expr::Function(function) = value {
        if function_name(function).as_deref() == Some("stack") {
            if let Some(argument) = first_argument(function) {
                value = argument;
            }
        }
    }
    let valid = match grouping {
        true => {
            matches!(value, Expr::Function(function) if call_kind(function, registry) == Some(CallKind::Aggregate))
        }
        false => matches!(value, Expr::Identifier(_) | Expr::CompoundIdentifier(_)),
    };
    if !valid {
        let form = match grouping {
            true => "one aggregate, such as sum(units), in a chart that aggregates",
            false => "a column in a chart that does not aggregate",
        };
        return Err(at(
            value.span(),
            format!("`{IMPUTE}` fills the measure of {mark}(), which is {form}; found `{value}`"),
        ));
    }
    Ok(Target {
        key,
        measure: value.clone(),
    })
}

/// Whether `value`, the stage 2 form of a key or of a channel value, is a function of the imputed
/// key, whose stage 2 form is `key` (`r[impute.groups.dependent]`): it holds a copy of the key, and
/// each of its column references is inside a copy. A discretizer of its own is a column of stage 1,
/// so it is not a function of the key.
pub(super) fn dependent(value: &Expr, key: &Expr) -> bool {
    struct Erase<'a> {
        key: &'a Expr,
    }
    impl VisitorMut for Erase<'_> {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
            let inside = expr == self.key
                || matches!(
                    (&*expr, self.key),
                    (Expr::CompoundIdentifier(parts), Expr::Identifier(key)) if parts.first() == Some(key)
                );
            if inside {
                *expr = Expr::value(Value::Null);
            }
            ControlFlow::Continue(())
        }
    }
    struct Reads;
    impl Visitor for Reads {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            match expr {
                Expr::Identifier(_) | Expr::CompoundIdentifier(_) | Expr::Subquery(_) => {
                    ControlFlow::Break(())
                }
                _ => ControlFlow::Continue(()),
            }
        }
    }
    let mut erased = value.clone();
    let _ = VisitMut::visit(&mut erased, &mut Erase { key });
    erased != *value && Visit::visit(&erased, &mut Reads).is_continue()
}

/// The columns that stage 3 reads of a grouping call: the column of the imputed key, and the
/// columns of the other keys that are groups, in channel order.
pub(super) struct GroupingKeys {
    pub(super) key: Expr,
    pub(super) groups: Vec<Expr>,
}

/// Stage 3 of a call: the pipe `CALL` of `impute` and the name of its flag column.
pub(super) struct Stage3 {
    pub(super) operator: PipeOperator,
    pub(super) flag: Ident,
}

/// Stage 3 of `call`: `CALL impute(<key>, [<groups>], <measure>, <fill>, <keys>, flag =>
/// <flag>) [OVER (PARTITION BY ...)]` over the columns of `stage5`. `grouping` holds the keys of a
/// grouping call.
#[allow(clippy::too_many_arguments)]
pub(super) fn stage3(
    call: &MarkCall,
    imputation: &Imputation,
    target: &Target,
    levels: &[Level],
    partition: &[Expr],
    grouping: Option<&GroupingKeys>,
    stage5: &[(Expr, Expr)],
    names: &mut Names,
    registry: &dyn Registry,
) -> Result<Stage3> {
    let span = imputation.option.span;
    let key_channel = &call.channels[target.key];
    let (key, groups) = match grouping {
        Some(keys) => (keys.key.clone(), keys.groups.clone()),
        None => {
            let key = replaced(&key_channel.value, stage5);
            let groups = row_groups(call, target, levels, &key, stage5);
            (key, groups)
        }
    };
    if !matches!(key, Expr::Identifier(_) | Expr::CompoundIdentifier(_)) {
        return Err(at(
            key_channel.value.span(),
            format!(
                "`{IMPUTE}` imputes along the key channel `{}`, which is a column or a discretizer in a chart that does not aggregate; found `{}`",
                key_channel.channel.name, key_channel.value
            ),
        ));
    }
    let measure = replaced(&target.measure, stage5);
    let mut groups: Vec<Expr> = (partition.iter().cloned())
        .chain(groups)
        .filter(|group| group != &key && group != &measure)
        .collect();
    dedup(&mut groups);
    let at_span = |name: &str| Ident::with_span(span, name);
    let mut args = vec![
        FunctionArg::Unnamed(FunctionArgExpr::Expr(key.clone())),
        FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Array(Array {
            elem: groups.clone(),
            named: false,
        }))),
        FunctionArg::Unnamed(FunctionArgExpr::Expr(measure)),
    ];
    match &imputation.fill {
        Fill::Value(value) => args.push(named(at_span("value"), (**value).clone())),
        Fill::Stat(stat, span) => args.push(named(
            at_span("method"),
            Expr::Value(Value::SingleQuotedString(stat.to_string()).with_span(*span)),
        )),
        Fill::Gap => args.push(named(at_span("value"), Expr::value(Value::Null))),
    }
    if let Some((_, frame)) = &imputation.frame {
        args.push(named(at_span("frame"), frame.clone()));
    }
    let discretizer = match &key_channel.value {
        Expr::Function(function) => match call_kind(function, registry) {
            Some(CallKind::Discretizer(kind)) => Some((kind, function)),
            _ => None,
        },
        _ => None,
    };
    let domain = match domain_order(key_channel) {
        Some(DomainOrder::List(list)) => Some(*list),
        _ => None,
    };
    match (&imputation.keys, domain, discretizer) {
        (Some(keys), _, _) => args.push(named(at_span("keys"), keys.clone())),
        (None, Some(domain), _) => args.push(named(at_span("keys"), domain)),
        (None, None, Some((Discretizer::Bin | Discretizer::TimeUnit, _))) => args.push(named(
            at_span("fill_gaps"),
            Expr::Value(Value::Boolean(true).with_span(span)),
        )),
        _ => {}
    }
    if let Some(extra_keys) = &imputation.extra_keys {
        args.push(named(at_span("extra_keys"), extra_keys.clone()));
    }
    let flag = names.fresh("imputed", span);
    args.push(named(at_span("flag"), column(&flag)));
    let mut function = function(IMPUTE, span, args);
    // The bins of a `bin` key with a partition come from the mesh of each partition
    // (`r[impute.keys.bin]`), and the key values of another key from the complete input of the call
    // (`r[impute.keys.complete-input]`).
    if let Some((Discretizer::Bin, bin)) = discretizer {
        // A partition column of the call partitions the imputation, so it is no group of it.
        let window = stage4::discretizer_window(call, partition, bin)?;
        let mut columns = Vec::new();
        for item in &window.partition_by {
            let item_column = replaced(item, stage5);
            let readable = match grouping {
                Some(_) => groups.contains(&item_column) || partition.contains(&item_column),
                None => matches!(
                    item_column,
                    Expr::Identifier(_) | Expr::CompoundIdentifier(_)
                ),
            };
            if !readable {
                return Err(at(
                    item.span(),
                    format!(
                        "`{IMPUTE}` fills the bins of each `{item}`, which is not a key of this chart"
                    ),
                ));
            }
            columns.push(item_column);
        }
        if !columns.is_empty() {
            if let Some(FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Array(array)))) =
                function_args(&mut function).get_mut(1)
            {
                array.elem.retain(|group| !columns.contains(group));
            }
            function.over = Some(WindowType::WindowSpec(WindowSpec {
                window_name: None,
                partition_by: columns,
                order_by: Vec::new(),
                window_frame: None,
            }));
        }
    }
    Ok(Stage3 {
        operator: PipeOperator::Call {
            function,
            alias: None,
        },
        flag,
    })
}

/// The arguments of the call `function`.
fn function_args(
    function: &mut datafusion::sql::sqlparser::ast::Function,
) -> &mut Vec<FunctionArg> {
    match &mut function.args {
        FunctionArguments::List(list) => &mut list.args,
        _ => unreachable!("a call of impute has a list of arguments"),
    }
}

/// The groups of a call that does not group: the columns that the other channels read, outside
/// the copies of the key, in channel order.
fn row_groups(
    call: &MarkCall,
    target: &Target,
    levels: &[Level],
    key: &Expr,
    stage5: &[(Expr, Expr)],
) -> Vec<Expr> {
    struct Columns(Vec<Expr>);
    impl Visitor for Columns {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            if let Expr::Identifier(_) | Expr::CompoundIdentifier(_) = expr {
                self.0.push(expr.clone());
            }
            ControlFlow::Continue(())
        }
    }
    let mut groups = Columns(Vec::new());
    for (index, (channel, level)) in call.channels.iter().zip(levels).enumerate() {
        if index == target.key || !matches!(level, Level::Row | Level::Discretized) {
            continue;
        }
        let value = replaced(&channel.value, stage5);
        if dependent(&value, key) {
            continue;
        }
        let _ = Visit::visit(&value, &mut groups);
    }
    groups.0
}

/// `exprs` without its repeated expressions, in the order of their first occurrence.
fn dedup(exprs: &mut Vec<Expr>) {
    let mut seen: Vec<Expr> = Vec::new();
    exprs.retain(|expr| {
        let new = !seen.contains(expr);
        if new {
            seen.push(expr.clone());
        }
        new
    });
}
