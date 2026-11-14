//! The split of a query over the built-in `events` table for a streaming runtime: for each read
//! of `events`, the stateless stages over it and the front that reduces their rows, and the
//! downstream that reads the fronts' outputs.
//!
//! A streaming runtime runs each frame's events through each read's stages once and applies the
//! rows that they keep to its front, whose output is a small table that the runtime maintains.
//! The downstream is the rest of the query as a batch plan over those tables, which reads each
//! through a [`Slot`], so a join or a union of several fronts is downstream work. After every
//! frame, the result equals the batch plan of the whole query over every event so far. The fronts
//! take rows that only grow, as `events` does, so none of them needs a retraction:
//!
//! - an `AGGREGATE`, grouped or not, whose accumulators only add rows;
//! - an `ORDER BY ... LIMIT`, whose kept rows only leave when a new row takes their place;
//! - a `MATCH_RECOGNIZE` ordered by `seq`, whose matches before its settled frontier are final.
//!
//! [`split`] rejects a query of another shape, whose caller runs the batch plan.

use std::{fmt, sync::Arc};

use datafusion::{
    arrow::{
        array::{ArrayRef, RecordBatch},
        datatypes::{Field, Schema, SchemaRef},
    },
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, DFSchemaRef, ScalarValue,
    },
    datasource::{provider_as_source, source_as_provider, MemTable, TableProvider},
    logical_expr::{Expr, Limit, LogicalPlan, LogicalPlanBuilder, SortExpr, TableScan, Volatility},
};

/// A query over `events`, split for a streaming runtime.
#[derive(Debug, Clone)]
pub struct Split {
    /// A front for each read of `events`, in the order of the plan.
    pub fronts: Vec<FrontSplit>,
    /// The rest of the query, which reads each front's output through its slot, and joins or
    /// unions the outputs of several fronts.
    pub downstream: LogicalPlan,
    /// The downstream as the latest rows of the front's output, for a query of one front whose
    /// output keeps its final rows, such as `MATCH_RECOGNIZE`.
    pub latest: Option<Latest>,
}

/// A read of `events`: the stages over its scan, the front over the stages, and the slot of the
/// front's output.
#[derive(Debug, Clone)]
pub struct FrontSplit {
    /// The stages: the plan of the rows that reach the front, over the table `events`.
    pub stages: LogicalPlan,
    /// The scan of `events` in `stages`, which a runtime replaces with a scan of each frame.
    pub scan: TableScan,
    pub front: Front,
    /// The table of the front's output, which the downstream reads.
    pub slot: Slot,
}

/// A downstream that starts with row-wise stages and an `ORDER BY ... LIMIT` over the front's
/// output. Over an output of final rows, which only accumulate, and open rows, the limit's rows
/// are the limit's rows of the first `skip + fetch` final rows in its order and the open rows, so
/// a runtime keeps those final rows instead of reading every final row on each frame.
#[derive(Debug, Clone)]
pub struct Latest {
    /// The downstream's stages, which read the front's output through the split's slot.
    pub stages: LogicalPlan,
    /// The first `skip + fetch` rows of the stages' rows that `candidates` holds, in the order.
    pub top: LogicalPlan,
    pub candidates: Slot,
    pub skip: usize,
    /// The rest of the downstream, over the limit's rows that `limited` holds.
    pub rest: LogicalPlan,
    pub limited: Slot,
}

/// The front of a streaming query, over the rows of the stages.
#[derive(Debug, Clone)]
pub enum Front {
    /// An `AGGREGATE`: its output is the group values, then the aggregates, of each group, and
    /// one row without groups, also over no rows.
    Aggregate {
        groups: Vec<Expr>,
        aggregates: Vec<Expr>,
    },
    /// An `ORDER BY ... LIMIT`: its output is the rows from `skip` to `skip + fetch` in the order
    /// of `order`, whose last key is `seq`, so the order has no ties.
    Limit {
        order: Vec<SortExpr>,
        skip: usize,
        fetch: usize,
    },
    /// A `MATCH_RECOGNIZE` ordered by `seq`: its output is a row of measures for each match.
    MatchRecognize(Box<MatchFront>),
}

