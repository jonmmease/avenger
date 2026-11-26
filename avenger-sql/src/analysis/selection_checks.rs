//! The checks of a script's selections against the tools that write them: each selection's
//! fields, the columns that its tools' clauses test, which every read of it needs among the
//! columns of its rows, the ids of `exclude`, and the warning of a mark that filters by its own
//! selection. The checks run after the pass, so a tool in one statement or one Quarto cell checks
//! the reads in every other.

use super::{
    lifting::Written,
    selection::{named, select_calls, single_name},
    Diagnostic, Pass,
};
use avenger_sql_syntax::{
    ast::{self, AstNode},
    SyntaxNode, TextRange,
};

/// The positional channels, whose columns a box or a lasso tests when its tool names none.
const POSITIONS: [&str; 6] = ["x", "y", "theta", "radius", "longitude", "latitude"];

/// A select tool of a mark.
#[derive(Clone, Debug)]
pub(super) struct Write {
    pub selection: String,
    /// The columns that the tool's clauses test, or `None` when the tool does not name them, as a
    /// point tool without keys.
    pub fields: Option<Vec<String>>,
    /// The mark's own `id`.
    pub id: Option<String>,
    pub statement: usize,
    /// The range of the mark's call.
    pub mark: TextRange,
}

/// A read of a selection.
#[derive(Clone, Debug)]
pub(super) struct Read {
    pub selection: String,
    pub range: TextRange,
    /// The columns of the rows that the read tests: the rows in scope of a query's read, and the
    /// items of its mark for a restyle, or `None` until the mark's channels are known.
    pub columns: Option<Vec<String>>,
    /// Whether the read restyles a mark.
    pub restyle: bool,
    /// The ids of `exclude`, each with its range.
    pub excluded: Vec<(String, TextRange)>,
    pub statement: usize,
}

