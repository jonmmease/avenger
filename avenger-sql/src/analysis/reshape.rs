//! `PIVOT`, `UNPIVOT` and `TABLESAMPLE`, after a source or as pipe operators.
//!
//! `PIVOT` is an aggregate: it groups by the columns that its aggregates and its `FOR` value don't
//! read, and computes each aggregate over the rows of each value of its `IN` list. `UNPIVOT` is a
//! union of one projection for each column group of its `IN` list. `TABLESAMPLE` has no plan in
//! DataFusion, so the pass approximates it with random numbers: `BERNOULLI` and `SYSTEM` keep each
//! row with the chance of the percent, so `SYSTEM` samples rows rather than blocks, and
//! `RESERVOIR` keeps the rows of the least numbers. `REPEATABLE (seed)` numbers the rows with a
//! hash of the seed and each row's place, so a sample repeats for the same rows in the same order.

use std::collections::HashSet;

use avenger_sql_syntax::{
    ast::{self, AstNode, LiteralKind},
    TextRange,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{Transformed, TreeNode},
        Column, Result, ScalarValue,
    },
    functions::expr_fn as f,
    functions_aggregate::count::count_udaf,
    functions_window::row_number::row_number_udwf,
    logical_expr::{
        binary_expr, expr::WindowFunction, utils::find_aggregate_exprs, Cast, Expr,
        LogicalPlanBuilder, Operator, SortExpr, WindowFunctionDefinition,
    },
};

use super::{
    expr::Scope,
    rows::{self, Col, Item, RangeVar, Rows},
    Pass,
};
use crate::library::{form::coercion, random, Ty};

fn cast(expr: Expr, ty: DataType) -> Expr {
    Expr::Cast(Cast::new(Box::new(expr), ty))
}

fn int(value: i64) -> Expr {
    Expr::Literal(ScalarValue::Int64(Some(value)), None)
}

fn float(value: f64) -> Expr {
    Expr::Literal(ScalarValue::Float64(Some(value)), None)
}

/// `func` over the window of `partition` in the order of `order`.
fn window(
    func: WindowFunctionDefinition,
    args: Vec<Expr>,
    partition: Vec<Expr>,
    order: Vec<SortExpr>,
) -> Expr {
    let mut window = WindowFunction::new(func, args);
    window.params.partition_by = partition;
    window.params.window_frame =
        datafusion::logical_expr::WindowFrame::new((!order.is_empty()).then_some(false));
    window.params.order_by = order;
    Expr::from(window)
}

/// The name of a column of a `PIVOT` for the value `value`: its string, `NULL`, or `_` and its
/// number.
fn value_name(value: &ScalarValue) -> String {
    if value.is_null() {
        return "NULL".to_string();
    }
    match value.try_as_str() {
        Some(Some(text)) => text.to_string(),
        _ => format!("_{value}").replace(['-', '.'], "_"),
    }
}

/// The range variable of the alias of a `PIVOT` or an `UNPIVOT`, over its output.
fn alias_var(alias: Option<ast::Alias>, columns: &[Col]) -> Vec<RangeVar> {
    let Some(name) = alias.and_then(|alias| alias.name()) else {
        return Vec::new();
    };
    let Some(text) = name.text() else {
        return Vec::new();
    };
    vec![RangeVar {
        name: text,
        definition: Some(name.syntax().text_range()),
        columns: columns.to_vec(),
        value: None,
    }]
}

