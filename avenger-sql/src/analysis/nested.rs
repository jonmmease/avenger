//! The sources that read the rows before them or enclosing names: `UNNEST`, `LATERAL` and the
//! pipe `WITH`.
//!
//! An `UNNEST` gives a row for each element of its arrays, as a source of its own or as the right
//! side of a join, where it reads the columns of each row of the left side, as GoogleSQL's implicit
//! lateral join does. A `LEFT JOIN UNNEST` keeps a row whose array is empty or `NULL`, with a
//! `NULL` element. An `UNNEST` of structs is a value table: its one column is the struct, and the
//! names of its fields read the fields. Several arrays pad the shorter ones with `NULL`, or stop
//! at the shortest with `mode => 'TRUNCATE'`. `WITH OFFSET` numbers the elements from 0.
//!
//! A `LATERAL` query reads the columns of the left side of its join as outer references, which
//! DataFusion's optimizer decorrelates. A pipe `WITH` name is a query that the operators after it,
//! and the queries nested in them, read as a table.

use avenger_sql_syntax::{ast, ast::AstNode, TextRange};
use datafusion::{
    arrow::datatypes::DataType,
    common::{Column, ScalarValue, UnnestOptions},
    functions::core::expr_fn as core,
    functions_aggregate::bool_and_or::bool_or_udaf,
    functions_nested::expr_fn as nested,
    functions_window::row_number::row_number_udwf,
    logical_expr::{
        expr::WindowFunction, when, Expr, LogicalPlan, LogicalPlanBuilder, Subquery, WindowFrame,
        WindowFunctionDefinition,
    },
};
use std::sync::Arc;

use super::{
    expr::{Outer, Scope},
    model::PlaceKind,
    rows::{Col, Item, RangeVar, Rows},
    Pass, WithName,
};
use crate::library::Ty;

/// A name, and the range of its text.
type Named = (String, TextRange);

