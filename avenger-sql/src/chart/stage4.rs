//! Stage 4: the stacks and the window functions of the channels and of the draw order, in one
//! `EXTEND` after stage 2, over the columns of the earlier stages.
//!
//! The mark names three windows (`statistics/stacking.md` of the spec):
//!
//! - `panel`: `PARTITION BY` the partition columns.
//! - `stacks`: `panel` and the column of the key channel.
//! - `stacked`: `stacks` with the implied order, the default window of `stack`.
//!
//! A window resolves by the inheritance of SQL: a name alone, or a name with an `ORDER BY` and a
//! frame on a window without an order. The implied order follows the legends: the domain order of
//! each series channel, reversed in a vertical stack so that the first category of the legend is
//! the top segment, then the tiebreak, the argument of the stack and each other channel column,
//! ascending.

use avenger_chart::functions::Kind;
use datafusion::{
    common::{DFSchema, Result},
    sql::sqlparser::{
        ast::{
            Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList, FunctionArguments,
            Ident, ObjectName, OrderByExpr, OrderByOptions, Spanned, WindowSpec, WindowType,
        },
        tokenizer::Span,
    },
};

use super::{
    call::MarkCall,
    classify::{first_argument, unknown_window, CallKind, Level, PANEL, STACKED, STACKS},
    lifting::{calls, column_name, replaced, Added, Env, Names},
    roles::{self, Roles, SERIES},
};
use crate::{error::at, syntax::function_name};

/// The stage 4 columns of the call: one for each distinct stack and window function, with its
/// arguments and its window items over the columns of `stage5`, and its named windows resolved.
#[allow(clippy::too_many_arguments)]
pub(super) fn windows(
    call: &MarkCall,
    levels: &[Level],
    partition: &[Expr],
    stage5: &[(Expr, Expr)],
    schema: &DFSchema,
    discretized: &[(Expr, Expr)],
    names: &mut Names,
    env: &mut dyn Env,
) -> Result<Vec<Added>> {
    let uses: Vec<(&str, &Expr)> = (call.channels.iter().map(|c| (c.channel.name, &c.value)))
        .chain(call.order.iter().map(|item| ("order", &item.expr)))
        .collect();
    let mut found: Vec<(&str, Expr)> = Vec::new();
    for (channel, value) in &uses {
        for from in calls(value, env, &|kind| kind == CallKind::Window, &|kind| {
            matches!(kind, CallKind::Discretizer(_))
        }) {
            if !found.iter().any(|(_, f)| f == &from) {
                found.push((channel, from));
            }
        }
    }
    if found.is_empty() {
        return Ok(Vec::new());
    }
    let needs_roles = found.iter().any(|(_, from)| match from {
        Expr::Function(function) => is_stack(function) || names_stacks(function),
        _ => false,
    });
    let roles = match needs_roles {
        true => roles::decide(call, levels, schema, discretized, env)?,
        false => None,
    };
    let mut added = Vec::new();
    for (channel, from) in found {
        let Expr::Function(function) = &from else {
            unreachable!("a window is a call")
        };
        let stack = is_stack(function);
        if stack {
            check_roles(call, roles.as_ref(), function)?;
        }
        let mut stage = function.clone();
        super::lifting::replace(&mut stage.args, stage5);
        let argument = first_argument(&stage).cloned();
        let windows = Named {
            call,
            levels,
            partition,
            stage5,
            roles: roles.as_ref(),
            argument: argument.clone(),
        };
        let mut window = windows.resolve(function, stack)?;
        for item in window.partition_by.iter_mut() {
            *item = replaced(item, stage5);
        }
        for item in window.order_by.iter_mut() {
            item.expr = replaced(&item.expr, stage5);
        }
        // A stack window whose `PARTITION BY` lacks a partition column crosses panels, which the
        // spec reports as a warning. DataFusion 55.2 keeps the warnings of its SQL planner to
        // itself (`SqlToRel::add_warning` is crate-private), so the mark planner reports none.
        if stack && window.order_by.is_empty() {
            return Err(at(
                function.name.span(),
                "a stack needs an order: stack(n) OVER (stacks ORDER BY ...)",
            ));
        }
        stage.over = Some(WindowType::WindowSpec(window));
        let name = function_name(function).unwrap_or_default();
        let argument = argument.as_ref().and_then(column_name);
        let base = match (stack, argument) {
            (true, Some(argument)) => format!("{argument}_stack"),
            (true, None) => format!("{channel}_stack"),
            (false, Some(argument)) => format!("{name}_{argument}_window"),
            (false, None) => format!("{name}_window"),
        };
        let ident = names.fresh(&base, from.span());
        added.push(Added {
            from: from.clone(),
            name: ident,
            stage: Expr::Function(stage),
        });
    }
    Ok(added)
}

