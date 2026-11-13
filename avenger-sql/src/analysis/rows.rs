//! The rows of a query so far: their DataFusion plan, their columns, the range variables in scope,
//! and the keys of their order.
//!
//! The pass resolves names itself, by GoogleSQL's rules, and refers to the columns of the plan by
//! their DataFusion columns, so a plan's names only need to be valid in DataFusion:
//!
//! - Every column of a plan has a qualifier, and no two plans of an analysis share one, so the
//!   schema of a join never holds a name two times. A source takes the name of its range
//!   variable as its qualifier, and the columns that an operator computes take a qualifier of
//!   the operator's, such as `_2`. The column keeps the name that the query gives it, so a mark
//!   names a channel's layer column after it. A column without a name is `$col1`, `$col2` and
//!   so on, after its place.
//! - A range variable reads the row of its source as the source wrote it, after `SET`, `DROP`,
//!   `RENAME` and `EXTEND`, as in GoogleSQL, so its columns stay in the plan as hidden columns
//!   until an operator ends the range variable.
//!
//! The order follows the design of the earlier planner: DataFusion guarantees only the order of
//! a final sort, so an `ORDER BY` records its keys, an operator that keeps the order carries them
//! through as columns of the plan, and the sort happens at a `LIMIT` and at the end of a
//! statement.

use std::collections::HashSet;

use avenger_sql_syntax::TextRange;
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        tree_node::{Transformed, TreeNode},
        Column, DFSchema, ExprSchema, Result, TableReference,
    },
    logical_expr::{utils::find_window_exprs, Expr, LogicalPlan, LogicalPlanBuilder, SortExpr},
};

/// A column of the rows, visible or of a range variable.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Col {
    /// The name as the query writes it, or the empty string for a column without a name.
    pub name: String,
    /// The column of the plan.
    pub column: Column,
    pub definition: Option<TextRange>,
}

/// A range variable, and the columns of its row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RangeVar {
    pub name: String,
    pub definition: Option<TextRange>,
    pub columns: Vec<Col>,
}

/// The rows of a query so far.
#[derive(Clone, Debug)]
pub(crate) struct Rows {
    /// The plan, or `None` after an error, when the columns come from the syntax.
    pub plan: Option<LogicalPlan>,
    /// The visible columns, in order.
    pub columns: Vec<Col>,
    pub vars: Vec<RangeVar>,
    /// The keys of the order, expressions of columns of the plan.
    pub order: Vec<SortExpr>,
}

/// What a projection computes or carries.
pub(crate) enum Item {
    /// A column of the input, as it is.
    Carry(Col),
    /// An expression, as a new column of the name.
    Compute {
        name: String,
        expr: Expr,
        definition: Option<TextRange>,
    },
}

impl Rows {
    pub(crate) fn new(plan: LogicalPlan, columns: Vec<Col>, vars: Vec<RangeVar>) -> Self {
        Self {
            plan: Some(plan),
            columns,
            vars,
            order: Vec::new(),
        }
    }

    /// The rows after an error: the columns that the syntax gives, without a plan.
    pub(crate) fn failed(columns: Vec<Col>, vars: Vec<RangeVar>) -> Self {
        Self {
            plan: None,
            columns,
            vars,
            order: Vec::new(),
        }
    }

    /// The rows without their plan, after an error in an operator that keeps the columns.
    pub(crate) fn fail(self) -> Self {
        Self::failed(self.columns, self.vars)
    }

    pub(crate) fn schema(&self) -> Option<&DFSchema> {
        self.plan.as_ref().map(|plan| plan.schema().as_ref())
    }

    /// The type of the column `col`, when the rows have a plan.
    pub(crate) fn ty(&self, col: &Col) -> Option<DataType> {
        let schema = self.schema()?;
        let field = schema.field_from_column(&col.column).ok()?;
        Some(field.data_type().clone())
    }

    /// The rows without an order, for an operator that doesn't keep one.
    pub(crate) fn unordered(mut self) -> Self {
        self.order.clear();
        self
    }
}

