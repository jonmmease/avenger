//! Selections: a `DECLARE name SELECTION`, which a select tool writes, and its reads, which are
//! true for the rows that it selects.
//!
//! A selection is not a value: a read of it lowers to a call of avenger-selection-datafusion's
//! `selected` on a placeholder of its state, `$__sel_<name>`, which a run binds with
//! `bind_selections` before it binds the values of the variables. A read needs rows, so a
//! selection can't stand where a constant does, and it can't be compared or assigned.

use std::collections::BTreeSet;

use avenger_selection::Resolution;
use avenger_sql_syntax::{
    ast::{self, AstNode},
    TextRange,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{TreeNode, TreeNodeRecursion},
        Result, ScalarValue,
    },
    logical_expr::{
        expr::{Placeholder, ScalarFunction},
        Expr, LogicalPlan,
    },
};

use super::{expr::Scope, model::Definition, Pass, StatementPlan};

/// A selection of a script, by its `DECLARE`.
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub name: String,
    /// How the selection combines the clauses of its owners, `'global'` by default.
    pub resolution: Resolution,
    /// The range of its name in its `DECLARE`.
    pub definition: TextRange,
}

/// The name of the placeholder of the state of the selection `name`, without its `$`, as a run
/// binds it.
pub fn placeholder(name: &str) -> String {
    format!("__sel_{name}")
}

/// The name of the selection whose state the placeholder `id`, with or without its `$`, holds.
fn selection_of_placeholder(id: &str) -> Option<&str> {
    id.trim_start_matches('$').strip_prefix("__sel_")
}

/// The names of the selections whose states `plan` reads, by the placeholders of its `selected`
/// calls.
pub fn reads(plan: &LogicalPlan) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    plan.apply_with_subqueries(|node| {
        node.apply_expressions(|expr| {
            expr.apply(|expr| {
                if let Expr::Placeholder(Placeholder { id, .. }) = expr {
                    if let Some(name) = selection_of_placeholder(id) {
                        names.insert(name.to_string());
                    }
                }
                Ok(TreeNodeRecursion::Continue)
            })
        })
    })?;
    Ok(names)
}

/// Whether `declare` declares selections: its type is `SELECTION`.
pub(super) fn declares_selection(declare: &ast::DeclareStmt) -> bool {
    let Some(ast::Type::NamedType(named)) = declare.ty() else {
        return false;
    };
    let name = (named.name_ref()).and_then(|name| name.text());
    name.is_some_and(|name| name.eq_ignore_ascii_case("SELECTION")) && named.type_params().is_none()
}

/// The resolutions of a selection, by the values of the option `resolution`.
const RESOLUTIONS: [(&str, Resolution); 3] = [
    ("global", Resolution::Global),
    ("union", Resolution::Union),
    ("intersect", Resolution::Intersect),
];

/// The resolution of the value `text` of the option `resolution`.
fn resolution(text: &str) -> Option<Resolution> {
    (RESOLUTIONS.iter())
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
        .map(|(_, resolution)| *resolution)
}

/// The value of the option `resolution` of `resolution`.
pub fn resolution_name(resolution: Resolution) -> &'static str {
    (RESOLUTIONS.iter())
        .find(|(_, other)| *other == resolution)
        .map(|(name, _)| *name)
        .expect("every resolution has a name")
}

/// Whether `expr` reads a selection directly, as a selection's name or a `selected` call does.
pub(super) fn is_read(expr: &Expr) -> bool {
    matches!(expr, Expr::ScalarFunction(call) if call.func.name() == "selected")
}

