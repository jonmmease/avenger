//! Queries: the sources of a `FROM` clause, and the pipe operators, each a step over the rows of
//! the operators before it.

use std::collections::HashSet;

use avenger_sql_syntax::{
    ast::{self, AstNode, LiteralKind},
    TextRange,
};
use datafusion::{
    common::{Column, DFSchema, ExprSchema, JoinType, Result, ScalarValue},
    datasource::provider_as_source,
    logical_expr::{
        utils::find_aggregate_exprs, Expr, ExprSchemable, LogicalPlan, LogicalPlanBuilder, SortExpr,
    },
};

use super::{
    expr::{inferred_name, Scope},
    model::Definition,
    rows::{self, plan_name, Col, Item, RangeVar, Rows},
    Pass,
};
use crate::library::{form::supertype, Ty};

/// A kind of join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

impl Kind {
    fn join_type(self) -> JoinType {
        match self {
            Kind::Inner | Kind::Cross => JoinType::Inner,
            Kind::Left => JoinType::Left,
            Kind::Right => JoinType::Right,
            Kind::Full => JoinType::Full,
        }
    }
}

/// The name and the range of an alias, when it has a name.
fn alias_name(alias: Option<ast::Alias>) -> Option<(String, TextRange)> {
    let name = alias?.name()?;
    Some((name.text()?, name.syntax().text_range()))
}

/// The sort of an item's suffix, `ASC` or `DESC` with `NULLS FIRST` or `NULLS LAST`, or `ASC` for
/// a group item of `GROUP AND ORDER BY` without one.
fn suffix(suffix: Option<ast::OrderSuffix>, ascending: bool) -> Option<(bool, bool)> {
    let Some(suffix) = suffix else {
        return ascending.then_some((true, true));
    };
    let asc = suffix.desc_kw().is_none();
    let given =
        suffix.asc_kw().is_some() || suffix.desc_kw().is_some() || suffix.nulls_kw().is_some();
    if !given && !ascending {
        return None;
    }
    let nulls_first = match (suffix.first_kw(), suffix.last_kw()) {
        (Some(_), _) => true,
        (_, Some(_)) => false,
        _ => asc,
    };
    Some((asc, nulls_first))
}

/// A placeholder of an item whose expression failed, which keeps its name for the model.
fn unknown() -> Expr {
    Expr::Literal(ScalarValue::Null, None)
}

