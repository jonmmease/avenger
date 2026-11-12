//! The mark call of a chart call, read against the parameters of its mark: its channels, its
//! properties and `coord`, its `impute` option, and its draw order.

use std::ops::ControlFlow;

use avenger_chart::functions::{self, Channel, Kind, Mark, COORD};
use datafusion::{
    common::Result,
    sql::sqlparser::{
        ast::{
            Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentClause,
            FunctionArguments, Ident, OrderByExpr, Spanned, Visit, Visitor,
        },
        tokenizer::Span,
    },
};

use crate::{error::at, syntax::function_name};

/// A mark call: the call as written, its mark, and its arguments by parameter.
pub(super) struct MarkCall {
    /// The call as written.
    pub(super) function: Function,
    pub(super) mark: &'static Mark,
    /// The channels, in the order of the mark's parameters.
    pub(super) channels: Vec<ChannelValue>,
    /// The properties, the options of the plot and the layer, and `coord`, in the order of the
    /// mark's parameters.
    pub(super) properties: Vec<(Ident, Expr)>,
    /// The `impute` option, a parameter of the mark calls that lift that the lifting reads
    /// as written ([`super::impute`]), with its name as written.
    pub(super) impute: Option<(Ident, Expr)>,
    /// The draw order.
    pub(super) order: Vec<OrderByExpr>,
}

/// The parameter of the mark calls that lift that imputes the rows of its key channel, after
/// the parameters of the mark.
pub(super) const IMPUTE: &str = "impute";

impl MarkCall {
    /// The span of the mark's name.
    pub(super) fn span(&self) -> Span {
        self.function.name.span()
    }
}

/// The value of a channel: its kind when `scaled` or `direct` wraps it, and the value inside.
pub(super) struct ChannelValue {
    /// The parameter name as written.
    pub(super) name: Ident,
    pub(super) channel: &'static Channel,
    /// The call of `scaled` or `direct` around the value, whose first argument is the value.
    pub(super) wrapper: Option<(Kind, Function)>,
    pub(super) value: Expr,
}

impl ChannelValue {
    /// The options of the wrapper after its value, such as `scale => ...`.
    pub(super) fn options(&self) -> &[FunctionArg] {
        match &self.wrapper {
            Some((
                _,
                Function {
                    args: FunctionArguments::List(list),
                    ..
                },
            )) => &list.args[1..],
            _ => &[],
        }
    }
}

