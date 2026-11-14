//! The set operations: `UNION`, `INTERSECT` and `EXCEPT` of the rows and of queries, by position
//! or by name, as GoogleSQL matches their columns.
//!
//! By position, the queries have as many columns as the rows, and the result takes the names of
//! the rows. `BY NAME` and `STRICT CORRESPONDING` match the columns by name, and the queries need
//! the names of the rows; `CORRESPONDING` keeps the names that every query has. The modes choose
//! the names otherwise: `INNER` keeps the names of every query, `LEFT` those of the rows, and
//! `FULL` or `OUTER` those of any query, with `NULL` where a query lacks one. A list of names,
//! `BY NAME ON (...)` or `CORRESPONDING BY (...)`, is the result's names, in its order. Each
//! column of the result takes the common type of its columns.

use avenger_sql_syntax::{ast, ast::AstNode, TextRange};
use datafusion::{
    common::{Column, ScalarValue},
    logical_expr::{Cast, Expr, LogicalPlan, LogicalPlanBuilder},
};

use super::{
    model::PlaceKind,
    rows::{Col, Item, Rows},
    Pass,
};
use crate::library::{form::supertype, Ty};

/// How the set operation chooses the names of its result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Names {
    /// By position.
    Position,
    /// The names of the rows, which every query has, and no others.
    Strict,
    /// The names that every query has.
    Inner,
    /// The names of the rows.
    Left,
    /// The names of any query.
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Union,
    Intersect,
    Except,
}

