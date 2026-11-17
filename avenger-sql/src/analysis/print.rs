//! The printer of the stages of a script's lifted calls: [`Analysis::lift`] writes each pipe
//! `AGGREGATE` that holds a mark call with statistics in its channels as the staged pipe query
//! that a user could type for it, and keeps the rest of the text as written.
//!
//! The printer writes the calls that it generates in the layout of nested calls: a call or an
//! array that fits in 100 characters on its line stays on one line, and one that does not breaks
//! after its opening, with one argument to a line, two spaces deeper than the depth of its line,
//! each with a comma, and its closing on a line of its own at that depth. The depth of a line is
//! the column of its first word, and of a pipe operator's line the column of the operator's
//! keyword, after `|> `. The calls inside the arguments follow the rule again.
//!
//! The pass plans the stages of a lifted call itself, so the printer reads a record that the pass
//! keeps of each call: the ranges of the text of its channels, its aggregates, and its windows,
//! the names of their columns, and the keys and measures that its windows partition and order by.

use avenger_sql_syntax::{ast, ast::AstNode, Offset as TextSize, SyntaxKind, TextRange};

use super::Analysis;
use crate::library::{DefaultWindow, FunctionKind, Library, WindowOrder};

/// A script as the script that its lifted calls plan, and the ranges of the text that the
/// printer wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lifted {
    pub text: String,
    pub generated: Vec<TextRange>,
}

/// A pipe `AGGREGATE` that holds lifted calls.
#[derive(Clone, Debug)]
pub struct LiftedOperator {
    /// The operator, from its `|>`.
    pub(crate) range: TextRange,
    /// The query before the operator.
    pub(crate) input: TextRange,
    /// Each group item as written, with its alias, and the name of its column.
    pub(crate) groups: Vec<(TextRange, String)>,
    pub(crate) items: Vec<PrintedItem>,
}

/// An item of an `AGGREGATE` that holds lifted calls.
#[derive(Clone, Debug)]
pub(crate) struct PrintedItem {
    /// The item's expression, without its alias.
    pub range: TextRange,
    pub name: String,
    /// The outermost aggregates of the expression that are not lifted calls, such as a flat mark.
    pub aggregates: Vec<TextRange>,
    pub calls: Vec<PrintedCall>,
}

/// A lifted call: the text of its channels and their parts.
#[derive(Clone, Debug)]
pub(crate) struct PrintedCall {
    pub range: TextRange,
    pub channels: Vec<PrintedChannel>,
    /// The keys of the call's `ORDER BY`.
    pub order: Vec<TextRange>,
    pub parts: Vec<Part>,
    /// The indices in `channels` of the position, series, and other keys of the windows, and of
    /// the measures without windows.
    pub position: Vec<usize>,
    pub series: Vec<usize>,
    pub other: Vec<usize>,
    pub measures: Vec<usize>,
    pub reversed: bool,
    pub aggregating: bool,
    /// Whether the branch filled the keys that a stacked path's series lack with zero.
    pub filled: bool,
}

/// A channel value, inside its channel function and `datum` or `value`, the name of its column,
/// and whether it is a key.
#[derive(Clone, Debug)]
pub(crate) struct PrintedChannel {
    pub range: TextRange,
    pub name: String,
    pub key: bool,
}

/// An aggregate or a window inside a call's channels or its `ORDER BY`.
#[derive(Clone, Debug)]
pub(crate) struct Part {
    pub range: TextRange,
    pub kind: PartKind,
    pub window: Option<PrintedWindow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartKind {
    Aggregate,
    /// A window on the rows, before the aggregate.
    Rows,
    /// A window on the aggregated rows.
    Aggregated,
}

/// The text of a window: the call before its `OVER`, the window of the call that it names, its
/// own keys and frame, and whether it reads the order of its rows.
#[derive(Clone, Debug)]
pub(crate) struct PrintedWindow {
    pub head: TextRange,
    pub over: Option<TextRange>,
    pub named: Option<String>,
    pub order: Vec<TextRange>,
    pub frame: Option<TextRange>,
    pub reads_rows: bool,
}

/// The function of the library that `call` calls, when its name has one part, and not a
/// constructor of a chart's option, which the parameter of its argument names.
fn function<'l>(
    call: &ast::CallExpr,
    library: &'l Library,
) -> Option<&'l crate::library::Function> {
    let names = call.path()?.names()?;
    let [_] = names.as_slice() else {
        return None;
    };
    let called = crate::library::context::called(library, call.syntax())?;
    called.fills.is_none().then_some(called.function)
}

