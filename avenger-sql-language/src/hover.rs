//! Hover: Markdown on the name, the call, or the value of a set at a cursor.

use avenger_sql::{
    analysis::{Column, Definition},
    library::{Function, FORM},
    types::googlesql_name,
};
use avenger_sql_syntax::{
    ast::{self, AstNode},
    SyntaxKind, SyntaxNode, TextRange,
};

use crate::{first_line, node_at, Document, Offset};

/// What an editor shows over a range of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub range: TextRange,
    pub markdown: String,
}

/// The longest text of a definition that a hover quotes, in characters.
const QUOTE: usize = 60;

/// The strings of a set that the hover of a parameter that takes constructors shows, before `…`.
/// Completion in the string lists them all.
pub(crate) const STRINGS: usize = 3;

impl Document {
    /// Markdown on what is at `offset`: a name that the analysis resolved, a string that a
    /// parameter takes as one value of a set, or in a data script, an option, a schema, or a
    /// name that a statement defines.
    pub fn hover(&self, offset: Offset) -> Option<Hover> {
        let offset = self.clamp(offset);
        let model = &self.model();
        let root = self.root();
        if let Some(value) = (model.enum_values.iter()).find(|v| v.range.contains_inclusive(offset))
        {
            let function = call_node(&root, value.range)
                .and_then(|call| self.called(&call))
                .map(|called| called.function);
            let text = self.slice(value.range);
            let mut markdown = format!("```googlesql\n{text}\n```\n\nA value of `{}`", value.param);
            if let Some(function) = function {
                if let Some(param) = function.param_named(&value.param) {
                    markdown.push_str(&format!(": {}", param.doc));
                }
                let values: Vec<String> = (function.values_of(&value.param).into_iter())
                    .map(|value| format!("'{value}'"))
                    .collect();
                if !values.is_empty() {
                    markdown.push_str(&format!("\n\nValues: {}", values.join(", ")));
                }
            }
            return Some(Hover {
                range: value.range,
                markdown,
            });
        }
        if let Some(hover) = self.argument_name(&root, offset) {
            return Some(hover);
        }
        if let Some((range, markdown)) = self.data_hover(&root, offset) {
            return Some(Hover { range, markdown });
        }
        let Some(name) = model.name_at(offset) else {
            // A read of a table that this script defines, which the analysis leaves unresolved
            // while the table has no schema, as a chart's own table before its files are read.
            let table = self.table_at(offset)?;
            let markdown = crate::data::table_hover(self.definitions(), table.own?);
            return Some(Hover {
                range: table.range,
                markdown,
            });
        };
        // A constructor of a chart's option, by the name that its place gives it.
        if let Some(constructor) = (model.constructors.iter()).rfind(|c| c.range == name.range) {
            let function = self.library.constructor(&constructor.constructor)?;
            let written = self.slice(name.range);
            return Some(Hover {
                range: name.range,
                markdown: constructor_hover(function, written, &constructor.place),
            });
        }
        let markdown = match &name.definition {
            Definition::Column(column) => self.column(column),
            Definition::RangeVariable { name: var, .. } => {
                let columns = (model.operator_at(offset).into_iter())
                    .flat_map(|scope| &scope.range_variables)
                    .find(|range_variable| range_variable.name.eq_ignore_ascii_case(var))
                    .map(|range_variable| columns(&range_variable.columns))
                    .unwrap_or_default();
                format!("```googlesql\n{var}\n```\n\nA range variable of the columns:\n\n{columns}")
            }
            Definition::Field { name, ty } => {
                format!(
                    "```googlesql\n{}\n```\n\nA field of a struct.",
                    typed(name, ty.as_ref().map(googlesql_name))
                )
            }
            Definition::Table { names } => self.table_read_hover(names, offset)?,
            Definition::Query { name, definition } => {
                let columns = (definition.and_then(|range| self.query_columns(&root, range)))
                    .map(|output| format!("\n\nA query of the columns:\n\n{}", columns(output)))
                    .unwrap_or_default();
                format!("```googlesql\n{name}\n```{columns}")
            }
            Definition::Variable { name, ty, .. } => match ty {
                Some(ty) => format!("```googlesql\nDECLARE {name} {}\n```", googlesql_name(ty)),
                None => format!("```googlesql\nDECLARE {name}\n```"),
            },
            Definition::Module { names } => {
                format!("```googlesql\nMODULE {}\n```", names.join("."))
            }
            Definition::Function {
                name: function_name,
                definition,
                ..
            } => match (definition, self.library.get(function_name)) {
                (Some(definition), _) => {
                    let header = node_at(&root, *definition, SyntaxKind::PATH)
                        .and_then(|path| path.parent())
                        .and_then(ast::CreateFunctionStmt::cast)
                        .map(|create| create_header(&create))
                        .unwrap_or_else(|| function_name.clone());
                    format!("```googlesql\n{header}\n```\n\nA temporary function.")
                }
                (None, Some(function)) => {
                    // The form that the call resolves to tells apart the forms of a function
                    // that has several.
                    let resolved = (function.forms.len() > 1)
                        .then(|| call_range(&root, name.range))
                        .flatten()
                        .and_then(|range| (model.calls.iter()).rfind(|call| call.range == range))
                        .map(|call| format!("\n\nResolved as `{}`", call.form))
                        .unwrap_or_default();
                    brief(function, &resolved)
                }
                (None, None) => {
                    format!("```googlesql\n{function_name}\n```\n\nA function of a module.")
                }
            },
        };
        Some(Hover {
            range: name.range,
            markdown,
        })
    }

