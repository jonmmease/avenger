//! The roles of the channels of a mark call, from the orientation rules of the spec
//! (`marks/channel-expressions.md`): the key channel, along which the marks are placed, the
//! measure channel, which a stack accumulates, and the series channels, whose domain orders order
//! a stack (`r[stack.order.domain]`, `r[stack.order.legend]`).

use std::ops::ControlFlow;

use avenger_chart::functions::Kind;
use datafusion::{
    arrow::datatypes::DataType,
    common::{DFSchema, Result},
    sql::sqlparser::ast::{
        Array, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArguments, Spanned, Value,
        Visit, Visitor,
    },
};

use super::{
    call::{ChannelValue, MarkCall},
    classify::{call_kind, CallKind, Level},
    lifting::Env,
};
use crate::{error::at, syntax::function_name};

/// The roles of a call: its key channel, when it has one, and its measure channel, as indexes of
/// its channels, and whether its stacks are vertical, which reverses the domain orders.
pub(super) struct Roles {
    pub(super) key: Option<usize>,
    pub(super) measure: Option<usize>,
    pub(super) vertical: bool,
    /// Whether the mark is an arc, a polar bar whose `theta` is the measure and that has no
    /// `radius`, which stacks without a key (`orient.arc`).
    pub(super) arc: bool,
}

/// The series channels, in the order of the implied stack order.
pub(super) const SERIES: [&str; 8] = [
    "color",
    "fill",
    "stroke",
    "opacity",
    "stroke_dash",
    "stroke_width",
    "pattern",
    "detail",
];

/// The marks that decide an orientation, with their position channels: the key of a vertical
/// mark, then its measure.
fn positions(mark: &str) -> Option<(&'static str, &'static str)> {
    match mark {
        "bar" | "area" | "line" | "symbol" | "text" => Some(("x", "y")),
        "polar_bar" | "polar_area" | "polar_line" | "polar_symbol" | "polar_text" => {
            Some(("theta", "radius"))
        }
        _ => None,
    }
}

/// The roles of `call`, whose channels have `levels`: `orient.rule.derivation`, then
/// `orient.rule.discrete`, then `orient.rule.property`. A polar bar whose `theta` is the measure
/// and that has no `radius` is an arc, with no key channel (`orient.arc`). `schema` types the
/// values of level 0 and 1, with the columns of the discretizers.
pub(super) fn decide(
    call: &MarkCall,
    levels: &[Level],
    schema: &DFSchema,
    discretized: &[(Expr, Expr)],
    env: &mut dyn Env,
) -> Result<Option<Roles>> {
    let Some((first, second)) = positions(call.mark.name) else {
        return Ok(None);
    };
    let index = |name: &str| call.channels.iter().position(|c| c.channel.name == name);
    let (a, b) = (index(first), index(second));
    let derived = |i: Option<usize>| i.is_some_and(|i| derivation(&call.channels[i].value, env));
    let horizontal = match (derived(a), derived(b)) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    };
    let horizontal = match horizontal {
        Some(horizontal) => {
            check_orient(call, horizontal, if horizontal { first } else { second })?;
            horizontal
        }
        None => {
            let mut discrete = |i: Option<usize>| -> Result<bool> {
                match i {
                    Some(i) => is_discrete(&call.channels[i], levels[i], schema, discretized, env),
                    None => Ok(false),
                }
            };
            match (discrete(a)?, discrete(b)?) {
                (false, true) => {
                    check_orient(call, true, first)?;
                    true
                }
                (true, false) => {
                    check_orient(call, false, second)?;
                    false
                }
                _ => orient_property(call).is_some_and(|(orient, _)| orient == "horizontal"),
            }
        }
    };
    let (key, measure) = match horizontal {
        true => (b, a),
        false => (a, b),
    };
    if call.mark.name == "polar_bar" && horizontal && b.is_none() {
        return Ok(Some(Roles {
            key: None,
            measure,
            vertical: false,
            arc: true,
        }));
    }
    Ok(Some(Roles {
        key,
        measure,
        vertical: !horizontal,
        arc: false,
    }))
}

/// Whether `value` has an aggregate derivation or a stack (`orient.rule.derivation`).
fn derivation(value: &Expr, env: &dyn Env) -> bool {
    struct Find<'a> {
        env: &'a dyn Env,
    }
    impl Visitor for Find<'_> {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            match expr {
                Expr::Function(function)
                    if call_kind(function, self.env) == Some(CallKind::Aggregate)
                        || function_name(function).as_deref() == Some("stack") =>
                {
                    ControlFlow::Break(())
                }
                _ => ControlFlow::Continue(()),
            }
        }
    }
    Visit::visit(value, &mut Find { env }).is_break()
}

/// Whether the channel is discrete (`orient.discrete`): its value is a discretizer, a string, or
/// a boolean, or its `scaled` sets a discrete scale type.
fn is_discrete(
    channel: &ChannelValue,
    level: Level,
    schema: &DFSchema,
    discretized: &[(Expr, Expr)],
    env: &mut dyn Env,
) -> Result<bool> {
    if discrete_scale(channel) {
        return Ok(true);
    }
    if let Expr::Function(function) = &channel.value {
        if matches!(call_kind(function, env), Some(CallKind::Discretizer(_))) {
            return Ok(true);
        }
    }
    if level > Level::Discretized {
        return Ok(false);
    }
    let mut value = channel.value.clone();
    super::lifting::replace(&mut value, discretized);
    let data_type = env.data_type(value, schema)?;
    Ok(text_or_bool(&data_type))
}