impl Pass<'_> {
    /// Records `read`, once for its range, as a lifted call plans its arguments again.
    pub(super) fn read(&mut self, read: Read) {
        let known = (self.selection_reads.iter())
            .any(|other| other.range == read.range && other.selection == read.selection);
        if !known {
            self.selection_reads.push(read);
        }
    }

    /// The reads of selections inside `range`, the value of a channel that a restyle tests: they
    /// test the items of their mark, or none after the restyle's error.
    pub(super) fn restyle_reads(&mut self, range: TextRange, restyled: bool) {
        match restyled {
            true => {
                for read in &mut self.selection_reads {
                    if range.contains_range(read.range) {
                        read.restyle = true;
                        read.columns = None;
                    }
                }
            }
            false => (self.selection_reads).retain(|read| !range.contains_range(read.range)),
        }
    }

    /// Records the select tools of the mark call at `range`, whose arguments are `list` and
    /// whose channels are `written`, and gives its restyles the columns of its items.
    pub(super) fn mark_selections(
        &mut self,
        list: Option<&ast::ArgList>,
        written: &[Written],
        range: TextRange,
    ) {
        let columns: Vec<(String, String)> = (written.iter())
            .map(|channel| {
                let column = selection_field(list, &channel.channel)
                    .unwrap_or_else(|| crate::functions::declaration::column_name(&channel.value));
                (channel.channel.clone(), column)
            })
            .collect();
        for read in &mut self.selection_reads {
            if read.restyle && read.columns.is_none() && range.contains_range(read.range) {
                read.columns = Some(columns.iter().map(|(_, column)| column.clone()).collect());
            }
        }
        if self
            .selection_writes
            .iter()
            .any(|write| write.mark == range)
        {
            return;
        }
        let id = named(list, "id").and_then(|expr| string(&expr));
        for select in select_calls(list) {
            let first = (select.arg_list()).and_then(|list| list.arguments().next());
            let Some((selection, _)) = first.and_then(|arg| match arg {
                ast::Argument::Arg(arg) => single_name(&arg.expr()?),
                _ => None,
            }) else {
                continue;
            };
            let column = |channel: &str| {
                (columns.iter())
                    .find(|(name, _)| name == channel)
                    .map(|(_, column)| column.clone())
            };
            let positions = || {
                Some(
                    POSITIONS
                        .iter()
                        .filter_map(|channel| column(channel))
                        .collect(),
                )
            };
            let kind = named(select.arg_list().as_ref(), "type");
            let kind = kind.as_ref().and_then(|kind| match kind {
                ast::Expr::CallExpr(call) => Some((call_name(call)?, call.arg_list())),
                _ => None,
            });
            let fields = match kind {
                None => positions(),
                Some((name, list)) if name == "interval" => {
                    match named(list.as_ref(), "channels").and_then(|channels| strings(&channels)) {
                        Some(channels) => channels.iter().map(|channel| column(channel)).collect(),
                        None => positions(),
                    }
                }
                Some((name, _)) if name == "lasso" => positions(),
                Some((name, list)) if name == "point" => {
                    named(list.as_ref(), "keys").and_then(|keys| strings(&keys))
                }
                Some(_) => None,
            };
            self.selection_writes.push(Write {
                selection: selection.to_string(),
                fields,
                id: id.clone(),
                statement: self.statement,
                mark: range,
            });
        }
    }

    /// The errors of the reads of selections whose rows lack a field of the selection, and of
    /// `exclude` ids that name no mark that writes it, and the warnings of a mark that filters by
    /// its own selection. A selection that no tool of the script writes has nothing to check.
    pub(super) fn check_selections(&mut self, root: &SyntaxNode) {
        let reads = std::mem::take(&mut self.selection_reads);
        let writes = std::mem::take(&mut self.selection_writes);
        for read in &reads {
            let writers: Vec<&Write> = (writes.iter())
                .filter(|write| write.selection.eq_ignore_ascii_case(&read.selection))
                .collect();
            let fields: Vec<&String> = (writers.iter())
                .flat_map(|write| write.fields.iter().flatten())
                .collect();
            if let Some(columns) = &read.columns {
                let lacks = |field: &&String| {
                    !columns
                        .iter()
                        .any(|column| column.eq_ignore_ascii_case(field))
                };
                if let Some(field) = fields.iter().copied().find(lacks) {
                    let name = &read.selection;
                    let message = match read.restyle {
                        true => format!(
                            "`{name}` tests `{field}`, which the items of this mark lack, so it \
                             can't restyle them: filter the rows with `WHERE {name}` before the \
                             mark reads them, or restyle by a column that its items carry"
                        ),
                        false => format!(
                            "`{name}` tests `{field}`, which these rows lack: read it where the \
                             rows carry `{field}`, or name the column that they carry with the \
                             `selection_field` of the tool's channel"
                        ),
                    };
                    self.error(read.range, message);
                }
            }
            // A selection that no tool of the script writes has no ids to check.
            for (id, at) in read.excluded.iter().filter(|_| !writers.is_empty()) {
                let writes_it = (writers.iter()).any(|write| write.id.as_deref() == Some(id));
                if !writes_it {
                    self.error(
                        *at,
                        format!(
                            "`{id}` is not the `id` of a mark whose select tool writes `{}`, \
                             which `exclude` names",
                            read.selection
                        ),
                    );
                }
            }
            if read.restyle {
                continue;
            }
            // A read filters the rows of a mark when it comes before the mark in the mark's own
            // pipe query, and not in a query that the mark's query joins or reads otherwise.
            let pipeline = query_of(root, read.range);
            let filters = |write: &&&Write| {
                write.statement == read.statement
                    && read.range.start() < write.mark.start()
                    && query_of(root, write.mark) == pipeline
            };
            for write in writers.iter().filter(filters) {
                let skipped =
                    (read.excluded.iter()).any(|(id, _)| write.id.as_deref() == Some(id.as_str()));
                if skipped {
                    continue;
                }
                let name = &read.selection;
                let fix = match &write.id {
                    Some(id) => {
                        format!("skip its clauses with `selected({name}, exclude => '{id}')`")
                    }
                    None => format!(
                        "give the mark an `id` and skip its clauses with `selected({name}, \
                         exclude => ...)`"
                    ),
                };
                self.warnings.push(Diagnostic {
                    range: read.range,
                    message: format!(
                        "`{name}` filters the rows of the mark that writes it, so its rows change \
                         under the pointer: {fix}"
                    ),
                    code: None,
                });
            }
        }
    }
}

/// The range of the innermost query of `root` that holds `range`.
fn query_of(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let element = root.covering_element(range);
    let node = element.as_node().cloned().or_else(|| element.parent())?;
    (node.ancestors().find_map(ast::Query::cast)).map(|query| query.syntax().text_range())
}

/// The name of the function that `call` calls, in lower case, when it has one name.
fn call_name(call: &ast::CallExpr) -> Option<String> {
    match call.path()?.names()?.as_slice() {
        [name] => Some(name.to_ascii_lowercase()),
        _ => None,
    }
}

/// The text of the string literal `expr`.
fn string(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::Literal(literal) => literal.string_value(),
        _ => None,
    }
}

/// The texts of the string literals of the array `expr`.
fn strings(expr: &ast::Expr) -> Option<Vec<String>> {
    match expr {
        ast::Expr::ArrayExpr(array) => array.exprs().map(|element| string(&element)).collect(),
        _ => None,
    }
}

/// The `selection_field` of the channel function of the channel `channel` of the call `list`,
/// which names the column of its clauses.
fn selection_field(list: Option<&ast::ArgList>, channel: &str) -> Option<String> {
    match named(list, channel)? {
        ast::Expr::CallExpr(call) if call_name(&call).as_deref() == Some(channel) => {
            named(call.arg_list().as_ref(), "selection_field").and_then(|field| string(&field))
        }
        _ => None,
    }
}
