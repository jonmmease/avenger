//! Selections: a `DECLARE name SELECTION`, which a select tool writes, and its reads, which are
//! true for the rows that it selects.
//!
//! A selection is not a value: a read of it lowers to a call of avenger-selection-datafusion's
//! `selected` on a placeholder of its state, `$__sel_<name>`, which a run binds with
//! `bind_selections` before it binds the values of the variables. A read needs rows, so a
//! selection can't stand where a constant does, and it can't be compared or assigned.

use avenger_selection::Resolution;
use avenger_sql_syntax::{
    ast::{self, AstNode},
    TextRange,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{tree_node::TreeNode, ScalarValue},
    logical_expr::{
        expr::{Placeholder, ScalarFunction},
        Expr,
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

    /// The read of `selection` at `range` in `scope`, whether it selects each row, with what an
    /// empty selection selects and the ids of the marks whose clauses it skips, or `None` after
    /// the error of a scope without rows.
    pub(super) fn read_selection(
        &mut self,
        selection: &Selection,
        range: TextRange,
        scope: Scope,
        empty: Option<&str>,
        without: &[String],
    ) -> Option<Expr> {
        if !has_rows(&scope) {
            self.error(range, not_a_value(&selection.name, scope.clause));
            return None;
        }
        Some(selected(selection, empty, without))
    }

    /// A call of `selected(selection, empty => ..., exclude => ...)`: the read of the selection,
    /// with what an empty selection selects and the ids of the marks whose clauses it skips.
    pub(super) fn selected_call(
        &mut self,
        call: &ast::CallExpr,
        scope: Scope,
        range: TextRange,
    ) -> Option<Expr> {
        let list = call.arg_list();
        let args = self.arguments(list.iter().flat_map(|list| list.arguments()), "selected")?;
        let reference = match args.positional.as_slice() {
            [(value, _)] => single_name(value),
            _ => None,
        };
        let Some((name, at)) = reference else {
            let at = args.positional.first().map_or(range, |(_, at)| *at);
            self.error(
                at,
                "`selected` takes one selection first, as `selected(brush)`",
            );
            return None;
        };
        let Some(selection) = self.use_selection(&name, at) else {
            self.error(at, format!("`{name}` is not a selection of the script"));
            return None;
        };
        let mut empty = None;
        let mut without = Vec::new();
        for (param, value, at) in &args.named {
            match param.as_str() {
                "empty" => {
                    let text = match value {
                        ast::Expr::Literal(literal) => literal.string_value(),
                        _ => None,
                    };
                    match text.map(|text| text.to_ascii_lowercase()) {
                        Some(text) if text == "all" || text == "none" => empty = Some(text),
                        _ => {
                            self.error(*at, "`empty` is 'all' or 'none'");
                            return None;
                        }
                    }
                }
                "exclude" => match ids(value) {
                    Some(ids) => without = ids,
                    None => {
                        self.error(
                            *at,
                            "`exclude` takes the id of a mark, as 'scatter', or an array of \
                             them, as ['scatter', 'bars']",
                        );
                        return None;
                    }
                },
                other => {
                    self.error(
                        *at,
                        format!(
                            "`selected` takes the options `empty` and `exclude`, not `{other}`"
                        ),
                    );
                    return None;
                }
            }
        }
        self.read_selection(&selection, range, scope, empty.as_deref(), &without)
    }
}

impl Pass<'_> {
    /// The name of the selection that `value`, the first argument of a select tool, writes: a
    /// selection that the script declares, by its name.
    pub(super) fn written_selection(&mut self, value: &ast::Expr) -> Option<String> {
        let reference = single_name(value);
        let selection = reference.and_then(|(name, at)| self.use_selection(&name, at));
        if selection.is_none() {
            self.error(
                value.syntax().text_range(),
                "a select tool writes a selection that the script declares, as `select(brush)` \
                 after `DECLARE brush SELECTION`",
            );
        }
        selection.map(|selection| selection.name)
    }

    /// The id of the mark call `fname` at `range`, whose arguments are `list`, when it has a select
    /// tool and no `id`: the mark's name and the call's number among such calls of the script,
    /// so it stays while the script keeps its marks with select tools in their order.
    pub(super) fn generated_id(
        &mut self,
        list: Option<&ast::ArgList>,
        fname: &str,
        range: TextRange,
    ) -> Option<String> {
        if named(list, "id").is_some() || select_calls(list).next().is_none() {
            return None;
        }
        if let Some((_, id)) = self.generated_ids.iter().find(|(at, _)| *at == range) {
            return Some(id.clone());
        }
        let id = format!("{fname}_{}", self.generated_ids.len() + 1);
        self.generated_ids.push((range, id.clone()));
        Some(id)
    }
}

/// The name of the one name that `value` reads, and its range.
pub(super) fn single_name(value: &ast::Expr) -> Option<(String, TextRange)> {
    let ast::Expr::PathExpr(path) = value else {
        return None;
    };
    match path.path()?.name_refs().collect::<Vec<_>>().as_slice() {
        [name] => Some((name.text()?, name.syntax().text_range())),
        _ => None,
    }
}

/// The value of the named argument `name` of the call whose arguments are `list`.
pub(super) fn named(list: Option<&ast::ArgList>, name: &str) -> Option<ast::Expr> {
    (list.into_iter().flat_map(|list| list.arguments())).find_map(|argument| match argument {
        ast::Argument::NamedArg(arg)
            if (arg.name_ref().and_then(|name| name.text()))
                .is_some_and(|text| text.eq_ignore_ascii_case(name)) =>
        {
            arg.expr()
        }
        _ => None,
    })
}

