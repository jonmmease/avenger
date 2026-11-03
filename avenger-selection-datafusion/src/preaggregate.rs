//! Mosaic-style pre-aggregation: each aggregate of a query that a moving selection filters,
//! rewritten into a **rollup** over a **cells subplan**. The cells subplan aggregates the rows into
//! states by the query's groups and a key for each dimension of the moving selection, and skips
//! the moving owner's clauses. The rollup keeps the keys that the moving clauses cover and merges
//! their states. The rewrite is exact because the moving selection's membership depends only on
//! the keys: the cell of a gridded range or a lasso, or the values of a set's columns.
//!
//! The rewritten query reads the moving clauses only above each cells subplan, so a dataflow that
//! keeps the result of each cells subplan, keyed by the subplan, runs only the rollups when the
//! selection moves. [`selection_reads`](crate::selection_reads) tells such a dataflow where each
//! part of the query stops reading the moving clauses.

use std::{collections::BTreeSet, sync::Arc};

use avenger_datafusion_aggregate_state::{merge, state, Family};
use avenger_selection::{Condition, PixelGrid, Resolution, SelectionState};
use datafusion::{
    common::{
        plan_err,
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, Result, ScalarValue,
    },
    logical_expr::{
        expr::{AggregateFunction, AggregateFunctionParams, ScalarFunction},
        lit,
        utils::split_conjunction_owned,
        Aggregate, Expr, ExprFunctionExt, ExprSchemable, Filter, LogicalPlan, LogicalPlanBuilder,
        Projection, ScalarUDF, Volatility,
    },
};

use crate::{
    functions::{InKeys, PixelCell, SELECTED_PARAMETERS},
    lower::{fold_options, is_selected, Options},
};

/// The selection that is about to move: its parameter in the query, the owner whose clause
/// moves, and each dimension of the owner's selection tool.
#[derive(Clone, Debug)]
pub struct Focus {
    /// The id of the placeholder that holds the selection's state, such as `$brush`.
    pub parameter: String,
    /// The owner whose clause moves.
    pub owner: String,
    pub dimensions: Vec<FocusDimension>,
}

/// A dimension of a focused selection tool: a column that its conditions check, and the grid of
/// its cells for a gridded range, or `None` for a column of a set.
#[derive(Clone, Debug)]
pub struct FocusDimension {
    pub column: String,
    pub grid: Option<PixelGrid>,
}

/// Why a site runs its direct plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectReason {
    /// No aggregate reads the filter through operators of one row.
    NoAggregate,
    /// An operator between the filter and the aggregate is not a projection, a filter, or an
    /// alias, or an expression there is volatile or reads the moving selection again.
    UnsupportedOperator,
    /// An aggregate has no aggregate-state family, or has `DISTINCT`, an aggregate `ORDER BY`, a
    /// null treatment, or grouping sets.
    UnsupportedAggregate,
    /// The call selects no row for an empty selection, `empty => 'none'`.
    EmptySelectsNone,
    /// A focused column is not a column of the aggregate's input, or a projection changes it.
    MissingKey,
    /// The filtered rows belong to the faceted owner of the moving selection, which skips only
    /// its own panel's clauses.
    FacetedOwner,
    /// The aggregate is in the input of another pre-aggregated aggregate, whose cells subplan
    /// reads it as written.
    Nested,
}

/// A query prepared for a moving selection: the query with each pre-aggregated aggregate replaced
/// by its rollup over its cells subplan, and the strategy of each `selected` call on the focused
/// selection.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub plan: LogicalPlan,
    pub strategies: Vec<Strategy>,
}

/// What a query does for one `selected` call on the focused selection.
#[derive(Clone, Debug)]
pub enum Strategy {
    /// The aggregate above the call is a rollup over its cells subplan.
    Preaggregated(Preaggregation),
    /// The call skips the moving owner's clauses, so a move does not change its rows.
    Unaffected,
    /// The query runs as written, and each move filters the rows again.
    Direct(DirectReason),
}

/// The upper bound of the rows of a cells table for each group of the query's groups: the product
/// of the cell counts of the gridded keys over the range of their scales, times the distinct values
/// of `value_keys`, which the caller estimates.
#[derive(Clone, Debug, PartialEq)]
pub struct SizeBound {
    pub cells_per_group: u64,
    pub value_keys: Vec<String>,
}