/// Reads the mark call `expr` against its mark: an unknown mark, an unknown parameter, and a
/// parameter given two times are errors at their text. Each `scaled` and `direct` is the complete
/// value of its channel (V5).
pub(super) fn read(expr: &Expr) -> Result<MarkCall> {
    let Expr::Function(function) = expr else {
        return Err(at(
            expr.span(),
            format!("a lifting takes a mark call, not `{expr}`"),
        ));
    };
    let name = function_name(function).unwrap_or_default();
    let Some(mark) = functions::mark(&name) else {
        return Err(at(
            function.name.span(),
            format!("`{}` is not a mark function", function.name),
        ));
    };
    let FunctionArguments::List(list) = &function.args else {
        return Err(at(
            function.name.span(),
            format!("{name}() takes a list of arguments"),
        ));
    };
    let parameters: Vec<&str> = (mark.channels.iter().map(|channel| channel.name))
        .chain(mark.properties.iter().map(|property| property.name))
        .chain(mark.options.iter().map(|option| option.name))
        .chain([COORD])
        .collect();
    let mut impute = None;
    let mut given: Vec<Option<(Ident, Expr)>> = vec![None; parameters.len()];
    for (position, argument) in list.args.iter().enumerate() {
        let (index, ident, value) = match argument {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(value)) => {
                let Some(parameter) = parameters.get(position) else {
                    return Err(at(
                        value.span(),
                        format!("{name}() takes {} arguments", parameters.len()),
                    ));
                };
                (
                    position,
                    Ident::with_span(value.span(), *parameter),
                    value.clone(),
                )
            }
            FunctionArg::Named {
                name: ident,
                arg: FunctionArgExpr::Expr(value),
                ..
            } => {
                let written = match ident.quote_style {
                    Some(_) => ident.value.clone(),
                    None => ident.value.to_lowercase(),
                };
                if written == IMPUTE && !parameters.contains(&IMPUTE) {
                    if impute.is_some() {
                        return Err(at(ident.span, format!("{name}() takes `{IMPUTE}` once")));
                    }
                    impute = Some((ident.clone(), value.clone()));
                    continue;
                }
                let Some(index) = parameters.iter().position(|p| *p == written) else {
                    return Err(at(
                        ident.span,
                        format!("{name}() has no parameter `{}`", ident.value),
                    ));
                };
                (index, ident.clone(), value.clone())
            }
            other => {
                return Err(at(
                    other.span(),
                    format!("{name}() takes no argument `{other}`"),
                ))
            }
        };
        if given[index].is_some() {
            return Err(at(
                ident.span,
                format!("{name}() takes `{}` once", parameters[index]),
            ));
        }
        given[index] = Some((ident, value));
    }
    let order = (list.clauses.iter())
        .find_map(|clause| match clause {
            FunctionArgumentClause::OrderBy(items) => Some(items.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let mut channels = Vec::new();
    let mut properties = Vec::new();
    for (index, argument) in given.into_iter().enumerate() {
        let Some((ident, value)) = argument else {
            continue;
        };
        match mark.channels.get(index) {
            Some(channel) => channels.push(channel_value(channel, ident, value)?),
            None => properties.push((ident, value)),
        }
    }
    Ok(MarkCall {
        function: function.clone(),
        mark,
        channels,
        properties,
        impute,
        order,
    })
}

/// The value of `channel`, without the `scaled` or `direct` that wraps it.
fn channel_value(channel: &'static Channel, name: Ident, value: Expr) -> Result<ChannelValue> {
    let (wrapper, inner) = match &value {
        Expr::Function(function) => match function_name(function).and_then(|n| Kind::of(&n)) {
            Some(kind) => {
                let FunctionArguments::List(list) = &function.args else {
                    return Err(at(
                        function.name.span(),
                        "scaled() and direct() take a value",
                    ));
                };
                let Some(FunctionArg::Unnamed(FunctionArgExpr::Expr(inner))) = list.args.first()
                else {
                    return Err(at(
                        function.name.span(),
                        format!("{}() takes a value first", kind.name()),
                    ));
                };
                if kind == Kind::Scaled && !has_scale(channel) {
                    return Err(at(
                        function.name.span(),
                        format!("`{}` has no scale, so its values are direct", channel.name),
                    ));
                }
                (Some((kind, function.clone())), inner.clone())
            }
            None => (None, value.clone()),
        },
        _ => (None, value.clone()),
    };
    if let Expr::Case { .. } = &inner {
        if inner.visit(&mut Wrapped).is_break() {
            return Err(at(
                inner.span(),
                "conditional channels, a CASE with scaled() or direct() in a result, are not supported in a mark call that lifts yet",
            ));
        }
    }
    let mut found = Wrapped;
    if let ControlFlow::Break(span) = inner.visit(&mut found) {
        return Err(at(
            span,
            "scaled() and direct() are valid only as the complete value of a channel",
        ));
    }
    Ok(ChannelValue {
        name,
        channel,
        wrapper,
        value: inner,
    })
}

/// Whether `channel` has a scale: it scales strings or numbers.
fn has_scale(channel: &Channel) -> bool {
    channel.field_kind() == Kind::Scaled || channel.value_kind() == Kind::Scaled
}

/// Breaks at a call of `scaled` or `direct`, with its span.
struct Wrapped;

impl Visitor for Wrapped {
    type Break = Span;

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Span> {
        match expr {
            Expr::Function(function)
                if function_name(function).is_some_and(|name| Kind::of(&name).is_some()) =>
            {
                ControlFlow::Break(function.name.span())
            }
            _ => ControlFlow::Continue(()),
        }
    }
}