/// What a join unnests: an `UNNEST`, or the path of an array of a row before it, as in
/// `FROM t, t.xs`.
#[derive(Clone, Copy)]
pub(super) enum Unnesting<'a> {
    Unnest(&'a ast::UnnestSource),
    Path(&'a ast::TableSource),
}

/// The arrays of an unnest, planned: each array, with its alias and the range of its text, the
/// alias and the range of `WITH OFFSET`, and the mode of several arrays.
pub(super) struct Arrays {
    pub arrays: Vec<(Option<Expr>, Option<Named>, TextRange)>,
    pub offset: Option<Named>,
    pub truncate: bool,
    pub strict: bool,
}

/// Whether `table`, a source after `left`, is the path of an array of a row of `left`: a path of
/// two or more names whose first is a range variable of `left`.
fn array_path(left: &Rows, table: &ast::TableSource) -> bool {
    let Some(names) = table.path().and_then(|path| path.names()) else {
        return false;
    };
    names.len() > 1 && (left.vars.iter()).any(|var| var.name.eq_ignore_ascii_case(&names[0]))
}

fn int(value: i64) -> Expr {
    Expr::Literal(ScalarValue::Int64(Some(value)), None)
}

impl Pass<'_> {
    /// The rows of one row and no columns, which a `FROM UNNEST(...)` unnests onto.
    pub(super) fn one_row(&mut self) -> Rows {
        let plan = LogicalPlanBuilder::empty(true)
            .build()
            .expect("an empty relation");
        Rows::new(plan, Vec::new(), Vec::new())
    }

    /// The join of `left` and the source `right`, which can read the columns of `left` when it is
    /// an `UNNEST` or a `LATERAL` query.
    pub(super) fn join_source(
        &mut self,
        left: Rows,
        right: Option<ast::Source>,
        op: Option<ast::JoinOp>,
        condition: Option<ast::JoinCondition>,
        range: TextRange,
    ) -> Rows {
        match right {
            Some(ast::Source::UnnestSource(unnest)) => {
                self.join_unnest(left, Unnesting::Unnest(&unnest), op, condition, range)
            }
            Some(ast::Source::TableSource(table)) if array_path(&left, &table) => {
                self.join_unnest(left, Unnesting::Path(&table), op, condition, range)
            }
            Some(ast::Source::SubquerySource(subquery)) if subquery.lateral_kw().is_some() => {
                let right = self.lateral(&left, &subquery);
                self.join(left, right, op, condition, range)
            }
            Some(source) => {
                let right = self.source(&source);
                self.join(left, right, op, condition, range)
            }
            None => Rows::failed(Vec::new(), Vec::new()),
        }
    }

    /// A join of `left` and an unnest of each of its rows. A `LEFT JOIN` keeps each row of
    /// `left`: with a `NULL` element where its array is empty or `NULL`, and with an `ON`
    /// condition, where no element meets it.
    fn join_unnest(
        &mut self,
        left: Rows,
        unnesting: Unnesting,
        op: Option<ast::JoinOp>,
        condition: Option<ast::JoinCondition>,
        range: TextRange,
    ) -> Rows {
        let left = left.unordered();
        let outer = match &op {
            Some(op) if op.right_kw().is_some() || op.full_kw().is_some() => {
                self.error(
                    range,
                    "RIGHT JOIN and FULL JOIN of an UNNEST are not supported",
                );
                return left.fail();
            }
            Some(op) => op.left_kw().is_some(),
            None => false,
        };
        let on = match condition {
            Some(ast::JoinCondition::UsingCondition(_)) => {
                self.error(range, "A JOIN of an UNNEST takes no USING");
                return left.fail();
            }
            Some(ast::JoinCondition::OnCondition(on)) => match on.expr() {
                Some(expr) => Some(expr),
                None => return left.fail(),
            },
            None => None,
        };
        let Some(on) = on else {
            return self.unnesting_onto(left, unnesting, outer, range);
        };
        if !outer {
            let rows = self.unnesting_onto(left, unnesting, false, range);
            return self.on_filter(rows, &on);
        }
        // Each row of `left` takes a number, and keeps its first unnested row, with `NULL`
        // elements, when none of its elements meets the condition.
        let Some(plan) = left.plan.clone() else {
            return left.fail();
        };
        let row_id = self.qualifiers.fresh("$row");
        let number = Expr::from(WindowFunction::new(
            WindowFunctionDefinition::WindowUDF(row_number_udwf()),
            Vec::new(),
        ));
        let mut exprs: Vec<Expr> = plan
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        let numbered = LogicalPlanBuilder::window_plan(plan, vec![number]).and_then(|plan| {
            let id = plan
                .schema()
                .columns()
                .last()
                .cloned()
                .expect("the window's column");
            exprs.push(Expr::Column(id).alias_qualified(Some(row_id.clone()), "$row"));
            LogicalPlanBuilder::from(plan).project(exprs)?.build()
        });
        let numbered = match numbered {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return left.fail();
            }
        };
        let before = numbered.schema().columns();
        let row_column = Column::new(Some(row_id), "$row");
        let row = Expr::Column(row_column.clone());
        // The number is a column through the unnest, which carries the visible columns, and is
        // hidden again after it.
        let visible = left.columns.len();
        let mut columns = left.columns.clone();
        columns.push(Col {
            name: "$row".to_string(),
            column: row_column.clone(),
            definition: None,
        });
        let numbered = Rows {
            plan: Some(numbered),
            columns,
            ..left
        };
        let mut rows = self.unnesting_onto(numbered, unnesting, true, range);
        if rows.columns.len() > visible {
            rows.columns.remove(visible);
        }
        let at = on.syntax().text_range();
        let matched = {
            let scope = Scope::of(&rows, "JOIN ON clause");
            self.place(at, PlaceKind::Filter, &scope, None);
            self.expr(&on, scope)
                .and_then(|planned| self.boolean(planned, at, "JOIN ON clause", scope))
        };
        let (Some(matched), Some(plan)) = (matched, rows.plan.clone()) else {
            return rows.fail();
        };
        let matched = core::coalesce(vec![
            matched,
            Expr::Literal(ScalarValue::Boolean(Some(false)), None),
        ]);
        let over_row = |function: WindowFunctionDefinition, args: Vec<Expr>| {
            let mut window = WindowFunction::new(function, args);
            window.params.partition_by = vec![row.clone()];
            window.params.window_frame = WindowFrame::new(None);
            Expr::from(window)
        };
        let any = over_row(
            WindowFunctionDefinition::AggregateUDF(bool_or_udaf()),
            vec![matched.clone()],
        );
        let first = over_row(
            WindowFunctionDefinition::WindowUDF(row_number_udwf()),
            Vec::new(),
        )
        .eq(int(1));
        let keep = matched.clone().or((!any).and(first));
        let fallback = rows.clone().fail();
        let kept = match super::rows::filter(
            Rows {
                plan: Some(plan),
                ..rows.clone()
            },
            keep,
        ) {
            Ok(kept) => kept,
            Err(error) => {
                self.datafusion_error(at, error);
                return fallback;
            }
        };
        // The columns of the unnest are `NULL` in a row that no element of matched.
        let plan = kept.plan.clone().expect("a plan");
        let exprs: Vec<Expr> = (plan.schema().columns().into_iter())
            .map(|column| match before.contains(&column) {
                true => Expr::Column(column),
                false => when(matched.clone(), Expr::Column(column.clone()))
                    .end()
                    .expect("a CASE")
                    .alias_qualified(column.relation.clone(), column.name.clone()),
            })
            .collect();
        match LogicalPlanBuilder::from(plan)
            .project(exprs)
            .and_then(|b| b.build())
        {
            Ok(plan) => Rows {
                plan: Some(plan),
                ..kept
            },
            Err(error) => {
                self.datafusion_error(at, error);
                fallback
            }
        }
    }

    /// The rows of `rows` that the condition `on` keeps, as the `ON` of an inner join.
    fn on_filter(&mut self, rows: Rows, on: &ast::Expr) -> Rows {
        let at = on.syntax().text_range();
        let predicate = {
            let scope = Scope::of(&rows, "JOIN ON clause");
            self.place(at, PlaceKind::Filter, &scope, None);
            self.expr(on, scope)
                .and_then(|planned| self.boolean(planned, at, "JOIN ON clause", scope))
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

    /// The rows of `base` with the unnest of `unnesting`.
    fn unnesting_onto(
        &mut self,
        base: Rows,
        unnesting: Unnesting,
        outer: bool,
        range: TextRange,
    ) -> Rows {
        match unnesting {
            Unnesting::Unnest(unnest) => self.unnest_onto(base, unnest, outer, range),
            Unnesting::Path(table) => self.unnest_path(base, table, outer, range),
        }
    }

    /// The rows of `base` with a row for each element of the array that the path of `table`
    /// reads from a row of `base`, as GoogleSQL takes `FROM t, t.xs`, with its alias and its
    /// `WITH OFFSET`.
    fn unnest_path(
        &mut self,
        base: Rows,
        table: &ast::TableSource,
        outer: bool,
        range: TextRange,
    ) -> Rows {
        let Some(path) = table.path() else {
            return base.fail();
        };
        let at = path.syntax().text_range();
        let array = self.path(&path, Scope::of(&base, "FROM clause"));
        let name = (table.alias().and_then(|alias| alias.name()))
            .and_then(|name| Some((name.text()?, name.syntax().text_range())))
            .or_else(|| Some((path.names()?.last()?.clone(), at)));
        let offset = table.with_offset().map(|offset| {
            let name = offset.alias().and_then(|a| a.name()).and_then(|n| n.text());
            (
                name.unwrap_or_else(|| "offset".into()),
                offset.syntax().text_range(),
            )
        });
        let spec = Arrays {
            arrays: vec![(array, name, at)],
            offset,
            truncate: false,
            strict: false,
        };
        self.unnest_arrays(base, spec, outer, range)
    }

    /// The rows of `base` with a row for each element of the arrays of `unnest`, which read the
    /// columns of `base`. When `outer` holds, a row whose array is empty or `NULL` stays, with a
    /// `NULL` element.
    pub(super) fn unnest_onto(
        &mut self,
        base: Rows,
        unnest: &ast::UnnestSource,
        outer: bool,
        range: TextRange,
    ) -> Rows {
        let base = base.unordered();
        let alias = unnest
            .alias()
            .and_then(|alias| alias.name())
            .and_then(|name| Some((name.text()?, name.syntax().text_range())));
        let offset = unnest.with_offset().map(|offset| {
            let name = offset.alias().and_then(|a| a.name()).and_then(|n| n.text());
            (
                name.unwrap_or_else(|| "offset".into()),
                offset.syntax().text_range(),
            )
        });
        // The arrays, each with its alias. They read the columns before the `UNNEST`, and call no
        // aggregate or window.
        {
            let range = unnest.syntax().text_range();
            self.record_operator(range, Some(&base), &base);
            self.place(
                range,
                PlaceKind::Argument,
                &Scope::of(&base, "UNNEST"),
                None,
            );
        }
        let mut arrays: Vec<(Option<Expr>, Option<Named>, TextRange)> = Vec::new();
        for aliased in unnest.aliased_exprs() {
            let at = aliased.syntax().text_range();
            let name = aliased
                .alias()
                .and_then(|a| a.name())
                .and_then(|name| Some((name.text()?, name.syntax().text_range())));
            let planned = aliased
                .expr()
                .and_then(|expr| self.expr(&expr, Scope::of(&base, "UNNEST")));
            arrays.push((planned, name, at));
        }
        if arrays.len() == 1 && arrays[0].1.is_none() {
            arrays[0].1 = alias.clone();
        } else if alias.is_some() {
            self.error(
                range,
                "The alias of an UNNEST of several arrays goes after each array",
            );
            return base.fail();
        }
        // `STRICT` unnests as `PAD` does, and fails where the arrays differ in length.
        let mut strict = false;
        let truncate = match unnest.named_arg() {
            None => false,
            Some(arg) => {
                let name = arg.name_ref().and_then(|n| n.text()).unwrap_or_default();
                let mode = arg.expr().and_then(|expr| match expr {
                    ast::Expr::Literal(literal) => literal.string_value(),
                    _ => None,
                });
                match (
                    name.to_ascii_lowercase().as_str(),
                    mode.as_deref().map(str::to_ascii_uppercase),
                ) {
                    ("mode", Some(mode)) if mode == "PAD" => false,
                    ("mode", Some(mode)) if mode == "TRUNCATE" => true,
                    ("mode", Some(mode)) if mode == "STRICT" => {
                        strict = true;
                        false
                    }
                    _ => {
                        self.error(
                            arg.syntax().text_range(),
                            "The argument of UNNEST is mode => 'PAD', 'TRUNCATE' or 'STRICT'",
                        );
                        return base.fail();
                    }
                }
            }
        };
        self.unnest_arrays(
            base,
            Arrays {
                arrays,
                offset,
                truncate,
                strict,
            },
            outer,
            range,
        )
    }

    /// The rows of `base` with a row for each element of `spec`'s arrays, planned over `base`.
    /// When `outer` holds, a row whose array is empty or `NULL` stays, with a `NULL` element.
    pub(super) fn unnest_arrays(
        &mut self,
        base: Rows,
        spec: Arrays,
        outer: bool,
        range: TextRange,
    ) -> Rows {
        let Arrays {
            arrays,
            offset,
            truncate,
            strict,
        } = spec;
        let elements: Vec<Option<Ty>> = (arrays.iter())
            .map(|(expr, _, at)| {
                let expr = expr.as_ref()?;
                let ty = self
                    .type_of(expr, Scope::of(&base, "UNNEST"))
                    .and_then(|ty| Ty::of_arrow(&ty));
                match ty {
                    Some(Ty::Array(element)) => Some(*element),
                    Some(Ty::Null) => Some(Ty::Int64),
                    Some(other) => {
                        self.error(
                            *at,
                            format!("Values referenced in UNNEST must be arrays. UNNEST contains expression of type {other}"),
                        );
                        None
                    }
                    None => None,
                }
            })
            .collect();
        // The columns that the syntax gives, for the model after an error.
        let mut names: Vec<(String, Option<TextRange>)> = (arrays.iter())
            .map(|(_, name, at)| match name {
                Some((name, def)) => (name.clone(), Some(*def)),
                None => (String::new(), Some(*at)),
            })
            .collect();
        if let Some((name, at)) = &offset {
            names.push((name.clone(), Some(*at)));
        }
        let mut failed = base.clone().fail();
        failed
            .columns
            .extend(names.iter().map(|(name, definition)| Col {
                name: name.clone(),
                column: Column::from_name(name),
                definition: *definition,
            }));
        if base.plan.is_none()
            || arrays.iter().any(|(expr, _, _)| expr.is_none())
            || elements.iter().any(Option::is_none)
        {
            return failed;
        }
        let visible = base.columns.clone();
        // Names of the plan that no other unnest of the analysis gives, since an unnest gives its
        // columns without a qualifier.
        let unique = self.qualifiers.fresh("$unnest").to_string();
        let mut items: Vec<Item> = visible.iter().cloned().map(Item::Carry).collect();
        let mut lengths = Vec::new();
        for (index, ((expr, _, _), element)) in arrays.iter().zip(&elements).enumerate() {
            let mut array = expr.clone().expect("an array");
            let list =
                Ty::Array(Box::new(element.clone().expect("a type"))).arrow(self.type_options);
            array = Expr::Cast(datafusion::logical_expr::Cast::new(Box::new(array), list));
            if outer {
                // An empty array unnests to no row, and a `NULL` one keeps its row.
                array = when(
                    nested::cardinality(array.clone()).eq(int(0)),
                    Expr::Literal(ScalarValue::Null, None),
                )
                .otherwise(array)
                .expect("a CASE");
            }
            lengths.push(nested::cardinality(array.clone()));
            if strict && index > 0 {
                // A `NULL` array has no elements.
                let length = |e: Expr| {
                    core::coalesce(vec![e, Expr::Literal(ScalarValue::UInt64(Some(0)), None)])
                };
                let differ = length(lengths[0].clone()).not_eq(length(lengths[index].clone()));
                array = crate::library::error::error_when(
                    differ,
                    array,
                    "Unnested arrays under STRICT mode must have equal lengths",
                    Vec::new(),
                );
            }
            items.push(Item::Compute {
                name: format!("{unique}_array{}", index + 1),
                expr: array,
                definition: None,
            });
        }
        if offset.is_some() || truncate {
            // The offsets, from 0, of the longest array.
            let longest = match lengths.len() {
                1 => lengths[0].clone(),
                _ => core::coalesce(vec![nested::array_max(nested::make_array(lengths.clone()))]),
            };
            let offsets = when(
                longest.clone().is_null(),
                Expr::Literal(ScalarValue::Null, None),
            )
            .otherwise(nested::range(
                int(0),
                Expr::Cast(datafusion::logical_expr::Cast::new(
                    Box::new(longest),
                    DataType::Int64,
                )),
                int(1),
            ))
            .expect("a CASE");
            items.push(Item::Compute {
                name: format!("{unique}_offset"),
                expr: offsets,
                definition: None,
            });
        }
        if truncate {
            for (index, length) in lengths.iter().enumerate() {
                items.push(Item::Compute {
                    name: format!("$length{}", index + 1),
                    expr: length.clone(),
                    definition: None,
                });
            }
        }
        let projected = self.projected(base, items, true, false, range);
        let mut lists: Vec<Col> = projected.columns[visible.len()..].to_vec();
        let lengths: Vec<Col> = match truncate {
            true => lists.split_off(lists.len() - arrays.len()),
            false => Vec::new(),
        };
        let Some(plan) = projected.plan.clone() else {
            return failed;
        };
        let columns: Vec<Column> = lists.iter().map(|col| col.column.clone()).collect();
        let plan = LogicalPlanBuilder::from(plan)
            .unnest_columns_with_options(columns, UnnestOptions::new().with_preserve_nulls(outer))
            .and_then(|b| b.build());
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return failed;
            }
        };
        // An unnest gives its columns without their qualifier.
        let mut lists: Vec<Col> = (lists.into_iter())
            .map(|col| Col {
                column: Column::from_name(col.column.name.clone()),
                ..col
            })
            .collect();
        let offset_col = (offset.is_some() || truncate).then(|| lists.pop().expect("the offsets"));
        let mut rows = Rows {
            plan: Some(plan),
            columns: visible.clone(),
            vars: projected.vars.clone(),
            order: Vec::new(),
        };
        if truncate {
            // Each array stops at the length of the shortest.
            let offset = Expr::Column(offset_col.clone().expect("the offsets").column);
            let shortest = (lengths.iter())
                .map(|col| offset.clone().lt(Expr::Column(col.column.clone())))
                .reduce(Expr::and)
                .expect("an array");
            let plan = LogicalPlanBuilder::from(rows.plan.clone().expect("a plan"))
                .filter(shortest)
                .and_then(|b| b.build());
            match plan {
                Ok(plan) => rows.plan = Some(plan),
                Err(error) => {
                    self.datafusion_error(range, error);
                    return failed;
                }
            }
        }
        // The columns of the elements, and of the offsets.
        let mut items: Vec<Item> = visible.iter().cloned().map(Item::Carry).collect();
        for ((list, (_, name, at)), _) in lists.iter().zip(&arrays).zip(&elements) {
            items.push(Item::Compute {
                name: name
                    .as_ref()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default(),
                expr: Expr::Column(list.column.clone()),
                definition: Some(name.as_ref().map_or(*at, |(_, def)| *def)),
            });
        }
        if let (Some(col), Some((name, at))) = (&offset_col, &offset) {
            items.push(Item::Compute {
                name: name.clone(),
                expr: Expr::Column(col.column.clone()),
                definition: Some(*at),
            });
        }
        let mut rows = self.projected(rows, items, true, false, range);
        // An array of structs is a value table, whose fields its names read.
        if let ([Some(Ty::Struct(fields))], Some(value)) =
            (elements.as_slice(), rows.columns.get(visible.len()))
        {
            let value = value.clone();
            let mut items: Vec<Item> = rows.columns.iter().cloned().map(Item::Carry).collect();
            for (field, _) in fields {
                items.push(Item::Compute {
                    name: field.clone(),
                    expr: core::get_field(Expr::Column(value.column.clone()), field.as_str()),
                    definition: value.definition,
                });
            }
            let count = rows.columns.len();
            let mut with_fields = self.projected(rows, items, true, false, range);
            let fields = with_fields.columns.split_off(count);
            with_fields.vars.push(RangeVar {
                name: value.name.clone(),
                definition: value.definition,
                columns: fields,
                value: Some(value.column.clone()),
            });
            rows = with_fields;
        }
        rows
    }

    /// The rows of a `LATERAL` query, which reads the columns of `left` as outer references.
    fn lateral(&mut self, left: &Rows, subquery: &ast::SubquerySource) -> Rows {
        let range = subquery.syntax().text_range();
        let Some(query) = subquery.paren_query().and_then(|paren| paren.query()) else {
            return Rows::failed(Vec::new(), Vec::new());
        };
        self.outers.push(Outer::of(Scope::of(left, "LATERAL")));
        let inner = self.query(&query);
        self.outers.pop();
        let alias = subquery
            .alias()
            .and_then(|alias| alias.name())
            .and_then(|name| Some((name.text()?, name.syntax().text_range())));
        let rows = self.close(inner, alias, false, range);
        let Some(plan) = rows.plan.clone() else {
            return rows;
        };
        let outer_ref_columns = plan.all_out_ref_exprs();
        let plan = LogicalPlan::Subquery(Subquery {
            subquery: Arc::new(plan),
            outer_ref_columns,
            spans: Default::default(),
        });
        Rows {
            plan: Some(plan),
            ..rows
        }
    }

    /// The rows of a pipe `WITH`: its input, as it is, with each name in the scope of the
    /// operators after it. The query of a name doesn't read the input, and reads the names before
    /// it.
    pub(super) fn pipe_with(&mut self, rows: Rows, op: &ast::PipeWith) -> Rows {
        for named in op.named_queries() {
            let (Some(name), Some(query)) = (
                named
                    .name()
                    .and_then(|name| Some((name.text()?, name.syntax().text_range()))),
                named.paren_query().and_then(|paren| paren.query()),
            ) else {
                continue;
            };
            let query_rows = self.query(&query);
            self.with_names.push(WithName {
                name: name.0,
                definition: name.1,
                rows: query_rows,
            });
        }
        rows
    }
}