/// The names of the qualifiers of an analysis, which no two plans share.
#[derive(Debug, Default)]
pub(crate) struct Qualifiers {
    used: HashSet<String>,
}

impl Qualifiers {
    /// A qualifier that no plan of the analysis has: `base`, or `base_2`, `base_3` and so on. The
    /// columns that an operator computes take the base `_`, as `_1`, `_2`.
    pub(crate) fn fresh(&mut self, base: &str) -> TableReference {
        let base = match base {
            "" | "_" => "_",
            base => base,
        };
        let mut n = 1;
        loop {
            let name = match (base, n) {
                ("_", n) => format!("_{n}"),
                (base, 1) => base.to_string(),
                (base, n) => format!("{base}_{n}"),
            };
            if self.used.insert(name.clone()) {
                return TableReference::bare(name);
            }
            n += 1;
        }
    }
}

/// The name of the column of a plan for the column `name` at `index` of its operator's output:
/// the name, or `$col1` and so on for a column without one.
pub(crate) fn plan_name(name: &str, index: usize) -> String {
    match name {
        "" => format!("$col{}", index + 1),
        name => name.to_string(),
    }
}

/// The rows of a projection of `items` over `rows`, which keeps the range variables when `vars`
/// holds and the order when `order` holds, carrying their columns as hidden columns. A
/// projection with window functions computes them in a window node first, and ends the order.
pub(crate) fn project(
    qualifiers: &mut Qualifiers,
    rows: Rows,
    items: Vec<Item>,
    vars: bool,
    order: bool,
) -> Result<Rows> {
    let Some(plan) = rows.plan else {
        let columns = items
            .into_iter()
            .map(|item| match item {
                Item::Carry(col) => col,
                Item::Compute {
                    name, definition, ..
                } => Col {
                    column: Column::from_name(name.clone()),
                    name,
                    definition,
                },
            })
            .collect();
        let vars = if vars { rows.vars } else { Vec::new() };
        return Ok(Rows::failed(columns, vars));
    };
    let mut qualifier = None;
    let mut exprs = Vec::new();
    let mut seen: HashSet<Column> = HashSet::new();
    let mut columns = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        match item {
            Item::Carry(col) => {
                if seen.insert(col.column.clone()) {
                    exprs.push(Expr::Column(col.column.clone()));
                }
                columns.push(col);
            }
            Item::Compute {
                name,
                expr,
                definition,
            } => {
                let field = plan_name(&name, index);
                let mut q = qualifier
                    .get_or_insert_with(|| qualifiers.fresh("_"))
                    .clone();
                while seen.contains(&Column::new(Some(q.clone()), &field)) {
                    q = qualifiers.fresh("_");
                }
                let column = Column::new(Some(q.clone()), &field);
                seen.insert(column.clone());
                exprs.push(expr.alias_qualified(Some(q), field));
                columns.push(Col {
                    name,
                    column,
                    definition,
                });
            }
        }
    }
    let windows = find_window_exprs(&exprs);
    let keep_order = order && windows.is_empty();
    let vars = if vars { rows.vars } else { Vec::new() };
    let keys = if keep_order { rows.order } else { Vec::new() };
    let hidden = (vars.iter().flat_map(|var| var.columns.iter()))
        .map(|col| col.column.clone())
        .chain(
            keys.iter()
                .flat_map(|key| key.expr.column_refs().into_iter().cloned()),
        );
    for column in hidden {
        if seen.insert(column.clone()) {
            exprs.push(Expr::Column(column));
        }
    }
    let plan = match windows.is_empty() {
        true => plan,
        false => LogicalPlanBuilder::window_plan(plan, windows.clone())?,
    };
    let exprs = (exprs.into_iter())
        .map(|expr| rebase(expr, &windows))
        .collect::<Result<Vec<_>>>()?;
    let plan = LogicalPlanBuilder::from(plan).project(exprs)?.build()?;
    Ok(Rows {
        plan: Some(plan),
        columns,
        vars,
        order: keys,
    })
}