    /// The parameter of the named argument whose name is at `offset`, in a call of a function of
    /// the library: its form, as the function's form writes it, or the forms of a parameter that
    /// takes constructors in place of its type, its documentation, and the constructors of the
    /// parts of its target.
    fn argument_name(&self, root: &SyntaxNode, offset: Offset) -> Option<Hover> {
        // The named argument whose name holds the offset, not an argument whose value holds the
        // call of the name, as at the start of `title` in `x => x(a, title => ...)`.
        let holds = |named: &ast::NamedArg| {
            (named.name_ref())
                .is_some_and(|name| name.syntax().text_range().contains_inclusive(offset))
        };
        let named = (root.token_at_offset(offset))
            .flat_map(|token| token.parent_ancestors())
            .filter_map(ast::NamedArg::cast)
            .find(holds)?;
        let name_ref = named.name_ref()?;
        let range = name_ref.syntax().text_range();
        let param = name_ref.text()?;
        let call = named.syntax().parent()?.parent()?;
        let called = self.called(&call)?;
        let function = called.function;
        let resolved = (self.model().calls.iter())
            .rfind(|form| form.range == call.text_range())
            .and_then(|form| form.index);
        let takes = |form: &avenger_sql::library::Form| {
            (form.signature.args.iter()).position(|arg| arg.param.eq_ignore_ascii_case(&param))
        };
        let (index, arg) =
            match resolved.and_then(|index| Some((index, takes(&function.forms[index])?))) {
                Some(found) => found,
                None => (function.forms.iter())
                    .enumerate()
                    .find_map(|(index, form)| Some((index, takes(form)?)))?,
            };
        let label = function.signatures_named(&called.name).swap_remove(index);
        let named = function.param_named(&param);
        let forms = named.and_then(|param| param.forms(Some(STRINGS)));
        let lines = match &forms {
            Some(forms) => forms.lines.join("\n"),
            None => label.text[label.args[arg].clone()].to_string(),
        };
        let mut markdown = format!(
            "```googlesql\n{lines}\n```\n\nA parameter of `{}`",
            called.name
        );
        if let Some(param) = named {
            markdown.push_str(&format!(": {}", param.doc));
            if param.takes.markup {
                markdown.push_str("\n\nIts strings are Typst markup.");
            }
        }
        if let Some(groups) = forms.and_then(|forms| forms.groups()) {
            markdown.push_str(&format!("\n\n{groups}"));
        }
        Some(Hover { range, markdown })
    }

    fn column(&self, column: &Column) -> String {
        let ty = column.ty.as_ref().map(googlesql_name);
        let mut markdown = format!("```googlesql\n{}\n```", typed(&column.name, ty));
        if let Some(definition) = column.definition {
            let text = first_line(self.slice(definition), QUOTE);
            markdown.push_str(&format!("\n\nDefined by `{text}`"));
        }
        markdown
    }

    /// The output columns of the query that the pipe `WITH` name or the temporary table at
    /// `definition` names: those of its last operator.
    fn query_columns(&self, root: &SyntaxNode, definition: TextRange) -> Option<&[Column]> {
        let query = match node_at(root, definition, SyntaxKind::NAME) {
            Some(name) => ast::NamedQuery::cast(name.parent()?)?
                .paren_query()?
                .query()?,
            None => {
                let path = node_at(root, definition, SyntaxKind::PATH)?;
                ast::CreateTableStmt::cast(path.parent()?)?.query()?
            }
        };
        let last = match query.pipe_operators().last() {
            Some(operator) => operator.syntax().text_range(),
            None => match (query.from_clause(), query.paren_query()) {
                (Some(from), _) => from.syntax().text_range(),
                (None, Some(paren)) => paren.syntax().text_range(),
                (None, None) => return None,
            },
        };
        (self.model().operators.iter())
            .rfind(|scope| scope.range == last)
            .map(|scope| scope.output.as_slice())
    }
}

