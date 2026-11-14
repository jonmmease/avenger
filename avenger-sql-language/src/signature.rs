//! Signature help: the forms of the function whose arguments hold a cursor.

use std::ops::Range;

use avenger_sql::library::{Arity, FormArg, Function};
use avenger_sql_syntax::{
    ast::{self, AstNode, CreateStmt},
    SyntaxKind, SyntaxNode,
};

use crate::{hover::callee, statements, Document, Offset};

/// The forms of a call's function, and the form and the parameter at the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInfo>,
    /// The index of the form that the call takes, among `signatures`.
    pub active_signature: usize,
}

/// A form of a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureInfo {
    /// The form as GoogleSQL writes it, such as `SUBSTR(value: STRING, position: INT64[,
    /// length: INT64]) -> STRING`.
    pub label: String,
    /// The function's summary.
    pub documentation: Option<String>,
    pub parameters: Vec<ParameterInfo>,
    /// The index of the parameter at the cursor, among `parameters`, when the form has one there.
    pub active_parameter: Option<usize>,
}

/// A parameter of a form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterInfo {
    /// The byte range of the parameter in the form's label.
    pub range: Range<usize>,
    pub documentation: Option<String>,
}

/// Where the cursor is among the arguments of a call.
struct At {
    /// The index of the argument among the call's positional arguments, when it is positional.
    position: Option<usize>,
    /// The name of a named argument, in lower case.
    name: Option<String>,
    /// The names of the call's named arguments, in lower case.
    named: Vec<String>,
    /// The number of the call's positional arguments, with the one at the cursor.
    positional: usize,
}

impl Document {
    /// The forms of the function of the call, table function, or pipe `CALL` whose argument list
    /// holds `offset`, when its function is a function of the library or a temporary function.
    pub fn signature_help(&self, offset: Offset) -> Option<SignatureHelp> {
        let offset = self.clamp(offset);
        let root = self.root();
        let token = root.token_at_offset(offset).left_biased()?;
        let list = (token.parent_ancestors())
            .filter_map(ast::ArgList::cast)
            .find(|list| {
                let open = list.l_paren().map(|paren| paren.text_range().end());
                let close = list.r_paren().map(|paren| paren.text_range().start());
                open.is_some_and(|open| open <= offset) && close.is_none_or(|close| offset <= close)
            })?;
        let call = list.syntax().parent()?;
        let name = callee(&call)?;
        let at = at(&list, offset);
        if let Some(create) = temporary_function(&root, &name, offset) {
            return Some(SignatureHelp {
                signatures: vec![temporary_signature(&create, &at)],
                active_signature: 0,
            });
        }
        let function = self.library.get(&name)?;
        let resolved = (self.analysis.model.calls.iter())
            .rfind(|form| form.range == call.text_range())
            .and_then(|form| form.index);
        let active_signature = resolved
            .or_else(|| (function.forms.iter()).position(|form| takes(&form.signature.args, &at)))
            .unwrap_or(0);
        let signatures = (function.forms.iter())
            .zip(function.signatures())
            .map(|(form, label)| SignatureInfo {
                parameters: (form.signature.args.iter())
                    .zip(label.args)
                    .map(|(arg, range)| ParameterInfo {
                        range,
                        documentation: function_doc(function, arg),
                    })
                    .collect(),
                active_parameter: active(&form.signature.args, &at),
                label: label.text,
                documentation: Some(function.summary.clone()),
            })
            .collect();
        Some(SignatureHelp {
            signatures,
            active_signature,
        })
    }
}