/// The pieces of a front of `MATCH_RECOGNIZE`, which a runtime runs on the rows that each needs.
#[derive(Debug, Clone)]
pub struct MatchFront {
    /// The rows' positions and conditions: the window projection under the clause's node, over
    /// the stages' rows that `staged` holds.
    pub conditions: LogicalPlan,
    pub staged: Slot,
    /// The measures: the plan over the node's rows that `node_rows` holds, one for each row of
    /// each match and one for each empty match.
    pub measures: LogicalPlan,
    pub node_rows: Slot,
    pub call: avenger_datafusion_transforms::match_recognize::MatchRecognizeCall,
    /// The partition expressions, over the stages' rows.
    pub partition: Vec<Expr>,
    /// The column `seq` of the stages' rows, the clause's order.
    pub seq: Column,
    /// The most rows that a condition reads before its row, by `PREV`, and after it, by `NEXT`.
    pub back: usize,
    pub ahead: usize,
}

/// Why a query doesn't split, and runs as a batch plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub message: String,
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn reject<T>(message: impl Into<String>) -> Result<T, Rejection> {
    Err(Rejection {
        message: message.into(),
    })
}

/// A table that a streaming runtime fills, and the plan that reads it under the names and
/// qualifiers of the output that it replaces.
#[derive(Debug, Clone)]
pub struct Slot {
    table: Arc<MemTable>,
    schema: SchemaRef,
    pub plan: LogicalPlan,
}

impl Slot {
    /// The slot of the output `schema`, as the table `name`.
    pub fn new(name: &str, schema: &DFSchemaRef) -> Slot {
        let fields: Vec<Field> = (schema.fields().iter().enumerate())
            .map(|(i, f)| Field::new(format!("c{i}"), f.data_type().clone(), true))
            .collect();
        let arrow = Arc::new(Schema::new(fields));
        let table = Arc::new(MemTable::try_new(arrow.clone(), vec![vec![]]).expect("a table"));
        let exprs: Vec<Expr> = (schema.iter().enumerate())
            .map(|(i, (qualifier, field))| {
                let column = Expr::Column(Column::new(Some(name), format!("c{i}")));
                column.alias_qualified(qualifier.cloned(), field.name())
            })
            .collect();
        let plan = LogicalPlanBuilder::scan(name, provider_as_source(table.clone()), None)
            .and_then(|b| b.project(exprs))
            .and_then(|b| b.build())
            .expect("the plan of a slot");
        Slot {
            table,
            schema: arrow,
            plan,
        }
    }

    /// Replaces the rows with none.
    pub fn set_empty(&self) {
        let mut batches = self.table.batches[0]
            .try_write()
            .expect("no plan reads a slot while the runtime fills it");
        *batches = Vec::new();
    }

    /// Replaces the rows with `columns`, a batch of the replaced output's columns in order.
    pub fn set(&self, columns: Vec<ArrayRef>) {
        let batch = RecordBatch::try_new(self.schema.clone(), columns).expect("the slot's columns");
        let mut batches = self.table.batches[0]
            .try_write()
            .expect("no plan reads a slot while the runtime fills it");
        *batches = vec![batch];
    }
}

