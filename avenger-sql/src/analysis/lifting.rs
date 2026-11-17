//! Statistics in the channels of a mark call: a mark call in an item of a pipe `AGGREGATE` whose
//! channels hold aggregates or windows plans as a branch of the operator, the staged query that a
//! user could type for it.
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE bar(x => species, y => COUNT(*)) AS chart GROUP BY island
//! ```
//!
//! plans as
//!
//! ```sql
//! FROM penguins
//! |> AGGREGATE COUNT(*) AS `COUNT(*)` GROUP BY island, species
//! |> AGGREGATE bar(x => species, y => `COUNT(*)`) AS chart GROUP BY island
//! ```
//!
//! In a call that holds an aggregate, each channel that holds neither an aggregate nor a window
//! on the aggregated rows is a key, and the call aggregates once for each combination of the
//! operator's groups and its keys. A measure reads only keys, groups, and constants outside its
//! aggregates. A window's level follows from what it reads: a window that reads an aggregate, or
//! names `position` or `series`, runs on the aggregated rows, and any other runs on the rows.
//! Each channel's column takes the text of its value, which titles its axis or its legend. The
//! pass plans a branch for each such call over the operator's input, aggregated to one row for
//! each group, and joins the branches on the groups.
//!
//! The three windows of a call, `panel`, `position`, and `series`, partition by the groups, and
//! by the position keys or the series keys. A window over one of them that reads the order of its
//! rows takes the window's default order after its own, where GoogleSQL leaves the order
//! unspecified or requires one.

use avenger_chart::functions::{channel_function, Kind};
use avenger_datafusion_transforms::impute::{self, ImputeCall, Named};
use avenger_sql_syntax::{ast, ast::AstNode, TextRange};
use datafusion::{
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, DFSchema, ExprSchema, Result,
    },
    logical_expr::{
        expr::{AggregateFunction, ScalarFunction, WindowFunction},
        lit,
        utils::{find_aggregate_exprs, find_window_exprs},
        when, Expr, ExprSchemable, LogicalPlan, LogicalPlanBuilder, SortExpr, WindowFrameUnits,
    },
};

use super::{
    print::{self, PrintedCall, PrintedChannel},
    query::regroup,
    rows, Pass,
};
use crate::{
    functions::kind,
    library::{Library, WindowOrder},
};

/// The qualifier of the column that stands for a window of a mark call in the partition of a
/// window that names it, until the branch resolves it.
const MARK_WINDOW: &str = "__mark_window";

/// The channels whose keys are positions, and whose keys are series.
const POSITIONS: &[&str] = &["x", "y", "x2", "y2", "theta", "radius", "theta2", "radius2"];
const SERIES: &[&str] = &[
    "color",
    "fill",
    "stroke",
    "stroke_dash",
    "pattern",
    "detail",
];

/// The marks that draw paths, whose stacks fill the keys that a series lacks with zero.
const PATHS: &[&str] = &["line", "area", "trail", "polar_line", "polar_area"];

/// The partition of the window `name`, `panel`, `position`, or `series`, of a mark call, which the
/// branch resolves.
pub(crate) fn mark_window(name: &str) -> Expr {
    Expr::Column(Column::new(Some(MARK_WINDOW), name.to_ascii_lowercase()))
}

/// The window of the mark call that the partition of `window` names.
fn named(window: &WindowFunction) -> Option<&str> {
    (window.params.partition_by.iter()).find_map(|expr| match expr {
        Expr::Column(column)
            if (column.relation.as_ref()).is_some_and(|r| r.table() == MARK_WINDOW) =>
        {
            Some(column.name.as_str())
        }
        _ => None,
    })
}

/// Whether `expr` holds a window that names `position` or `series`.
fn names_late(expr: &Expr) -> bool {
    let mut late = false;
    let _ = expr.apply(|expr| {
        if let Expr::WindowFunction(window) = expr {
            late |= matches!(named(window), Some("position" | "series"));
        }
        Ok(TreeNodeRecursion::Continue)
    });
    late
}