/// The aggregates and windows of `roots`, the channel values and the `ORDER BY` keys of a call
/// that `aggregating` says aggregates, by `library`. The arguments that `lump` evaluates in its
/// own scope, `weight` and `keep`, hold no part of the call.
pub(crate) fn parts(roots: &[ast::Expr], aggregating: bool, library: &Library) -> Vec<Part> {
    let mut parts = Vec::new();
    for root in roots {
        let mut skipped: Vec<TextRange> = Vec::new();
        for node in root.syntax().descendants() {
            if skipped
                .iter()
                .any(|range| range.contains_range(node.text_range()))
            {
                continue;
            }
            let Some(call) = ast::CallExpr::cast(node.clone()) else {
                continue;
            };
            let Some(function) = function(&call, library) else {
                continue;
            };
            let own = own_scope(&call, function);
            skipped.extend(own.iter().copied());
            let over = call.over_clause();
            let window = over.is_some()
                || (function.kind == FunctionKind::Window && function.default_window.is_some());
            if !window {
                if function.kind == FunctionKind::Aggregate {
                    parts.push(Part {
                        range: call.syntax().text_range(),
                        kind: PartKind::Aggregate,
                        window: None,
                    });
                }
                continue;
            }
            let head = match &over {
                Some(over) => TextRange::new(call.syntax().text_range().start(), over_start(over)),
                None => call.syntax().text_range(),
            };
            let spec = over.as_ref().and_then(|over| over.window_spec());
            let named = (over.as_ref().and_then(|over| over.name_ref()))
                .or_else(|| spec.as_ref().and_then(|spec| spec.name_ref()))
                .and_then(|name| name.text())
                .map(|name| name.to_ascii_lowercase())
                .filter(|name| matches!(name.as_str(), "panel" | "position" | "series"));
            let order: Vec<TextRange> = (spec.as_ref().and_then(|spec| spec.order_by()))
                .iter()
                .flat_map(|order| order.order_keys())
                .map(|key| key.syntax().text_range())
                .collect();
            let frame = spec.as_ref().and_then(|spec| spec.frame());
            let bounded_rows = frame.as_ref().is_some_and(|frame| {
                let text = frame.syntax().text().to_string().to_ascii_uppercase();
                frame.rows_kw().is_some()
                    && !(text.contains("UNBOUNDED PRECEDING")
                        && text.contains("UNBOUNDED FOLLOWING"))
            });
            let default = over.is_none().then_some(function.default_window).flatten();
            let reads_rows = function.window_order == WindowOrder::Rows || bounded_rows;
            let holds_aggregate = (call.syntax().descendants().skip(1))
                .filter(|node| {
                    !own.iter()
                        .any(|range| range.contains_range(node.text_range()))
                })
                .filter_map(ast::CallExpr::cast)
                .any(|inner| {
                    inner.over_clause().is_none()
                        && function_kind(&inner, library) == Some(FunctionKind::Aggregate)
                });
            let late = aggregating
                && (holds_aggregate
                    || matches!(named.as_deref(), Some("position" | "series"))
                    || default == Some(DefaultWindow::Position));
            parts.push(Part {
                range: call.syntax().text_range(),
                kind: match late {
                    true => PartKind::Aggregated,
                    false => PartKind::Rows,
                },
                window: Some(PrintedWindow {
                    head,
                    over: over.as_ref().map(|over| over.syntax().text_range()),
                    named: named.or_else(|| {
                        (default == Some(DefaultWindow::Position)).then(|| "position".to_string())
                    }),
                    order,
                    frame: frame.map(|frame| frame.syntax().text_range()),
                    reads_rows,
                }),
            });
        }
    }
    parts
}

/// The arguments of `call` that `function` evaluates in its own scope: the `weight` and the
/// `keep` of `lump`, which aggregate the rows of each category.
fn own_scope(call: &ast::CallExpr, function: &crate::library::Function) -> Vec<TextRange> {
    if function.name != "lump" {
        return Vec::new();
    }
    let list = call.arg_list();
    (list.iter().flat_map(|list| list.arguments()))
        .filter(|argument| match argument {
            ast::Argument::NamedArg(named) => (named.name_ref().and_then(|name| name.text()))
                .is_some_and(|name| matches!(name.as_str(), "weight" | "keep")),
            _ => false,
        })
        .map(|argument| argument.syntax().text_range())
        .collect()
}

fn function_kind(call: &ast::CallExpr, library: &Library) -> Option<FunctionKind> {
    function(call, library).map(|function| function.kind)
}

