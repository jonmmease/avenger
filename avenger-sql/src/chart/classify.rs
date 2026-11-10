//! The classification of the checked expressions of a chart call: the level of each
//! subexpression, which names the first stage that can compute it, and the syntax rules of the
//! calls in them (V1, V6, V8).

use std::ops::ControlFlow;

use datafusion::{
    common::Result,
    sql::sqlparser::{
        ast::{
            Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentClause,
            FunctionArguments, Ident, Spanned, Value, Visit, Visitor, WindowType,
        },
        tokenizer::Span,
    },
};

use crate::{
    error::{at, at_with_help},
    registry::Registry,
    syntax::function_name,
};

/// The level of a subexpression: the first stage that can compute it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Level {
    /// A literal, or an expression over constants: it stays in stage 5.
    Constant,
    /// An expression of the input rows: its consumer computes it.
    Row,
    /// A discretizer: stage 1.
    Discretized,
    /// An aggregate: stage 2.
    Aggregated,
    /// A stack or a window: stage 4.
    Windowed,
}

/// A discretizer of stage 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Discretizer {
    Bin,
    TimeUnit,
    Lump,
}

impl Discretizer {
    /// The discretizer that a function named `name` is.
    fn of(name: &str) -> Option<Self> {
        match name {
            "bin" => Some(Self::Bin),
            "timeunit" => Some(Self::TimeUnit),
            "lump" => Some(Self::Lump),
            _ => None,
        }
    }

    /// The suffix of the name of its column, such as `_bin` in `horsepower_bin`.
    pub(super) fn suffix(self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::TimeUnit => "unit",
            Self::Lump => "lump",
        }
    }

    /// What it computes, for its errors.
    fn computes(self) -> &'static str {
        match self {
            Self::Bin => "its bins",
            Self::TimeUnit => "its units",
            Self::Lump => "its categories",
        }
    }

    /// The verb of its help.
    fn verb(self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::TimeUnit => "truncate",
            Self::Lump => "lump",
        }
    }
}

/// The kind of a call in a checked expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CallKind {
    Discretizer(Discretizer),
    Aggregate,
    /// `stack`, or a window function or an aggregate with `OVER`.
    Window,
}

/// The kind of the call `function`, by the functions of `registry`.
pub(super) fn call_kind(function: &Function, registry: &dyn Registry) -> Option<CallKind> {
    let name = function_name(function)?;
    if let Some(discretizer) = Discretizer::of(&name) {
        return Some(CallKind::Discretizer(discretizer));
    }
    if name == "stack" || function.over.is_some() {
        return Some(CallKind::Window);
    }
    if registry.is_aggregate(&name) {
        return Some(CallKind::Aggregate);
    }
    registry.is_window(&name).then_some(CallKind::Window)
}

/// The level of `expr`: the highest level of its column references and of the calls that it
/// contains outside their arguments.
pub(super) fn level(expr: &Expr, registry: &dyn Registry) -> Level {
    let mut visitor = LevelOf {
        registry,
        calls: Vec::new(),
        level: Level::Constant,
    };
    let _ = expr.visit(&mut visitor);
    visitor.level
}

/// The expressions of the arguments of `function`.
pub(super) fn arguments(function: &Function) -> impl Iterator<Item = &Expr> {
    let args: &[FunctionArg] = match &function.args {
        FunctionArguments::List(list) => &list.args,
        _ => &[],
    };
    args.iter().filter_map(|argument| match argument {
        FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))
        | FunctionArg::Named {
            arg: FunctionArgExpr::Expr(expr),
            ..
        }
        | FunctionArg::ExprNamed {
            arg: FunctionArgExpr::Expr(expr),
            ..
        } => Some(expr),
        _ => None,
    })
}

/// The first argument of `function`, when it is an expression.
pub(super) fn first_argument(function: &Function) -> Option<&Expr> {
    match &function.args {
        FunctionArguments::List(list) => match list.args.first() {
            Some(FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))) => Some(expr),
            _ => None,
        },
        _ => None,
    }
}

/// Collects the level of an expression: the stack marks the calls whose arguments the visit is
/// in, which count for their call and not for the expression.
struct LevelOf<'a> {
    registry: &'a dyn Registry,
    calls: Vec<bool>,
    level: Level,
}