impl Pass<'_> {
    pub(super) fn set_operation(
        &mut self,
        rows: Rows,
        op: &ast::PipeSetOperation,
        range: TextRange,
    ) -> Rows {
        let kind = if op.intersect_kw().is_some() {
            Kind::Intersect
        } else if op.except_kw().is_some() {
            Kind::Except
        } else {
            Kind::Union
        };
        let all = op.all_kw().is_some();
        let text = format!(
            "{} {}",
            match kind {
                Kind::Union => "UNION",
                Kind::Intersect => "INTERSECT",
                Kind::Except => "EXCEPT",
            },
            if all { "ALL" } else { "DISTINCT" }
        );
        let mode = op.set_op_mode().map(|mode| {
            if mode.inner_kw().is_some() {
                Names::Inner
            } else if mode.left_kw().is_some() {
                Names::Left
            } else {
                Names::Full
            }
        });
        let by_name = op.by_name();
        let corresponding = op.corresponding();
        let names = match (&by_name, &corresponding, mode) {
            (None, None, _) => Names::Position,
            (_, _, Some(mode)) => mode,
            (Some(_), _, None) => Names::Strict,
            (None, Some(corresponding), None) => match corresponding.strict_kw() {
                Some(_) => Names::Strict,
                None => Names::Inner,
            },
        };
        let list: Option<Vec<(String, TextRange)>> = (by_name.as_ref().and_then(|b| b.name_list()))
            .or_else(|| corresponding.as_ref().and_then(|c| c.name_list()))
            .map(|list| {
                self.place_without_scope(list.syntax().text_range(), PlaceKind::ColumnList);
                list.name_refs()
                    .filter_map(|name| Some((name.text()?, name.syntax().text_range())))
                    .collect()
            });
        let mut inputs = vec![(rows.unordered(), "pipe input table".to_string(), range)];
        for (index, paren) in op.paren_queries().enumerate() {
            let rows = match paren.query() {
                Some(query) => self.query(&query).unordered(),
                None => Rows::failed(Vec::new(), Vec::new()),
            };
            inputs.push((
                rows,
                format!("pipe argument query {}", index + 1),
                paren.syntax().text_range(),
            ));
        }
        let Some(layout) = self.layout(&inputs, names, list.as_deref(), &text, range) else {
            return Rows::failed(Vec::new(), Vec::new());
        };
        let columns: Vec<Col> = (layout.names.iter())
            .map(|(name, definition)| Col {
                name: name.clone(),
                column: Column::from_name(name),
                definition: *definition,
            })
            .collect();
        let failed = || Rows::failed(columns.clone(), Vec::new());
        // The common type of each column of the result.
        let mut types = Vec::new();
        for (position, (name, _)) in layout.names.iter().enumerate() {
            let observed: Option<Vec<(Ty, bool)>> = (inputs.iter().zip(&layout.columns))
                .map(|((rows, _, _), columns)| match columns[position] {
                    Some(index) => Some((Ty::of_arrow(&rows.ty(&rows.columns[index])?)?, false)),
                    None => Some((Ty::Null, true)),
                })
                .collect();
            let Some(observed) = observed else {
                return failed();
            };
            match supertype(&observed) {
                Some(ty) => types.push(ty),
                None => {
                    let shown: Vec<String> = (observed.iter())
                        .filter(|(ty, _)| *ty != Ty::Null)
                        .map(|(ty, _)| ty.to_string())
                        .collect();
                    let what = match names {
                        Names::Position => format!("Column {}", position + 1),
                        _ => format!("Column {name}"),
                    };
                    self.error(
                        range,
                        format!(
                            "{what} in {text} has incompatible types: {}",
                            shown.join(", ")
                        ),
                    );
                    return failed();
                }
            }
        }
        // Each input as the columns of the result, under names of their places.
        let mut plans: Vec<LogicalPlan> = Vec::new();
        for ((rows, _, _), columns) in inputs.iter().zip(&layout.columns) {
            let Some(plan) = rows.plan.clone() else {
                return failed();
            };
            let exprs: Vec<Expr> = (columns.iter().zip(&types).enumerate())
                .map(|(position, (index, ty))| {
                    let arrow = ty.arrow(self.type_options);
                    let value = match index {
                        Some(index) => {
                            let col = &rows.columns[*index];
                            let expr = Expr::Column(col.column.clone());
                            match rows.ty(col) {
                                Some(current)
                                    if crate::library::exact(&current, ty, self.type_options) =>
                                {
                                    expr
                                }
                                _ => Expr::Cast(Cast::new(Box::new(expr), arrow)),
                            }
                        }
                        None => Expr::Cast(Cast::new(
                            Box::new(Expr::Literal(ScalarValue::Null, None)),
                            arrow,
                        )),
                    };
                    value.alias(format!("$set{}", position + 1))
                })
                .collect();
            let projected = LogicalPlanBuilder::from(plan)
                .project(exprs)
                .and_then(|b| b.build());
            // INTERSECT ALL and EXCEPT ALL match the copies of a row by their numbers, so a row
            // stays as many times as GoogleSQL keeps it.
            let projected = match (kind, all) {
                (Kind::Intersect | Kind::Except, true) => projected.and_then(numbered),
                _ => projected,
            };
            match projected {
                Ok(plan) => plans.push(plan),
                Err(error) => {
                    self.datafusion_error(range, error);
                    return failed();
                }
            }
        }
        let mut plans = plans.into_iter();
        let mut plan = plans.next().expect("the rows");
        for right in plans {
            let builder = LogicalPlanBuilder::from(plan);
            let combined = match (kind, all) {
                (Kind::Union, true) => builder.union(right).and_then(|b| b.build()),
                (Kind::Union, false) => builder.union_distinct(right).and_then(|b| b.build()),
                (Kind::Intersect, _) => builder
                    .build()
                    .and_then(|left| LogicalPlanBuilder::intersect(left, right, false)),
                (Kind::Except, _) => builder
                    .build()
                    .and_then(|left| LogicalPlanBuilder::except(left, right, false)),
            };
            plan = match combined {
                Ok(plan) => plan,
                Err(error) => {
                    self.datafusion_error(range, error);
                    return failed();
                }
            };
        }
        let items = (layout.names.iter().enumerate())
            .map(|(position, (name, definition))| Item::Compute {
                name: name.clone(),
                expr: Expr::Column(Column::from_name(format!("$set{}", position + 1))),
                definition: *definition,
            })
            .collect();
        self.projected(
            Rows::new(plan, Vec::new(), Vec::new()),
            items,
            false,
            false,
            range,
        )
    }

    /// The names of the result of a set operation, and the column of each input at each name, or
    /// `None` with an error.
    fn layout(
        &mut self,
        inputs: &[(Rows, String, TextRange)],
        names: Names,
        list: Option<&[(String, TextRange)]>,
        text: &str,
        range: TextRange,
    ) -> Option<Layout> {
        let rows = &inputs[0].0;
        if names == Names::Position {
            for (input, what, at) in &inputs[1..] {
                if input.columns.len() != rows.columns.len() {
                    let count = |n: usize| match n {
                        1 => "1 column".to_string(),
                        n => format!("{n} columns"),
                    };
                    self.error(
                        *at,
                        format!(
                            "Queries in {text} have mismatched column count; pipe input table has {}, {what} has {}",
                            count(rows.columns.len()),
                            count(input.columns.len())
                        ),
                    );
                    return None;
                }
            }
            return Some(Layout {
                names: (rows.columns.iter())
                    .map(|col| (col.name.clone(), col.definition))
                    .collect(),
                columns: (inputs.iter())
                    .map(|_| (0..rows.columns.len()).map(Some).collect())
                    .collect(),
            });
        }
        let lower = |rows: &Rows| -> Vec<String> {
            rows.columns
                .iter()
                .map(|col| col.name.to_ascii_lowercase())
                .collect()
        };
        for (input, what, at) in inputs {
            let names = lower(input);
            if let Some(name) = (names.iter().enumerate())
                .find(|(index, name)| names[..*index].contains(name))
                .map(|(index, _)| &input.columns[index].name)
            {
                self.error(
                    *at,
                    format!("Duplicate columns found when using BY NAME in set operations: {name} in {what}"),
                );
                return None;
            }
        }
        let has = |input: &Rows, name: &str| lower(input).contains(&name.to_ascii_lowercase());
        let named: Vec<(String, Option<TextRange>)> = match list {
            Some(list) => {
                for (name, at) in list {
                    let missing = match names {
                        Names::Full => !inputs.iter().any(|(input, _, _)| has(input, name)),
                        Names::Left => !has(rows, name),
                        _ => inputs.iter().any(|(input, _, _)| !has(input, name)),
                    };
                    if missing {
                        self.error(*at, format!("Column {name} in the list of {text} is not a column of its queries"));
                        return None;
                    }
                }
                if names == Names::Strict {
                    for (input, what, at) in inputs {
                        if input.columns.len() != list.len() {
                            self.error(
                                *at,
                                format!(
                                    "The columns of {what} are not the names of the list of {text}"
                                ),
                            );
                            return None;
                        }
                    }
                }
                (list.iter())
                    .map(|(name, at)| {
                        let definition = (inputs.iter())
                            .flat_map(|(input, _, _)| input.columns.iter())
                            .find(|col| col.name.eq_ignore_ascii_case(name))
                            .and_then(|col| col.definition)
                            .or(Some(*at));
                        (name.clone(), definition)
                    })
                    .collect()
            }
            None => {
                let of = |input: &Rows| -> Vec<(String, Option<TextRange>)> {
                    (input.columns.iter())
                        .map(|col| (col.name.clone(), col.definition))
                        .collect()
                };
                match names {
                    Names::Strict => {
                        let mut want = lower(rows);
                        want.sort();
                        for (input, what, at) in &inputs[1..] {
                            let mut found = lower(input);
                            found.sort();
                            if found != want {
                                let shown = |input: &Rows| -> String {
                                    let names: Vec<&str> =
                                        input.columns.iter().map(|col| col.name.as_str()).collect();
                                    format!("[{}]", names.join(", "))
                                };
                                self.error(
                                    *at,
                                    format!(
                                        "BY NAME requires all input queries to have identical column names, but pipe input table has {} and {what} has {}",
                                        shown(rows),
                                        shown(input)
                                    ),
                                );
                                return None;
                            }
                        }
                        of(rows)
                    }
                    Names::Inner => (of(rows).into_iter())
                        .filter(|(name, _)| inputs.iter().all(|(input, _, _)| has(input, name)))
                        .collect(),
                    Names::Left => of(rows),
                    Names::Full | Names::Position => {
                        let mut all: Vec<(String, Option<TextRange>)> = Vec::new();
                        for (input, _, _) in inputs {
                            for (name, definition) in of(input) {
                                if !all.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                                    all.push((name, definition));
                                }
                            }
                        }
                        all
                    }
                }
            }
        };
        if named.is_empty() {
            self.error(
                range,
                format!("The queries of {text} have no column names in common"),
            );
            return None;
        }
        let columns = (inputs.iter())
            .map(|(input, _, _)| {
                (named.iter())
                    .map(|(name, _)| {
                        (input.columns.iter()).position(|col| col.name.eq_ignore_ascii_case(name))
                    })
                    .collect()
            })
            .collect();
        Some(Layout {
            names: named,
            columns,
        })
    }
}

/// `plan` with the column `$copy`, the number of each row among the rows of its values, from 1.
fn numbered(plan: LogicalPlan) -> datafusion::common::Result<LogicalPlan> {
    use datafusion::{
        functions_window::row_number::row_number_udwf,
        logical_expr::{expr::WindowFunction, WindowFunctionDefinition},
    };
    let columns: Vec<Expr> = plan
        .schema()
        .columns()
        .into_iter()
        .map(Expr::Column)
        .collect();
    let mut window = WindowFunction::new(
        WindowFunctionDefinition::WindowUDF(row_number_udwf()),
        Vec::new(),
    );
    window.params.partition_by = columns.clone();
    let window = Expr::from(window);
    let name = window.schema_name().to_string();
    let plan = LogicalPlanBuilder::window_plan(plan, vec![window])?;
    let mut exprs = columns;
    exprs.push(Expr::Column(Column::from_name(name)).alias("$copy"));
    LogicalPlanBuilder::from(plan).project(exprs)?.build()
}

/// The names of the result of a set operation, and for each input, the index of its column at
/// each name, or `None` for a `NULL`.
struct Layout {
    names: Vec<(String, Option<TextRange>)>,
    columns: Vec<Vec<Option<usize>>>,
}