/// The split of `plan`, the plan of a query, whose catalog's table of events is `events`, or why
/// it doesn't split. Each read of `events` has a front of its own, and the downstream reads the
/// fronts' outputs.
pub fn split(plan: &LogicalPlan, events: &Arc<dyn TableProvider>) -> Result<Split, Rejection> {
    let count = |subqueries: bool| {
        let mut scans = 0;
        let visit = |node: &LogicalPlan| {
            if reads(node, events) {
                scans += 1;
            }
            Ok(TreeNodeRecursion::Continue)
        };
        match subqueries {
            true => plan.apply_with_subqueries(visit),
            false => plan.apply(visit),
        }
        .expect("a walk of the plan");
        scans
    };
    let scans = count(false);
    if scans == 0 {
        return reject("the query doesn't read events");
    }
    // A subquery of an expression that reads events has no front that the runtime evaluates.
    if count(true) > scans {
        return reject("a subquery of an expression reads events");
    }
    let mut paths = Vec::new();
    paths_to_events(plan, events, &mut Vec::new(), &mut paths);
    // The analyzer names each scan of a table on its own, so the output nodes of two fronts
    // differ. Two equal ones would both read the first front's slot, which holds the same rows.
    let mut fronts = Vec::new();
    let mut replaced = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let name = match index {
            0 => "front".to_string(),
            n => format!("front_{n}"),
        };
        let (front, node) = front_of(path, &name)?;
        fronts.push(front);
        replaced.push(node);
    }
    // A function whose value changes as time passes, anywhere: a stream would read it at other
    // times than the batch plan does, and an optimized plan holds its value as a constant.
    if let Some(name) = time_function(plan) {
        return reject(format!(
            "the query reads the function {name}, whose value changes as time passes"
        ));
    }
    let mut downstream = plan.clone();
    for (front, node) in fronts.iter().zip(&replaced) {
        downstream = replace(&downstream, node, &front.slot.plan);
    }
    let latest = match fronts.as_slice() {
        [only] if matches!(only.front, Front::MatchRecognize(_)) => latest(&downstream, &only.slot),
        _ => None,
    };
    Ok(Split {
        fronts,
        downstream,
        latest,
    })
}

/// The front of the read of `events` at the end of `path`, the nodes from the plan's root down to
/// the scan, and the node of the front's output, which the slot of the table `name` replaces.
fn front_of(path: &[LogicalPlan], name: &str) -> Result<(FrontSplit, LogicalPlan), Rejection> {
    let LogicalPlan::TableScan(scan) = path.last().expect("the scan") else {
        unreachable!("the path ends at the scan");
    };
    // The stages: the stateless operators right above the scan.
    let mut top = path.len() - 1;
    while top > 0 && stateless(&path[top - 1])? {
        top -= 1;
    }
    if top == 0 {
        return reject("the query has no front over events, so its result grows with every event");
    }
    let stages = &path[top];
    if let Some((front, measures)) = match_recognize(path, top)? {
        let front = Front::MatchRecognize(Box::new(front));
        return Ok(front_split(name, stages, scan, front, &path[measures]));
    }
    // Another table function over the stages, below any aggregate or limit.
    let reduced = (0..top)
        .rev()
        .find(|&i| matches!(path[i], LogicalPlan::Aggregate(_) | LogicalPlan::Limit(_)));
    let lowest = reduced.map_or(0, |i| i + 1);
    for node in path[lowest..top].iter().rev() {
        if let LogicalPlan::Extension(extension) = node {
            return reject(format!(
                "the table function {} over events isn't a streaming front",
                extension.node.name()
            ));
        }
    }
    let node = &path[top - 1];
    if has_subquery(node) {
        return reject("a subquery in the front over events, which the runtime doesn't evaluate");
    }
    let (front, replaced) = match node {
        LogicalPlan::Aggregate(aggregate) => {
            if (aggregate.group_expr.iter()).any(|e| matches!(e, Expr::GroupingSet(_))) {
                return reject("an AGGREGATE over events with ROLLUP, CUBE or GROUPING SETS");
            }
            let front = Front::Aggregate {
                groups: aggregate.group_expr.clone(),
                aggregates: aggregate.aggr_expr.clone(),
            };
            (front, node)
        }
        LogicalPlan::Sort(sort) => {
            let Some(LogicalPlan::Limit(limit)) = top.checked_sub(2).map(|i| &path[i]) else {
                return reject(
                    "an ORDER BY over events without a LIMIT, whose result grows with every event",
                );
            };
            let (skip, fetch) = limits(limit)?;
            let last_is_seq = (sort.expr.last()).is_some_and(|key| is_seq(&key.expr, stages));
            if !last_is_seq {
                return reject("an ORDER BY ... LIMIT over events whose last key isn't seq, so ties have no order of their own");
            }
            let front = Front::Limit {
                order: sort.expr.clone(),
                skip,
                fetch,
            };
            (front, &path[top - 2])
        }
        LogicalPlan::Window(_) => return reject(
            "a window over events, which reads every row, as a window function or HAVING MAX does",
        ),
        LogicalPlan::Join(_) => return reject("a join over events"),
        LogicalPlan::Union(_) => return reject("a set operation over events"),
        other => {
            return reject(format!(
                "the operator {} over events isn't a streaming front",
                other.display()
            ))
        }
    };
    Ok(front_split(name, stages, scan, front, replaced))
}

