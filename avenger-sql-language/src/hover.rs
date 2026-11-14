//! Hover: Markdown on the name, the call, or the value of a set at a cursor.

use avenger_sql::{
    analysis::{Column, Definition},
    library::{entry, Accept, Function},
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

impl Document {
    /// Markdown on what is at `offset`: a name that the analysis resolved, or a string that a
    /// parameter takes as one value of a set.
    pub fn hover(&self, offset: Offset) -> Option<Hover> {
        let offset = self.clamp(offset);
        let model = &self.analysis.model;
        let root = self.root();
        if let Some(value) = (model.enum_values.iter()).find(|v| v.range.contains_inclusive(offset))
        {
            let function =
                call_function(&root, value.range).and_then(|name| self.library.get(&name));
            let text = self.slice(value.range);
            let mut markdown = format!("```googlesql\n{text}\n```\n\nA value of `{}`", value.param);
            if let Some(function) = function {
                if let Some(param) = function.param_named(&value.param) {
                    markdown.push_str(&format!(": {}", param.doc));
                }
                let values = one_of(function, &value.param);
                if !values.is_empty() {
                    markdown.push_str(&format!("\n\nValues: {}", values.join(", ")));
                }
            }
            return Some(Hover {
                range: value.range,
                markdown,
            });
        }
        let name = model.name_at(offset)?;
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
            Definition::Table { names } => {
                let path = names.join(".");
                let table = self.catalog.table(names)?;
                let schema = table.schema();
                let columns: Vec<String> = (schema.fields().iter())
                    .map(|field| {
                        format!(
                            "- `{}`: {}",
                            field.name(),
                            googlesql_name(field.data_type())
                        )
                    })
                    .collect();
                format!("```googlesql\nTABLE {path}\n```\n\n{}", columns.join("\n"))
            }
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
                    let resolved = call_range(&root, name.range)
                        .and_then(|range| (model.calls.iter()).rfind(|call| call.range == range))
                        .map(|call| format!("Resolved as `{}`\n\n", call.form))
                        .unwrap_or_default();
                    format!("{resolved}{}", entry(function))
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
        (self.analysis.model.operators.iter())
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

/// The values of the sets that the parameter `param` of `function` takes, as string literals.
fn one_of(function: &Function, param: &str) -> Vec<String> {
    let mut values: Vec<String> = Vec::new();
    for arg in (function.forms.iter()).flat_map(|form| &form.signature.args) {
        if let (true, Accept::OneOf(set)) = (arg.param.eq_ignore_ascii_case(param), &arg.accept) {
            for value in set {
                let value = format!("'{value}'");
                if !values.contains(&value) {
                    values.push(value);
                }
            }
        }
    }
    values
}

/// The name of the function of the call whose arguments hold `range`.
fn call_function(root: &SyntaxNode, range: TextRange) -> Option<String> {
    let element = root.covering_element(range);
    let start = match element {
        avenger_sql_syntax::SyntaxElement::Node(node) => node,
        avenger_sql_syntax::SyntaxElement::Token(token) => token.parent()?,
    };
    start
        .ancestors()
        .filter(|node| node.kind() == SyntaxKind::ARG_LIST)
        .find_map(|list| callee(&list.parent()?))
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