/// Where `offset` is among the arguments of `list`.
fn at(list: &ast::ArgList, offset: Offset) -> At {
    let mut at = At {
        position: None,
        name: None,
        named: Vec::new(),
        positional: 0,
    };
    // The argument at the cursor is the one after the commas before it.
    let index = (list.syntax().children_with_tokens())
        .filter(|element| {
            element.kind() == SyntaxKind::COMMA && element.text_range().end() <= offset
        })
        .count();
    let mut at_cursor = false;
    for (i, argument) in list.arguments().enumerate() {
        let here = i == index;
        at_cursor |= here;
        match argument {
            ast::Argument::NamedArg(named) => {
                let name = (named.name_ref().and_then(|name| name.text()))
                    .map(|name| name.to_ascii_lowercase());
                if here {
                    at.name = name.clone();
                }
                at.named.extend(name);
            }
            ast::Argument::NamedTableArg(named) => {
                let name = (named.name_ref().and_then(|name| name.text()))
                    .map(|name| name.to_ascii_lowercase());
                if here {
                    at.name = name.clone();
                }
                at.named.extend(name);
            }
            ast::Argument::Arg(_) | ast::Argument::TableArg(_) => {
                if here {
                    at.position = Some(at.positional);
                }
                at.positional += 1;
            }
        }
    }
    // A cursor after the last argument starts a positional one.
    if !at_cursor {
        at.position = Some(at.positional);
        at.positional += 1;
    }
    at
}

/// The index of the argument of a form with `args` at the cursor `at`.
fn active(args: &[FormArg], at: &At) -> Option<usize> {
    if let Some(name) = &at.name {
        return (args.iter()).position(|arg| arg.param.eq_ignore_ascii_case(name));
    }
    let position = at.position?;
    let positional: Vec<usize> = (args.iter().enumerate())
        .filter(|(_, arg)| !arg.named_only)
        .map(|(index, _)| index)
        .collect();
    match positional.get(position) {
        Some(&index) => Some(index),
        None => (positional.last().copied()).filter(|&index| args[index].arity == Arity::Repeated),
    }
}

/// Whether a form with `args` takes the call's positional arguments and its named arguments.
fn takes(args: &[FormArg], at: &At) -> bool {
    let positional: Vec<&FormArg> = args.iter().filter(|arg| !arg.named_only).collect();
    let fits = positional.len() >= at.positional
        || positional
            .last()
            .is_some_and(|arg| arg.arity == Arity::Repeated);
    fits && (at.named.iter())
        .all(|name| args.iter().any(|arg| arg.param.eq_ignore_ascii_case(name)))
}

/// The doc of the parameter of `arg`, when the function has one.
fn function_doc(function: &Function, arg: &FormArg) -> Option<String> {
    (function.param_named(&arg.param)).map(|param| param.doc.clone())
}

/// The `CREATE FUNCTION` of the script's temporary function `name` before `offset`, the last
/// one when the script replaces it.
fn temporary_function(
    root: &SyntaxNode,
    name: &str,
    offset: Offset,
) -> Option<ast::CreateFunctionStmt> {
    statements(root)
        .filter_map(|stmt| match stmt {
            ast::Stmt::CreateFunctionStmt(create) => Some(create),
            _ => None,
        })
        .filter(|create| create.syntax().text_range().end() <= offset)
        .filter(|create| {
            (create.name_path().and_then(|path| path.names()))
                .is_some_and(|names| names.join(".").eq_ignore_ascii_case(name))
        })
        .last()
}

/// The signature of a temporary function, as its `CREATE` writes its parameters.
fn temporary_signature(create: &ast::CreateFunctionStmt, at: &At) -> SignatureInfo {
    let name = (create.name_path())
        .map(|path| path.syntax().to_string())
        .unwrap_or_default();
    let mut label = format!("{name}(");
    let mut parameters = Vec::new();
    let mut names = Vec::new();
    for (index, parameter) in (create.param_list().iter())
        .flat_map(|list| list.parameters())
        .enumerate()
    {
        if index > 0 {
            label.push_str(", ");
        }
        let text = collapse(&parameter.syntax().to_string());
        parameters.push(ParameterInfo {
            range: label.len()..label.len() + text.len(),
            documentation: None,
        });
        label.push_str(&text);
        names.push(
            parameter
                .name()
                .and_then(|name| name.text())
                .unwrap_or_default(),
        );
    }
    label.push(')');
    if let Some(returns) = create.returns_clause() {
        label.push(' ');
        label.push_str(&collapse(&returns.syntax().to_string()));
    }
    let active_parameter = match &at.name {
        Some(name) => names
            .iter()
            .position(|param| param.eq_ignore_ascii_case(name)),
        None => at.position.filter(|&position| position < parameters.len()),
    };
    SignatureInfo {
        label,
        documentation: None,
        parameters,
        active_parameter,
    }
}

/// `text` with each run of whitespace as one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