/// The start of the `OVER` clause `over`, after the spaces before it.
fn over_start(over: &ast::OverClause) -> TextSize {
    over.syntax().text_range().start()
}

/// The outermost aggregates of `expr`, an item of an `AGGREGATE`, outside its lifted `calls`.
pub(crate) fn aggregates(
    expr: &ast::Expr,
    calls: &[PrintedCall],
    library: &Library,
) -> Vec<TextRange> {
    let mut found: Vec<TextRange> = Vec::new();
    for node in expr.syntax().descendants() {
        let range = node.text_range();
        if calls.iter().any(|call| call.range.contains_range(range))
            || found.iter().any(|outer| outer.contains_range(range))
        {
            continue;
        }
        let Some(call) = ast::CallExpr::cast(node) else {
            continue;
        };
        if call.over_clause().is_none()
            && function_kind(&call, library) == Some(FunctionKind::Aggregate)
        {
            found.push(range);
        }
    }
    found
}

/// The query that holds `node`, the start of its text.
pub(crate) fn query_start(node: &avenger_sql_syntax::SyntaxNode) -> Option<TextSize> {
    (node.ancestors())
        .find(|node| node.kind() == SyntaxKind::QUERY)
        .map(|query| query.text_range().start())
}

/// `name` as a quoted name.
fn quoted(name: &str) -> String {
    format!("`{}`", name.replace('\\', "\\\\").replace('`', "\\`"))
}

/// The alias of an item named `name`, which an item without a name takes none of.
fn alias(name: &str) -> String {
    match name.is_empty() {
        true => String::new(),
        false => format!(" AS {}", quoted(name)),
    }
}

/// `text` as the column `name`: itself when it names the column already.
fn aliased(text: &str, name: &str) -> String {
    let identifier = (name.chars().next()).is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    match (text == name && identifier) || text == quoted(name) {
        true => text.to_string(),
        false => format!("{text} AS {}", quoted(name)),
    }
}

/// `text` with its spaces collapsed.
fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of the script and the replacements of its ranges that the stages so far wrote.
struct Writer<'t> {
    text: &'t str,
    replaced: Vec<(TextRange, String)>,
}

impl Writer<'_> {
    /// The text of `range`, with the outermost replacements inside it.
    fn at(&self, range: TextRange) -> String {
        if let Some((_, text)) = self.replaced.iter().rev().find(|(r, _)| *r == range) {
            return text.clone();
        }
        // The latest replacement of each range, as each stage writes its ranges again.
        let mut inside: Vec<&(TextRange, String)> = Vec::new();
        for replaced in self.replaced.iter().rev() {
            let (r, _) = replaced;
            if range.contains_range(*r) && *r != range && !inside.iter().any(|(o, _)| o == r) {
                inside.push(replaced);
            }
        }
        inside.sort_by_key(|(r, _)| (r.start(), std::cmp::Reverse(r.end())));
        let mut out = String::new();
        let mut at = range.start();
        for (r, text) in inside {
            if r.start() < at {
                continue;
            }
            out.push_str(&self.text[TextRange::new(at, r.start())]);
            out.push_str(text);
            at = r.end();
        }
        out.push_str(&self.text[TextRange::new(at, range.end())]);
        out
    }

    fn replace(&mut self, range: TextRange, text: String) {
        self.replaced.push((range, text));
    }
}

/// The level of a window's partition and order: on the rows, by the text of the keys, or on the
/// aggregated rows, by the names of their columns.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Rows,
    Aggregated,
}

impl PrintedCall {
    /// The keys `indices` as the window `level` reads them.
    fn keys(&self, writer: &Writer, indices: &[usize], level: Level) -> Vec<String> {
        (indices.iter())
            .map(|&index| &self.channels[index])
            .filter(|channel| level == Level::Aggregated || !self.holds_window(channel.range))
            .map(|channel| match level {
                Level::Rows => writer.at(channel.range),
                Level::Aggregated => quoted(&channel.name),
            })
            .collect()
    }

    fn holds_window(&self, range: TextRange) -> bool {
        (self.parts.iter()).any(|part| part.window.is_some() && range.contains_range(part.range))
    }