/// The front `front` over `stages`, whose output is the node `replaced`, and that node.
fn front_split(
    name: &str,
    stages: &LogicalPlan,
    scan: &TableScan,
    front: Front,
    replaced: &LogicalPlan,
) -> (FrontSplit, LogicalPlan) {
    let split = FrontSplit {
        stages: stages.clone(),
        scan: scan.clone(),
        front,
        slot: Slot::new(name, replaced.schema()),
    };
    (split, replaced.clone())
}

/// The downstream as the latest rows of the front's output, when it starts with row-wise stages
/// over `slot` and an `ORDER BY ... LIMIT` of literals.
fn latest(downstream: &LogicalPlan, slot: &Slot) -> Option<Latest> {
    let path = path_to(downstream, &slot.plan)?;
    let mut at = path.len() - 1;
    while at > 0 && stateless(&path[at - 1]).unwrap_or(false) {
        at -= 1;
    }
    let (LogicalPlan::Sort(sort), Some(LogicalPlan::Limit(limit))) = (
        path.get(at.checked_sub(1)?)?,
        at.checked_sub(2).map(|i| &path[i]),
    ) else {
        return None;
    };
    let (skip, fetch) = limits(limit).ok()?;
    let stages = path[at].clone();
    let candidates = Slot::new("candidates", stages.schema());
    let top = LogicalPlanBuilder::from(candidates.plan.clone())
        .sort(sort.expr.clone())
        .and_then(|b| b.limit(0, Some(skip + fetch)))
        .and_then(|b| b.build())
        .ok()?;
    let limit_node = &path[at - 2];
    let limited = Slot::new("limited", limit_node.schema());
    Some(Latest {
        rest: replace(downstream, limit_node, &limited.plan),
        stages,
        top,
        candidates,
        skip,
        limited,
    })
}

/// The nodes from `plan` down to the node `target`, or `None` without one.
fn path_to(plan: &LogicalPlan, target: &LogicalPlan) -> Option<Vec<LogicalPlan>> {
    if plan == target {
        return Some(vec![plan.clone()]);
    }
    for input in plan.inputs() {
        if let Some(mut path) = path_to(input, target) {
            path.insert(0, plan.clone());
            return Some(path);
        }
    }
    None
}

/// `plan` with the node `target` replaced by `with`.
fn replace(plan: &LogicalPlan, target: &LogicalPlan, with: &LogicalPlan) -> LogicalPlan {
    plan.clone()
        .transform_down(|p| match &p == target {
            true => Ok(Transformed::new(
                with.clone(),
                true,
                TreeNodeRecursion::Jump,
            )),
            false => Ok(Transformed::no(p)),
        })
        .expect("a replacement")
        .data
}