impl Visitor for LevelOf<'_> {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        let inside = self.calls.iter().any(|call| *call);
        let kind = match expr {
            Expr::Function(function) => call_kind(function, self.registry),
            _ => None,
        };
        if !inside {
            let here = match (expr, kind) {
                (_, Some(CallKind::Discretizer(_))) => Level::Discretized,
                (_, Some(CallKind::Aggregate)) => Level::Aggregated,
                (_, Some(CallKind::Window)) => Level::Windowed,
                (Expr::Identifier(_) | Expr::CompoundIdentifier(_) | Expr::Subquery(_), _) => {
                    Level::Row
                }
                _ => Level::Constant,
            };
            self.level = self.level.max(here);
        }
        self.calls.push(kind.is_some());
        ControlFlow::Continue(())
    }

    fn post_visit_expr(&mut self, _expr: &Expr) -> ControlFlow<()> {
        self.calls.pop();
        ControlFlow::Continue(())
    }
}

/// Checks the calls of the checked expression `expr` (V1, V6, V8): the levels of their arguments,
/// the windows of the discretizers, `OVER` on a window function, and the constant options of the
/// transforms.
pub(super) fn check_calls(expr: &Expr, registry: &dyn Registry) -> Result<()> {
    let mut visitor = Calls { registry };
    match expr.visit(&mut visitor) {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(()),
    }
}

struct Calls<'a> {
    registry: &'a dyn Registry,
}

impl Visitor for Calls<'_> {
    type Break = datafusion::common::DataFusionError;

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        let Expr::Function(function) = expr else {
            return ControlFlow::Continue(());
        };
        match check_call(function, self.registry) {
            Ok(()) => ControlFlow::Continue(()),
            Err(error) => ControlFlow::Break(error),
        }
    }
}

/// Checks one call of a checked expression.
fn check_call(function: &Function, registry: &dyn Registry) -> Result<()> {
    let name = function_name(function).unwrap_or_default();
    match call_kind(function, registry) {
        Some(CallKind::Discretizer(discretizer)) => {
            let Some(argument) = first_argument(function) else {
                return Err(at(
                    function.name.span(),
                    format!("{name}() takes a value first"),
                ));
            };
            let argument_level = level(argument, registry);
            if argument_level > Level::Discretized {
                let (later, help) = match argument_level {
                    Level::Aggregated => (
                        "the aggregates",
                        format!(
                            "compute the aggregate before the chart with |> AGGREGATE, then {} its column",
                            discretizer.verb()
                        ),
                    ),
                    _ => (
                        "the windows",
                        format!(
                            "compute the window before the chart with |> EXTEND, then {} its column",
                            discretizer.verb()
                        ),
                    ),
                };
                return Err(at_with_help(
                    argument.span(),
                    format!(
                        "`{argument}` cannot be an argument of `{name}`, because {name} computes {} before {later}",
                        discretizer.computes()
                    ),
                    help,
                ));
            }
            check_options(function, &name)?;
            check_discretizer_window(function, &name, registry)
        }
        Some(CallKind::Aggregate) => {
            let mut items: Vec<&Expr> = arguments(function).collect();
            items.extend(function.filter.as_deref());
            if let FunctionArguments::List(list) = &function.args {
                for clause in &list.clauses {
                    if let FunctionArgumentClause::OrderBy(order) = clause {
                        items.extend(order.iter().map(|item| &item.expr));
                    }
                }
            }
            for item in items {
                if level(item, registry) > Level::Discretized {
                    return Err(at_with_help(
                        item.span(),
                        format!(
                            "`{item}` cannot be an argument of `{name}`, because {name} aggregates the rows before the aggregates and the windows"
                        ),
                        format!("compute it before the chart, then aggregate its column with {name}"),
                    ));
                }
            }
            Ok(())
        }
        Some(CallKind::Window) => {
            if function.over.is_none() && name != "stack" {
                return Err(at_with_help(
                    function.name.span(),
                    format!("{name}() is a window function, so it needs OVER"),
                    "OVER panel, for each panel",
                ));
            }
            for argument in arguments(function) {
                if level(argument, registry) > Level::Aggregated {
                    return Err(at_with_help(
                        argument.span(),
                        format!(
                            "`{argument}` cannot be an argument of `{name}`, because {name} computes beside the other windows"
                        ),
                        "compute the window before the chart with |> EXTEND",
                    ));
                }
            }
            if let Some(WindowType::WindowSpec(spec)) = &function.over {
                let items =
                    (spec.partition_by.iter()).chain(spec.order_by.iter().map(|item| &item.expr));
                for item in items {
                    if level(item, registry) > Level::Aggregated {
                        return Err(at(
                            item.span(),
                            format!("`{item}` cannot order or partition `{name}`, because {name} computes beside the other windows"),
                        ));
                    }
                }
                if name == "stack" && spec.window_frame.is_some() {
                    return Err(at(function.name.span(), "a stack takes no frame"));
                }
            }
            Ok(())
        }
        None => Ok(()),
    }
}