impl Pass<'_> {
    /// The rows of a `PIVOT`.
    pub(super) fn pivot(
        &mut self,
        rows: Rows,
        clause: &ast::PivotClause,
        range: TextRange,
    ) -> Rows {
        let rows = rows.unordered().without_vars();
        let aggregates: Vec<(Option<Expr>, Option<String>, TextRange)> = clause
            .aliased_exprs()
            .map(|aliased| {
                let at = aliased.syntax().text_range();
                let alias = aliased
                    .alias()
                    .and_then(|a| a.name())
                    .and_then(|n| n.text());
                let scope = Scope::of(&rows, "PIVOT").aggregates();
                (aliased.expr().and_then(|e| self.expr(&e, scope)), alias, at)
            })
            .collect();
        let for_expr = clause
            .column()
            .and_then(|e| self.expr(&e, Scope::of(&rows, "PIVOT")));
        let values: Vec<(Option<Expr>, Option<String>, TextRange)> =
            (clause.pivot_values().into_iter())
                .flat_map(|values| values.aliased_exprs().collect::<Vec<_>>())
                .map(|aliased| {
                    let at = aliased.syntax().text_range();
                    let alias = aliased
                        .alias()
                        .and_then(|a| a.name())
                        .and_then(|n| n.text());
                    (
                        aliased
                            .expr()
                            .and_then(|e| self.expr(&e, Scope::of(&rows, "PIVOT"))),
                        alias,
                        at,
                    )
                })
                .collect();
        let mut ok = true;
        if aggregates.len() > 1 && aggregates.iter().any(|(_, alias, _)| alias.is_none()) {
            self.error(
                range,
                "PIVOT expression must specify an alias unless it is the only pivot expression in the PIVOT clause",
            );
            ok = false;
        }
        for (expr, _, at) in &aggregates {
            let inner = match expr {
                Some(Expr::Cast(cast)) => Some(cast.expr.as_ref()),
                other => other.as_ref(),
            };
            if inner.is_some_and(|expr| !matches!(expr, Expr::AggregateFunction(_))) {
                self.error(*at, "PIVOT expression must be an aggregate function call");
                ok = false;
            }
        }
        // The output columns, for the model after an error.
        let names: Vec<String> = (values.iter())
            .flat_map(|(value, alias, _)| {
                let name = alias.clone().unwrap_or_else(|| match value {
                    Some(Expr::Literal(value, _)) => value_name(value),
                    _ => String::new(),
                });
                aggregates.iter().map(move |(_, agg, _)| match agg {
                    Some(agg) => format!("{agg}_{name}"),
                    None => name.clone(),
                })
            })
            .collect();
        let all = aggregates.iter().all(|(e, _, _)| e.is_some())
            && values.iter().all(|(e, _, _)| e.is_some());
        let (Some(plan), Some(for_expr), true, true) = (rows.plan.clone(), for_expr, ok, all)
        else {
            let columns = names.iter().map(|name| self.syntax_col(name)).collect();
            return Rows::failed(columns, Vec::new());
        };
        // The values, in the type of the `FOR` expression.
        let scope = Scope::of(&rows, "PIVOT");
        let for_ty = self
            .type_of(&for_expr, scope)
            .and_then(|ty| Ty::of_arrow(&ty));
        let mut typed_values = Vec::new();
        for (value, _, at) in &values {
            let value = value.clone().expect("a value");
            let ty = self.type_of(&value, scope).and_then(|ty| Ty::of_arrow(&ty));
            let literal = matches!(value, Expr::Literal(..));
            match (&for_ty, ty) {
                (Some(want), Some(ty)) if coercion(&ty, literal, want).is_some() => {
                    typed_values.push(self.coerce_to(value, want, scope).expect("a typed value"));
                }
                (Some(want), Some(ty)) => {
                    self.error(
                        *at,
                        format!("PIVOT IN list item must be type {want} to match the PIVOT FOR expression; found {ty}"),
                    );
                    return Rows::failed(Vec::new(), Vec::new());
                }
                _ => return Rows::failed(Vec::new(), Vec::new()),
            }
        }
        // The group columns: those that no aggregate and not the `FOR` expression reads.
        let mut read: HashSet<Column> = for_expr.column_refs().into_iter().cloned().collect();
        for (expr, _, _) in &aggregates {
            read.extend(
                expr.as_ref()
                    .expect("an aggregate")
                    .column_refs()
                    .into_iter()
                    .cloned(),
            );
        }
        let groups: Vec<Col> = (rows.columns.iter())
            .filter(|col| !read.contains(&col.column))
            .cloned()
            .collect();
        // Each aggregate over the rows of each value.
        let mut outputs = Vec::new();
        for ((value, value_alias, _), typed) in values.iter().zip(&typed_values) {
            let condition =
                binary_expr(for_expr.clone(), Operator::IsNotDistinctFrom, typed.clone());
            let name = value_alias.clone().unwrap_or_else(|| match value {
                Some(Expr::Literal(value, _)) => value_name(value),
                _ => String::new(),
            });
            for (expr, alias, _) in &aggregates {
                let filtered = filtered(expr.clone().expect("an aggregate"), &condition);
                let name = match alias {
                    Some(alias) => format!("{alias}_{name}"),
                    None => name.clone(),
                };
                outputs.push((name, filtered));
            }
        }
        let filtered_exprs: Vec<Expr> = outputs.iter().map(|(_, expr)| expr.clone()).collect();
        let aggregates = find_aggregate_exprs(&filtered_exprs);
        let group_exprs: Vec<Expr> = groups
            .iter()
            .map(|col| Expr::Column(col.column.clone()))
            .collect();
        let plan = LogicalPlanBuilder::from(plan)
            .aggregate(group_exprs, aggregates.clone())
            .and_then(|b| b.build());
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                self.datafusion_error(range, error);
                return Rows::failed(Vec::new(), Vec::new());
            }
        };
        let mut items: Vec<Item> = groups.into_iter().map(Item::Carry).collect();
        for (name, expr) in outputs {
            match rows::rebase(expr, &aggregates) {
                Ok(expr) => items.push(Item::Compute {
                    name,
                    expr,
                    definition: Some(range),
                }),
                Err(error) => {
                    self.datafusion_error(range, error);
                    return Rows::failed(Vec::new(), Vec::new());
                }
            }
        }
        let rows = self.projected(
            Rows::new(plan, Vec::new(), Vec::new()),
            items,
            false,
            false,
            range,
        );
        let vars = alias_var(clause.alias(), &rows.columns);
        Rows { vars, ..rows }
    }

    /// A column without a plan, for the model after an error.
    fn syntax_col(&self, name: &str) -> Col {
        Col {
            name: name.to_string(),
            column: Column::from_name(name),
            definition: None,
        }
    }

    /// The rows of an `UNPIVOT`.
    pub(super) fn unpivot(
        &mut self,
        rows: Rows,
        clause: &ast::UnpivotClause,
        range: TextRange,
    ) -> Rows {
        let rows = rows.unordered().without_vars();
        let include_nulls = clause.include_kw().is_some();
        let value_names: Vec<(String, TextRange)> = (clause.path_list().into_iter())
            .flat_map(|list| list.paths().collect::<Vec<_>>())
            .filter_map(|path| {
                let names = path.names()?;
                Some((names.last()?.clone(), path.syntax().text_range()))
            })
            .collect();
        let label_name = clause
            .path()
            .and_then(|path| path.names())
            .and_then(|names| names.last().cloned())
            .unwrap_or_default();
        let mut ok = true;
        let mut groups: Vec<(Vec<Expr>, Expr, TextRange)> = Vec::new();
        for item in clause.unpivot_items() {
            let at = item.syntax().text_range();
            let mut columns = Vec::new();
            let mut names = Vec::new();
            for path in item.path_list().iter().flat_map(|list| list.paths()) {
                names.push(
                    path.names()
                        .and_then(|n| n.last().cloned())
                        .unwrap_or_default(),
                );
                match self.expr_of_path(&path, &rows) {
                    Some(expr) => columns.push(expr),
                    None => ok = false,
                }
            }
            if columns.len() != value_names.len() && ok {
                self.error(
                    at,
                    "The number of output value columns does not match the number of columns in each column group of the UNPIVOT IN clause",
                );
                ok = false;
            }
            let label = match item.literal() {
                Some(literal) => match literal.kind() {
                    Some(LiteralKind::String) => {
                        Expr::Literal(ScalarValue::Utf8(literal.string_value()), None)
                    }
                    Some(LiteralKind::Int) => Expr::Literal(
                        ScalarValue::Int64(literal.int_value().and_then(|v| i64::try_from(v).ok())),
                        None,
                    ),
                    _ => {
                        self.error(literal.syntax().text_range(), "The label of an UNPIVOT column group is a string or an integer literal");
                        ok = false;
                        continue;
                    }
                },
                None => Expr::Literal(ScalarValue::Utf8(Some(names.join("_"))), None),
            };
            groups.push((columns, label, at));
        }
        let mut output_names: Vec<String> = Vec::new();
        let read: HashSet<Column> = (groups.iter())
            .flat_map(|(columns, _, _)| columns.iter())
            .flat_map(|expr| expr.column_refs().into_iter().cloned())
            .collect();
        let others: Vec<Col> = (rows.columns.iter())
            .filter(|col| !read.contains(&col.column))
            .cloned()
            .collect();
        output_names.extend(others.iter().map(|col| col.name.clone()));
        output_names.extend(value_names.iter().map(|(name, _)| name.clone()));
        output_names.push(label_name.clone());
        let failed = |pass: &Self| {
            Rows::failed(
                output_names
                    .iter()
                    .map(|name| pass.syntax_col(name))
                    .collect(),
                Vec::new(),
            )
        };
        let Some(plan) = rows.plan.clone().filter(|_| ok && !groups.is_empty()) else {
            return failed(self);
        };
        // Each value column takes one type, and each label one type.
        let scope = Scope::of(&rows, "UNPIVOT");
        let type_of =
            |pass: &Self, expr: &Expr| pass.type_of(expr, scope).and_then(|ty| Ty::of_arrow(&ty));
        for position in 0..value_names.len() {
            let want = type_of(self, &groups[0].0[position]);
            for (columns, _, at) in &groups[1..] {
                let found = type_of(self, &columns[position]);
                if found != want {
                    let shown = |ty: &Option<Ty>| {
                        ty.as_ref()
                            .map_or("an unknown type".to_string(), Ty::to_string)
                    };
                    self.error(
                        *at,
                        format!(
                            "The datatype of column does not match other datatypes in the IN clause. Expected {}, Found {}",
                            shown(&want),
                            shown(&found)
                        ),
                    );
                    return failed(self);
                }
            }
        }
        let label_ty = type_of(self, &groups[0].1);
        if groups
            .iter()
            .any(|(_, label, _)| type_of(self, label) != label_ty)
        {
            self.error(
                range,
                "The labels of the column groups of UNPIVOT take one type",
            );
            return failed(self);
        }
        let mut plans = Vec::new();
        for (columns, label, _) in &groups {
            let mut exprs: Vec<Expr> = (others.iter().enumerate())
                .map(|(i, col)| Expr::Column(col.column.clone()).alias(format!("$other{}", i + 1)))
                .collect();
            exprs.extend(
                (columns.iter().enumerate())
                    .map(|(i, expr)| expr.clone().alias(format!("$value{}", i + 1))),
            );
            exprs.push(label.clone().alias("$label"));
            let mut builder = LogicalPlanBuilder::from(plan.clone());
            if !include_nulls {
                let kept = (columns.iter())
                    .map(|expr| expr.clone().is_not_null())
                    .reduce(Expr::or)
                    .expect("a column");
                builder = match builder.filter(kept) {
                    Ok(builder) => builder,
                    Err(error) => {
                        self.datafusion_error(range, error);
                        return failed(self);
                    }
                };
            }
            match builder.project(exprs).and_then(|b| b.build()) {
                Ok(plan) => plans.push(plan),
                Err(error) => {
                    self.datafusion_error(range, error);
                    return failed(self);
                }
            }
        }
        let mut plans = plans.into_iter();
        let mut plan = plans.next().expect("a column group");
        for next in plans {
            plan = match LogicalPlanBuilder::from(plan)
                .union(next)
                .and_then(|b| b.build())
            {
                Ok(plan) => plan,
                Err(error) => {
                    self.datafusion_error(range, error);
                    return failed(self);
                }
            };
        }
        let mut items: Vec<Item> = (others.iter().enumerate())
            .map(|(i, col)| Item::Compute {
                name: col.name.clone(),
                expr: Expr::Column(Column::from_name(format!("$other{}", i + 1))),
                definition: col.definition,
            })
            .collect();
        items.extend(
            (value_names.iter().enumerate()).map(|(i, (name, at))| Item::Compute {
                name: name.clone(),
                expr: Expr::Column(Column::from_name(format!("$value{}", i + 1))),
                definition: Some(*at),
            }),
        );
        items.push(Item::Compute {
            name: label_name,
            expr: Expr::Column(Column::from_name("$label")),
            definition: clause.path().map(|path| path.syntax().text_range()),
        });
        let rows = self.projected(
            Rows::new(plan, Vec::new(), Vec::new()),
            items,
            false,
            false,
            range,
        );
        let vars = alias_var(clause.alias(), &rows.columns);
        Rows { vars, ..rows }
    }

    /// The column of the rows that `path` names, which an `UNPIVOT` reads.
    fn expr_of_path(&mut self, path: &ast::Path, rows: &Rows) -> Option<Expr> {
        let names = path.names()?;
        let [name] = names.as_slice() else {
            self.error(
                path.syntax().text_range(),
                "UNPIVOT reads a column by its name alone",
            );
            return None;
        };
        let found: Vec<&Col> = (rows.columns.iter())
            .filter(|col| col.name.eq_ignore_ascii_case(name))
            .collect();
        match found.as_slice() {
            [col] => Some(Expr::Column(col.column.clone())),
            [] => {
                self.error(
                    path.syntax().text_range(),
                    format!("Unrecognized name: {name}"),
                );
                None
            }
            _ => {
                self.error(
                    path.syntax().text_range(),
                    format!("Column name {name} is ambiguous"),
                );
                None
            }
        }
    }

    /// The rows of a `TABLESAMPLE`.
    pub(super) fn sample(
        &mut self,
        rows: Rows,
        clause: &ast::SampleClause,
        range: TextRange,
    ) -> Rows {
        let rows = rows.unordered();
        let method = (clause.method().and_then(|m| m.text()))
            .unwrap_or_default()
            .to_ascii_uppercase();
        let size = clause.sample_size();
        let by_rows = size.as_ref().is_some_and(|size| size.rows_kw().is_some());
        let unit = if by_rows { "ROWS" } else { "PERCENT" };
        let valid = matches!(
            (method.as_str(), by_rows),
            ("BERNOULLI" | "SYSTEM", false) | ("RESERVOIR", true)
        );
        if !matches!(method.as_str(), "BERNOULLI" | "SYSTEM" | "RESERVOIR") {
            self.error(range, format!("Unsupported sampling method {method}"));
            return rows.fail();
        }
        if !valid {
            self.error(range, format!("{method}/{unit} is not supported"));
            return rows.fail();
        }
        let constant = |pass: &mut Self, expr: Option<ast::Expr>| -> Option<f64> {
            let expr = expr?;
            let at = expr.syntax().text_range();
            let empty = datafusion::common::DFSchema::empty();
            let scope = Scope {
                columns: &[],
                vars: &[],
                schema: Some(&empty),
                clause: "TABLESAMPLE",
                aggregates: false,
                windows: false,
                groups: Some(&[]),
            };
            match pass.expr(&expr, scope)? {
                Expr::Literal(ScalarValue::Int64(Some(n)), _) if n >= 0 => Some(n as f64),
                Expr::Literal(ScalarValue::Float64(Some(n)), _) if n >= 0.0 => Some(n),
                _ => {
                    pass.error(
                        at,
                        "The size of TABLESAMPLE is a non-negative number literal",
                    );
                    None
                }
            }
        };
        let Some(amount) = constant(self, size.as_ref().and_then(|size| size.expr())) else {
            return rows.fail();
        };
        if !by_rows && amount > 100.0 {
            self.error(range, "PERCENT value must be in the range [0, 100]");
            return rows.fail();
        }
        let seed = match clause.repeatable() {
            Some(repeatable) => match constant(self, repeatable.expr()) {
                Some(seed) => Some(seed as i64),
                None => return rows.fail(),
            },
            None => None,
        };
        let partition: Vec<Expr> = match size.as_ref().and_then(|size| size.partition_by()) {
            Some(partition) => {
                if !by_rows {
                    self.error(
                        partition.syntax().text_range(),
                        "PARTITION BY is only supported with RESERVOIR sampling",
                    );
                    return rows.fail();
                }
                let planned: Option<Vec<Expr>> = partition
                    .exprs()
                    .map(|expr| self.expr(&expr, Scope::of(&rows, "TABLESAMPLE")))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .collect();
                match planned {
                    Some(partition) => partition,
                    None => return rows.fail(),
                }
            }
            None => Vec::new(),
        };
        let weight = clause.weight().map(|weight| {
            let name = weight.alias().and_then(|a| a.name()).and_then(|n| n.text());
            (
                name.unwrap_or_else(|| "weight".to_string()),
                weight.syntax().text_range(),
            )
        });
        if rows.plan.is_none() {
            let mut rows = rows.fail();
            if let Some((name, _)) = &weight {
                rows.columns.push(self.syntax_col(name));
            }
            return rows;
        }
        let visible = rows.columns.clone();
        let carry =
            |columns: &[Col]| -> Vec<Item> { columns.iter().cloned().map(Item::Carry).collect() };
        // The random number of each row, and the count of each partition for a weight.
        let random = match seed {
            Some(seed) => random::seeded(
                int(seed),
                cast(
                    window(
                        WindowFunctionDefinition::WindowUDF(row_number_udwf()),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    ),
                    DataType::Int64,
                ),
            ),
            None => f::random(),
        };
        let mut items = carry(&visible);
        items.push(Item::Compute {
            name: "$random".into(),
            expr: random,
            definition: None,
        });
        if by_rows && weight.is_some() {
            items.push(Item::Compute {
                name: "$count".into(),
                expr: window(
                    WindowFunctionDefinition::AggregateUDF(count_udaf()),
                    vec![int(1)],
                    partition.clone(),
                    Vec::new(),
                ),
                definition: None,
            });
        }
        let mut numbered = self.projected(rows, items, true, false, range);
        let hidden: Vec<Col> = numbered.columns.split_off(visible.len());
        numbered.columns = visible.clone();
        let Some(plan) = numbered.plan.clone() else {
            return numbered;
        };
        let random = Expr::Column(hidden[0].column.clone());
        let sampled = match by_rows {
            false => LogicalPlanBuilder::from(plan)
                .filter(random.lt(float(amount / 100.0)))
                .and_then(|b| b.build())
                .map(|plan| Rows {
                    plan: Some(plan),
                    ..numbered.clone()
                }),
            true if partition.is_empty() => LogicalPlanBuilder::from(plan)
                .sort(vec![SortExpr::new(random, true, true)])
                .and_then(|b| b.limit(0, Some(amount as usize)))
                .and_then(|b| b.build())
                .map(|plan| Rows {
                    plan: Some(plan),
                    ..numbered.clone()
                }),
            true => {
                // The rows of the least numbers of each partition.
                let mut items = carry(&visible);
                items.extend(hidden.iter().cloned().map(Item::Carry));
                items.push(Item::Compute {
                    name: "$rank".into(),
                    expr: window(
                        WindowFunctionDefinition::WindowUDF(row_number_udwf()),
                        Vec::new(),
                        partition,
                        vec![SortExpr::new(random, true, true)],
                    ),
                    definition: None,
                });
                let mut ranked = self.projected(numbered.clone(), items, true, false, range);
                let rank = ranked.columns.pop().expect("the rank");
                ranked.columns = visible.clone();
                match ranked.plan.clone() {
                    Some(plan) => LogicalPlanBuilder::from(plan)
                        .filter(Expr::Column(rank.column).lt_eq(int(amount as i64)))
                        .and_then(|b| b.build())
                        .map(|plan| Rows {
                            plan: Some(plan),
                            ..ranked
                        }),
                    None => Ok(ranked),
                }
            }
        };
        let fallback = numbered.clone().fail();
        let sampled = self.built(sampled, range, fallback);
        let Some((name, at)) = weight else {
            return sampled;
        };
        // The weight of each row: the number of rows that it stands for.
        let expr = match by_rows {
            false => float(100.0 / amount),
            true => {
                let count = cast(Expr::Column(hidden[1].column.clone()), DataType::Float64);
                count.clone() / f::least(vec![count, float(amount)])
            }
        };
        let mut items = carry(&sampled.columns);
        items.push(Item::Compute {
            name,
            expr,
            definition: Some(at),
        });
        self.projected(sampled, items, true, false, range)
    }
}

/// `expr`, an aggregate, over the rows where `condition` holds, beside its own filter.
fn filtered(expr: Expr, condition: &Expr) -> Expr {
    let result: Result<Transformed<Expr>> = expr.transform_down(|e| match e {
        Expr::AggregateFunction(mut aggregate) => {
            let filter = match aggregate.params.filter.take() {
                Some(filter) => filter.and(condition.clone()),
                None => condition.clone(),
            };
            aggregate.params.filter = Some(Box::new(filter));
            Ok(Transformed::new(
                Expr::AggregateFunction(aggregate),
                true,
                datafusion::common::tree_node::TreeNodeRecursion::Jump,
            ))
        }
        other => Ok(Transformed::no(other)),
    });
    result.map(|t| t.data).expect("a transform without errors")
}