/// The front of a `MATCH_RECOGNIZE` right over the stages at `path[top]`, and the index of its
/// measures in `path`, or `None` for another front. The pass plans the clause as windows of the
/// positions and of `PREV` and `NEXT`, a projection of the conditions, the clause's node, a
/// projection of the columns of each variable, an aggregate, and a projection of the measures.
fn match_recognize(
    path: &[LogicalPlan],
    top: usize,
) -> Result<Option<(MatchFront, usize)>, Rejection> {
    let mut at = top;
    let mut windows = Vec::new();
    while at > 0 {
        let LogicalPlan::Window(window) = &path[at - 1] else {
            break;
        };
        windows.extend(window.window_expr.iter().cloned());
        at -= 1;
    }
    if windows.is_empty() || at < 2 {
        return Ok(None);
    }
    let conditions = &path[at - 1];
    let node = &path[at - 2];
    let LogicalPlan::Projection(_) = conditions else {
        return Ok(None);
    };
    let LogicalPlan::Extension(extension) = node else {
        return Ok(None);
    };
    let Some(call) = (extension.node.as_any())
        .downcast_ref::<avenger_datafusion_transforms::table_function::TableFunctionNode>()
        .and_then(avenger_datafusion_transforms::match_recognize::call_of)
    else {
        return Ok(None);
    };
    // The measures: the aggregate over the copies, under windows of HAVING MAX, and its
    // projection.
    let mut up = at - 2;
    let unexpected =
        || reject("MATCH_RECOGNIZE over events with a plan that the runtime doesn't read");
    if up == 0 || !matches!(path[up - 1], LogicalPlan::Projection(_)) {
        return unexpected();
    }
    up -= 1;
    while up > 0 && matches!(path[up - 1], LogicalPlan::Window(_)) {
        up -= 1;
    }
    if up < 2
        || !matches!(path[up - 1], LogicalPlan::Aggregate(_))
        || !matches!(path[up - 2], LogicalPlan::Projection(_))
    {
        return unexpected();
    }
    let measures = up - 2;
    if path[measures..top].iter().any(has_subquery) {
        return reject("a subquery in the front over events, which the runtime doesn't evaluate");
    }
    // The order of the positions, and the reach of PREV and NEXT.
    let stages = &path[top];
    let (mut partition, mut seq, mut back, mut ahead) = (Vec::new(), None, 0, 0);
    for expr in &windows {
        let Expr::WindowFunction(function) = expr else {
            continue;
        };
        let offset = match function.params.args.get(1) {
            None => 1,
            Some(Expr::Literal(ScalarValue::Int64(Some(k)), _)) if *k >= 0 => *k as usize,
            Some(_) => return reject("a PREV or NEXT over events whose offset isn't a literal"),
        };
        match function.fun.name() {
            "row_number" => {
                partition = function.params.partition_by.clone();
                seq = match function.params.order_by.as_slice() {
                    [key] if key.asc => match &key.expr {
                        Expr::Column(column) if is_seq(&key.expr, stages) => Some(column.clone()),
                        _ => None,
                    },
                    _ => None,
                };
            }
            "lag" => back = back.max(offset),
            "lead" => ahead = ahead.max(offset),
            _ => {}
        }
    }
    let Some(seq) = seq else {
        return reject("a MATCH_RECOGNIZE over events whose ORDER BY isn't seq ascending, the order in which events arrive");
    };
    let staged = Slot::new("staged", stages.schema());
    let node_rows = Slot::new("node_rows", node.schema());
    let front = MatchFront {
        conditions: replace(conditions, stages, &staged.plan),
        staged,
        measures: replace(&path[measures], node, &node_rows.plan),
        node_rows,
        call,
        partition,
        seq,
        back,
        ahead,
    };
    Ok(Some((front, measures)))
}

/// Whether `node` is a scan of the table `events`, under any name.
fn reads(node: &LogicalPlan, events: &Arc<dyn TableProvider>) -> bool {
    let LogicalPlan::TableScan(scan) = node else {
        return false;
    };
    match source_as_provider(&scan.source) {
        Ok(provider) => std::ptr::addr_eq(Arc::as_ptr(&provider), Arc::as_ptr(events)),
        Err(_) => false,
    }
}

/// The paths of nodes from `plan` down to each of its scans of `events`, after `prefix`, in `out`.
fn paths_to_events(
    plan: &LogicalPlan,
    events: &Arc<dyn TableProvider>,
    prefix: &mut Vec<LogicalPlan>,
    out: &mut Vec<Vec<LogicalPlan>>,
) {
    prefix.push(plan.clone());
    if reads(plan, events) {
        out.push(prefix.clone());
    }
    for input in plan.inputs() {
        paths_to_events(input, events, prefix, out);
    }
    prefix.pop();
}

