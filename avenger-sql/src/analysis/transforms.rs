//! The transforms of avenger-datafusion-transforms whose arguments read other scopes than a row:
//! `lump`, whose `weight` reads the rows of each category and whose `keep` reads the categories,
//! and the table function `impute`, of a pipe `CALL` and of `FROM`.
//!
//! A call of `lump` plans as the placeholder window of [`lump::window`], whose type is the lump's
//! struct, and the window node of its projection plans it through [`lump::plan_windows`].

use avenger_datafusion_transforms::{
    impute::{self, ImputeCall, Named},
    lump::{self, Keep, LumpCall, Nulls},
    table_function::{ArgAt, ArgError},
};
use avenger_sql_syntax::{
    ast::{self, AstNode},
    TextRange,
};
use datafusion::{
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, ScalarValue,
    },
    logical_expr::{utils::find_aggregate_exprs, Expr},
};

use super::{
    expr::Scope,
    model::{CallForm, Definition},
    rows::{Col, Rows},
    Pass,
};
use crate::{
    library::{Need, Over},
    types,
};

/// The arguments of a call: the positional ones, and the named ones with their names in lower
/// case, each with its range.
struct Arguments {
    positional: Vec<(ast::Expr, TextRange)>,
    named: Vec<(String, ast::Expr, TextRange)>,
}

/// `text` with its first letter in upper case, as the pass's messages start.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The columns that `expr` reads outside its aggregates and its occurrences of `category`.
fn bare_columns(expr: &Expr, category: &Expr) -> Vec<Column> {
    let stripped = expr.clone().transform_down(|e| {
        let found = matches!(e, Expr::AggregateFunction(_)) || e == *category;
        match found {
            true => Ok(Transformed::new(
                Expr::Literal(ScalarValue::Null, None),
                true,
                TreeNodeRecursion::Jump,
            )),
            false => Ok(Transformed::no(e)),
        }
    });
    match stripped {
        Ok(stripped) => stripped.data.column_refs().into_iter().cloned().collect(),
        Err(_) => Vec::new(),
    }
}