/// The window of the discretizer `function`: `OVER ()` without one, or the written window with
/// `panel` resolved. V6 has checked that it names no other window.
pub(super) fn discretizer_window(
    call: &MarkCall,
    partition: &[Expr],
    function: &Function,
) -> Result<WindowSpec> {
    let named = Named {
        call,
        levels: &[],
        partition,
        stage5: &[],
        roles: None,
        argument: None,
    };
    named.resolve(function, false)
}

/// Whether `function` is a call of `stack`.
fn is_stack(function: &Function) -> bool {
    function_name(function).as_deref() == Some("stack")
}

/// Whether the window of `function` names `stacks` or `stacked`.
fn names_stacks(function: &Function) -> bool {
    let name = match &function.over {
        Some(WindowType::NamedWindow(name)) => Some(name),
        Some(WindowType::WindowSpec(spec)) => spec.window_name.as_ref(),
        None => None,
    };
    name.is_some_and(|name| {
        let name = name.value.to_lowercase();
        name == STACKS || name == STACKED
    })
}

/// A stack needs a key channel and a measure channel (V11). A polar bar whose `theta` is the
/// measure and that has no `radius` is an arc, which stacks without a key.
fn check_roles(call: &MarkCall, roles: Option<&Roles>, stack: &Function) -> Result<()> {
    let span = stack.name.span();
    let mark = call.mark.name;
    let Some(roles) = roles else {
        return Err(at(
            span,
            format!(
                "stack needs a key channel and a measure channel, and {mark}() has no orientation"
            ),
        ));
    };
    let (first, second) = match call.mark.name.starts_with("polar_") {
        true => ("theta", "radius"),
        false => ("x", "y"),
    };
    let (key, measure) = match roles.vertical {
        true => (first, second),
        false => (second, first),
    };
    if roles.measure.is_none() {
        return Err(at(
            span,
            format!("stack needs a measure channel; this {mark} has no {measure}"),
        ));
    }
    if roles.key.is_none() && !roles.arc {
        return Err(at(
            span,
            format!("stack needs a key channel; this {mark} has no {key}"),
        ));
    }
    Ok(())
}

/// The named windows of a call, for one stack or window function, whose argument breaks the ties
/// of the implied order.
struct Named<'a> {
    call: &'a MarkCall,
    levels: &'a [Level],
    partition: &'a [Expr],
    stage5: &'a [(Expr, Expr)],
    roles: Option<&'a Roles>,
    argument: Option<Expr>,
}

