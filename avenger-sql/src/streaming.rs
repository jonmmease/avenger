//! The split of a query over the built-in `events` table for a streaming runtime: the stateless
//! stages that read `events`, the front that reduces their rows, and the downstream that reads the
//! front's output.
//!
//! A streaming runtime runs each frame's events through the stages once and applies the rows
//! that they keep to the front, whose output is a small table that the runtime maintains. The
//! downstream is the rest of the query as a batch plan over that table, which reads it through a
//! [`Slot`]. After every frame, the result equals the batch plan of the whole query over every
//! event so far. The fronts take rows that only grow, as `events` does, so none of them needs a
//! retraction:
//!
//! - an `AGGREGATE`, grouped or not, whose accumulators only add rows;
//! - an `ORDER BY ... LIMIT`, whose kept rows only leave when a new row takes their place.
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
    /// The stages: the plan of the rows that reach the front, over the table `events`.
    pub stages: LogicalPlan,
    /// The scan of `events` in `stages`, which a runtime replaces with a scan of each frame.
    pub scan: TableScan,
    pub front: Front,
    /// The rest of the query, which reads the front's output through `slot`.
    pub downstream: LogicalPlan,
    pub slot: Slot,
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
/// it doesn't split.
pub fn split(plan: &LogicalPlan, events: &Arc<dyn TableProvider>) -> Result<Split, Rejection> {
    // The path from the plan's root to its one scan of `events`.
    let mut scans = 0;
    plan.apply(|node| {
        if reads(node, events) {
            scans += 1;
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .expect("a walk of the plan");
    match scans {
        0 => return reject("the query doesn't read events"),
        1 => {}
        n => return reject(format!("the query reads events {n} times")),
    }
    let path = path_to_events(plan, events).expect("the scan of events");
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
    // A table function over the stages, such as MATCH_RECOGNIZE, which plans windows under its
    // node: the first one above the stages, below any aggregate or limit.
    let reduced = (0..top)
        .rev()
        .find(|&i| matches!(path[i], LogicalPlan::Aggregate(_) | LogicalPlan::Limit(_)));
    let lowest = reduced.map_or(0, |i| i + 1);
    for node in path[lowest..top].iter().rev() {
        if let LogicalPlan::Extension(extension) = node {
            let name = format!("{}", node.display());
            return match name.starts_with("MatchRecognize") {
                true => reject("MATCH_RECOGNIZE over events isn't a streaming front yet"),
                false => reject(format!(
                    "the table function {} over events isn't a streaming front",
                    extension.node.name()
                )),
            };
        }
    }
    let stages = &path[top];
    let node = &path[top - 1];
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
    let slot = Slot::new("front", replaced.schema());
    let downstream = plan
        .clone()
        .transform_down(|p| match &p == replaced {
            true => Ok(Transformed::new(
                slot.plan.clone(),
                true,
                TreeNodeRecursion::Jump,
            )),
            false => Ok(Transformed::no(p)),
        })
        .expect("the downstream")
        .data;
    Ok(Split {
        stages: stages.clone(),
        scan: scan.clone(),
        front,
        downstream,
        slot,
    })
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

/// The nodes from `plan` down to its scan of `events`, or `None` without one.
fn path_to_events(plan: &LogicalPlan, events: &Arc<dyn TableProvider>) -> Option<Vec<LogicalPlan>> {
    if reads(plan, events) {
        return Some(vec![plan.clone()]);
    }
    for input in plan.inputs() {
        if let Some(mut path) = path_to_events(input, events) {
            path.insert(0, plan.clone());
            return Some(path);
        }
    }
    None
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