impl Pass<'_> {
    /// The rows of `query`, before its result's sort.
    pub(super) fn query(&mut self, query: &ast::Query) -> Rows {
        let mut rows = match (query.from_clause(), query.paren_query()) {
            (Some(from), _) => {
                let rows = match from.source() {
                    Some(source) => self.source(&source),
                    None => Rows::failed(Vec::new(), Vec::new()),
                };
                self.record_operator(from.syntax().text_range(), None, &rows);
                rows
            }
            (None, Some(paren)) => {
                let rows = self.subquery(&paren, query.alias(), true);
                self.record_operator(paren.syntax().text_range(), None, &rows);
                rows
            }
            _ => Rows::failed(Vec::new(), Vec::new()),
        };
        for operator in query.pipe_operators() {
            let input = rows.clone();
            rows = self.pipe(rows, &operator);
            self.record_operator(operator.syntax().text_range(), Some(&input), &rows);
        }
        rows
    }

    /// The rows of `rows` after the pipe operator `operator`.
    fn pipe(&mut self, rows: Rows, operator: &ast::PipeOperator) -> Rows {
        let range = operator.syntax().text_range();
        match operator {
            ast::PipeOperator::PipeWhere(op) => self.pipe_where(rows, op),
            ast::PipeOperator::PipeSelect(op) => {
                let rows = self.select(rows, op.select_entries(), op.window_clause(), false, range);
                match op.distinct_kw() {
                    Some(_) => self.distinct(rows, range),
                    None => rows,
                }
            }
            ast::PipeOperator::PipeExtend(op) => {
                self.select(rows, op.select_entries(), op.window_clause(), true, range)
            }
            ast::PipeOperator::PipeSet(op) => self.set(rows, op, range),
            ast::PipeOperator::PipeDrop(op) => self.drop(rows, op),
            ast::PipeOperator::PipeRename(op) => self.rename(rows, op, range),
            ast::PipeOperator::PipeAs(op) => {
                let Some((name, definition)) = op
                    .name()
                    .and_then(|name| Some((name.text()?, name.syntax().text_range())))
                else {
                    return rows;
                };
                let columns = rows.columns.clone();
                Rows {
                    vars: vec![RangeVar {
                        name,
                        definition: Some(definition),
                        columns,
                    }],
                    ..rows
                }
            }
            ast::PipeOperator::PipeAggregate(op) => self.aggregate(rows, op, range),
            ast::PipeOperator::PipeDistinct(_) => self.distinct(rows, range),
            ast::PipeOperator::PipeOrderBy(op) => self.order_by(rows, op, range),
            ast::PipeOperator::PipeLimit(op) => self.limit(rows, op, range),
            ast::PipeOperator::PipeJoin(op) => {
                let right = match op.source() {
                    Some(source) => self.source(&source),
                    None => Rows::failed(Vec::new(), Vec::new()),
                };
                self.join(rows, right, op.join_op(), op.join_condition(), range)
            }
            ast::PipeOperator::PipeSetOperation(_)
            | ast::PipeOperator::PipeCall(_)
            | ast::PipeOperator::PipeSample(_)
            | ast::PipeOperator::PipeWith(_)
            | ast::PipeOperator::PipePivot(_)
            | ast::PipeOperator::PipeUnpivot(_) => {
                self.error(range, "avenger-sql doesn't plan this pipe operator yet");
                rows.fail()
            }
        }
    }

    /// The rows of a projection, or after DataFusion's error, the columns of `items` without a
    /// plan.
    fn projected(
        &mut self,
        rows: Rows,
        items: Vec<Item>,
        vars: bool,
        order: bool,
        range: TextRange,
    ) -> Rows {
        let columns = (items.iter())
            .map(|item| match item {
                Item::Carry(col) => col.clone(),
                Item::Compute {
                    name, definition, ..
                } => Col {
                    name: name.clone(),
                    column: Column::from_name(name),
                    definition: *definition,
                },
            })
            .collect();
        let fallback = match vars {
            true => rows.vars.clone(),
            false => Vec::new(),
        };
        match rows::project(&mut self.qualifiers, rows, items, vars, order) {
            Ok(rows) => rows,
            Err(error) => {
                self.datafusion_error(range, error);
                Rows::failed(columns, fallback)
            }
        }
    }

    /// The rows of a plan that DataFusion built, or after its error, `fallback`.
    fn built(&mut self, result: Result<Rows>, range: TextRange, fallback: Rows) -> Rows {
        match result {
            Ok(rows) => rows,
            Err(error) => {
                self.datafusion_error(range, error);
                fallback
            }
        }
    }

    // --- Sources ---

    fn source(&mut self, source: &ast::Source) -> Rows {
        let range = source.syntax().text_range();
        match source {
            ast::Source::TableSource(table) => self.table(table),
            ast::Source::SubquerySource(subquery) => {
                if subquery.lateral_kw().is_some() {
                    self.error(range, "avenger-sql doesn't plan LATERAL yet");
                }
                match subquery.paren_query() {
                    Some(paren) => self.subquery(&paren, subquery.alias(), false),
                    None => Rows::failed(Vec::new(), Vec::new()),
                }
            }
            ast::Source::ParenJoin(paren) => match paren.source() {
                Some(source) => self.source(&source),
                None => Rows::failed(Vec::new(), Vec::new()),
            },
            ast::Source::Join(join) => {
                let left = match join.lhs() {
                    Some(lhs) => self.source(&lhs),
                    None => Rows::failed(Vec::new(), Vec::new()),
                };
                let right = match join.rhs() {
                    Some(rhs) => self.source(&rhs),
                    None => Rows::failed(Vec::new(), Vec::new()),
                };
                self.join(left, right, join.join_op(), join.join_condition(), range)
            }
            ast::Source::UnnestSource(_)
            | ast::Source::TvfSource(_)
            | ast::Source::PivotSource(_)
            | ast::Source::UnpivotSource(_)
            | ast::Source::SampleSource(_) => {
                self.error(range, "avenger-sql doesn't plan this source yet");
                Rows::failed(Vec::new(), Vec::new())
            }
        }
    }

    /// A table of the catalog, with the range variable of its alias or its name.
    fn table(&mut self, source: &ast::TableSource) -> Rows {
        let Some(path) = source.path() else {
            return Rows::failed(Vec::new(), Vec::new());
        };
        let path_range = path.syntax().text_range();
        let Some(names) = path.names() else {
            return Rows::failed(Vec::new(), Vec::new());
        };
        if source.lateral_kw().is_some() {
            self.error(
                path_range,
                "LATERAL takes a query in parentheses or a table function call",
            );
        }
        if let Some(offset) = source.with_offset() {
            self.error(
                offset.syntax().text_range(),
                "avenger-sql doesn't plan WITH OFFSET yet",
            );
        }
        let (name, definition) = alias_name(source.alias())
            .unwrap_or_else(|| (names.last().cloned().unwrap_or_default(), path_range));
        let Some(table) = self.catalog.table(&names) else {
            self.error(path_range, format!("Table not found: {}", names.join(".")));
            return Rows::failed(Vec::new(), Vec::new());
        };
        self.use_name(
            path_range,
            Definition::Table {
                names: table.names.clone(),
            },
        );
        let qualifier = self.qualifiers.fresh(&name);
        let plan = LogicalPlanBuilder::scan(
            qualifier.clone(),
            provider_as_source(table.provider.clone()),
            None,
        )
        .and_then(|builder| builder.build());
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(path_range, error);
                return Rows::failed(Vec::new(), Vec::new());
            }
        };
        let columns: Vec<Col> = (table.schema().fields().iter())
            .map(|field| Col {
                name: field.name().clone(),
                column: Column::new(Some(qualifier.clone()), field.name()),
                definition: Some(path_range),
            })
            .collect();
        let vars = vec![RangeVar {
            name,
            definition: Some(definition),
            columns: columns.clone(),
        }];
        Rows::new(plan, columns, vars)
    }

    /// The rows of a query in parentheses, as a source with the range variable of its alias. The
    /// rows keep their order when `ordered` holds, for a query that starts with one.
    fn subquery(
        &mut self,
        paren: &ast::ParenQuery,
        alias: Option<ast::Alias>,
        ordered: bool,
    ) -> Rows {
        let inner = match paren.query() {
            Some(query) => self.query(&query),
            None => Rows::failed(Vec::new(), Vec::new()),
        };
        let alias = alias_name(alias);
        let base = alias.as_ref().map_or("_", |(name, _)| name.as_str());
        let qualifier = self.qualifiers.fresh(base);
        let mut names = HashSet::new();
        let mut exprs = Vec::new();
        let mut columns = Vec::new();
        for (index, col) in inner.columns.iter().enumerate() {
            let field = plan_name(&col.name, index);
            let qualifier = match names.insert(field.clone()) {
                true => qualifier.clone(),
                false => self.qualifiers.fresh(base),
            };
            exprs.push(
                Expr::Column(col.column.clone()).alias_qualified(Some(qualifier.clone()), &field),
            );
            columns.push(Col {
                name: col.name.clone(),
                column: Column::new(Some(qualifier), field),
                definition: col.definition,
            });
        }
        let vars = match &alias {
            Some((name, definition)) => vec![RangeVar {
                name: name.clone(),
                definition: Some(*definition),
                columns: columns.clone(),
            }],
            None => Vec::new(),
        };
        let order = match ordered {
            true => inner.order.clone(),
            false => Vec::new(),
        };
        for key in &order {
            for column in key.expr.column_refs() {
                exprs.push(Expr::Column(column.clone()));
            }
        }
        let Some(plan) = inner.plan else {
            return Rows::failed(columns, vars);
        };
        let range = paren.syntax().text_range();
        let plan = LogicalPlanBuilder::from(plan)
            .project(exprs)
            .and_then(|b| b.build());
        match plan {
            Ok(plan) => Rows {
                order,
                ..Rows::new(plan, columns, vars)
            },
            Err(error) => {
                self.datafusion_error(range, error);
                Rows::failed(columns, vars)
            }
        }
    }

    /// The join of `left` and `right` by the operator `op` on `condition`.
    fn join(
        &mut self,
        left: Rows,
        right: Rows,
        op: Option<ast::JoinOp>,
        condition: Option<ast::JoinCondition>,
        range: TextRange,
    ) -> Rows {
        let (left, right) = (left.unordered(), right.unordered());
        let kind = match &op {
            Some(op) if op.comma().is_some() || op.cross_kw().is_some() => Kind::Cross,
            Some(op) if op.left_kw().is_some() => Kind::Left,
            Some(op) if op.right_kw().is_some() => Kind::Right,
            Some(op) if op.full_kw().is_some() => Kind::Full,
            _ => Kind::Inner,
        };
        let mut columns = left.columns.clone();
        columns.extend(right.columns.iter().cloned());
        let mut vars = left.vars.clone();
        let mut ok = true;
        for var in &right.vars {
            if vars.iter().any(|v| v.name.eq_ignore_ascii_case(&var.name)) {
                self.error(
                    var.definition.unwrap_or(range),
                    format!("Duplicate table alias {} in the same FROM clause", var.name),
                );
                ok = false;
            }
            vars.push(var.clone());
        }
        if let Some(natural) = op.as_ref().and_then(|op| op.natural_kw()) {
            self.error(natural.text_range(), "Natural join not supported");
            ok = false;
        }
        let failed = |columns: Vec<Col>, vars: Vec<RangeVar>| Rows::failed(columns, vars);
        let schema = match (&left.plan, &right.plan) {
            (Some(l), Some(r)) => l.schema().join(r.schema()).ok(),
            _ => None,
        };
        match (kind, &condition) {
            (Kind::Cross, _) => {
                let (Some(l), Some(r), true) = (left.plan, right.plan, ok) else {
                    return failed(columns, vars);
                };
                let plan = LogicalPlanBuilder::from(l)
                    .cross_join(r)
                    .and_then(|b| b.build());
                let fallback = failed(columns.clone(), vars.clone());
                self.built(
                    plan.map(|plan| Rows::new(plan, columns, vars)),
                    range,
                    fallback,
                )
            }
            (_, None) => {
                self.error(
                    range,
                    "A JOIN needs an ON or a USING clause, or is a CROSS JOIN",
                );
                failed(columns, vars)
            }
            (_, Some(ast::JoinCondition::OnCondition(on))) => {
                let condition = {
                    let scope = Scope {
                        columns: &columns,
                        vars: &vars,
                        schema: schema.as_ref(),
                        clause: "JOIN ON clause",
                        aggregates: false,
                        windows: false,
                    };
                    on.expr().and_then(|expr| {
                        let at = expr.syntax().text_range();
                        let planned = self.expr(&expr, scope)?;
                        self.boolean(planned, at, "JOIN ON clause", scope)
                    })
                };
                let (Some(l), Some(r), Some(condition), true) =
                    (left.plan, right.plan, condition, ok)
                else {
                    return failed(columns, vars);
                };
                let plan = LogicalPlanBuilder::from(l)
                    .join_on(r, kind.join_type(), [condition])
                    .and_then(|b| b.build());
                let fallback = failed(columns.clone(), vars.clone());
                self.built(
                    plan.map(|plan| Rows::new(plan, columns, vars)),
                    range,
                    fallback,
                )
            }
            (_, Some(ast::JoinCondition::UsingCondition(using))) => {
                self.using(left, right, kind, using, vars, ok, range)
            }
        }
    }

    /// The join of `left` and `right` on the columns of `USING`, with each key once, first: the
    /// left key of an inner or a left join, the right key of a right join, and the first key that
    /// isn't `NULL` of a full join, in the common type of the keys.
    #[allow(clippy::too_many_arguments)]
    fn using(
        &mut self,
        left: Rows,
        right: Rows,
        kind: Kind,
        using: &ast::UsingCondition,
        vars: Vec<RangeVar>,
        mut ok: bool,
        range: TextRange,
    ) -> Rows {
        let mut keys: Vec<(Col, Col, TextRange)> = Vec::new();
        for name_ref in using.name_list().iter().flat_map(|list| list.name_refs()) {
            let at = name_ref.syntax().text_range();
            let Some(name) = name_ref.text() else {
                ok = false;
                continue;
            };
            let side = |rows: &Rows| -> Vec<Col> {
                (rows.columns.iter())
                    .filter(|col| col.name.eq_ignore_ascii_case(&name))
                    .cloned()
                    .collect()
            };
            let (l, r) = (side(&left), side(&right));
            match (l.as_slice(), r.as_slice()) {
                ([l], [r]) => {
                    let info = Self::column_info(&left, l);
                    self.use_name(at, Definition::Column(info));
                    keys.push((l.clone(), r.clone(), at));
                }
                ([], _) => {
                    self.error(
                        at,
                        format!("Column {name} in USING clause not found on left side of join"),
                    );
                    ok = false;
                }
                (_, []) => {
                    self.error(
                        at,
                        format!("Column {name} in USING clause not found on right side of join"),
                    );
                    ok = false;
                }
                _ => {
                    self.error(at, format!("Column {name} in USING clause is ambiguous"));
                    ok = false;
                }
            }
        }
        let is_key = |col: &Col, side: usize| {
            keys.iter().any(|key| match side {
                0 => key.0.column == col.column,
                _ => key.1.column == col.column,
            })
        };
        let mut columns: Vec<Col> = keys.iter().map(|(l, _, _)| l.clone()).collect();
        columns.extend(left.columns.iter().filter(|col| !is_key(col, 0)).cloned());
        columns.extend(right.columns.iter().filter(|col| !is_key(col, 1)).cloned());
        let (Some(l), Some(r), true) = (left.plan.clone(), right.plan.clone(), ok) else {
            return Rows::failed(columns, vars);
        };
        // The equalities of the keys, in their common types.
        let schema = match l.schema().join(r.schema()) {
            Ok(schema) => schema,
            Err(error) => {
                self.datafusion_error(range, error);
                return Rows::failed(columns, vars);
            }
        };
        let type_of = |col: &Col| -> Option<Ty> {
            Ty::of_arrow(schema.field_from_column(&col.column).ok()?.data_type())
        };
        let mut conditions = Vec::new();
        let mut items = Vec::new();
        for (lk, rk, at) in &keys {
            let (Some(lt), Some(rt)) = (type_of(lk), type_of(rk)) else {
                return Rows::failed(columns, vars);
            };
            let Some(ty) = supertype(&[(lt.clone(), false), (rt.clone(), false)]) else {
                self.error(
                    *at,
                    format!("Column {} in USING has incompatible types on either side of the join: {lt} and {rt}", lk.name),
                );
                return Rows::failed(columns, vars);
            };
            let arrow = ty.arrow(self.type_options);
            let options = self.type_options;
            let side = |col: &Col| -> Expr {
                let expr = Expr::Column(col.column.clone());
                match schema.field_from_column(&col.column) {
                    Ok(field) if crate::library::exact(field.data_type(), &ty, options) => expr,
                    _ => Expr::Cast(datafusion::logical_expr::Cast::new(
                        Box::new(expr),
                        arrow.clone(),
                    )),
                }
            };
            let (le, re) = (side(lk), side(rk));
            conditions.push(le.clone().eq(re.clone()));
            let key = match kind {
                Kind::Right => (rk, re),
                Kind::Full => {
                    let coalesced = datafusion::functions::core::expr_fn::coalesce(vec![le, re]);
                    items.push(Item::Compute {
                        name: lk.name.clone(),
                        expr: coalesced,
                        definition: lk.definition,
                    });
                    continue;
                }
                _ => (lk, le),
            };
            items.push(match &key.1 {
                Expr::Column(column) if *column == key.0.column => Item::Carry(Col {
                    name: lk.name.clone(),
                    ..key.0.clone()
                }),
                expr => Item::Compute {
                    name: lk.name.clone(),
                    expr: expr.clone(),
                    definition: lk.definition,
                },
            });
        }
        let condition = conditions.into_iter().reduce(Expr::and);
        let plan = LogicalPlanBuilder::from(l)
            .join_on(r, kind.join_type(), condition)
            .and_then(|b| b.build());
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return Rows::failed(columns, vars);
            }
        };
        items.extend(columns[keys.len()..].iter().cloned().map(Item::Carry));
        let joined = Rows::new(plan, Vec::new(), vars);
        self.projected(joined, items, true, false, range)
    }

    // --- Pipe operators ---

    fn pipe_where(&mut self, rows: Rows, op: &ast::PipeWhere) -> Rows {
        let Some(condition) = op.condition() else {
            return rows.fail();
        };
        let at = condition.syntax().text_range();
        let predicate = {
            let scope = Scope::of(&rows, "WHERE clause");
            self.expr(&condition, scope)
                .and_then(|expr| self.boolean(expr, at, "WHERE clause", scope))
        };
        let (Some(predicate), Some(plan)) = (predicate, rows.plan.clone()) else {
            return rows.fail();
        };
        let plan = LogicalPlanBuilder::from(plan)
            .filter(predicate)
            .and_then(|b| b.build());
        let fallback = rows.clone().fail();
        self.built(
            plan.map(|plan| Rows {
                plan: Some(plan),
                ..rows
            }),
            at,
            fallback,
        )
    }

    /// The rows of a pipe `SELECT`, or of a pipe `EXTEND` when `extend`, which adds its items to
    /// the columns and keeps the range variables.
    fn select(
        &mut self,
        rows: Rows,
        entries: ast::AstChildren<ast::SelectEntry>,
        window: Option<ast::WindowClause>,
        extend: bool,
        range: TextRange,
    ) -> Rows {
        let clause = match extend {
            true => "pipe EXTEND",
            false => "pipe SELECT",
        };
        let mut ok = true;
        if let Some(window) = window {
            self.error(
                window.syntax().text_range(),
                "avenger-sql doesn't plan the WINDOW clause yet",
            );
            ok = false;
        }
        let mut items: Vec<Item> = match extend {
            true => rows.columns.iter().cloned().map(Item::Carry).collect(),
            false => Vec::new(),
        };
        let scope = Scope::of(&rows, clause).windows();
        for entry in entries {
            match entry {
                ast::SelectEntry::SelectItem(item) => {
                    let expr = item.expr();
                    let alias = alias_name(item.alias());
                    let name = match &alias {
                        Some((name, _)) => name.clone(),
                        None => expr.as_ref().map(inferred_name).unwrap_or_default(),
                    };
                    let definition = alias
                        .map(|(_, range)| range)
                        .or_else(|| expr.as_ref().map(|expr| expr.syntax().text_range()));
                    let planned = expr.and_then(|expr| self.expr(&expr, scope));
                    items.push(match planned {
                        Some(Expr::Column(column)) if column.name == name => Item::Carry(Col {
                            name,
                            column,
                            definition,
                        }),
                        Some(expr) => Item::Compute {
                            name,
                            expr,
                            definition,
                        },
                        None => {
                            ok = false;
                            Item::Compute {
                                name,
                                expr: unknown(),
                                definition,
                            }
                        }
                    });
                }
                ast::SelectEntry::Wildcard(wildcard) => match self.wildcard(&wildcard, scope) {
                    Some(expanded) => items.extend(expanded),
                    None => ok = false,
                },
            }
        }
        if !ok {
            let rows = rows::project(
                &mut self.qualifiers,
                Rows::failed(Vec::new(), rows.vars.clone()),
                items,
                extend,
                false,
            )
            .unwrap_or_else(|_| Rows::failed(Vec::new(), Vec::new()));
            return rows;
        }
        self.projected(rows, items, extend, true, range)
    }

    /// The items of a wildcard: `*`, or `t.*` of a range variable, without the columns of its
    /// `EXCEPT` and with the expressions of its `REPLACE`.
    fn wildcard(&mut self, wildcard: &ast::Wildcard, scope: Scope) -> Option<Vec<Item>> {
        let range = wildcard.syntax().text_range();
        let mut columns: Vec<Col> = match wildcard.path() {
            None => scope.columns.to_vec(),
            Some(path) => {
                let names = path.names()?;
                let refs: Vec<ast::NameRef> = path.name_refs().collect();
                let var = match names.as_slice() {
                    [name] => scope
                        .vars
                        .iter()
                        .find(|var| var.name.eq_ignore_ascii_case(name)),
                    _ => None,
                };
                let Some(var) = var else {
                    self.error(
                        path.syntax().text_range(),
                        format!(
                            "avenger-sql expands the wildcard of a range variable, and {} is none",
                            names.join(".")
                        ),
                    );
                    return None;
                };
                self.use_name(
                    refs[0].syntax().text_range(),
                    Definition::RangeVariable {
                        name: var.name.clone(),
                        definition: var.definition,
                    },
                );
                var.columns.clone()
            }
        };
        columns.retain(|col| !col.name.is_empty());
        let mut ok = true;
        if let Some(except) = wildcard.except_list() {
            for name_ref in except.name_list().iter().flat_map(|list| list.name_refs()) {
                let name = name_ref.text()?;
                let before = columns.len();
                columns.retain(|col| !col.name.eq_ignore_ascii_case(&name));
                if columns.len() == before {
                    self.error(
                        name_ref.syntax().text_range(),
                        format!("Column {name} in SELECT * EXCEPT list does not exist"),
                    );
                    ok = false;
                }
            }
        }
        let mut items: Vec<Item> = columns.into_iter().map(Item::Carry).collect();
        if let Some(replace) = wildcard.replace_list() {
            for item in replace.replace_items() {
                let Some(name_node) = item.name() else {
                    ok = false;
                    continue;
                };
                let name = name_node.text()?;
                let planned = item.expr().and_then(|expr| self.expr(&expr, scope));
                let position = items.iter().position(|item| match item {
                    Item::Carry(col) => col.name.eq_ignore_ascii_case(&name),
                    Item::Compute { name: n, .. } => n.eq_ignore_ascii_case(&name),
                });
                match (position, planned) {
                    (Some(position), Some(expr)) => {
                        let original = match &items[position] {
                            Item::Carry(col) => col.name.clone(),
                            Item::Compute { name, .. } => name.clone(),
                        };
                        items[position] = Item::Compute {
                            name: original,
                            expr,
                            definition: Some(name_node.syntax().text_range()),
                        };
                    }
                    (None, _) => {
                        self.error(
                            name_node.syntax().text_range(),
                            format!("Column {name} in SELECT * REPLACE list does not exist"),
                        );
                        ok = false;
                    }
                    (_, None) => ok = false,
                }
            }
        }
        let _ = range;
        ok.then_some(items)
    }

    /// The index of the one column of `rows` named `name`, or `None` with an error at `at`.
    fn unique_column(
        &mut self,
        rows: &Rows,
        name: &str,
        at: TextRange,
        clause: &str,
    ) -> Option<usize> {
        let found: Vec<usize> = (rows.columns.iter().enumerate())
            .filter(|(_, col)| col.name.eq_ignore_ascii_case(name))
            .map(|(index, _)| index)
            .collect();
        match found.as_slice() {
            [index] => {
                let info = Self::column_info(rows, &rows.columns[*index]);
                self.use_name(at, Definition::Column(info));
                Some(*index)
            }
            [] => {
                self.error(at, format!("Column {name} in {clause} does not exist"));
                None
            }
            _ => {
                self.error(at, format!("Column name {name} in {clause} is ambiguous"));
                None
            }
        }
    }

    /// The rows of a pipe `SET`: each assignment replaces a column in its place. The range
    /// variables keep the values before it.
    fn set(&mut self, rows: Rows, op: &ast::PipeSet, range: TextRange) -> Rows {
        let mut items: Vec<Item> = rows.columns.iter().cloned().map(Item::Carry).collect();
        let mut assigned = HashSet::new();
        let mut ok = true;
        for assignment in op.assignments() {
            let (Some(target), Some(value)) = (assignment.name_ref(), assignment.expr()) else {
                ok = false;
                continue;
            };
            let at = target.syntax().text_range();
            let Some(name) = target.text() else {
                ok = false;
                continue;
            };
            let planned = self.expr(&value, Scope::of(&rows, "pipe SET").windows());
            let Some(index) = self.unique_column(&rows, &name, at, "pipe SET") else {
                ok = false;
                continue;
            };
            if !assigned.insert(index) {
                self.error(
                    at,
                    format!("Column {name} is assigned more than once in pipe SET"),
                );
                ok = false;
                continue;
            }
            let col = &rows.columns[index];
            items[index] = Item::Compute {
                name: col.name.clone(),
                expr: planned.unwrap_or_else(|| {
                    ok = false;
                    unknown()
                }),
                definition: Some(at),
            };
        }
        if !ok {
            return rows.fail();
        }
        self.projected(rows, items, true, true, range)
    }

    /// The rows of a pipe `DROP`: the rows without the named columns, which the range variables
    /// keep.
    fn drop(&mut self, mut rows: Rows, op: &ast::PipeDrop) -> Rows {
        let mut ok = true;
        for name_ref in op.name_refs() {
            let at = name_ref.syntax().text_range();
            let Some(name) = name_ref.text() else {
                ok = false;
                continue;
            };
            let found: Vec<Col> = (rows.columns.iter())
                .filter(|col| col.name.eq_ignore_ascii_case(&name))
                .cloned()
                .collect();
            match found.first() {
                Some(col) => {
                    let info = Self::column_info(&rows, col);
                    self.use_name(at, Definition::Column(info));
                    rows.columns
                        .retain(|col| !col.name.eq_ignore_ascii_case(&name));
                }
                None => {
                    self.error(at, format!("Column {name} in pipe DROP does not exist"));
                    ok = false;
                }
            }
        }
        match ok {
            true => rows,
            false => rows.fail(),
        }
    }

    /// The rows of a pipe `RENAME`: each named column takes its new name in its place.
    fn rename(&mut self, rows: Rows, op: &ast::PipeRename, range: TextRange) -> Rows {
        let mut items: Vec<Item> = rows.columns.iter().cloned().map(Item::Carry).collect();
        let mut renamed = HashSet::new();
        let mut ok = true;
        for item in op.rename_items() {
            let (Some(from), Some(to)) = (item.name_ref(), item.name()) else {
                ok = false;
                continue;
            };
            let (Some(name), Some(new)) = (from.text(), to.text()) else {
                ok = false;
                continue;
            };
            let at = from.syntax().text_range();
            let Some(index) = self.unique_column(&rows, &name, at, "pipe RENAME") else {
                ok = false;
                continue;
            };
            if !renamed.insert(index) {
                self.error(
                    at,
                    format!("Column {name} is renamed more than once in pipe RENAME"),
                );
                ok = false;
                continue;
            }
            items[index] = Item::Compute {
                name: new,
                expr: Expr::Column(rows.columns[index].column.clone()),
                definition: Some(to.syntax().text_range()),
            };
        }
        if !ok {
            return rows.fail();
        }
        self.projected(rows, items, true, true, range)
    }

    /// The rows of a pipe `DISTINCT`, or of `SELECT DISTINCT`: one row for each distinct row of
    /// the columns, without an order. A range variable keeps its columns among them.
    fn distinct(&mut self, rows: Rows, range: TextRange) -> Rows {
        let vars = rows.vars.clone();
        let items = rows.columns.iter().cloned().map(Item::Carry).collect();
        let projected = self.projected(rows, items, false, false, range);
        let visible: HashSet<Column> = (projected.columns.iter())
            .map(|col| col.column.clone())
            .collect();
        let vars: Vec<RangeVar> = (vars.into_iter())
            .map(|var| RangeVar {
                columns: (var.columns.into_iter())
                    .filter(|col| visible.contains(&col.column))
                    .collect(),
                ..var
            })
            .collect();
        let Some(plan) = projected.plan.clone() else {
            return Rows::failed(projected.columns, vars);
        };
        let plan = LogicalPlanBuilder::from(plan)
            .distinct()
            .and_then(|b| b.build());
        let fallback = Rows::failed(projected.columns.clone(), vars.clone());
        self.built(
            plan.map(|plan| Rows::new(plan, projected.columns, vars)),
            range,
            fallback,
        )
    }

    /// The rows of a pipe `ORDER BY`: the keys replace the order. An integer literal is the
    /// position of a column, from 1.
    fn order_by(&mut self, rows: Rows, op: &ast::PipeOrderBy, range: TextRange) -> Rows {
        let Some(order) = op.order_by() else {
            return rows.fail();
        };
        let mut keys = Vec::new();
        let mut ok = true;
        for key in order.order_keys() {
            let Some(expr) = key.expr() else {
                ok = false;
                continue;
            };
            let at = expr.syntax().text_range();
            let asc = key.desc_kw().is_none();
            let nulls_first = match (key.first_kw(), key.last_kw()) {
                (Some(_), _) => true,
                (_, Some(_)) => false,
                _ => asc,
            };
            let planned = match &expr {
                ast::Expr::Literal(literal) if literal.kind() == Some(LiteralKind::Int) => {
                    let position = literal.int_value().unwrap_or(0) as usize;
                    match position
                        .checked_sub(1)
                        .and_then(|index| rows.columns.get(index))
                    {
                        Some(col) => Some(Expr::Column(col.column.clone())),
                        None => {
                            self.error(
                                at,
                                format!("ORDER BY column number exceeds input table column count: {position} vs {}", rows.columns.len()),
                            );
                            None
                        }
                    }
                }
                _ => self.expr(&expr, Scope::of(&rows, "pipe ORDER BY").windows()),
            };
            match planned {
                Some(expr) => keys.push(SortExpr::new(expr, asc, nulls_first)),
                None => ok = false,
            }
        }
        if !ok {
            return rows.fail();
        }
        let fallback = rows.clone().fail();
        let result = rows::order_by(&mut self.qualifiers, rows, keys);
        self.built(result, range, fallback)
    }

    /// The rows of a pipe `LIMIT`, whose count and offset are `INT64` constants.
    fn limit(&mut self, rows: Rows, op: &ast::PipeLimit, range: TextRange) -> Rows {
        let fetch = op.count().and_then(|count| self.constant(&count));
        let skip = match op.limit_offset() {
            Some(offset) => offset
                .expr()
                .and_then(|offset| self.constant(&offset))
                .map(Some),
            None => Some(None),
        };
        let (Some(fetch), Some(skip)) = (fetch, skip) else {
            return rows.fail();
        };
        let fallback = rows.clone().fail();
        let result = rows::limit(rows, skip, Some(fetch));
        self.built(result, range, fallback)
    }

    /// The `INT64` constant of a `LIMIT` or an `OFFSET`: a literal or a query parameter, which
    /// reads no columns.
    fn constant(&mut self, expr: &ast::Expr) -> Option<Expr> {
        let at = expr.syntax().text_range();
        let empty = DFSchema::empty();
        let scope = Scope {
            columns: &[],
            vars: &[],
            schema: Some(&empty),
            clause: "LIMIT",
            aggregates: false,
            windows: false,
        };
        let planned = self.expr(expr, scope)?;
        match &planned {
            Expr::Literal(ScalarValue::Int64(Some(value)), _) if *value >= 0 => Some(planned),
            Expr::Literal(ScalarValue::Int64(Some(_)), _) => {
                self.error(at, "LIMIT and OFFSET take a non-negative integer");
                None
            }
            Expr::Placeholder(_) => Some(Expr::Cast(datafusion::logical_expr::Cast::new(
                Box::new(planned),
                datafusion::arrow::datatypes::DataType::Int64,
            ))),
            _ => {
                let name = match planned.get_type(&empty) {
                    Ok(ty) => crate::types::googlesql_name(&ty),
                    Err(_) => "an expression".into(),
                };
                self.error(
                    at,
                    format!(
                        "LIMIT ... OFFSET ... expects an INT64 literal or parameter, got {name}"
                    ),
                );
                None
            }
        }
    }

    /// The rows of a pipe `AGGREGATE`: one row for each group of the group items, with the group
    /// columns, then the items, each an expression over aggregate functions and the group items.
    /// A suffix `ASC` or `DESC`, or `GROUP AND ORDER BY`, orders the rows by its item: the group
    /// items first, then the items.
    fn aggregate(&mut self, rows: Rows, op: &ast::PipeAggregate, range: TextRange) -> Rows {
        let rows = rows.unordered();
        let group_by = op.group_by();
        let ascending = group_by
            .as_ref()
            .is_some_and(|group| group.and_kw().is_some());
        let mut ok = true;
        struct Output {
            expr: Option<Expr>,
            name: String,
            definition: Option<TextRange>,
            order: Option<(bool, bool)>,
            range: TextRange,
        }
        let mut groups = Vec::new();
        for item in group_by.iter().flat_map(|group| group.grouping_items()) {
            let item_range = item.syntax().text_range();
            let ast::GroupingItem::GroupItem(item) = item else {
                self.error(
                    item_range,
                    "avenger-sql doesn't plan grouping sets, ROLLUP or CUBE yet",
                );
                ok = false;
                continue;
            };
            let expr = item.expr();
            if let Some(ast::Expr::Literal(literal)) = &expr {
                if literal.kind() == Some(LiteralKind::Int) {
                    self.error(item_range, "GROUP BY ordinal not allowed in pipe AGGREGATE");
                    ok = false;
                    continue;
                }
            }
            let alias = alias_name(item.alias());
            let name = match &alias {
                Some((name, _)) => name.clone(),
                None => expr.as_ref().map(inferred_name).unwrap_or_default(),
            };
            let definition = alias
                .map(|(_, range)| range)
                .or_else(|| expr.as_ref().map(|expr| expr.syntax().text_range()));
            let planned = expr.and_then(|expr| self.expr(&expr, Scope::of(&rows, "GROUP BY")));
            groups.push(Output {
                expr: planned,
                name,
                definition,
                order: suffix(item.order_suffix(), ascending),
                range: item_range,
            });
        }
        let mut items = Vec::new();
        for item in op.aggregate_items() {
            let item_range = item.syntax().text_range();
            let expr = item.expr();
            let alias = alias_name(item.alias());
            let name = match &alias {
                Some((name, _)) => name.clone(),
                None => expr.as_ref().map(inferred_name).unwrap_or_default(),
            };
            let definition = alias
                .map(|(_, range)| range)
                .or_else(|| expr.as_ref().map(|expr| expr.syntax().text_range()));
            let scope = Scope::of(&rows, "pipe AGGREGATE").aggregates();
            let planned = expr.and_then(|expr| self.expr(&expr, scope));
            items.push(Output {
                expr: planned,
                name,
                definition,
                order: suffix(item.order_suffix(), false),
                range: item_range,
            });
        }
        let names: Vec<Col> = (groups.iter().chain(&items))
            .map(|output| Col {
                name: output.name.clone(),
                column: Column::from_name(output.name.clone()),
                definition: output.definition,
            })
            .collect();
        let failed = || Rows::failed(names.clone(), Vec::new());
        for item in &items {
            if let Some(expr) = &item.expr {
                if find_aggregate_exprs([expr]).is_empty() {
                    self.error(
                        item.range,
                        "Pipe AGGREGATE cannot include non-aggregate expressions",
                    );
                    ok = false;
                }
            }
        }
        let all = groups
            .iter()
            .chain(&items)
            .all(|output| output.expr.is_some());
        let (Some(plan), true, true) = (rows.plan.clone(), ok, all) else {
            return failed();
        };
        let item_exprs: Vec<Expr> = items
            .iter()
            .map(|item| item.expr.clone().expect("an item"))
            .collect();
        let aggregates = find_aggregate_exprs(&item_exprs);
        let mut group_exprs: Vec<Expr> = Vec::new();
        for group in &groups {
            let expr = group.expr.clone().expect("a group");
            if !group_exprs.contains(&expr) {
                group_exprs.push(expr);
            }
        }
        let plan = LogicalPlanBuilder::from(plan)
            .aggregate(group_exprs.clone(), aggregates.clone())
            .and_then(|b| b.build());
        let plan: LogicalPlan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return failed();
            }
        };
        let output = plan.schema().columns();
        let mut projection = Vec::new();
        for group in &groups {
            let expr = group.expr.as_ref().expect("a group");
            let index = group_exprs
                .iter()
                .position(|e| e == expr)
                .expect("a group expression");
            let column = output[index].clone();
            projection.push(match expr {
                Expr::Column(c) if c.name == group.name => Item::Carry(Col {
                    name: group.name.clone(),
                    column,
                    definition: group.definition,
                }),
                _ => Item::Compute {
                    name: group.name.clone(),
                    expr: Expr::Column(column),
                    definition: group.definition,
                },
            });
        }
        let grouped: Vec<(Expr, Column)> = group_exprs
            .iter()
            .cloned()
            .zip(output.iter().cloned())
            .collect();
        for item in &items {
            let expr = item.expr.clone().expect("an item");
            let rebased = rows::rebase(expr, &aggregates).and_then(|expr| regroup(expr, &grouped));
            let rebased = match rebased {
                Ok(expr) => expr,
                Err(error) => {
                    self.datafusion_error(item.range, error);
                    return failed();
                }
            };
            if let Some(column) = rebased
                .column_refs()
                .into_iter()
                .find(|c| !output.contains(c))
            {
                self.error(
                    item.range,
                    format!(
                        "The item reads the column {}, which is neither grouped nor aggregated",
                        column.name
                    ),
                );
                return failed();
            }
            projection.push(Item::Compute {
                name: item.name.clone(),
                expr: rebased,
                definition: item.definition,
            });
        }
        let aggregated = Rows::new(plan, Vec::new(), Vec::new());
        let mut rows = self.projected(aggregated, projection, false, false, range);
        rows.order = (groups.iter().chain(&items).zip(&rows.columns))
            .filter_map(|(output, col)| {
                let (asc, nulls_first) = output.order?;
                Some(SortExpr::new(
                    Expr::Column(col.column.clone()),
                    asc,
                    nulls_first,
                ))
            })
            .collect();
        rows
    }
}

/// `expr` with each group expression of `grouped` written as its column of the aggregate's
/// output, so an item reads a group item, as `SUM(x) + y GROUP BY y` does.
fn regroup(expr: Expr, grouped: &[(Expr, Column)]) -> Result<Expr> {
    use datafusion::common::tree_node::{Transformed, TreeNode};
    expr.transform_down(|e| match grouped.iter().find(|(group, _)| *group == e) {
        Some((_, column)) => Ok(Transformed::yes(Expr::Column(column.clone()))),
        None => Ok(Transformed::no(e)),
    })
    .map(|t| t.data)
}
