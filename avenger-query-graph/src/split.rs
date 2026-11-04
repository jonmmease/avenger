//! Filters split into stacks of filters, so a conjunct that reads fewer inputs runs below one that
//! reads more, and the cut rule can cut between them.

use datafusion::{
    common::{
        tree_node::{Transformed, TreeNode},
        Result,
    },
    logical_expr::{
        utils::{conjunction, split_conjunction_owned},
        Filter, LogicalPlan,
    },
};

use crate::reads::Reads;

/// `plan` with each filter whose conjunction has conjuncts that read different inputs split into
/// a stack of filters. A conjunct whose inputs are a subset of another's is below it, conjuncts
/// that read the same inputs share a filter, conjuncts are otherwise in their written order, and
/// conjuncts that call a volatile or a stable function are on top.
pub(crate) fn split_filters(plan: LogicalPlan) -> Result<LogicalPlan> {
    Ok(plan
        .transform_up(|plan| {
            let LogicalPlan::Filter(filter) = plan else {
                return Ok(Transformed::no(plan));
            };
            let schema = filter.input.schema().clone();
            let conjuncts = split_conjunction_owned(filter.predicate.clone());
            if conjuncts.len() < 2 {
                return Ok(Transformed::no(LogicalPlan::Filter(filter)));
            }
            let mut conjuncts = (conjuncts.into_iter())
                .map(|expr| Ok((Reads::of_expr(&expr, &schema)?, expr)))
                .collect::<Result<Vec<_>>>()?;
            conjuncts.sort_by_key(|(reads, _)| (reads.volatile, reads.count()));
            let mut groups: Vec<(Reads, Vec<_>)> = Vec::new();
            for (reads, expr) in conjuncts {
                match groups.iter_mut().find(|(group, _)| *group == reads) {
                    Some((_, exprs)) => exprs.push(expr),
                    None => groups.push((reads, vec![expr])),
                }
            }
            if groups.len() < 2 {
                return Ok(Transformed::no(LogicalPlan::Filter(filter)));
            }
            let mut input = filter.input.clone();
            for (_, exprs) in groups {
                let predicate = conjunction(exprs).expect("a group has a conjunct");
                input = LogicalPlan::Filter(Filter::try_new(predicate, input)?).into();
            }
            Ok(Transformed::yes(input.as_ref().clone()))
        })?
        .data)
}