    /// The text of the window `part` at `level`, with its partition and its default order.
    fn window(&self, writer: &Writer, part: &Part, groups: &[String], level: Level) -> String {
        let window = part.window.as_ref().expect("a window part");
        let head = writer.at(window.head).trim_end().to_string();
        let Some(name) = &window.named else {
            return match &window.over {
                Some(over) => format!("{head} {}", writer.at(*over)),
                None => format!("{head} OVER ()"),
            };
        };
        let position = self.keys(writer, &self.position, level);
        let series = self.keys(writer, &self.series, level);
        let other = self.keys(writer, &self.other, level);
        let mut partition: Vec<String> = groups.to_vec();
        match name.as_str() {
            "position" => partition.extend(position.iter().cloned()),
            "series" => partition.extend(series.iter().cloned()),
            _ => {}
        }
        let mut order: Vec<String> = window.order.iter().map(|key| writer.at(*key)).collect();
        if window.reads_rows {
            let ascending = |key: &String| format!("{key} ASC NULLS LAST");
            let legend = |key: &String| match self.reversed {
                true => format!("{key} DESC NULLS FIRST"),
                false => ascending(key),
            };
            let measures: Vec<String> = match level {
                Level::Rows => Vec::new(),
                Level::Aggregated => (self.measures.iter())
                    .map(|&index| writer.at(self.channels[index].range))
                    .collect(),
            };
            let default: Vec<String> = match name.as_str() {
                "panel" => (position.iter().map(ascending))
                    .chain(series.iter().map(legend))
                    .chain(other.iter().map(ascending))
                    .chain(measures.iter().map(ascending))
                    .collect(),
                "position" => (series.iter().map(legend))
                    .chain(other.iter().map(ascending))
                    .chain(measures.iter().map(ascending))
                    .collect(),
                _ => (position.iter().map(ascending))
                    .chain(other.iter().map(ascending))
                    .chain(measures.iter().map(ascending))
                    .collect(),
            };
            let written: Vec<String> = (window.order.iter())
                .map(|key| collapsed(&writer.at(*key)))
                .collect();
            for key in default {
                let expr = key
                    .trim_end_matches(" ASC NULLS LAST")
                    .trim_end_matches(" DESC NULLS FIRST")
                    .to_string();
                let known = partition.iter().any(|p| collapsed(p) == collapsed(&expr))
                    || written.iter().any(|own| {
                        collapsed(own)
                            .trim_end_matches(" DESC")
                            .trim_end_matches(" ASC")
                            == collapsed(&expr)
                    });
                if !known {
                    order.push(key);
                }
            }
        }
        let mut spec = Vec::new();
        if !partition.is_empty() {
            spec.push(format!("PARTITION BY {}", partition.join(", ")));
        }
        if !order.is_empty() {
            spec.push(format!("ORDER BY {}", order.join(", ")));
        }
        if let Some(frame) = window.frame {
            spec.push(writer.at(frame));
        }
        format!("{head} OVER ({})", spec.join(" "))
    }