/// The calls of `select(...)` in the `tools` of the mark call whose arguments are `list`.
pub(super) fn select_calls(list: Option<&ast::ArgList>) -> impl Iterator<Item = ast::CallExpr> {
    let tools = named(list, "tools");
    (tools
        .into_iter()
        .flat_map(|tools| tools.syntax().descendants()))
    .filter_map(ast::CallExpr::cast)
    .filter(|call| {
        let names = call
            .path()
            .and_then(|path| path.names())
            .unwrap_or_default();
        matches!(names.as_slice(), [name] if name.eq_ignore_ascii_case("select"))
    })
}

impl Pass<'_> {
    /// The channel argument `expr` of a mark, at `at`, with a searched `CASE` or an `IF` over
    /// selections as a restyle that the renderer tests, or `None` after the error of another read
    /// of a selection in the channel, which a conditional encoding can't hold.
    pub(super) fn restyle(&mut self, expr: Expr, at: TextRange) -> Option<Expr> {
        if !reads_selection(&expr) {
            return Some(expr);
        }
        let mut error = None;
        let restyled = crate::functions::kind::map_value(&expr, &mut |value| {
            restyled(value).unwrap_or_else(|message| {
                error = Some(message);
                value.clone()
            })
        });
        match error {
            Some(message) => {
                self.error(at, message);
                None
            }
            None => Some(restyled),
        }
    }
}

/// Whether `expr` reads a selection.
fn reads_selection(expr: &Expr) -> bool {
    expr.exists(|expr| Ok(is_read(expr))).unwrap_or(false)
}

/// The restyle of the channel value `value`, a searched `CASE` over selections, as `IF` plans
/// too: a call of `restyle` with the value of its `ELSE`, and the predicate and the value of each
/// `WHEN`, in their order.
fn restyled(value: &Expr) -> Result<Expr, String> {
    use prost::Message;
    let Expr::Case(case) = value else {
        return Err(
            "a channel reads a selection through a CASE WHEN or an IF of selections, which the \
             renderer tests, as `color => CASE WHEN brush THEN species ELSE value('lightgray') \
             END`: to compute with a selection, filter by it with `WHERE brush`"
                .into(),
        );
    };
    if case.expr.is_some() {
        return Err(
            "a restyle by selections is a searched CASE, CASE WHEN brush THEN ..., which tests \
             each selection in its WHEN"
                .into(),
        );
    }
    // The pass gives a CASE without an ELSE a NULL one.
    let default = (case.else_expr.as_ref()).filter(|default| {
        !matches!(default.as_ref(), Expr::Literal(value, _) if value.is_null())
            && !matches!(default.as_ref(), Expr::Cast(cast) if matches!(cast.expr.as_ref(), Expr::Literal(value, _) if value.is_null()))
    });
    let Some(default) = default else {
        return Err(
            "a CASE of a channel over selections takes an ELSE: the value of the rows that no \
             selection selects"
                .into(),
        );
    };
    let mut conditions = Vec::with_capacity(case.when_then_expr.len());
    for (when, then) in &case.when_then_expr {
        if reads_selection(then) || reads_selection(default) {
            return Err(
                "the values of a restyle are values of the channel, which read no selection".into(),
            );
        }
        conditions.push((predicate(when)?.encode_to_vec(), (**then).clone()));
    }
    Ok(crate::functions::kind::restyle(
        (**default).clone(),
        conditions,
    ))
}

/// The predicate of the condition `when` of a restyle: selections, `selected(...)` calls, and
/// their combinations with `AND`, `OR`, and `NOT`.
fn predicate(when: &Expr) -> Result<avenger_chart::proto::Predicate, String> {
    use avenger_chart::proto::{predicate::Predicate, EmptySelection, PredicateList, Selected};
    use datafusion::logical_expr::{BinaryExpr, Operator};
    let list = |exprs: [&Expr; 2]| -> Result<PredicateList, String> {
        Ok(PredicateList {
            predicates: exprs.into_iter().map(predicate).collect::<Result<_, _>>()?,
        })
    };
    let predicate = match when {
        Expr::ScalarFunction(call) if is_read(when) => {
            let Some(Expr::Placeholder(Placeholder { id, .. })) = call.args.first() else {
                return Err("a selection's test reads its state".into());
            };
            let selection = (id.trim_start_matches('$').strip_prefix("__sel_"))
                .expect("the state of a selection")
                .to_string();
            let literal = |index: usize| match call.args.get(index) {
                Some(Expr::Literal(value, _)) if !value.is_null() => Some(value.clone()),
                _ => None,
            };
            if literal(5).is_some() {
                return Err(
                    "a restyle tests every clause of its selections, so its `selected` takes no \
                     `exclude`"
                        .into(),
                );
            }
            let none = literal(4).is_some_and(|empty| empty == ScalarValue::from("none"));
            Predicate::Selected(Selected {
                selection,
                empty: match none {
                    true => EmptySelection::None,
                    false => EmptySelection::All,
                } as i32,
                exclude_own: false,
            })
        }
        Expr::BinaryExpr(BinaryExpr {
            left,
            op: Operator::And,
            right,
        }) => Predicate::All(list([left, right])?),
        Expr::BinaryExpr(BinaryExpr {
            left,
            op: Operator::Or,
            right,
        }) => Predicate::Any(list([left, right])?),
        Expr::Not(inner) => Predicate::Negate(Box::new(predicate(inner)?)),
        _ => {
            return Err(
                "a restyle's condition combines selections with AND, OR, and NOT, and tests \
                 nothing else of a row: compute the other test in a column and filter by it, as \
                 `|> WHERE year > 2000`"
                    .into(),
            )
        }
    };
    Ok(avenger_chart::proto::Predicate {
        predicate: Some(predicate),
    })
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