impl Pass<'_> {
    /// The selections of a `DECLARE ... SELECTION`, which plans nothing: each name's selection,
    /// with the resolution of its `OPTIONS`.
    pub(super) fn declare_selection(&mut self, declare: &ast::DeclareStmt) -> Vec<StatementPlan> {
        if let Some(expr) = declare.expr() {
            self.error(
                expr.syntax().text_range(),
                "a SELECTION takes no DEFAULT: it starts empty, and its tools write it",
            );
            return Vec::new();
        }
        let mut resolved = Resolution::Global;
        if let Some(options) = declare.options() {
            let mut seen = false;
            for entry in options
                .syntax()
                .children()
                .filter_map(ast::OptionEntry::cast)
            {
                let Some(name) = entry.name() else {
                    continue;
                };
                let (text, at) = (name.text().unwrap_or_default(), name.syntax().text_range());
                if !text.eq_ignore_ascii_case("resolution") {
                    self.error(
                        at,
                        format!("a SELECTION takes the option `resolution`, not `{text}`"),
                    );
                    return Vec::new();
                }
                if seen {
                    self.error(at, "a SELECTION sets `resolution` once");
                    return Vec::new();
                }
                seen = true;
                let value = match entry.expr() {
                    Some(ast::Expr::Literal(literal)) => literal.string_value(),
                    _ => None,
                };
                let at = (entry.expr()).map_or(at, |expr| expr.syntax().text_range());
                match value.as_deref().and_then(resolution) {
                    Some(found) => resolved = found,
                    None => {
                        self.error(at, "`resolution` is 'global', 'union', or 'intersect'");
                        return Vec::new();
                    }
                }
            }
        }
        for name in declare.names() {
            let (Some(text), at) = (name.text(), name.syntax().text_range()) else {
                continue;
            };
            if self.declared(&text) {
                self.error(at, format!("Variable '{text}' redeclaration"));
                continue;
            }
            self.selections.push(Selection {
                name: text,
                resolution: resolved,
                definition: at,
            });
        }
        Vec::new()
    }

    /// Whether the script declares a variable or a selection of the name `name`.
    pub(super) fn declared(&self, name: &str) -> bool {
        let named = |other: &str| other.eq_ignore_ascii_case(name);
        (self.variables.iter()).any(|variable| named(&variable.name))
            || (self.selections.iter()).any(|selection| named(&selection.name))
    }

    /// The selection `name`, as the name at `range` of the model, when the script declares one.
    pub(super) fn use_selection(&mut self, name: &str, range: TextRange) -> Option<Selection> {
        let selection = (self.selections.iter())
            .find(|selection| selection.name.eq_ignore_ascii_case(name))
            .cloned()?;
        self.use_name(
            range,
            Definition::Selection {
                name: selection.name.clone(),
                definition: Some(selection.definition),
                resolution: selection.resolution,
            },
        );
        Some(selection)
    }

    /// The read of the selection `name` at `range` in `scope`, whether it selects each row, when
    /// the script declares one: `Some(None)` after an error, for a scope without rows.
    pub(super) fn selection_read(
        &mut self,
        name: &str,
        range: TextRange,
        scope: Scope,
    ) -> Option<Option<Expr>> {
        let selection = self.use_selection(name, range)?;
        if !has_rows(&scope) {
            self.error(range, not_a_value(&selection.name, scope.clause));
            return Some(None);
        }
        Some(Some(selected(&selection, None, &[])))
    }

    /// A call of `selected(selection, empty => ..., exclude => ...)`: the read of the selection,
    /// with what an empty selection selects and the ids of the marks whose clauses it skips.
    pub(super) fn selected_call(
        &mut self,
        call: &ast::CallExpr,
        scope: Scope,
        range: TextRange,
    ) -> Option<Expr> {
        let args: Vec<ast::Argument> = (call.arg_list())
            .map(|list| list.arguments().collect())
            .unwrap_or_default();
        let first = match args.first() {
            Some(ast::Argument::Arg(arg)) => Some(arg),
            _ => None,
        };
        let reference = first.and_then(|arg| match arg.expr() {
            Some(ast::Expr::PathExpr(path)) => {
                let refs: Vec<ast::NameRef> = path.path()?.name_refs().collect();
                match refs.as_slice() {
                    [name] => Some((name.text()?, name.syntax().text_range())),
                    _ => None,
                }
            }
            _ => None,
        });
        let Some((name, at)) = reference else {
            let at = first.map_or(range, |arg| arg.syntax().text_range());
            self.error(
                at,
                "`selected` takes a selection first, as `selected(brush)`",
            );
            return None;
        };
        let Some(selection) = self.use_selection(&name, at) else {
            self.error(at, format!("`{name}` is not a selection of the script"));
            return None;
        };
        if !has_rows(&scope) {
            self.error(range, not_a_value(&selection.name, scope.clause));
            return None;
        }
        let mut empty = None;
        let mut without = Vec::new();
        for arg in args.iter().skip(1) {
            let at = arg.syntax().text_range();
            let ast::Argument::NamedArg(named) = arg else {
                self.error(
                    at,
                    "`selected` takes its options by name: `empty`, `exclude`",
                );
                return None;
            };
            let param = (named.name_ref())
                .and_then(|name| name.text())
                .unwrap_or_default();
            let value = named.expr();
            match param.to_ascii_lowercase().as_str() {
                "empty" => {
                    let text = match &value {
                        Some(ast::Expr::Literal(literal)) => literal.string_value(),
                        _ => None,
                    };
                    match text.map(|text| text.to_ascii_lowercase()) {
                        Some(text) if text == "all" || text == "none" => empty = Some(text),
                        _ => {
                            self.error(at, "`empty` is 'all' or 'none'");
                            return None;
                        }
                    }
                }
                "exclude" => match value.as_ref().and_then(ids) {
                    Some(ids) => without = ids,
                    None => {
                        self.error(
                            at,
                            "`exclude` takes the id of a mark, as 'scatter', or an array of \
                             them, as ['scatter', 'bars']",
                        );
                        return None;
                    }
                },
                other => {
                    self.error(
                        at,
                        format!(
                            "`selected` takes the options `empty` and `exclude`, not `{other}`"
                        ),
                    );
                    return None;
                }
            }
        }
        Some(selected(&selection, empty.as_deref(), &without))
    }
}