/// Whether `data_type` is text or a boolean.
fn text_or_bool(data_type: &DataType) -> bool {
    match data_type {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean => true,
        DataType::Dictionary(_, values) => text_or_bool(values),
        _ => false,
    }
}

/// Whether the `scale` option of the channel's `scaled` sets a discrete scale type, as a
/// constructor of the type or as `type => 'band'`, `'point'`, `'ordinal'`, or `'nested_band'`.
fn discrete_scale(channel: &ChannelValue) -> bool {
    const DISCRETE: [&str; 4] = ["band", "point", "ordinal", "nested_band"];
    if !matches!(channel.wrapper, Some((Kind::Scaled, _))) {
        return false;
    }
    let Some(scale) = option(channel.options(), "scale") else {
        return false;
    };
    struct Find;
    impl Visitor for Find {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            let Expr::Function(function) = expr else {
                return ControlFlow::Continue(());
            };
            let name = function_name(function).unwrap_or_default();
            let typed = DISCRETE.iter().any(|kind| name == format!("{kind}_scale"));
            let literal = name == "scale"
                && matches!(
                    named(function, "type"),
                    Some(Expr::Value(value)) if matches!(&value.value, Value::SingleQuotedString(s) if DISCRETE.contains(&s.as_str()))
                );
            match typed || literal {
                true => ControlFlow::Break(()),
                false => ControlFlow::Continue(()),
            }
        }
    }
    Visit::visit(scale, &mut Find).is_break()
}

/// The value of the named argument `name` of `function`.
pub(super) fn named<'a>(function: &'a Function, name: &str) -> Option<&'a Expr> {
    let FunctionArguments::List(list) = &function.args else {
        return None;
    };
    option(&list.args, name)
}

/// The value of the named argument `name` of `args`.
fn option<'a>(args: &'a [FunctionArg], name: &str) -> Option<&'a Expr> {
    args.iter().find_map(|argument| match argument {
        FunctionArg::Named {
            name: option,
            arg: FunctionArgExpr::Expr(value),
            ..
        } if option.value.eq_ignore_ascii_case(name) => Some(value),
        _ => None,
    })
}

/// The `orient` property of the call, in lowercase, with its value, when it is a string literal.
fn orient_property(call: &MarkCall) -> Option<(String, &Expr)> {
    (call.properties.iter())
        .find(|(name, _)| name.value.eq_ignore_ascii_case("orient"))
        .and_then(|(_, value)| match value {
            Expr::Value(literal) => match &literal.value {
                Value::SingleQuotedString(s) => Some((s.to_lowercase(), value)),
                _ => None,
            },
            _ => None,
        })
}

/// Checks that a written `orient` agrees with the orientation that a rule before the property rule
/// decides: horizontal when `horizontal`, with its measure on `measure`.
fn check_orient(call: &MarkCall, horizontal: bool, measure: &str) -> Result<()> {
    let Some((written, value)) = orient_property(call) else {
        return Ok(());
    };
    let decided = match horizontal {
        true => "horizontal",
        false => "vertical",
    };
    if written == decided {
        return Ok(());
    }
    Err(at(
        value.span(),
        format!(
            "this {} measures `{measure}`, so its orient is '{decided}', not '{written}'",
            call.mark.name
        ),
    ))
}

/// Whether the mark `mark` has an `orient` property, which the stage 5 call writes from the roles.
pub(super) fn has_orient(mark: &str) -> bool {
    matches!(mark, "bar" | "area" | "polar_bar" | "polar_area")
}

/// The written `orient` of `call`, when it has one.
pub(super) fn writes_orient(call: &MarkCall) -> bool {
    (call.properties.iter()).any(|(name, _)| name.value.eq_ignore_ascii_case("orient"))
}

/// A written order of the domain of a series channel, which the legend follows.
pub(super) enum DomainOrder {
    /// A list, of a `domain` of its `scale` or of the preferred categories of a `category_fit`:
    /// the categories of the list in its order, then the others, ascending.
    List(Box<Expr>),
    /// The `order => 'descending'` of a `category_fit`: from the greatest category to the least.
    Descending,
}

/// The written order of the domain of the series channel `channel`, or `None` for the ascending
/// order of its values.
pub(super) fn domain_order(channel: &ChannelValue) -> Option<DomainOrder> {
    let scale = option(channel.options(), "scale")?;
    struct Find(Option<DomainOrder>);
    impl Visitor for Find {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            let Expr::Function(function) = expr else {
                return ControlFlow::Continue(());
            };
            let order = match function_name(function).as_deref() {
                Some("scale") => match named(function, "domain") {
                    Some(list @ Expr::Array(_)) => Some(DomainOrder::List(Box::new(list.clone()))),
                    _ => None,
                },
                Some("category_fit") => match named(function, "order") {
                    Some(list @ Expr::Array(_)) => Some(DomainOrder::List(Box::new(list.clone()))),
                    Some(Expr::Value(value)) => match &value.value {
                        Value::SingleQuotedString(sort)
                            if sort.eq_ignore_ascii_case("descending") =>
                        {
                            Some(DomainOrder::Descending)
                        }
                        Value::SingleQuotedString(_) => None,
                        _ => Some(DomainOrder::List(Box::new(Expr::Array(Array {
                            elem: vec![Expr::Value(value.clone())],
                            named: false,
                        })))),
                    },
                    _ => None,
                },
                _ => None,
            };
            match order {
                Some(order) => {
                    self.0 = Some(order);
                    ControlFlow::Break(())
                }
                None => ControlFlow::Continue(()),
            }
        }
    }
    let mut find = Find(None);
    let _ = Visit::visit(scale, &mut find);
    find.0
}