    /// The stages of the call over its input, as pipe operators, then the text of the call over
    /// the columns of the last stage. `groups` are the group items as written, and `names` their
    /// columns.
    fn stages(
        &self,
        writer: &mut Writer,
        groups: &[String],
        names: &[String],
        indent: &str,
        root: &avenger_sql_syntax::SyntaxNode,
    ) -> (Vec<String>, Doc) {
        let mut stages = Vec::new();
        let mut extended: Vec<String> = Vec::new();
        let by_size = |kind: PartKind| {
            let mut parts: Vec<&Part> =
                self.parts.iter().filter(|part| part.kind == kind).collect();
            parts.sort_by_key(|part| part.range.len());
            parts
        };
        // The windows on the rows, innermost first.
        for part in by_size(PartKind::Rows) {
            let text = self.window(writer, part, groups, Level::Rows);
            let name = collapsed(&writer.text[part.range]);
            if !extended.contains(&name) {
                stages.push(format!("|> EXTEND {text} AS {}", quoted(&name)));
                extended.push(name.clone());
            }
            writer.replace(part.range, quoted(&name));
        }
        let group_columns: Vec<String> = names.iter().map(|name| quoted(name)).collect();
        if self.aggregating {
            let mut items: Vec<String> = Vec::new();
            let mut aggregates: Vec<&Part> = by_size(PartKind::Aggregate);
            aggregates.reverse();
            let outermost: Vec<&Part> = (aggregates.iter())
                .filter(|part| {
                    !(self.parts.iter()).any(|other| {
                        other.kind == PartKind::Aggregate
                            && other.range != part.range
                            && other.range.contains_range(part.range)
                    })
                })
                .copied()
                .collect();
            let mut named: Vec<String> = Vec::new();
            for part in outermost {
                let text = writer.at(part.range);
                let name = collapsed(&writer.text[part.range]);
                if !named.contains(&name) {
                    items.push(format!("{text} AS {}", quoted(&name)));
                    named.push(name.clone());
                }
                writer.replace(part.range, quoted(&name));
            }
            let mut keys: Vec<String> = groups.to_vec();
            let mut keyed: Vec<String> = Vec::new();
            for channel in self.channels.iter().filter(|channel| channel.key) {
                if keyed.contains(&channel.name) {
                    writer.replace(channel.range, quoted(&channel.name));
                    continue;
                }
                keys.push(aliased(&writer.at(channel.range), &channel.name));
                keyed.push(channel.name.clone());
                writer.replace(channel.range, quoted(&channel.name));
            }
            let mut stage = String::from("|> AGGREGATE");
            if !items.is_empty() {
                stage.push(' ');
                stage.push_str(&items.join(", "));
            }
            if !keys.is_empty() {
                stage.push_str(" GROUP BY ");
                stage.push_str(&keys.join(", "));
            }
            stages.push(stage);
            // A stacked path fills the keys that its series lack with zero.
            if self.filled {
                if let [position] = self.position.as_slice() {
                    let key = quoted(&self.channels[*position].name);
                    let series: Vec<String> = (self.series.iter())
                        .map(|&index| quoted(&self.channels[index].name))
                        .collect();
                    let read: Vec<String> = (self.parts.iter())
                        .filter(|part| part.kind == PartKind::Aggregated)
                        .flat_map(|window| {
                            (self.parts.iter())
                                .filter(|part| {
                                    part.kind == PartKind::Aggregate
                                        && window.range.contains_range(part.range)
                                })
                                .map(|part| quoted(&collapsed(&writer.text[part.range])))
                                .collect::<Vec<_>>()
                        })
                        .fold(Vec::new(), |mut read, name| {
                            if !read.contains(&name) {
                                read.push(name);
                            }
                            read
                        });
                    if let Some((value, others)) = read.split_first() {
                        let text = |text: &str| Doc::Text(text.to_string());
                        let array = |items: &[String]| Doc::Group {
                            open: "[".to_string(),
                            items: items
                                .iter()
                                .map(|item| (String::new(), text(item)))
                                .collect(),
                            clauses: Vec::new(),
                            close: "]",
                        };
                        let mut items = vec![
                            (String::new(), text(&key)),
                            (String::new(), array(&series)),
                            (String::new(), text(value)),
                            ("value => ".to_string(), text("0")),
                        ];
                        if !group_columns.is_empty() {
                            items.push(("partition_by => ".to_string(), array(&group_columns)));
                        }
                        if !others.is_empty() {
                            items.push(("flag => ".to_string(), text("'imputed'")));
                        }
                        let call = Doc::Group {
                            open: "impute(".to_string(),
                            items,
                            clauses: Vec::new(),
                            close: ")",
                        };
                        stages.push(operator(indent, "|> CALL ", &call, "", ""));
                        for other in others {
                            stages.push(format!("|> SET {other} = IF(imputed, 0, {other})"));
                        }
                    }
                }
            }
        }
        // The windows on the aggregated rows, innermost first.
        let late_groups: Vec<String> = match self.aggregating {
            true => group_columns.clone(),
            false => groups.to_vec(),
        };
        let level = match self.aggregating {
            true => Level::Aggregated,
            false => Level::Rows,
        };
        for part in by_size(PartKind::Aggregated) {
            let text = self.window(writer, part, &late_groups, level);
            let name = collapsed(&writer.text[part.range]);
            if !extended.contains(&name) {
                stages.push(format!("|> EXTEND {text} AS {}", quoted(&name)));
                extended.push(name.clone());
            }
            writer.replace(part.range, quoted(&name));
        }
        // Each channel's value under its name, and the keys of the `ORDER BY`.
        let mut projection: Vec<String> = match self.aggregating {
            true => group_columns.clone(),
            false => names.iter().map(|name| quoted(name)).collect(),
        };
        let mut projected: Vec<String> = Vec::new();
        for channel in &self.channels {
            if !projected.contains(&channel.name) {
                projection.push(aliased(&writer.at(channel.range), &channel.name));
                projected.push(channel.name.clone());
            }
            writer.replace(channel.range, quoted(&channel.name));
        }
        for key in &self.order {
            let name = collapsed(&writer.text[*key]);
            if !projected.contains(&name) {
                projection.push(aliased(&writer.at(*key), &name));
                projected.push(name.clone());
            }
            writer.replace(*key, quoted(&name));
        }
        if !self.aggregating {
            // Without an aggregate, the groups are columns of the input.
            projection.splice(
                0..names.len(),
                groups
                    .iter()
                    .zip(names)
                    .map(|(group, name)| format!("{group} AS {}", quoted(name))),
            );
        }
        stages.push(format!("|> SELECT {}", projection.join(", ")));
        let call = (root.covering_element(self.range).ancestors())
            .find(|node| node.kind() == SyntaxKind::CALL_EXPR && node.text_range() == self.range);
        let mark = match call {
            Some(call) => writer.doc(&call),
            None => Doc::Text(one_line(&writer.at(self.range))),
        };
        (stages, mark)
    }
}