/// Whether `scope` has rows, which a read of a selection tests.
fn has_rows(scope: &Scope) -> bool {
    !(scope.columns.is_empty() && scope.vars.is_empty())
}

/// The error of a read of the selection `name` where a value goes, in the clause `clause`.
pub(super) fn not_a_value(name: &str, clause: &str) -> String {
    format!(
        "`{name}` is a selection, which tests rows, not a value: {clause} can't read it, so test \
         rows with it, as `WHERE {name}`"
    )
}

/// The string literals of `expr`: one, or an array of them.
fn ids(expr: &ast::Expr) -> Option<Vec<String>> {
    match expr {
        ast::Expr::Literal(literal) => Some(vec![literal.string_value()?]),
        ast::Expr::ArrayExpr(array) => (array.exprs())
            .map(|element| match element {
                ast::Expr::Literal(literal) => literal.string_value(),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// The call of `selected` on the state of `selection`, with what an empty selection selects,
/// and the owners whose clauses it skips.
fn selected(selection: &Selection, empty: Option<&str>, without: &[String]) -> Expr {
    let null = || Expr::Literal(ScalarValue::Null, None);
    let state = Expr::Placeholder(Placeholder::new_with_field(
        format!("${}", placeholder(&selection.name)),
        None,
    ));
    let mut args = vec![state];
    if empty.is_some() || !without.is_empty() {
        args.extend([null(), null(), null()]);
        args.push(match empty {
            Some(empty) => Expr::Literal(ScalarValue::from(empty), None),
            None => null(),
        });
    }
    if !without.is_empty() {
        let ids: Vec<ScalarValue> = without
            .iter()
            .map(|id| ScalarValue::from(id.as_str()))
            .collect();
        let list = ScalarValue::List(ScalarValue::new_list_nullable(&ids, &DataType::Utf8));
        args.push(Expr::Literal(list, None));
    }
    Expr::ScalarFunction(ScalarFunction::new_udf(
        avenger_selection_datafusion::selected(),
        args,
    ))
}