/// The pre-aggregation of one aggregate: the guard of the states that its rollup answers, and the
/// bound of its cells.
#[derive(Clone, Debug)]
pub struct Preaggregation {
    focus: Focus,
}

impl Preaggregation {
    /// Whether the rollup serves the selection's state `state`: the moving owner's clauses check
    /// only the focused columns, a gridded range with the focus's grid, a polygon with cells with
    /// the focus's grids of both of its columns, and a set with value keys, and the selection
    /// resolves by intersection when the owner has clauses.
    pub fn serves(&self, state: &SelectionState) -> bool {
        let mut moving = (state.clauses.iter())
            .filter(|clause| clause.owner == self.focus.owner)
            .peekable();
        if moving.peek().is_none() {
            return true;
        }
        if state.resolution != Resolution::Intersect {
            return false;
        }
        let dimension = |column: &str| (self.focus.dimensions.iter()).find(|d| d.column == column);
        let gridded = |column: &str, grid: &PixelGrid| {
            dimension(column)
                .and_then(|dimension| dimension.grid.as_ref())
                .is_some_and(|focused| grid.same(focused))
        };
        moving
            .flat_map(|clause| clause.conditions.iter())
            .all(|condition| match condition {
                Condition::Range {
                    column,
                    cells: Some(cells),
                    ..
                } => gridded(column, &cells.grid),
                Condition::Polygon {
                    x_column,
                    y_column,
                    cells: Some(cells),
                    ..
                } => gridded(x_column, &cells.grids[0]) && gridded(y_column, &cells.grids[1]),
                Condition::Set { columns, .. } => columns.iter().all(|column| {
                    dimension(column).is_some_and(|dimension| dimension.grid.is_none())
                }),
                _ => false,
            })
    }

    /// The bound of the cells table's rows for each group.
    pub fn size(&self) -> SizeBound {
        let mut cells_per_group: u64 = 1;
        let mut value_keys = Vec::new();
        for dimension in &self.focus.dimensions {
            match &dimension.grid {
                Some(grid) => {
                    let cells = (grid.scale.numeric_interval_range())
                        .map(|(a, b)| ((a - b).abs() / grid.cell).ceil() as u64)
                        .unwrap_or(1);
                    cells_per_group = cells_per_group.saturating_mul(cells.max(1));
                }
                None => value_keys.push(dimension.column.clone()),
            }
        }
        SizeBound {
            cells_per_group,
            value_keys,
        }
    }
}

/// The rollup over `cells`, the cells subplan of `aggregate` with `groups` of its groups first, the
/// focused keys, and a state of each of `families`: the rows whose keys the moving clauses of the
/// selection `parameter` cover, merged by the groups, with the aggregate's schema.
fn rollup(
    aggregate: &Aggregate,
    cells: LogicalPlan,
    groups: usize,
    families: &[Family],
    parameter: Expr,
    focus: &Focus,
) -> Result<LogicalPlan> {
    let schema = cells.schema().clone();
    let column = |index: usize| Expr::Column(Column::from(schema.qualified_field(index)));
    let keys = focus.dimensions.len();
    let mut fields = Vec::new();
    for (index, dimension) in focus.dimensions.iter().enumerate() {
        fields.push(lit(dimension.column.as_str()));
        fields.push(column(groups + index));
    }
    let in_keys = Arc::new(ScalarUDF::from(InKeys::new())).call(vec![
        parameter,
        lit(focus.owner.as_str()),
        datafusion::functions::core::expr_fn::named_struct(fields),
    ]);
    let group_exprs: Vec<Expr> = (0..groups).map(column).collect();
    let merges: Vec<Expr> = (families.iter().enumerate())
        .map(|(index, family)| {
            merge(*family, column(groups + keys + index)).alias(format!("__merge_{index}"))
        })
        .collect();
    let merged = LogicalPlanBuilder::from(cells)
        .filter(in_keys)?
        .aggregate(group_exprs, merges)?
        .build()?;
    let outputs = (0..groups + families.len())
        .map(|index| Expr::Column(Column::from(merged.schema().qualified_field(index))));
    let mut exprs = Vec::new();
    for (expr, (qualifier, field)) in outputs.zip(aggregate.schema.iter()) {
        if &expr.get_type(merged.schema())? != field.data_type() {
            return plan_err!(
                "the rollup's {} is not of the type {}",
                field.name(),
                field.data_type()
            );
        }
        exprs.push(expr.alias_qualified(qualifier.cloned(), field.name()));
    }
    Ok(LogicalPlan::Projection(Projection::try_new_with_schema(
        exprs,
        Arc::new(merged),
        aggregate.schema.clone(),
    )?))
}