/// Whether `expr` holds a window that names one of the windows of the call.
fn names_window(expr: &Expr) -> bool {
    let mut found = false;
    let _ = expr.apply(|expr| {
        if let Expr::WindowFunction(window) = expr {
            found |= named(window).is_some();
        }
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

/// A mark call whose channels compute statistics, which an `AGGREGATE` plans as a branch.
#[derive(Clone, Debug)]
pub(crate) struct Lifting {
    /// The planned call, as the items of the `AGGREGATE` hold it.
    pub mark: Expr,
    /// The name of the column of each channel value, the value inside its channel function and
    /// `datum` or `value`.
    pub names: Vec<(Expr, String)>,
    /// The keys and measures that the windows of the call read.
    pub windows: Windows,
    /// Whether the call aggregates.
    pub aggregating: bool,
    /// Whether the call draws a path whose measure stacks over `position`, which fills the keys
    /// that a series lacks with zero.
    pub fill: bool,
    /// The text of the call, which the printer of the stages reads.
    pub printed: PrintedCall,
}

/// What the windows `panel`, `position`, and `series` of a mark call partition and order by.
#[derive(Clone, Debug, Default)]
pub(crate) struct Windows {
    /// The keys of the position channels, the series channels, and the other channels, without the
    /// channels that name one of the windows.
    position: Vec<Expr>,
    series: Vec<Expr>,
    other: Vec<Expr>,
    /// The measures without windows, which end a default order.
    measures: Vec<Expr>,
    /// Whether the call's measure is in `y`, so that `position` orders the series from the top.
    reversed: bool,
    /// The names of the window functions that read the order of their rows.
    rows: Vec<String>,
}

/// A channel value of a mark call as the user wrote it: its channel, the planned value inside its
/// channel function and `datum` or `value`, its range, the text that names its column, and its
/// complete text.
pub(crate) struct Written {
    pub channel: String,
    /// The syntax of the value, inside its channel function and `datum` or `value`.
    pub node: ast::Expr,
    pub value: Expr,
    pub range: TextRange,
    pub text: String,
    pub full: String,
}

/// The value of a channel argument, inside its channel function and `datum` or `value`.
pub(crate) fn channel_value(arg: &Expr) -> &Expr {
    kind::unwrap(arg).value
}

/// The first argument of the call `value` when it wraps the value of the channel `channel`: the
/// channel's function, which only the outermost call can be, when `function` holds, or `datum`
/// or `value`.
fn wrapped(value: &ast::Expr, channel: &str, function: bool) -> Option<ast::Expr> {
    let ast::Expr::CallExpr(call) = value else {
        return None;
    };
    let names = call.path()?.names()?;
    let [name] = names.as_slice() else {
        return None;
    };
    let name = name.to_ascii_lowercase();
    let wraps = Kind::of(&name).is_some()
        || (function && name == channel && channel_function(channel).is_some());
    if !wraps {
        return None;
    }
    match call.arg_list()?.arguments().next()? {
        ast::Argument::Arg(arg) => arg.expr(),
        _ => None,
    }
}

/// The syntax of the value `value` of the channel `channel`, inside its channel function and
/// `datum` or `value`.
pub(crate) fn channel_node(value: &ast::Expr, channel: &str) -> ast::Expr {
    let mut value = value.clone();
    let mut function = true;
    while let Some(inner) = wrapped(&value, channel, function) {
        function = false;
        value = inner;
    }
    value
}

/// The text that names the column of the value `value` of the channel `channel`: the text inside
/// its channel function and `datum` or `value`, and inside each call of a function that `library`
/// titles by its first argument, as `bin(body_mass_g)` is titled `body_mass_g`.
pub(crate) fn channel_title(value: &ast::Expr, channel: &str, library: &Library) -> String {
    let mut value = channel_node(value, channel);
    while let ast::Expr::CallExpr(call) = &value {
        let titled = (call.path().and_then(|path| path.names())).is_some_and(|names| {
            match names.as_slice() {
                [name] => library.get(name).is_some_and(|f| f.titled_by_argument),
                _ => false,
            }
        });
        let first =
            (call.arg_list().and_then(|list| list.arguments().next())).and_then(|argument| {
                match argument {
                    ast::Argument::Arg(arg) => arg.expr(),
                    _ => None,
                }
            });
        match (titled, first) {
            (true, Some(first)) => value = first,
            _ => break,
        }
    }
    collapsed(&value)
}

/// The text of `value` with its spaces collapsed.
fn collapsed(value: &ast::Expr) -> String {
    let text = value.syntax().text().to_string();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of the value `value` of the channel `channel`, inside its channel function and
/// `datum` or `value`, with its spaces collapsed.
pub(crate) fn channel_text(value: &ast::Expr, channel: &str) -> String {
    collapsed(&channel_node(value, channel))
}

/// `expr` as an error names it: a column by its name, without its qualifier.
fn display(expr: &Expr) -> String {
    match expr {
        Expr::Column(column) => column.name.clone(),
        expr => expr.human_display().to_string(),
    }
}

/// Whether `expr` holds an aggregate function.
fn aggregates(expr: &Expr) -> bool {
    !find_aggregate_exprs([expr]).is_empty()
}

/// Whether `expr` holds a window function.
fn windows(expr: &Expr) -> bool {
    !find_window_exprs([expr]).is_empty()
}

/// The windows of `parts` whose arguments and keys hold no window, each once.
fn innermost_windows(parts: &[Expr]) -> Vec<Expr> {
    let mut found: Vec<Expr> = Vec::new();
    for part in parts {
        let _ = part.apply(|expr| {
            if matches!(expr, Expr::WindowFunction(_))
                && window_count(expr) == 1
                && !found.contains(expr)
            {
                found.push(expr.clone());
            }
            Ok(TreeNodeRecursion::Continue)
        });
    }
    found
}

/// The number of windows in `expr`, itself included.
fn window_count(expr: &Expr) -> usize {
    let mut count = 0;
    let _ = expr.apply(|expr| {
        if matches!(expr, Expr::WindowFunction(_)) {
            count += 1;
        }
        Ok(TreeNodeRecursion::Continue)
    });
    count
}

/// Whether `expr` reads a column.
fn reads_columns(expr: &Expr) -> bool {
    !expr.column_refs().is_empty()
}

/// Whether a channel value is a part of a branch rather than a constant: it reads a column, or
/// holds an aggregate or a window.
fn part(expr: &Expr) -> bool {
    reads_columns(expr) || aggregates(expr) || windows(expr)
}

/// `expr` with each aggregate and each of `known` replaced by a constant, so the columns that
/// remain are those that it reads outside them.
fn outside(expr: &Expr, known: &[&Expr]) -> Expr {
    expr.clone()
        .transform_down(|expr| {
            if matches!(expr, Expr::AggregateFunction(_)) || known.contains(&&expr) {
                return Ok(Transformed::new(lit(true), true, TreeNodeRecursion::Jump));
            }
            Ok(Transformed::no(expr))
        })
        .map(|transformed| transformed.data)
        .unwrap_or_else(|_| expr.clone())
}

/// Whether `expr` holds a window that names `position`, by its text or by a default window.
fn names_position(expr: &Expr) -> bool {
    let mut found = false;
    let _ = expr.apply(|expr| {
        if let Expr::WindowFunction(window) = expr {
            found |= named(window) == Some("position");
        }
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

/// The keys and measures of a call at one stage of its branch, which the windows of the call
/// partition and order by.
struct Staged<'a> {
    groups: &'a [Expr],
    position: &'a [Expr],
    series: &'a [Expr],
    other: &'a [Expr],
    measures: &'a [Expr],
}

impl Windows {
    /// `expr` with each window that names one of `names` resolved: its partition, and where the
    /// window reads the order of its rows, its own order then the window's default order. Only
    /// the order that GoogleSQL leaves unspecified changes: a function that reads only the peers
    /// of its order, or none, keeps its order as written.
    fn resolve(&self, expr: Expr, names: &[&str], staged: &Staged) -> Result<Expr> {
        let ascending = |expr: &Expr| SortExpr::new(expr.clone(), true, false);
        let series = |expr: &Expr| match self.reversed {
            true => SortExpr::new(expr.clone(), false, true),
            false => ascending(expr),
        };
        expr.transform_up(|expr| {
            let Expr::WindowFunction(window) = &expr else {
                return Ok(Transformed::no(expr));
            };
            let Some(name) = named(window).filter(|name| names.contains(name)) else {
                return Ok(Transformed::no(expr));
            };
            let name = name.to_string();
            let mut window: WindowFunction = window.as_ref().clone();
            let mut partition: Vec<Expr> = staged.groups.to_vec();
            match name.as_str() {
                "position" => partition.extend(staged.position.iter().cloned()),
                "series" => partition.extend(staged.series.iter().cloned()),
                _ => {}
            }
            let function = function_name(window.fun.name());
            // A frame of rows that is not the whole partition reads the order of the rows.
            let frame = &window.params.window_frame;
            let whole = frame.start_bound.is_unbounded() && frame.end_bound.is_unbounded();
            let reads_rows =
                self.rows.contains(&function) || (frame.units == WindowFrameUnits::Rows && !whole);
            let mut order = window.params.order_by.clone();
            if reads_rows {
                let default: Vec<SortExpr> = match name.as_str() {
                    "panel" => (staged.position.iter().map(ascending))
                        .chain(staged.series.iter().map(series))
                        .chain(staged.other.iter().map(ascending))
                        .chain(staged.measures.iter().map(ascending))
                        .collect(),
                    "position" => (staged.series.iter().map(series))
                        .chain(staged.other.iter().map(ascending))
                        .chain(staged.measures.iter().map(ascending))
                        .collect(),
                    _ => (staged.position.iter().map(ascending))
                        .chain(staged.other.iter().map(ascending))
                        .chain(staged.measures.iter().map(ascending))
                        .collect(),
                };
                for key in default {
                    let known = partition.contains(&key.expr)
                        || order.iter().any(|own| own.expr == key.expr);
                    if !known {
                        order.push(key);
                    }
                }
            }
            window.params.partition_by = partition;
            window.params.order_by = order;
            Ok(Transformed::yes(Expr::from(window)))
        })
        .map(|transformed| transformed.data)
    }
}

impl Pass<'_> {
    /// Records `mark`, the planned call `name` at `range` of a mark in an item of a pipe
    /// `AGGREGATE` grouped by `groups`, when its channels or its `ORDER BY` hold an aggregate or a
    /// window, after checking the grain of the call. `written` holds its channel values, and
    /// `order` the keys of its `ORDER BY` at `order_range`. Returns whether the call has no error.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lift(
        &mut self,
        name: &str,
        mark: &Expr,
        written: &[Written],
        order: &[SortExpr],
        order_nodes: &[ast::Expr],
        order_range: Option<TextRange>,
        groups: Option<&[Expr]>,
        range: TextRange,
    ) -> bool {
        let holds = |test: fn(&Expr) -> bool| {
            written.iter().any(|value| test(&value.value))
                || order.iter().any(|key| test(&key.expr))
        };
        let aggregating = holds(aggregates);
        if !aggregating && !holds(windows) {
            return true;
        }
        let Some(groups) = groups else {
            self.error(
                range,
                format!(
                    "{name} computes statistics in its channels, which an AGGREGATE with \
                     GROUPING SETS, ROLLUP, or CUBE cannot hold"
                ),
            );
            return false;
        };
        // A key holds no aggregate and no window over `position` or `series`, which run on the
        // aggregated rows of a call that aggregates.
        let is_key = |value: &Expr| part(value) && !aggregates(value) && !names_late(value);
        let keys: Vec<&Expr> = (written.iter())
            .map(|value| &value.value)
            .filter(|value| is_key(value))
            .collect();
        let known: Vec<&Expr> = keys.iter().copied().chain(groups).collect();
        let mut ok = true;
        if aggregating {
            for value in written
                .iter()
                .filter(|value| part(&value.value) && !is_key(&value.value))
            {
                let rest = outside(&value.value, &known);
                let mut columns: Vec<&Column> = (rest.column_refs().into_iter())
                    .filter(|column| {
                        (column.relation.as_ref()).is_none_or(|r| r.table() != MARK_WINDOW)
                    })
                    .collect();
                columns.sort_by(|a, b| a.name.cmp(&b.name));
                if let Some(column) = columns.first() {
                    self.error(
                        value.range,
                        format!(
                            "{} reads the column {} outside its aggregates, which is neither a \
                             key of {name} nor a group",
                            value.text, column.name
                        ),
                    );
                    ok = false;
                }
            }
            for key in order {
                if !aggregates(&key.expr) && !known.contains(&&key.expr) {
                    self.error(
                        order_range.unwrap_or(range),
                        format!(
                            "The ORDER BY of {name} orders by {}, which is neither a channel of \
                             the call, a group, nor an aggregate",
                            display(&key.expr)
                        ),
                    );
                    ok = false;
                }
            }
        }
        if !ok {
            return false;
        }
        // A column takes its name, and a lifted value takes its text, or its complete text when
        // another channel of the call takes the name. A column that meets a lifted value before
        // it under its name keeps the name, and the lifted value takes its complete text.
        let mut names: Vec<(Expr, String)> = Vec::new();
        let taken = |name: &str, names: &[(Expr, String)]| names.iter().any(|(_, n)| n == name);
        for value in written {
            if names.iter().any(|(expr, _)| expr == &value.value) {
                continue;
            }
            let mut name = match &value.value {
                Expr::Column(column) => {
                    let earlier = names.iter().position(|(expr, n)| {
                        n == &column.name && !matches!(expr, Expr::Column(_))
                    });
                    if let Some(earlier) = earlier {
                        let full = (written.iter())
                            .find(|value| value.value == names[earlier].0)
                            .map(|value| value.full.clone());
                        if let Some(full) = full.filter(|full| !taken(full, &names)) {
                            names[earlier].1 = full;
                        }
                    }
                    column.name.clone()
                }
                _ if taken(&value.text, &names) => value.full.clone(),
                _ => value.text.clone(),
            };
            let base = name.clone();
            let mut number = 2;
            while taken(&name, &names) {
                name = format!("{base}_{number}");
                number += 1;
            }
            names.push((value.value.clone(), name));
        }
        // The keys of the windows leave out each channel that names one of them.
        let mut partitions = Windows::default();
        for value in written.iter().filter(|value| is_key(&value.value)) {
            if names_window(&value.value) {
                continue;
            }
            let list = match value.channel.as_str() {
                channel if POSITIONS.contains(&channel) => &mut partitions.position,
                channel if SERIES.contains(&channel) => &mut partitions.series,
                _ => &mut partitions.other,
            };
            if !list.contains(&value.value) {
                list.push(value.value.clone());
            }
        }
        for value in written
            .iter()
            .filter(|value| part(&value.value) && !is_key(&value.value))
        {
            if !windows(&value.value) && !partitions.measures.contains(&value.value) {
                partitions.measures.push(value.value.clone());
            }
            partitions.reversed |= matches!(value.channel.as_str(), "y" | "y2");
        }
        let library = self.library;
        let mut functions: Vec<String> = Vec::new();
        for expr in written
            .iter()
            .map(|value| &value.value)
            .chain(order.iter().map(|key| &key.expr))
        {
            let _ = expr.apply(|expr| {
                if let Expr::WindowFunction(window) = expr {
                    functions.push(function_name(window.fun.name()));
                }
                Ok(TreeNodeRecursion::Continue)
            });
        }
        partitions.rows = (functions.into_iter())
            .filter(|function| {
                library
                    .get(function)
                    .is_some_and(|f| f.window_order == WindowOrder::Rows)
            })
            .map(|function| function.to_ascii_lowercase())
            .collect();
        let fill = aggregating
            && PATHS.contains(&name.to_ascii_lowercase().as_str())
            && written
                .iter()
                .any(|value| !is_key(&value.value) && names_position(&value.value));
        // The text of the call, by the same roles, for the printer of the stages.
        let channels: Vec<&Written> = written.iter().filter(|value| part(&value.value)).collect();
        let printed_channels: Vec<PrintedChannel> = (channels.iter())
            .map(|value| PrintedChannel {
                range: value.node.syntax().text_range(),
                name: (names.iter())
                    .find(|(expr, _)| expr == &value.value)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| value.text.clone()),
                key: is_key(&value.value),
            })
            .collect();
        let indices = |list: &[Expr]| -> Vec<usize> {
            (list.iter())
                .filter_map(|expr| channels.iter().position(|value| &value.value == expr))
                .collect()
        };
        let roots: Vec<ast::Expr> = (channels.iter().map(|value| value.node.clone()))
            .chain(order_nodes.iter().cloned())
            .collect();
        let printed = PrintedCall {
            range,
            channels: printed_channels,
            order: order_nodes
                .iter()
                .map(|node| node.syntax().text_range())
                .collect(),
            parts: print::parts(&roots, aggregating, library),
            position: indices(&partitions.position),
            series: indices(&partitions.series),
            other: indices(&partitions.other),
            measures: indices(&partitions.measures),
            reversed: partitions.reversed,
            aggregating,
            filled: false,
        };
        self.liftings.push(Lifting {
            mark: mark.clone(),
            names,
            windows: partitions,
            aggregating,
            fill,
            printed,
        });
        true
    }
}

/// The name of a window function in the library, the name of its UDF in lower case: a
/// transform's options are its arguments, so its name holds none.
fn function_name(udf: &str) -> String {
    udf.to_ascii_lowercase()
}

/// Whether `ty` is the span type, a struct of a `start` and an `end`.
fn is_span(ty: &datafusion::arrow::datatypes::DataType) -> bool {
    match ty {
        datafusion::arrow::datatypes::DataType::Struct(fields) => {
            fields.iter().any(|field| field.name() == "start")
                && fields.iter().any(|field| field.name() == "end")
        }
        _ => false,
    }
}

/// Whether `value` over `schema` is a span, or a field of a span.
fn span_or_field(value: &Expr, schema: &DFSchema) -> bool {
    if value.get_type(schema).is_ok_and(|ty| is_span(&ty)) {
        return true;
    }
    match value {
        Expr::ScalarFunction(ScalarFunction { func, args })
            if func.name() == "get_field" && !args.is_empty() =>
        {
            args[0].get_type(schema).is_ok_and(|ty| is_span(&ty))
        }
        _ => false,
    }
}

/// `exprs` with each of `from` written as the matching expression of `to`.
fn replaced(exprs: Vec<Expr>, from: &[Expr], to: &[Expr]) -> Result<Vec<Expr>> {
    (exprs.into_iter())
        .map(|expr| {
            expr.transform_down(|expr| match from.iter().position(|f| f == &expr) {
                Some(index) => Ok(Transformed::new(
                    to[index].clone(),
                    true,
                    TreeNodeRecursion::Jump,
                )),
                None => Ok(Transformed::no(expr)),
            })
            .map(|transformed| transformed.data)
        })
        .collect()
}

/// The branch of `lifting` over `input`, grouped by `groups`, aliased `alias`: the windows on the
/// rows, an aggregate by the groups and the call's keys when the call aggregates, the zero fill of
/// a stacked path, the windows on the aggregated rows, a projection of each channel's value under
/// its name, and the mark aggregated by the groups. Its columns are `__group_0`, `__group_1`, and
/// so on, then `__mark`.
pub(crate) fn branch(
    input: LogicalPlan,
    groups: &[Expr],
    lifting: &Lifting,
    alias: &str,
) -> Result<(LogicalPlan, bool)> {
    let Expr::AggregateFunction(mark) = &lifting.mark else {
        unreachable!("a lifting holds a mark aggregate");
    };
    // The values of the channels and the keys of the call's `ORDER BY`, without constants, then
    // the keys and measures of the windows, which each stage writes as its columns.
    let values: Vec<&Expr> = (mark.params.args.iter())
        .map(channel_value)
        .filter(|value| part(value))
        .collect();
    let order: Vec<&Expr> = mark.params.order_by.iter().map(|key| &key.expr).collect();
    let calls = &lifting.windows;
    let lengths = [
        values.len(),
        order.len(),
        calls.position.len(),
        calls.series.len(),
        calls.other.len(),
        calls.measures.len(),
    ];
    let mut parts: Vec<Expr> = (values.iter().chain(&order).copied())
        .chain(&calls.position)
        .chain(&calls.series)
        .chain(&calls.other)
        .chain(&calls.measures)
        .cloned()
        .collect();
    let split = |parts: &[Expr]| -> Vec<Vec<Expr>> {
        let mut rest = parts;
        lengths
            .iter()
            .map(|&length| {
                let (head, tail) = rest.split_at(length);
                rest = tail;
                head.to_vec()
            })
            .collect()
    };
    let late = |expr: &Expr| lifting.aggregating && (aggregates(expr) || names_late(expr));
    // The windows on the rows, innermost first, each stage over the columns of the one before. A
    // window over `panel` there orders by the keys that hold no window.
    let plain = |exprs: &[Expr]| -> Vec<Expr> {
        exprs
            .iter()
            .filter(|expr| !windows(expr))
            .cloned()
            .collect()
    };
    let (row_position, row_series, row_other) = (
        plain(&calls.position),
        plain(&calls.series),
        plain(&calls.other),
    );
    let row_names: &[&str] = match lifting.aggregating {
        true => &["panel"],
        false => &["panel", "position", "series"],
    };
    let mut plan = input;
    loop {
        let found: Vec<Expr> = (innermost_windows(&parts).into_iter())
            .filter(|window| !late(window))
            .collect();
        if found.is_empty() {
            break;
        }
        let staged = Staged {
            groups,
            position: &row_position,
            series: &row_series,
            other: &row_other,
            measures: &[],
        };
        let resolved: Vec<Expr> = (found.iter())
            .map(|window| calls.resolve(window.clone(), row_names, &staged))
            .collect::<Result<_>>()?;
        parts = replaced(parts, &found, &resolved)?;
        plan = rows::windowed(plan, resolved.clone())?;
        parts = (parts.into_iter())
            .map(|part| rows::rebase(part, &resolved))
            .collect::<Result<_>>()?;
    }
    // The aggregate by the groups and the keys, when the call aggregates.
    let mut staged_groups: Vec<Expr> = groups.to_vec();
    if lifting.aggregating {
        let split_parts = split(&parts);
        let mut group_exprs: Vec<Expr> = groups.to_vec();
        for value in &split_parts[0] {
            if !aggregates(value) && !names_late(value) && !group_exprs.contains(value) {
                group_exprs.push(value.clone());
            }
        }
        let read: Vec<&Expr> = split_parts[0].iter().chain(&split_parts[1]).collect();
        let aggregate_exprs = find_aggregate_exprs(read.iter().copied());
        let aggregated = LogicalPlanBuilder::from(plan)
            .aggregate(group_exprs.clone(), aggregate_exprs.clone())?
            .build()?;
        let grouped: Vec<(Expr, Column)> = (group_exprs.iter().cloned())
            .zip(aggregated.schema().columns())
            .collect();
        let stage = |expr: &Expr| -> Result<Expr> {
            regroup(rows::rebase(expr.clone(), &aggregate_exprs)?, &grouped)
        };
        staged_groups = groups.iter().map(stage).collect::<Result<_>>()?;
        parts = parts.iter().map(stage).collect::<Result<_>>()?;
        plan = aggregated;
    }
    // The windows on the aggregated rows, with their partitions and orders.
    let split_parts = split(&parts);
    let staged = Staged {
        groups: &staged_groups,
        position: &split_parts[2],
        series: &split_parts[3],
        other: &split_parts[4],
        measures: &split_parts[5],
    };
    parts = (parts.into_iter())
        .map(|part| calls.resolve(part, &["panel", "position", "series"], &staged))
        .collect::<Result<_>>()?;
    // A stacked path fills the keys that a series lacks with zero before its windows.
    let mut fills = false;
    if lifting.fill {
        (plan, fills) = filled(plan, values.as_slice(), &parts, &staged)?;
    }
    loop {
        let found = innermost_windows(&parts);
        if found.is_empty() {
            break;
        }
        plan = rows::windowed(plan, found.clone())?;
        parts = (parts.into_iter())
            .map(|part| rows::rebase(part, &found))
            .collect::<Result<_>>()?;
    }
    let split_parts = split(&parts);
    let (staged_values, staged_order) = (&split_parts[0], &split_parts[1]);
    // The groups, each channel value under its name, and the keys of the `ORDER BY`.
    let mut projection = Vec::new();
    for (index, group) in staged_groups.iter().enumerate() {
        projection.push(group.clone().alias(format!("__group_{index}")));
    }
    let mut named: Vec<(&Expr, String)> = Vec::new();
    for (value, staged) in values.iter().zip(staged_values) {
        if named.iter().any(|(expr, _)| expr == value) {
            continue;
        }
        let name = (lifting.names.iter())
            .find(|(expr, _)| expr == *value)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| value.human_display().to_string());
        projection.push(staged.clone().alias(name.clone()));
        named.push((value, name));
    }
    for (index, key) in staged_order.iter().enumerate() {
        projection.push(key.clone().alias(format!("__order_{index}")));
    }
    let projected = LogicalPlanBuilder::from(plan)
        .project(projection)?
        .build()?;
    // The call over the projection's columns.
    let column = |value: &Expr| -> Option<Expr> {
        (named.iter())
            .find(|(expr, _)| *expr == value)
            .map(|(_, name)| Expr::Column(Column::from_name(name.clone())))
    };
    let args = (mark.params.args.iter())
        .map(|arg| {
            kind::map_value(arg, &mut |value| {
                column(value).unwrap_or_else(|| value.clone())
            })
        })
        .collect();
    let order_by = (mark.params.order_by.iter().enumerate())
        .map(|(index, key)| SortExpr {
            expr: Expr::Column(Column::from_name(format!("__order_{index}"))),
            ..key.clone()
        })
        .collect();
    let restaged = Expr::AggregateFunction(AggregateFunction::new_udf(
        mark.func.clone(),
        args,
        mark.params.distinct,
        mark.params.filter.clone(),
        order_by,
        mark.params.null_treatment,
    ));
    let group_columns: Vec<Expr> = (0..groups.len())
        .map(|index| Expr::Column(Column::from_name(format!("__group_{index}"))))
        .collect();
    let plan = LogicalPlanBuilder::from(projected)
        .aggregate(group_columns, vec![restaged.alias("__mark")])?
        .alias(alias)?
        .build()?;
    Ok((plan, fills))
}

/// `plan`, the aggregated rows of a stacked path, with a row of zeros for each position key and
/// series that a group lacks: the first aggregate that the stack reads is imputed with zero, and
/// the others are zero on the added rows. A path with one position key and a measure that is a
/// span, or a field of a span, fills; any other keeps its rows.
fn filled(
    plan: LogicalPlan,
    values: &[&Expr],
    parts: &[Expr],
    staged: &Staged,
) -> Result<(LogicalPlan, bool)> {
    let [key] = staged.position else {
        return Ok((plan, false));
    };
    let schema = plan.schema().clone();
    // The staged measures that stack over `position`, and the aggregates that they read.
    let stacked: Vec<&Expr> = (values.iter().zip(parts))
        .filter(|(value, _)| names_position(value))
        .map(|(_, part)| part)
        .filter(|part| span_or_field(part, &schema))
        .collect();
    let mut read: Vec<Column> = Vec::new();
    for part in &stacked {
        for column in part.column_refs() {
            if staged.measures_column(column) && !read.contains(column) {
                read.push(column.clone());
            }
        }
    }
    let (Some(value), Expr::Column(key)) = (read.first().cloned(), key) else {
        return Ok((plan, false));
    };
    let groups: Vec<Column> = (staged.series.iter())
        .filter_map(|expr| match expr {
            Expr::Column(column) => Some(column.clone()),
            _ => None,
        })
        .collect();
    let partition: Vec<Column> = (staged.groups.iter())
        .filter_map(|expr| match expr {
            Expr::Column(column) => Some(column.clone()),
            _ => None,
        })
        .collect();
    let zero = |column: &Column| -> Result<Expr> {
        let ty = schema.field_from_column(column)?.data_type().clone();
        Ok(lit(datafusion::common::ScalarValue::new_zero(&ty)?))
    };
    let mut named = vec![
        ("value".to_string(), Named::Value(zero(&value)?)),
        ("flag".to_string(), Named::Name("__imputed".to_string())),
    ];
    if !partition.is_empty() {
        named.push(("partition_by".to_string(), Named::Columns(partition)));
    }
    let call = ImputeCall {
        key: key.clone(),
        groups,
        value: value.clone(),
        named,
    };
    let imputed = impute::plan(plan, call)
        .map_err(|error| datafusion::common::DataFusionError::Plan(error.to_string()))?;
    // The other aggregates that the stack reads are zero on the added rows.
    let others = &read[1..];
    if others.is_empty() {
        return Ok((imputed, true));
    }
    let projection = (imputed.schema().columns().into_iter())
        .map(|column| -> Result<Expr> {
            Ok(match others.contains(&column) {
                true => when(Expr::Column(Column::from_name("__imputed")), zero(&column)?)
                    .otherwise(Expr::Column(column.clone()))?
                    .alias(column.name.clone()),
                false => Expr::Column(column),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let plan = LogicalPlanBuilder::from(imputed)
        .project(projection)?
        .build()?;
    Ok((plan, true))
}

impl Staged<'_> {
    /// Whether `column` is the column of an aggregate, which no group or key names.
    fn measures_column(&self, column: &Column) -> bool {
        let keyed = (self.groups.iter())
            .chain(self.position)
            .chain(self.series)
            .chain(self.other)
            .any(|expr| matches!(expr, Expr::Column(c) if c == column));
        !keyed
    }
}