/// The longest line of the layout of nested calls, in characters.
const WIDTH: usize = 100;

/// Text that the printer generates in the layout of nested calls: text that stays on one line, or
/// a call or an array, whose items break onto lines of their own when it does not fit on its line.
#[derive(Clone, Debug)]
enum Doc {
    Text(String),
    /// A call, `name(` with `)`, or an array, `[` with `]`: its items, each with the text before
    /// its value, such as `x => `, and the clauses after its items, such as an `ORDER BY`.
    Group {
        open: String,
        items: Vec<(String, Doc)>,
        clauses: Vec<String>,
        close: &'static str,
    },
}

/// The number of characters of `text`.
fn width(text: &str) -> usize {
    text.chars().count()
}

impl Doc {
    /// The text on one line.
    fn flat(&self) -> String {
        match self {
            Doc::Text(text) => text.clone(),
            Doc::Group {
                open,
                items,
                clauses,
                close,
            } => {
                let items: Vec<String> = (items.iter())
                    .map(|(before, item)| format!("{before}{}", item.flat()))
                    .collect();
                let mut text = format!("{open}{}", items.join(", "));
                for clause in clauses {
                    text.push(' ');
                    text.push_str(clause);
                }
                text.push_str(close);
                text
            }
        }
    }

    /// The text at `column` of a line of depth `depth`, which `suffix` characters follow on its
    /// line: on one line when it fits in [`WIDTH`], and otherwise broken after its opening with
    /// one item to a line, two spaces deeper than `depth`, each with a comma, and its closing on a
    /// line of its own at `depth`. Each item follows the rule again at its own line's depth.
    fn render(&self, column: usize, depth: usize, suffix: usize) -> String {
        let flat = self.flat();
        let Doc::Group {
            open,
            items,
            clauses,
            close,
        } = self
        else {
            return flat;
        };
        if column + width(&flat) + suffix <= WIDTH || (items.is_empty() && clauses.is_empty()) {
            return flat;
        }
        let inner = depth + 2;
        let indent = " ".repeat(inner);
        let mut text = format!("{open}\n");
        for (before, item) in items {
            let value = item.render(inner + width(before), inner, 1);
            text.push_str(&format!("{indent}{before}{value},\n"));
        }
        for clause in clauses {
            text.push_str(&format!("{indent}{clause}\n"));
        }
        text.push_str(&" ".repeat(depth));
        text.push_str(close);
        text
    }
}

/// `text`, an expression, on one line: its spaces collapsed outside its literals, without spaces
/// inside its brackets or a comma before a closing bracket. Text with a comment stays as written.
fn one_line(text: &str) -> String {
    use avenger_sql_syntax::lexer::tokenize;
    let (tokens, _) = tokenize(text);
    if (tokens.iter()).any(|token| token.kind == SyntaxKind::COMMENT) {
        return text.to_string();
    }
    let solid: Vec<(SyntaxKind, &str, bool)> = {
        let mut solid = Vec::new();
        let mut spaced = false;
        for token in &tokens {
            match token.kind {
                SyntaxKind::WHITESPACE => spaced = true,
                kind => {
                    solid.push((kind, &text[token.range], spaced));
                    spaced = false;
                }
            }
        }
        solid
    };
    let closing = |kind: SyntaxKind| matches!(kind, SyntaxKind::R_PAREN | SyntaxKind::R_BRACK);
    let mut out = String::new();
    let mut previous: Option<SyntaxKind> = None;
    for (index, (kind, token, spaced)) in solid.iter().enumerate() {
        let next = solid.get(index + 1).map(|(kind, _, _)| *kind);
        if *kind == SyntaxKind::COMMA && next.is_some_and(closing) {
            continue;
        }
        let opened = matches!(previous, Some(SyntaxKind::L_PAREN | SyntaxKind::L_BRACK));
        if *spaced && !out.is_empty() && !opened && !closing(*kind) && *kind != SyntaxKind::COMMA {
            out.push(' ');
        }
        out.push_str(token);
        previous = Some(*kind);
    }
    out
}