impl Pass<'_> {
    /// The arguments of `arguments`, or `None` after an error: the positional arguments come
    /// before the named ones, and `function` takes no `TABLE` argument among them.
    fn arguments(
        &mut self,
        arguments: impl Iterator<Item = ast::Argument>,
        function: &str,
    ) -> Option<Arguments> {
        let mut args = Arguments {
            positional: Vec::new(),
            named: Vec::new(),
        };
        let mut ok = true;
        for argument in arguments {
            let at = argument.syntax().text_range();
            match &argument {
                ast::Argument::Arg(arg) => {
                    if !args.named.is_empty() {
                        self.error(at, "A positional argument comes before the named arguments");
                        ok = false;
                    }
                    match arg.expr() {
                        Some(expr) => args.positional.push((expr, at)),
                        None => ok = false,
                    }
                }
                ast::Argument::NamedArg(arg) => {
                    let name = arg.name_ref().and_then(|name| name.text());
                    match (name, arg.expr()) {
                        (Some(name), Some(expr)) => {
                            let name = name.to_ascii_lowercase();
                            if args.named.iter().any(|(seen, ..)| *seen == name) {
                                self.error(
                                    at,
                                    format!("The argument {name} of {function} is given twice"),
                                );
                                ok = false;
                            }
                            args.named.push((name, expr, at));
                        }
                        _ => ok = false,
                    }
                }
                ast::Argument::TableArg(_) | ast::Argument::NamedTableArg(_) => {
                    self.error(at, format!("{function} takes no TABLE argument here"));
                    ok = false;
                }
            }
        }
        ok.then_some(args)
    }

    /// The string literal `expr`, the argument `name` of `function`.
    fn literal_text(&mut self, expr: &ast::Expr, function: &str, name: &str) -> Option<String> {
        let at = expr.syntax().text_range();
        let one = self.one_row();
        let planned = self.expr(expr, Scope::of(&one, "a constant argument"))?;
        match planned {
            Expr::Literal(ScalarValue::Utf8(Some(value)), _)
            | Expr::Literal(ScalarValue::Utf8View(Some(value)), _)
            | Expr::Literal(ScalarValue::LargeUtf8(Some(value)), _) => Some(value),
            _ => {
                self.error(
                    at,
                    format!("The argument {name} of {function} must be a STRING literal"),
                );
                None
            }
        }
    }

    /// A call of `lump`: the lump of each row's category, as a placeholder window.
    pub(super) fn lump(
        &mut self,
        call: &ast::CallExpr,
        scope: Scope,
        range: TextRange,
    ) -> Option<Expr> {
        if !scope.windows {
            self.error(
                range,
                format!("Analytic function lump not allowed in {}", scope.clause),
            );
            return None;
        }
        let list = call.arg_list();
        if let Some(list) = &list {
            let clauses = list.distinct_kw().is_some()
                || list.star_arg().is_some()
                || list.null_treatment().is_some()
                || list.order_by().is_some()
                || list.having_clause().is_some()
                || list.call_limit().is_some();
            if clauses {
                self.error(
                    range,
                    "lump takes its category and named arguments, and no other clause",
                );
                return None;
            }
        }
        let Some(over) = call.over_clause() else {
            self.error(
                range,
                "Analytic function lump cannot be called without an OVER clause",
            );
            return None;
        };
        let args = self.arguments(list.iter().flat_map(|list| list.arguments()), "lump")?;
        let [(category, _)] = args.positional.as_slice() else {
            self.error(range, "lump takes one positional argument, the category");
            return None;
        };
        let function = self.library.get("lump").expect("lump is in the library");
        for (name, _, at) in &args.named {
            if name == "category" || function.param_named(name).is_none() {
                self.error(*at, format!("lump has no parameter named `{name}`"));
                return None;
            }
        }
        let named = |name: &str| (args.named.iter()).find(|(n, ..)| n == name);

        let category = self.expr(category, scope.arguments())?;
        let window = self.over(
            &over,
            "lump",
            Over::Required {
                order_by: Need::Forbidden,
                frame: false,
            },
            scope.arguments(),
        )?;
        // The weight: an aggregate of the rows of each category.
        let weight = match named("weight") {
            Some((_, expr, _)) => {
                let at = expr.syntax().text_range();
                let weight_scope = Scope {
                    clause: "the weight of lump",
                    windows: false,
                    ..scope.aggregates()
                };
                let weight = self.expr(expr, weight_scope)?;
                if find_aggregate_exprs([&weight]).is_empty() {
                    self.error(at, "The weight of lump is an aggregate of the rows of each category, such as SUM(tons)");
                    return None;
                }
                if let Some(column) = bare_columns(&weight, &category).first() {
                    self.error(at, format!("The weight of lump reads the rows of each category through aggregates, and {} is not in one", column.name));
                    return None;
                }
                let ty = self.type_of(&weight, scope)?;
                if !ty.is_numeric() && !ty.is_null() {
                    let ty = types::googlesql_name(&ty);
                    self.error(at, format!("The weight of lump is a number; found {ty}"));
                    return None;
                }
                Some(weight)
            }
            None => None,
        };
        // `n` or `keep`.
        let keep = match (named("n"), named("keep")) {
            (Some((_, expr, _)), None) => {
                let at = expr.syntax().text_range();
                let one = self.one_row();
                match self.expr(expr, Scope::of(&one, "the n of lump"))? {
                    Expr::Literal(ScalarValue::Int64(Some(n)), _) if n > 0 => Keep::First(n),
                    _ => {
                        self.error(at, "The n of lump is a positive INT64 literal");
                        return None;
                    }
                }
            }
            (None, Some((_, expr, _))) => {
                let at = expr.syntax().text_range();
                let ranked =
                    lump::ranked(weight.as_ref().unwrap_or(&lump::count_star()), &category);
                let keep_scope = Scope {
                    clause: "the keep of lump",
                    groups: None,
                    ..scope.aggregates().windows()
                };
                let saved = self.categories.replace(ranked);
                let keep = self.expr(expr, keep_scope);
                self.categories = saved;
                let keep = keep?;
                if let Some(column) = bare_columns(&keep, &category).first() {
                    self.error(at, format!("The keep of lump reads the rows of each category through aggregates, such as SUM(tons) > 10, and {} is not in one", column.name));
                    return None;
                }
                let ty = self.type_of(&keep, scope)?;
                if !matches!(
                    ty,
                    datafusion::arrow::datatypes::DataType::Boolean
                        | datafusion::arrow::datatypes::DataType::Null
                ) {
                    let ty = types::googlesql_name(&ty);
                    self.error(
                        at,
                        format!("The keep of lump is a condition, a BOOL; found {ty}"),
                    );
                    return None;
                }
                Keep::Condition(keep)
            }
            _ => {
                self.error(range, "lump takes exactly one of n and keep");
                return None;
            }
        };
        let other = match named("other") {
            Some((_, expr, _)) => self.literal_text(expr, "lump", "other")?,
            None => "Other".to_string(),
        };
        let null_category = match named("null_category") {
            Some((_, expr, at)) => {
                let at = *at;
                let mode = self.literal_text(expr, "lump", "null_category")?;
                match Nulls::parse(&mode) {
                    Some(mode) => mode,
                    None => {
                        self.error(at, format!("The null_category of lump is 'keep', 'other' or 'drop'; found '{mode}'"));
                        return None;
                    }
                }
            }
            None => Nulls::Keep,
        };
        let category_type = self.type_of(&category, scope)?;
        let id = self.qualifiers.fresh("lump").table().to_string();
        let expr = lump::window(
            LumpCall {
                category,
                partition: window.partition,
                weight,
                keep,
                other,
                null_category,
            },
            &id,
        );
        let ty = types::googlesql_name(&category_type);
        self.model.calls.push(CallForm {
            range,
            form: format!(
                "lump({ty}) -> STRUCT<value {ty}, other STRING, weight FLOAT64, category_count INT64>"
            ),
        });
        Some(expr)
    }

    /// The pipe `CALL` of `op` over `rows`.
    pub(super) fn pipe_call(&mut self, rows: Rows, op: &ast::PipeCall, range: TextRange) -> Rows {
        let Some(path) = op.path() else {
            return rows.fail();
        };
        let Some(names) = path.names() else {
            return rows.fail();
        };
        let path_range = path.syntax().text_range();
        if !matches!(names.as_slice(), [name] if name.eq_ignore_ascii_case("impute")) {
            let message = match self.temp_function(&names) {
                Some(_) => "avenger-sql doesn't plan a pipe CALL of a temporary table function yet; call it in FROM, with TABLE arguments".to_string(),
                None => format!("Table-valued function not found: {}", names.join(".")),
            };
            self.error(path_range, message);
            return rows.fail();
        }
        let alias = (op.alias().and_then(|alias| alias.name()))
            .and_then(|name| Some((name.text()?, name.syntax().text_range())));
        let arguments = op.arg_list().into_iter().flat_map(|list| list.arguments());
        self.impute(rows, arguments, alias, path_range, range)
    }

    /// `FROM impute(TABLE t, ...)`, whose first argument is its input, a table or a query.
    pub(super) fn impute_source(&mut self, source: &ast::TvfSource, path_range: TextRange) -> Rows {
        let range = source.syntax().text_range();
        let mut arguments = source
            .arg_list()
            .into_iter()
            .flat_map(|list| list.arguments());
        let input = match arguments.next() {
            Some(ast::Argument::TableArg(arg)) => match arg.path().and_then(|path| path.names()) {
                Some(names) => self.table_rows(&names, arg.syntax().text_range()),
                None => return Rows::failed(Vec::new(), Vec::new()),
            },
            Some(ast::Argument::Arg(arg)) => match arg.expr() {
                Some(ast::Expr::SubqueryExpr(subquery)) => {
                    match subquery.paren_query().and_then(|paren| paren.query()) {
                        Some(query) => {
                            let inner = self.query(&query);
                            self.close(inner, None, false, range)
                        }
                        None => return Rows::failed(Vec::new(), Vec::new()),
                    }
                }
                _ => {
                    self.error(arg.syntax().text_range(), "The first argument of impute in FROM is its input, as TABLE name or a query in parentheses");
                    return Rows::failed(Vec::new(), Vec::new());
                }
            },
            _ => {
                self.error(range, "The first argument of impute in FROM is its input, as TABLE name or a query in parentheses");
                return Rows::failed(Vec::new(), Vec::new());
            }
        };
        let alias = (source.alias().and_then(|alias| alias.name()))
            .and_then(|name| Some((name.text()?, name.syntax().text_range())))
            .or_else(|| Some(("impute".to_string(), path_range)));
        self.impute(input, arguments, alias, path_range, range)
    }

    /// The rows of `impute` over `rows`, of the arguments after the input, with the range
    /// variable of `alias`. The arguments name the visible columns of `rows`, as a table.
    fn impute(
        &mut self,
        rows: Rows,
        arguments: impl Iterator<Item = ast::Argument>,
        alias: Option<(String, TextRange)>,
        path_range: TextRange,
        range: TextRange,
    ) -> Rows {
        self.use_name(
            path_range,
            Definition::Function {
                name: "impute".to_string(),
            },
        );
        let Some(args) = self.arguments(arguments, "impute") else {
            return rows.fail();
        };
        // The input is a table of the visible columns, which the arguments name.
        let rows = self.close(rows, None, false, range);
        let Some(plan) = rows.plan.clone() else {
            return rows.fail();
        };
        if args.positional.len() != 3 {
            self.error(range, format!("impute takes 3 positional arguments, the key, the group columns and the column to impute; found {}", args.positional.len()));
            return rows.fail();
        }
        let scope = Scope::of(&rows, "an argument of impute");
        // The ranges of the arguments and of the elements of their arrays, for the errors of
        // `impute::plan`.
        let elements = |expr: &ast::Expr| -> Option<Vec<ast::Expr>> {
            match expr {
                ast::Expr::ArrayExpr(array) => Some(array.exprs().collect()),
                _ => None,
            }
        };
        let ranges = |expr: &ast::Expr| -> Vec<TextRange> {
            (elements(expr).unwrap_or_default().iter())
                .map(|element| element.syntax().text_range())
                .collect()
        };
        let column_of = |pass: &mut Self, expr: &ast::Expr, what: &str| -> Option<Column> {
            match pass.expr(expr, scope)? {
                Expr::Column(column) => Some(column),
                _ => {
                    pass.error(
                        expr.syntax().text_range(),
                        format!("The {what} of impute is a column of its input"),
                    );
                    None
                }
            }
        };
        let key = column_of(self, &args.positional[0].0, "key");
        let groups_expr = &args.positional[1].0;
        let groups = match elements(groups_expr) {
            Some(exprs) => (exprs.iter())
                .map(|expr| column_of(self, expr, "group column"))
                .collect::<Option<Vec<_>>>(),
            None => {
                self.error(
                    groups_expr.syntax().text_range(),
                    "The group columns of impute are an array of columns, such as [region]",
                );
                None
            }
        };
        let value = column_of(self, &args.positional[2].0, "column to impute");
        let mut named = Vec::new();
        let mut ok = true;
        for (name, expr, _) in &args.named {
            let arg = match name.as_str() {
                "flag" => self.literal_text(expr, "impute", "flag").map(Named::Name),
                "partition_by" => match elements(expr) {
                    Some(exprs) => (exprs.iter())
                        .map(|expr| column_of(self, expr, "partition column"))
                        .collect::<Option<Vec<_>>>()
                        .map(Named::Columns),
                    None => column_of(self, expr, "partition column")
                        .map(|column| Named::Columns(vec![column])),
                },
                "frame" | "keys" | "extra_keys" if elements(expr).is_some() => {
                    let one = self.one_row();
                    let constant = Scope::of(&one, "a constant argument");
                    (elements(expr).unwrap_or_default().iter())
                        .map(|element| self.expr(element, constant))
                        .collect::<Option<Vec<_>>>()
                        .map(Named::List)
                }
                _ => {
                    let one = self.one_row();
                    self.expr(expr, Scope::of(&one, "a constant argument"))
                        .map(Named::Value)
                }
            };
            match arg {
                Some(arg) => named.push((name.clone(), arg)),
                None => ok = false,
            }
        }
        let (Some(key), Some(groups), Some(value), true) = (key, groups, value, ok) else {
            return rows.fail();
        };
        let flag = (args.named.iter())
            .find(|(name, ..)| name == "flag")
            .and_then(|(_, _, at)| {
                named
                    .iter()
                    .find_map(|(name, arg)| match (name.as_str(), arg) {
                        ("flag", Named::Name(flag)) => Some((flag.clone(), *at)),
                        _ => None,
                    })
            });
        let call = ImputeCall {
            key,
            groups,
            value,
            named,
        };
        let planned = match impute::plan(plan, call) {
            Ok(planned) => planned,
            Err(ArgError { at, message }) => {
                let positional = |i: usize| &args.positional[i].0;
                let at = match at {
                    ArgAt::Call => range,
                    ArgAt::Positional(i) => args.positional[i].1,
                    ArgAt::Element(i, j) => ranges(positional(i))
                        .get(j)
                        .copied()
                        .unwrap_or(args.positional[i].1),
                    ArgAt::Named(i) => args.named[i].2,
                    ArgAt::NamedElement(i, j) => ranges(&args.named[i].1)
                        .get(j)
                        .copied()
                        .unwrap_or(args.named[i].2),
                };
                self.error(at, sentence(&message));
                return rows.fail();
            }
        };
        let mut columns = rows.columns.clone();
        if let Some((flag, at)) = flag {
            columns.push(Col {
                name: flag.clone(),
                column: Column::from_name(flag),
                definition: Some(at),
            });
        }
        let imputed = Rows::new(planned, columns, Vec::new());
        self.close(imputed, alias, false, range)
    }
}