/// `expr` with each of `bases`, the aggregates of an aggregate node or the windows of a window
/// node, written as the column of its output.
pub(crate) fn rebase(expr: Expr, bases: &[Expr]) -> Result<Expr> {
    if bases.is_empty() {
        return Ok(expr);
    }
    expr.transform_down(|e| match bases.contains(&e) {
        true => Ok(Transformed::yes(Expr::Column(Column::from_name(
            e.schema_name().to_string(),
        )))),
        false => Ok(Transformed::no(e)),
    })
    .map(|t| t.data)
}

/// The rows in the order of `keys`, sort expressions over the input. A key that isn't a column of
/// the plan becomes a hidden column. A key that calls a window function reads a window node.
pub(crate) fn order_by(
    qualifiers: &mut Qualifiers,
    rows: Rows,
    keys: Vec<SortExpr>,
) -> Result<Rows> {
    let Some(plan) = rows.plan else {
        return Ok(Rows::failed(rows.columns, rows.vars));
    };
    let windows = find_window_exprs(keys.iter().map(|key| &key.expr));
    let plan = match windows.is_empty() {
        true => plan,
        false => LogicalPlanBuilder::window_plan(plan, windows.clone())?,
    };
    let input: Vec<Column> = match windows.is_empty() {
        true => plan.schema().columns(),
        // The window node adds its columns, which the projection below drops.
        false => plan.schema().columns()[..plan.schema().fields().len() - windows.len()].to_vec(),
    };
    let mut exprs: Vec<Expr> = input.iter().cloned().map(Expr::Column).collect();
    let mut order = Vec::new();
    let mut qualifier = None;
    for key in keys {
        let expr = rebase(key.expr.clone(), &windows)?;
        match &expr {
            Expr::Column(column) if input.contains(column) => order.push(SortExpr { expr, ..key }),
            _ => {
                let q = qualifier
                    .get_or_insert_with(|| qualifiers.fresh("_"))
                    .clone();
                let name = format!("$key{}", order.len() + 1);
                exprs.push(expr.alias_qualified(Some(q.clone()), &name));
                order.push(SortExpr {
                    expr: Expr::Column(Column::new(Some(q), name)),
                    ..key
                });
            }
        }
    }
    let plan = match exprs.len() == input.len() && windows.is_empty() {
        true => plan,
        false => LogicalPlanBuilder::from(plan).project(exprs)?.build()?,
    };
    Ok(Rows {
        plan: Some(plan),
        columns: rows.columns,
        vars: rows.vars,
        order,
    })
}

/// The first `fetch` rows after `skip`, in the order when the rows have one, which they keep.
pub(crate) fn limit(rows: Rows, skip: Option<Expr>, fetch: Option<Expr>) -> Result<Rows> {
    let Some(plan) = rows.plan else {
        return Ok(Rows::failed(rows.columns, rows.vars));
    };
    let mut builder = LogicalPlanBuilder::from(plan);
    if !rows.order.is_empty() {
        builder = builder.sort(rows.order.clone())?;
    }
    let plan = builder.limit_by_expr(skip, fetch)?.build()?;
    Ok(Rows {
        plan: Some(plan),
        ..rows
    })
}

/// The plan of the result of `rows`: sorted when it has an order, with its visible columns under
/// their names. Columns of one name each take a qualifier of their own, which DataFusion needs to
/// tell them apart.
pub(crate) fn finish(qualifiers: &mut Qualifiers, rows: Rows) -> Result<Option<LogicalPlan>> {
    let Some(plan) = rows.plan else {
        return Ok(None);
    };
    let mut builder = LogicalPlanBuilder::from(plan);
    if !rows.order.is_empty() {
        builder = builder.sort(rows.order)?;
    }
    let names: Vec<String> = (rows.columns.iter().enumerate())
        .map(|(index, col)| plan_name(&col.name, index))
        .collect();
    let exprs: Vec<Expr> = (rows.columns.iter().zip(&names))
        .map(|(col, name)| {
            let expr = Expr::Column(col.column.clone());
            match names.iter().filter(|other| *other == name).count() {
                1 => expr.alias(name),
                _ => expr.alias_qualified(Some(qualifiers.fresh("_")), name),
            }
        })
        .collect();
    Ok(Some(builder.project(exprs)?.build()?))
}