impl Writer<'_> {
    /// The text of `node`, an expression of the script, with the replacements so far, as a
    /// [`Doc`]: a call or an array that no replacement covers is a group of its arguments or its
    /// elements, and any other expression is its text on one line.
    fn doc(&self, node: &avenger_sql_syntax::SyntaxNode) -> Doc {
        let range = node.text_range();
        let text = || Doc::Text(one_line(&self.at(range)));
        if self.replaced.iter().any(|(replaced, _)| *replaced == range) {
            return text();
        }
        if let Some(call) = ast::CallExpr::cast(node.clone()) {
            let (Some(path), Some(list), None) = (call.path(), call.arg_list(), call.over_clause())
            else {
                return text();
            };
            if list.distinct_kw().is_some() || list.star_arg().is_some() {
                return text();
            }
            let mut items = Vec::new();
            for argument in list.arguments() {
                let item = match &argument {
                    ast::Argument::Arg(arg) => arg.expr().map(|expr| (String::new(), expr)),
                    ast::Argument::NamedArg(named) => (named.name_ref().zip(named.expr()))
                        .map(|(name, expr)| (format!("{} => ", name.syntax().text()), expr)),
                    _ => None,
                };
                let Some((before, expr)) = item else {
                    return text();
                };
                items.push((before, self.doc(expr.syntax())));
            }
            let clauses: Vec<String> = [
                list.null_treatment()
                    .map(|clause| clause.syntax().text_range()),
                list.having_clause()
                    .map(|clause| clause.syntax().text_range()),
                list.order_by().map(|clause| clause.syntax().text_range()),
                list.call_limit().map(|clause| clause.syntax().text_range()),
            ]
            .into_iter()
            .flatten()
            .map(|range| one_line(&self.at(range)))
            .collect();
            return Doc::Group {
                open: format!("{}(", one_line(&self.at(path.syntax().text_range()))),
                items,
                clauses,
                close: ")",
            };
        }
        if let Some(array) = ast::ArrayExpr::cast(node.clone()) {
            let Some(bracket) = array.l_brack() else {
                return text();
            };
            let head = TextRange::new(range.start(), bracket.text_range().start());
            let items = (array.exprs())
                .map(|expr| (String::new(), self.doc(expr.syntax())))
                .collect();
            return Doc::Group {
                open: format!("{}[", one_line(&self.at(head))),
                items,
                clauses: Vec::new(),
                close: "]",
            };
        }
        text()
    }
}

/// The pipe operator `head`, such as `|> AGGREGATE `, then `doc`, then `rest`, on a line that
/// starts with `indent`: on one line when it fits, and otherwise with `doc` in the layout of
/// nested calls and `clause`, such as a `GROUP BY`, on a line of its own at the depth of the
/// operator's keyword.
fn operator(indent: &str, head: &str, doc: &Doc, rest: &str, clause: &str) -> String {
    let column = width(indent) + width(head);
    let flat = doc.flat();
    let spaced = match clause.is_empty() {
        true => String::new(),
        false => format!(" {clause}"),
    };
    if column + width(&flat) + width(rest) + width(&spaced) <= WIDTH {
        return format!("{head}{flat}{rest}{spaced}");
    }
    let depth = width(indent) + 3;
    let mut text = format!("{head}{}{rest}", doc.render(column, depth, width(rest)));
    if !clause.is_empty() {
        text.push_str(&format!("\n{}{clause}", " ".repeat(depth)));
    }
    text
}

impl Analysis {
    /// The script `text`, whose analysis this is, as the script that its lifted calls plan:
    /// each pipe `AGGREGATE` that holds a mark call with statistics in its channels becomes the
    /// staged pipe query that a user could type for it, and the rest of the text stays as
    /// written. `None` for a script with an error.
    pub fn lift(&self, text: &str) -> Option<Lifted> {
        if !self.diagnostics.is_empty() {
            return None;
        }
        let root = avenger_sql_syntax::parse(text).syntax();
        let mut operators: Vec<&LiftedOperator> = self.lifted.iter().collect();
        operators.sort_by_key(|operator| operator.range.start());
        let mut out = String::new();
        let mut generated = Vec::new();
        let mut at = TextSize::from(0);
        for operator in operators {
            if operator.range.start() < at {
                continue;
            }
            out.push_str(&text[TextRange::new(at, operator.range.start())]);
            let start = TextSize::of(out.as_str());
            out.push_str(&operator.print(text, &root));
            generated.push(TextRange::new(start, TextSize::of(out.as_str())));
            at = operator.range.end();
        }
        out.push_str(&text[TextRange::new(at, TextSize::of(text))]);
        Some(Lifted {
            text: out,
            generated,
        })
    }
}

