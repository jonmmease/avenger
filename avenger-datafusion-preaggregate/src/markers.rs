use std::{cmp::Ordering, fmt, sync::Arc};

use datafusion::{
    common::{tree_node::Transformed, DFSchemaRef, Result},
    logical_expr::{Expr, Extension, LogicalPlan, LogicalPlanBuilder, UserDefinedLogicalNodeCore},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Hash)]
pub(crate) struct FilterSite {
    pub id: u64,
    pub input: LogicalPlan,
}
pub(crate) fn site(plan: &LogicalPlan) -> Option<&FilterSite> {
    match plan {
        LogicalPlan::Extension(e) => e.node.as_any().downcast_ref(),
        _ => None,
    }
}
pub(crate) fn rows(id: u64, input: LogicalPlan) -> LogicalPlan {
    LogicalPlan::Extension(Extension {
        node: Arc::new(FilterSite { id, input }),
    })
}
pub(crate) fn substitute(
    plan: &LogicalPlan,
    id: u64,
    predicate: Option<Expr>,
) -> Result<LogicalPlan> {
    Ok(plan
        .clone()
        .transform_up_with_subqueries(|plan| {
            if let Some(s) = site(&plan) {
                if s.id == id {
                    let replacement = match &predicate {
                        Some(p) => LogicalPlanBuilder::from(s.input.clone())
                            .filter(p.clone())?
                            .build()?,
                        None => s.input.clone(),
                    };
                    return Ok(Transformed::yes(replacement));
                }
            }
            Ok(Transformed::no(plan))
        })?
        .data)
}
impl UserDefinedLogicalNodeCore for FilterSite {
    fn name(&self) -> &str {
        "PreaggregateFilterSite"
    }
    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![&self.input]
    }
    fn schema(&self) -> &DFSchemaRef {
        self.input.schema()
    }
    fn expressions(&self) -> Vec<Expr> {
        vec![]
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PreaggregateFilterSite: {}", self.id)
    }
    fn with_exprs_and_inputs(
        &self,
        exprs: Vec<Expr>,
        mut inputs: Vec<LogicalPlan>,
    ) -> Result<Self> {
        if !exprs.is_empty() || inputs.len() != 1 {
            return datafusion::common::internal_err!(
                "filter site requires one input and no expressions"
            );
        }
        Ok(Self {
            id: self.id,
            input: inputs.remove(0),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MaterializationSite {
    pub id: u64,
    pub schema: DFSchemaRef,
}
impl PartialOrd for MaterializationSite {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.id
            .partial_cmp(&other.id)
            .filter(|order| *order != Ordering::Equal || self == other)
    }
}
impl UserDefinedLogicalNodeCore for MaterializationSite {
    fn name(&self) -> &str {
        "PreaggregateMaterializationSite"
    }
    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![]
    }
    fn schema(&self) -> &DFSchemaRef {
        &self.schema
    }
    fn expressions(&self) -> Vec<Expr> {
        vec![]
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PreaggregateMaterializationSite: {}", self.id)
    }
    fn with_exprs_and_inputs(&self, exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> Result<Self> {
        if !exprs.is_empty() || !inputs.is_empty() {
            return datafusion::common::internal_err!(
                "materialization site is a leaf without expressions"
            );
        }
        Ok(self.clone())
    }
}