/// `name: TYPE`, or `name` without a type.
fn typed(name: &str, ty: Option<String>) -> String {
    match ty {
        Some(ty) => format!("{name}: {ty}"),
        None => name.to_string(),
    }
}

/// A Markdown list of `columns` with their types.
fn columns(columns: &[Column]) -> String {
    (columns.iter())
        .filter(|column| !column.name.is_empty())
        .map(|column| match &column.ty {
            Some(ty) => format!("- `{}`: {}", column.name, googlesql_name(ty)),
            None => format!("- `{}`", column.name),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The call whose arguments hold `range`.
fn call_node(root: &SyntaxNode, range: TextRange) -> Option<SyntaxNode> {
    let element = root.covering_element(range);
    let start = match element {
        avenger_sql_syntax::SyntaxElement::Node(node) => node,
        avenger_sql_syntax::SyntaxElement::Token(token) => token.parent()?,
    };
    start
        .ancestors()
        .filter(|node| node.kind() == SyntaxKind::ARG_LIST)
        .find_map(|list| list.parent())
}

/// The hover of the constructor `function` under the name `written`, which fills `place`: its
/// form under that name, or `name(...)` when the form is longer than [`FORM`], the place, and its
/// summary and description.
fn constructor_hover(function: &Function, written: &str, place: &str) -> String {
    let form = match function.signatures_named(written).into_iter().next() {
        Some(label) if label.text.chars().count() <= FORM => label.text,
        _ => format!("{written}(...)"),
    };
    let mut markdown = format!(
        "```googlesql\n{form}\n```\n\nBuilds {place}.\n\n{}",
        function.summary
    );
    if !function.description.is_empty() && function.description != function.summary {
        markdown.push_str(&format!("\n\n{}", function.description));
    }
    markdown
}

/// The hover of the library function `function`: its forms, or `name(...)` and its result when a
/// form is longer than [`FORM`], then `resolved`, its summary, its description, and its notes.
/// The reference of the library holds its parameters, clauses, and examples, and the hover of an
/// argument's name shows its parameter.
fn brief(function: &Function, resolved: &str) -> String {
    let labels = function.signatures();
    let forms: Vec<String> = match labels
        .iter()
        .all(|label| label.text.chars().count() <= FORM)
    {
        true => labels.into_iter().map(|label| label.text).collect(),
        false => {
            let mut results: Vec<String> = (function.forms.iter())
                .map(|form| form.signature.result.to_string())
                .collect();
            results.dedup();
            vec![format!("{}(...) -> {}", function.name, results.join(" | "))]
        }
    };
    let mut markdown = format!(
        "```googlesql\n{}\n```{resolved}\n\n{}",
        forms.join("\n"),
        function.summary
    );
    if !function.description.is_empty() && function.description != function.summary {
        markdown.push_str(&format!("\n\n{}", function.description));
    }
    for note in &function.notes {
        markdown.push_str(&format!("\n\n**Note:** {note}"));
    }
    markdown
}

/// The name of the function that the call, table function source, or pipe `CALL` `node` calls,
/// without a `SAFE.` prefix.
pub(crate) fn callee(node: &SyntaxNode) -> Option<String> {
    let path = match node.kind() {
        SyntaxKind::CALL_EXPR => ast::CallExpr::cast(node.clone())?.path()?,
        SyntaxKind::TVF_SOURCE => ast::TvfSource::cast(node.clone())?.path()?,
        SyntaxKind::PIPE_CALL => ast::PipeCall::cast(node.clone())?.path()?,
        _ => return None,
    };
    let names = path.names()?;
    match names.as_slice() {
        [safe, name] if safe.eq_ignore_ascii_case("safe") => Some(name.clone()),
        _ => Some(names.join(".")),
    }
}

/// The range of the call whose function's name is at `range`.
fn call_range(root: &SyntaxNode, range: TextRange) -> Option<TextRange> {
    let path = node_at(root, range, SyntaxKind::PATH)?;
    let call = path.parent()?;
    matches!(
        call.kind(),
        SyntaxKind::CALL_EXPR | SyntaxKind::TVF_SOURCE | SyntaxKind::PIPE_CALL
    )
    .then(|| call.text_range())
}

/// The text of a `CREATE FUNCTION` before its body, on one line.
pub(crate) fn create_header(create: &ast::CreateFunctionStmt) -> String {
    let text = create.syntax().to_string();
    let start = create.syntax().text_range().start();
    let end = (create.function_body())
        .map(|body| usize::from(body.syntax().text_range().start() - start))
        .unwrap_or(text.len());
    text[..end].split_whitespace().collect::<Vec<_>>().join(" ")
}