impl LiftedOperator {
    /// The stages of the operator, one operator to a line, in the layout of nested calls, where
    /// `root` is the syntax of `text`.
    fn print(&self, text: &str, root: &avenger_sql_syntax::SyntaxNode) -> String {
        let groups: Vec<String> = (self.groups.iter())
            .map(|(range, _)| collapsed(&text[*range]))
            .collect();
        let names: Vec<String> = self.groups.iter().map(|(_, name)| name.clone()).collect();
        let indent = indentation(text, self.range.start());
        let group_by = match names.is_empty() {
            true => String::new(),
            false => format!(
                "GROUP BY {}",
                names
                    .iter()
                    .map(|name| quoted(name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        // One item that is one lifted call plans as its stages alone.
        if let [item] = self.items.as_slice() {
            if let [call] = item.calls.as_slice() {
                if call.range == item.range && item.aggregates.is_empty() {
                    let mut writer = Writer {
                        text,
                        replaced: Vec::new(),
                    };
                    let (stages, mark) = call.stages(&mut writer, &groups, &names, &indent, root);
                    let mut out: Vec<String> = stages;
                    let alias = alias(&item.name);
                    out.push(operator(&indent, "|> AGGREGATE ", &mark, &alias, &group_by));
                    return out.join(&format!("\n{indent}"));
                }
            }
        }
        // The other items aggregate as written, and each lifted call is a branch that joins them,
        // a query in parentheses whose operators are each on a line, two spaces deeper than the
        // depth of the join's operator.
        let input = collapsed(&text[self.input]);
        let inner = format!("{indent}     ");
        let mut writer = Writer {
            text,
            replaced: Vec::new(),
        };
        let mut base: Vec<String> = Vec::new();
        let mut branches: Vec<String> = Vec::new();
        let mut number = 0;
        for (i, item) in self.items.iter().enumerate() {
            for (j, aggregate) in item.aggregates.iter().enumerate() {
                let name = format!("__item_{i}_{j}");
                base.push(format!("{} AS {name}", writer.at(*aggregate)));
                writer.replace(*aggregate, format!("__items.{name}"));
            }
            for call in &item.calls {
                let alias = format!("__lifting_{number}");
                number += 1;
                let mut branch = Writer {
                    text,
                    replaced: Vec::new(),
                };
                let (stages, mark) = call.stages(&mut branch, &groups, &names, &inner, root);
                let mut lines = vec![input.clone()];
                lines.extend(stages);
                lines.push(operator(
                    &inner,
                    "|> AGGREGATE ",
                    &mark,
                    " AS __mark",
                    &group_by,
                ));
                let query = format!(
                    "(\n{inner}{}\n{indent}   ) AS {alias}",
                    lines.join(&format!("\n{inner}"))
                );
                let join = match names.is_empty() {
                    true => format!("|> CROSS JOIN {query}"),
                    false => {
                        let on: Vec<String> = (names.iter())
                            .map(|name| {
                                format!(
                                    "__items.{0} IS NOT DISTINCT FROM {alias}.{0}",
                                    quoted(name)
                                )
                            })
                            .collect();
                        format!("|> LEFT JOIN {query} ON {}", on.join(" AND "))
                    }
                };
                branches.push(join);
                writer.replace(call.range, format!("{alias}.__mark"));
            }
        }
        if base.is_empty() && names.is_empty() {
            base.push("COUNT(*) AS __rows".to_string());
        }
        let mut aggregate = String::from("|> AGGREGATE");
        if !base.is_empty() {
            aggregate.push(' ');
            aggregate.push_str(&base.join(", "));
        }
        if !groups.is_empty() {
            aggregate.push_str(&format!(" GROUP BY {}", groups.join(", ")));
        }
        let mut select: Vec<String> = (names.iter())
            .map(|name| format!("__items.{0} AS {0}", quoted(name)))
            .collect();
        for item in &self.items {
            select.push(format!("{}{}", writer.at(item.range), alias(&item.name)));
        }
        let mut out = vec![aggregate, "|> AS __items".to_string()];
        out.extend(branches);
        out.push(format!("|> SELECT {}", select.join(", ")));
        out.join(&format!("\n{indent}"))
    }
}

/// The spaces at the start of the line that holds `offset`.
fn indentation(text: &str, offset: TextSize) -> String {
    let before = &text[..usize::from(offset)];
    let start = before.rfind('\n').map_or(0, |newline| newline + 1);
    before[start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}