/// The options of a transform, its arguments after the first, are plan-time constants (V8),
/// except the `weight` and `keep` of `lump`, which are SQL expressions that it reads as text.
fn check_options(function: &Function, name: &str) -> Result<()> {
    let FunctionArguments::List(list) = &function.args else {
        return Ok(());
    };
    for argument in list.args.iter().skip(1) {
        let (option, value) = match argument {
            FunctionArg::Named {
                name: option,
                arg: FunctionArgExpr::Expr(value),
                ..
            } => (Some(option.value.to_lowercase()), value),
            FunctionArg::Unnamed(FunctionArgExpr::Expr(value)) => (None, value),
            _ => continue,
        };
        if name == "lump" && matches!(option.as_deref(), Some("weight" | "keep")) {
            continue;
        }
        let what = match &option {
            Some(option) => format!("option `{option}` of {name}()"),
            None => format!("an option of {name}()"),
        };
        constant(value, &what)?;
    }
    Ok(())
}

/// `expr` is a plan-time constant: it reads no column, no parameter, and no subquery (V8). `what`
/// names the option.
pub(super) fn constant(expr: &Expr, what: &str) -> Result<()> {
    let mut visitor = NotConstant;
    match expr.visit(&mut visitor) {
        ControlFlow::Break((span, why)) => Err(at(
            span,
            format!("{what} needs a plan-time constant; {why}"),
        )),
        ControlFlow::Continue(()) => Ok(()),
    }
}

/// Breaks at a column, a parameter, or a subquery, with its span and why.
struct NotConstant;

impl Visitor for NotConstant {
    type Break = (Span, String);

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Identifier(_) | Expr::CompoundIdentifier(_) => {
                ControlFlow::Break((expr.span(), format!("`{expr}` is a column")))
            }
            Expr::Value(value) if matches!(value.value, Value::Placeholder(_)) => {
                ControlFlow::Break((expr.span(), format!("`{expr}` is a parameter")))
            }
            Expr::Subquery(_) => {
                ControlFlow::Break((expr.span(), "a subquery is not a constant".to_string()))
            }
            _ => ControlFlow::Continue(()),
        }
    }
}

/// The window of a discretizer is `()`, `panel`, or a clause with only a `PARTITION BY` list of
/// items of level 0 or 1 (V1, V6).
fn check_discretizer_window(
    function: &Function,
    name: &str,
    registry: &dyn Registry,
) -> Result<()> {
    let form = format!("{name} takes OVER (), OVER panel, or a window of only a PARTITION BY list");
    match &function.over {
        None => Ok(()),
        Some(WindowType::NamedWindow(window)) => window_name(window),
        Some(WindowType::WindowSpec(spec)) => {
            if let Some(window) = &spec.window_name {
                window_name(window)?;
                if let Some(item) = spec.partition_by.first() {
                    return Err(at(
                        item.span(),
                        format!(
                            "the window `{}` has its partition, so it takes no PARTITION BY",
                            window.value
                        ),
                    ));
                }
            }
            if let Some(item) = spec.order_by.first() {
                return Err(at(item.expr.span(), form));
            }
            if spec.window_frame.is_some() {
                return Err(at(function.name.span(), form));
            }
            for item in &spec.partition_by {
                if level(item, registry) > Level::Discretized {
                    return Err(at(
                        item.span(),
                        format!("`{item}` cannot partition `{name}`, because {name} computes before the aggregates and the windows"),
                    ));
                }
            }
            Ok(())
        }
    }
}

/// The named windows of the mark: `panel`, the partition of the call, `stacks`, which adds the key
/// channel, and `stacked`, which adds the implied stack order.
pub(super) const PANEL: &str = "panel";
pub(super) const STACKS: &str = "stacks";
pub(super) const STACKED: &str = "stacked";

/// The error of a window name that the mark does not define (V6).
pub(super) fn unknown_window(window: &Ident) -> datafusion::common::DataFusionError {
    at(
        window.span,
        format!(
            "no window `{}` in this chart; the windows are panel, stacks, and stacked",
            window.value
        ),
    )
}

/// `window` names the window of a discretizer, `panel` (V6): a discretizer computes before the
/// stacks.
fn window_name(window: &Ident) -> Result<()> {
    match window.value.to_lowercase().as_str() {
        PANEL => Ok(()),
        STACKS | STACKED => Err(at(
            window.span,
            format!(
                "the window `{}` holds the stacks, which a discretizer computes before; a discretizer takes OVER (), OVER panel, or a PARTITION BY list",
                window.value
            ),
        )),
        _ => Err(unknown_window(window)),
    }
}