/// `plan`, an unoptimized plan whose selection state is the placeholder of the focus, prepared
/// for the focused selection's moves: each aggregate that a `selected` call on the selection
/// qualifies for is replaced by its rollup over its cells subplan, and each call on the selection
/// has a strategy. Bind the prepared plan's selections before optimizing it, and cut the plan at
/// each cells subplan before the optimizer moves the rollup's filter into it.
pub fn prepare(plan: &LogicalPlan, focus: &Focus) -> Result<Prepared> {
    let mut sites: Vec<LogicalPlan> = Vec::new();
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::Filter(filter) = node {
            if moving_call(&filter.predicate, focus).is_some() {
                sites.push(node.clone());
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    let mut strategies = Vec::new();
    let mut rollups: Vec<(LogicalPlan, LogicalPlan, usize)> = Vec::new();
    let mut reached = Vec::new();
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::Aggregate(aggregate) = node {
            if let Some((filter, strategy, replaced)) = analyze(aggregate, focus)? {
                if let Some(rollup) = replaced {
                    rollups.push((node.clone(), rollup, strategies.len()));
                }
                reached.push(filter);
                strategies.push(strategy);
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    for site in sites {
        if !reached.contains(&site) {
            strategies.push(match unaffected(&site, focus)? {
                true => Strategy::Unaffected,
                false => Strategy::Direct(DirectReason::NoAggregate),
            });
        }
    }
    // Replace each aggregate from the top, so an aggregate in the input of another one's cells
    // subplan keeps its direct plan.
    let mut applied = vec![false; rollups.len()];
    let plan = plan.clone().transform_down_with_subqueries(|node| {
        let mut replacement = None;
        for (index, (aggregate, rollup, _)) in rollups.iter().enumerate() {
            if aggregate == &node {
                applied[index] = true;
                replacement.get_or_insert_with(|| rollup.clone());
            }
        }
        Ok(match replacement {
            Some(rollup) => Transformed::new(rollup, true, TreeNodeRecursion::Jump),
            None => Transformed::no(node),
        })
    })?;
    for ((_, _, strategy), applied) in rollups.iter().zip(applied) {
        if !applied {
            strategies[*strategy] = Strategy::Direct(DirectReason::Nested);
        }
    }
    Ok(Prepared {
        plan: plan.data,
        strategies,
    })
}

/// The arguments of the `selected` call on the focused selection in `predicate`, if any.
fn moving_call<'a>(predicate: &'a Expr, focus: &Focus) -> Option<&'a [Expr]> {
    let mut found = None;
    predicate
        .apply(|expr| {
            if let Expr::ScalarFunction(ScalarFunction { func, args }) = expr {
                if is_selected(func)
                    && matches!(args.first(), Some(Expr::Placeholder(p)) if p.id == focus.parameter)
                {
                    found = Some(args.as_slice());
                    return Ok(TreeNodeRecursion::Stop);
                }
            }
            Ok(TreeNodeRecursion::Continue)
        })
        .expect("the walk does not fail");
    found
}

/// Whether `expr` reads the focus's placeholder.
fn reads_focus(expr: &Expr, focus: &Focus) -> bool {
    let mut reads = false;
    expr.apply(|expr| {
        if matches!(expr, Expr::Placeholder(p) if p.id == focus.parameter) {
            reads = true;
            return Ok(TreeNodeRecursion::Stop);
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .expect("the walk does not fail");
    reads
}

/// Whether `expr` is the same for every evaluation.
fn immutable(expr: &Expr) -> bool {
    let mut immutable = true;
    expr.apply(|expr| {
        let volatile = match expr {
            Expr::ScalarFunction(ScalarFunction { func, .. }) => {
                func.signature().volatility != Volatility::Immutable
            }
            Expr::ScalarSubquery(_) | Expr::Exists(_) | Expr::InSubquery(_) => true,
            Expr::WindowFunction(_) | Expr::AggregateFunction(_) => true,
            _ => false,
        };
        if volatile {
            immutable = false;
            return Ok(TreeNodeRecursion::Stop);
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .expect("the walk does not fail");
    immutable
}

/// The options of the `selected` call of the site `filter`.
fn site_options(filter: &Filter, focus: &Focus) -> Result<Options> {
    let args = moving_call(&filter.predicate, focus).expect("a site has a call");
    Options::parse(&fold_options(args, filter.input.schema())?)
}

/// Whether the call of the site `site` skips the moving owner's clauses: it excludes its own
/// owner, the moving owner, which is not faceted, or it lists the owner in `without`.
fn unaffected(site: &LogicalPlan, focus: &Focus) -> Result<bool> {
    let LogicalPlan::Filter(filter) = site else {
        return Ok(false);
    };
    let options = site_options(filter, focus)?.selected;
    let own = options.exclude_own.contains(&focus.owner) && !options.faceted;
    Ok(own || options.without.contains(&focus.owner))
}

/// The aggregate-state family of an aggregate function's name.
fn family(name: &str, aliases: &[String]) -> Option<Family> {
    (Family::ALL.into_iter())
        .find(|family| name == family.name() || aliases.iter().any(|a| a == family.name()))
}

/// The site under `aggregate`, its strategy, and the rollup that replaces the aggregate, or `None`
/// when the aggregate reads no site through operators of one row.
fn analyze(
    aggregate: &Aggregate,
    focus: &Focus,
) -> Result<Option<(LogicalPlan, Strategy, Option<LogicalPlan>)>> {
    // Walk down from the aggregate through operators of one row to the site.
    let mut between: Vec<&LogicalPlan> = Vec::new();
    let mut input = aggregate.input.as_ref();
    let filter = loop {
        match input {
            LogicalPlan::Filter(filter) if moving_call(&filter.predicate, focus).is_some() => {
                break filter;
            }
            LogicalPlan::Filter(Filter { input: next, .. })
            | LogicalPlan::Projection(Projection { input: next, .. })
            | LogicalPlan::SubqueryAlias(datafusion::logical_expr::SubqueryAlias {
                input: next,
                ..
            }) => {
                between.push(input);
                input = next.as_ref();
            }
            _ => return Ok(None),
        }
    };
    let site = LogicalPlan::Filter(filter.clone());
    let direct = |reason| Ok(Some((site.clone(), Strategy::Direct(reason), None)));
    let options = site_options(filter, focus)?.selected;
    if options.exclude_own.contains(&focus.owner) {
        return Ok(Some((
            site.clone(),
            if options.faceted {
                Strategy::Direct(DirectReason::FacetedOwner)
            } else {
                Strategy::Unaffected
            },
            None,
        )));
    }
    if options.without.contains(&focus.owner) {
        return Ok(Some((site, Strategy::Unaffected, None)));
    }
    if options.empty_selects_none {
        return direct(DirectReason::EmptySelectsNone);
    }

    // Only the site's call reads the moving selection, and the operators between are immutable.
    let others = split_conjunction_owned(filter.predicate.clone());
    if others
        .iter()
        .filter(|part| reads_focus(part, focus))
        .count()
        != 1
        || others
            .iter()
            .any(|part| reads_focus(part, focus) && moving_call(part, focus).is_none())
    {
        return direct(DirectReason::UnsupportedOperator);
    }
    if others.iter().any(|part| !immutable(part)) {
        return direct(DirectReason::UnsupportedOperator);
    }
    for operator in &between {
        if operator
            .expressions()
            .iter()
            .any(|expr| !immutable(expr) || reads_focus(expr, focus))
        {
            return direct(DirectReason::UnsupportedOperator);
        }
    }

    // Each aggregate has a family, and the groups are plain.
    if aggregate
        .group_expr
        .iter()
        .any(|expr| matches!(expr, Expr::GroupingSet(_)) || reads_focus(expr, focus))
    {
        return direct(DirectReason::UnsupportedAggregate);
    }
    let mut families = Vec::new();
    let mut states = Vec::new();
    for (index, expr) in aggregate.aggr_expr.iter().enumerate() {
        let inner = match expr {
            Expr::Alias(alias) => alias.expr.as_ref(),
            other => other,
        };
        let Expr::AggregateFunction(AggregateFunction {
            func,
            params:
                AggregateFunctionParams {
                    args,
                    distinct,
                    filter: condition,
                    order_by,
                    null_treatment,
                },
        }) = inner
        else {
            return direct(DirectReason::UnsupportedAggregate);
        };
        let (Some(family), [arg]) = (family(func.name(), func.aliases()), args.as_slice()) else {
            return direct(DirectReason::UnsupportedAggregate);
        };
        if *distinct
            || !order_by.is_empty()
            || null_treatment.is_some()
            || reads_focus(inner, focus)
        {
            return direct(DirectReason::UnsupportedAggregate);
        }
        let mut stated = state(family, arg.clone());
        if let Some(condition) = condition {
            stated = stated.filter(condition.as_ref().clone()).build()?;
        }
        families.push(family);
        states.push(stated.alias(format!("__state_{index}")));
    }

    // The keys: each focused column, unchanged from the site to the aggregate.
    let input_schema = aggregate.input.schema();
    let mut keys = Vec::new();
    for (index, dimension) in focus.dimensions.iter().enumerate() {
        let Ok((qualifier, field)) =
            input_schema.qualified_field_with_unqualified_name(&dimension.column)
        else {
            return direct(DirectReason::MissingKey);
        };
        let passes = between.iter().all(|operator| match operator {
            LogicalPlan::Projection(projection) => projection.expr.iter().any(|expr| {
                let expr = match expr {
                    Expr::Alias(alias) if alias.name == dimension.column => alias.expr.as_ref(),
                    other => other,
                };
                matches!(expr, Expr::Column(column) if column.name == dimension.column)
            }),
            _ => true,
        });
        if !passes {
            return direct(DirectReason::MissingKey);
        }
        let column = Expr::Column(Column::new(qualifier.cloned(), field.name()));
        let key = match &dimension.grid {
            Some(grid) => ScalarUDF::from(PixelCell::new(grid.clone())).call(vec![column]),
            None => column,
        };
        keys.push(key.alias(format!("__key_{index}")));
    }

    // The cells plan: the site's call skips the moving owner, and the aggregate groups by the keys
    // as well and keeps states.
    let mut args = moving_call(&filter.predicate, focus)
        .expect("a site has a call")
        .to_vec();
    let skipped: BTreeSet<String> = (options.without.iter().cloned())
        .chain([focus.owner.clone()])
        .collect();
    args.resize(SELECTED_PARAMETERS.len(), lit(ScalarValue::Null));
    let owners: Vec<ScalarValue> = skipped
        .into_iter()
        .map(|o| ScalarValue::Utf8(Some(o)))
        .collect();
    args[5] = lit(ScalarValue::List(ScalarValue::new_list_nullable(
        &owners,
        &datafusion::arrow::datatypes::DataType::Utf8,
    )));
    let parameter = args[0].clone();
    let predicate = filter.predicate.clone().transform(|expr| match &expr {
        Expr::ScalarFunction(ScalarFunction { func, args: old })
            if is_selected(func)
                && matches!(old.first(), Some(Expr::Placeholder(p)) if p.id == focus.parameter) =>
        {
            Ok(Transformed::yes(Expr::ScalarFunction(ScalarFunction {
                func: func.clone(),
                args: args.clone(),
            })))
        }
        _ => Ok(Transformed::no(expr)),
    })?;
    let fixed = LogicalPlan::Filter(Filter::try_new(predicate.data, filter.input.clone())?);
    let input = aggregate
        .input
        .as_ref()
        .clone()
        .transform_down(|node| {
            if node == site {
                Ok(Transformed::new(
                    fixed.clone(),
                    true,
                    TreeNodeRecursion::Jump,
                ))
            } else {
                Ok(Transformed::no(node))
            }
        })?
        .data;
    let groups: Vec<Expr> = (aggregate.group_expr.iter().cloned()).chain(keys).collect();
    let cells = LogicalPlanBuilder::from(input)
        .aggregate(groups, states)?
        .build()?;

    // The rollup must have the aggregate's types.
    let groups = aggregate.group_expr.len();
    let Ok(rollup) = rollup(aggregate, cells, groups, &families, parameter, focus) else {
        return direct(DirectReason::UnsupportedAggregate);
    };
    let preaggregation = Preaggregation {
        focus: focus.clone(),
    };
    Ok(Some((
        site,
        Strategy::Preaggregated(preaggregation),
        Some(rollup),
    )))
}