impl Named<'_> {
    /// The column of the key channel, over the columns of the earlier stages.
    fn key(&self) -> Option<Expr> {
        let key = self.roles?.key?;
        Some(replaced(&self.call.channels[key].value, self.stage5))
    }

    /// The window `name`: its partition and its order.
    fn named(&self, name: &Ident) -> Result<(Vec<Expr>, Vec<OrderByExpr>)> {
        let panel = self.partition.to_vec();
        match name.value.to_lowercase().as_str() {
            PANEL => Ok((panel, Vec::new())),
            STACKS => Ok((self.stacks(name)?, Vec::new())),
            STACKED => Ok((self.stacks(name)?, self.implied())),
            _ => Err(unknown_window(name)),
        }
    }

    /// The partition of `stacks`: the partition columns and the key column.
    fn stacks(&self, name: &Ident) -> Result<Vec<Expr>> {
        let mut partition = self.partition.to_vec();
        match (self.roles, self.key()) {
            (Some(_), Some(key)) => partition.push(key),
            (Some(roles), None) if roles.arc => {}
            _ => {
                return Err(at(
                    name.span,
                    format!(
                        "the window `{}` needs a key channel, which this {} has not",
                        name.value, self.call.mark.name
                    ),
                ))
            }
        }
        Ok(partition)
    }

    /// The window of `function` with its names resolved. A stack without a window has `stacked`.
    fn resolve(&self, function: &Function, stack: bool) -> Result<WindowSpec> {
        let spec = |partition_by, order_by, window_frame| WindowSpec {
            window_name: None,
            partition_by,
            order_by,
            window_frame,
        };
        match &function.over {
            None => {
                let stacked = Ident::with_span(function.name.span(), STACKED);
                let (partition, order) = match stack {
                    true => self.named(&stacked)?,
                    false => (Vec::new(), Vec::new()),
                };
                Ok(spec(partition, order, None))
            }
            Some(WindowType::NamedWindow(name)) => {
                let (partition, order) = self.named(name)?;
                Ok(spec(partition, order, None))
            }
            Some(WindowType::WindowSpec(written)) => match &written.window_name {
                None => Ok(written.clone()),
                Some(name) => {
                    if let Some(item) = written.partition_by.first() {
                        return Err(at(
                            item.span(),
                            format!(
                                "the window `{}` has its partition, so it takes no PARTITION BY",
                                name.value
                            ),
                        ));
                    }
                    let (partition, order) = self.named(name)?;
                    if let (false, Some(item)) = (order.is_empty(), written.order_by.first()) {
                        return Err(at(
                            item.expr.span(),
                            format!(
                                "the window `{}` has its order, so it takes no ORDER BY",
                                name.value
                            ),
                        ));
                    }
                    let order = match order.is_empty() {
                        true => written.order_by.clone(),
                        false => order,
                    };
                    Ok(spec(partition, order, written.window_frame.clone()))
                }
            },
        }
    }

    /// The implied order of `stacked`: the domain order of each series channel, reversed in a
    /// vertical stack, then the argument and each other channel column, ascending.
    fn implied(&self) -> Vec<OrderByExpr> {
        let vertical = self.roles.is_some_and(|roles| roles.vertical);
        let key = self.key();
        let mut order = Vec::new();
        let mut series: Vec<&Expr> = Vec::new();
        for name in SERIES {
            let Some((index, channel)) = (self.call.channels.iter().enumerate())
                .find(|(_, channel)| channel.channel.name == name)
            else {
                continue;
            };
            if !matches!(self.levels[index], Level::Row | Level::Discretized)
                || series.contains(&&channel.value)
            {
                continue;
            }
            series.push(&channel.value);
            let column = replaced(&channel.value, self.stage5);
            let lump = matches!(&channel.value, Expr::Function(f) if function_name(f).as_deref() == Some("lump"));
            let items = match (lump, roles::domain_list(channel)) {
                (true, _) => vec![
                    ascending(Expr::IsNotNull(Box::new(field(&column, "other")))),
                    descending(field(&column, "weight")),
                    ascending(field(&column, "value")),
                ],
                (false, Some(list)) => vec![ascending(call_of(
                    "array_position",
                    column.span(),
                    vec![list, column.clone()],
                ))],
                (false, None) => vec![ascending(column.clone())],
            };
            order.extend(items.into_iter().map(|item| match vertical {
                true => reversed(item),
                false => item,
            }));
        }
        let series: Vec<Expr> = series
            .iter()
            .map(|value| replaced(value, self.stage5))
            .collect();
        let mut tiebreak: Vec<Expr> = self.argument.iter().cloned().collect();
        for channel in &self.call.channels {
            if channel
                .wrapper
                .as_ref()
                .is_some_and(|(kind, _)| *kind == Kind::Direct)
            {
                continue;
            }
            let value = replaced(&channel.value, self.stage5);
            let is_column = matches!(value, Expr::Identifier(_) | Expr::CompoundIdentifier(_));
            if is_column
                && !self.partition.contains(&value)
                && key.as_ref() != Some(&value)
                && !series.contains(&value)
                && !tiebreak.contains(&value)
            {
                tiebreak.push(value);
            }
        }
        order.extend(tiebreak.into_iter().map(ascending));
        order
    }
}

/// The field `name` of the struct column `column`.
fn field(column: &Expr, name: &str) -> Expr {
    let field = Ident::new(name);
    match column {
        Expr::Identifier(ident) => Expr::CompoundIdentifier(vec![ident.clone(), field]),
        Expr::CompoundIdentifier(parts) => {
            Expr::CompoundIdentifier(parts.iter().cloned().chain([field]).collect())
        }
        other => Expr::CompoundIdentifier(vec![Ident::new(other.to_string()), field]),
    }
}

/// `expr ASC NULLS LAST`.
fn ascending(expr: Expr) -> OrderByExpr {
    OrderByExpr {
        expr,
        options: OrderByOptions {
            asc: Some(true),
            nulls_first: Some(false),
        },
        with_fill: None,
    }
}

/// `expr DESC NULLS LAST`.
fn descending(expr: Expr) -> OrderByExpr {
    OrderByExpr {
        expr,
        options: OrderByOptions {
            asc: Some(false),
            nulls_first: Some(false),
        },
        with_fill: None,
    }
}

/// `item` in the reverse order.
fn reversed(item: OrderByExpr) -> OrderByExpr {
    OrderByExpr {
        options: OrderByOptions {
            asc: item.options.asc.map(|asc| !asc),
            nulls_first: item.options.nulls_first.map(|first| !first),
        },
        ..item
    }
}

/// A call of `name` with the positional arguments `args`.
fn call_of(name: &str, span: Span, args: Vec<Expr>) -> Expr {
    Expr::Function(Function {
        name: ObjectName::from(vec![Ident::with_span(span, name)]),
        uses_odbc_syntax: false,
        parameters: FunctionArguments::None,
        args: FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: None,
            args: (args.into_iter())
                .map(|arg| FunctionArg::Unnamed(FunctionArgExpr::Expr(arg)))
                .collect(),
            clauses: Vec::new(),
        }),
        filter: None,
        null_treatment: None,
        over: None,
        within_group: Vec::new(),
    })
}
