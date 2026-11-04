//! What a plan reads of a run's inputs: the parameters that its expressions read, the owners
//! whose clauses its selection calls read, and whether it calls a volatile or a stable function.

use std::collections::{BTreeMap, BTreeSet};

use avenger_selection_datafusion::{selection_reads, Owners, SelectionRead};
use datafusion::{
    common::{
        tree_node::{TreeNode, TreeNodeRecursion},
        DFSchema, Result,
    },
    logical_expr::{
        expr::{AggregateFunction, ScalarFunction, WindowFunction},
        utils::merge_schema,
        Expr, LogicalPlan, Volatility,
    },
};

/// The inputs of a run that a plan reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    /// The ids of the placeholders of parameters, such as `$year`.
    pub params: BTreeSet<String>,
    /// The owners whose clauses the plan reads, by the id of the selection's placeholder.
    pub selections: BTreeMap<String, Owners>,
    /// Whether the plan calls a volatile or a stable function.
    pub volatile: bool,
}

impl Reads {
    /// The number of inputs, for ordering the conjuncts of a filter.
    pub fn count(&self) -> usize {
        self.params.len() + self.selections.len()
    }

    /// Add the inputs that `other` reads.
    pub fn add(&mut self, other: &Reads) {
        self.params.extend(other.params.iter().cloned());
        for (parameter, owners) in &other.selections {
            let merged = match self.selections.get(parameter) {
                Some(mine) => union(mine, owners),
                None => owners.clone(),
            };
            self.selections.insert(parameter.clone(), merged);
        }
        self.volatile |= other.volatile;
    }

    /// The selection reads, one for each selection.
    pub fn selection_reads(&self) -> Vec<SelectionRead> {
        (self.selections.iter())
            .map(|(parameter, owners)| SelectionRead {
                parameter: parameter.clone(),
                owners: owners.clone(),
            })
            .collect()
    }

    /// What `expr`, an expression over the rows of `schema`, reads, without its subqueries. Each
    /// selection call that `selection_reads` reports takes its selection's placeholder as its
    /// first argument. A placeholder is a parameter when it appears more often than its
    /// selection calls, so a parameter can share a selection's name.
    pub fn of_expr(expr: &Expr, schema: &DFSchema) -> Result<Reads> {
        let mut reads = Reads::default();
        let mut calls: BTreeMap<String, usize> = BTreeMap::new();
        for read in selection_reads(expr, schema)? {
            *calls.entry(read.parameter.clone()).or_default() += 1;
            reads.add(&Reads {
                selections: BTreeMap::from([(read.parameter, read.owners)]),
                ..Reads::default()
            });
        }
        expr.apply(|expr| {
            match expr {
                Expr::ScalarFunction(ScalarFunction { func, .. }) => {
                    reads.volatile |= func.signature().volatility != Volatility::Immutable;
                }
                Expr::AggregateFunction(AggregateFunction { func, .. }) => {
                    reads.volatile |= func.signature().volatility != Volatility::Immutable;
                }
                Expr::WindowFunction(window) => {
                    let WindowFunction { fun, .. } = window.as_ref();
                    reads.volatile |= fun.signature().volatility != Volatility::Immutable;
                }
                Expr::Placeholder(placeholder) => {
                    let calls = calls.entry(placeholder.id.clone()).or_default();
                    match *calls {
                        0 => {
                            reads.params.insert(placeholder.id.clone());
                        }
                        _ => *calls -= 1,
                    }
                }
                _ => {}
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        Ok(reads)
    }

    /// What the operator `plan` reads through its own expressions and their subqueries, without
    /// its inputs.
    pub fn of_operator(plan: &LogicalPlan) -> Result<Reads> {
        let schema = match plan {
            LogicalPlan::TableScan(scan) => scan.projected_schema.as_ref().clone(),
            _ => merge_schema(&plan.inputs()),
        };
        let mut reads = Reads::default();
        for expr in plan.expressions() {
            reads.add(&Reads::of_expr(&expr, &schema)?);
        }
        plan.apply_subqueries(|subquery| {
            reads.add(&Reads::of_plan(subquery)?);
            Ok(TreeNodeRecursion::Continue)
        })?;
        Ok(reads)
    }

    /// What the plan `plan` reads, with its inputs and its subqueries.
    pub fn of_plan(plan: &LogicalPlan) -> Result<Reads> {
        let mut reads = Reads::of_operator(plan)?;
        for input in plan.inputs() {
            reads.add(&Reads::of_plan(input)?);
        }
        Ok(reads)
    }
}

/// The owners that two reads of one selection read together.
fn union(a: &Owners, b: &Owners) -> Owners {
    match (a, b) {
        (Owners::AllBut(a), Owners::AllBut(b)) => {
            Owners::AllBut(a.intersection(b).cloned().collect())
        }
        (Owners::Only(a), Owners::Only(b)) => Owners::Only(a.union(b).cloned().collect()),
        (Owners::AllBut(skipped), Owners::Only(read))
        | (Owners::Only(read), Owners::AllBut(skipped)) => {
            Owners::AllBut(skipped.difference(read).cloned().collect())
        }
    }
}

/// Whether `owners` reads the clauses of `owner`.
pub(crate) fn reads_owner(owners: &Owners, owner: &str) -> bool {
    match owners {
        Owners::AllBut(skipped) => !skipped.contains(owner),
        Owners::Only(read) => read.contains(owner),
    }
}