/// Whether `node` is a stage: an operator that maps each row on its own. A stage that reads a
/// subquery or a function whose value can change between calls is an error, since a stream would
/// read it at another time than the batch plan does.
fn stateless(node: &LogicalPlan) -> Result<bool, Rejection> {
    let exprs: Vec<&Expr> = match node {
        LogicalPlan::Projection(projection) => projection.expr.iter().collect(),
        LogicalPlan::Filter(filter) => vec![&filter.predicate],
        LogicalPlan::SubqueryAlias(_) => Vec::new(),
        _ => return Ok(false),
    };
    for expr in exprs {
        let mut found = None;
        expr.apply(|e| {
            match e {
                Expr::ScalarSubquery(_) | Expr::Exists(_) | Expr::InSubquery(_) => {
                    found = Some("a subquery".to_string());
                }
                Expr::ScalarFunction(call)
                    if call.func.signature().volatility != Volatility::Immutable =>
                {
                    found = Some(format!("the function {}", call.func.name()));
                }
                _ => {}
            }
            Ok(match found {
                Some(_) => TreeNodeRecursion::Stop,
                None => TreeNodeRecursion::Continue,
            })
        })
        .expect("a walk of an expression");
        if let Some(what) = found {
            return reject(format!(
                "a stage over events reads {what}, whose value a stream reads at another time than the batch plan does"
            ));
        }
    }
    Ok(true)
}

/// Whether `node`'s own expressions hold a subquery.
fn has_subquery(node: &LogicalPlan) -> bool {
    let mut found = false;
    let _ = node.apply_expressions(|expr| {
        found |= expr
            .exists(|e| {
                Ok(matches!(
                    e,
                    Expr::ScalarSubquery(_) | Expr::Exists(_) | Expr::InSubquery(_)
                ))
            })
            .unwrap_or(false);
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

/// The name of a function that `plan` reads, in its subqueries too, whose value changes between
/// calls, such as `now`.
fn time_function(plan: &LogicalPlan) -> Option<String> {
    let mut found = None;
    let _ = plan.apply_with_subqueries(|node| {
        let _ = node.apply_expressions(|expr| {
            let _ = expr.apply(|e| {
                if let Expr::ScalarFunction(call) = e {
                    if call.func.signature().volatility != Volatility::Immutable {
                        found.get_or_insert_with(|| call.func.name().to_string());
                    }
                }
                Ok(TreeNodeRecursion::Continue)
            });
            Ok(TreeNodeRecursion::Continue)
        });
        Ok(TreeNodeRecursion::Continue)
    });
    found
}

/// The skip and the fetch of `limit`, which are literals.
fn limits(limit: &Limit) -> Result<(usize, usize), Rejection> {
    let value = |expr: &Option<Box<Expr>>| match expr.as_deref() {
        None => Ok(None),
        Some(Expr::Literal(ScalarValue::Int64(Some(n)), _)) if *n >= 0 => Ok(Some(*n as usize)),
        Some(_) => reject("a LIMIT or OFFSET over events that isn't a literal"),
    };
    let skip = value(&limit.skip)?.unwrap_or(0);
    let Some(fetch) = value(&limit.fetch)? else {
        return reject(
            "an OFFSET over events without a LIMIT, whose result grows with every event",
        );
    };
    Ok((skip, fetch))
}

/// Whether `expr`, over the output of `stages`, is the column `seq` of `events`.
fn is_seq(expr: &Expr, stages: &LogicalPlan) -> bool {
    let Expr::Column(column) = expr else {
        return false;
    };
    let mut column = column.clone();
    let mut node = stages;
    loop {
        match node {
            LogicalPlan::TableScan(_) => return column.name == "seq",
            LogicalPlan::Projection(projection) => {
                let Ok(index) = projection.schema.index_of_column(&column) else {
                    return false;
                };
                let mut source = &projection.expr[index];
                while let Expr::Alias(alias) = source {
                    source = alias.expr.as_ref();
                }
                let Expr::Column(next) = source else {
                    return false;
                };
                column = next.clone();
                node = projection.input.as_ref();
            }
            LogicalPlan::Filter(filter) => node = filter.input.as_ref(),
            LogicalPlan::SubqueryAlias(alias) => {
                let Ok(index) = alias.schema.index_of_column(&column) else {
                    return false;
                };
                let (qualifier, field) = alias.input.schema().qualified_field(index);
                column = Column::new(qualifier.cloned(), field.name());
                node = alias.input.as_ref();
            }
            _ => return false,
        }
    }
}
