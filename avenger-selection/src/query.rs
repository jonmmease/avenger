use std::{
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use datafusion::{
    common::{
        tree_node::{Transformed, TreeNodeRecursion},
        DFSchemaRef, Result as DFResult,
    },
    logical_expr::{Expr, Extension, LogicalPlan, LogicalPlanBuilder, UserDefinedLogicalNodeCore},
};

use crate::{ConsumerFilter, Error, ProducerDefinition, Result, SelectionSet};

/// Controls whether a query may use selection pre-aggregation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryPolicy {
    /// Allow supported optimizations, with direct execution as the fallback.
    #[default]
    Auto,
    /// Always apply the full predicate to the original query.
    /// Ordinary dataflow result caching still applies.
    ForceDirect,
}

/// Execution strategy selected for a query binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueryStrategy {
    Direct,
}

/// Why a binding uses the original query without pre-aggregation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectReason {
    /// The caller disabled pre-aggregation through QueryPolicy.
    Forced,
    /// The pre-aggregation planner is not implemented.
    PreaggregationNotImplemented,
}
impl fmt::Display for DirectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Forced => "direct execution requested by QueryPolicy::ForceDirect",
            Self::PreaggregationNotImplemented => "pre-aggregation is not implemented",
        })
    }
}

/// Inspectable strategy information that does not affect query results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryDiagnostics {
    pub strategy: QueryStrategy,
    pub direct_reason: Option<DirectReason>,
}
impl QueryDiagnostics {
    pub(crate) fn direct(reason: DirectReason) -> Self {
        Self {
            strategy: QueryStrategy::Direct,
            direct_reason: Some(reason),
        }
    }
}
impl fmt::Display for QueryDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.strategy)?;
        if let Some(reason) = self.direct_reason {
            write!(f, ": {reason}")?;
        }
        Ok(())
    }
}

/// A native query recipe with an owned, unbound selection-filter site.
/// Cloning retains the recipe, not a construction callback or selection snapshot.
#[derive(Clone, Debug)]
pub struct SelectionQuery(Arc<QueryDefinition>);

#[derive(Debug)]
struct QueryDefinition {
    filter: ConsumerFilter,
    plan: LogicalPlan,
    site: u64,
}

impl ConsumerFilter {
    /// Build a query once over a relation containing this consumer's filter site.
    ///
    /// The callback must return a plan that uses the supplied relation. Repeated
    /// uses and uses inside subqueries are supported. Only the owned site is
    /// replaced during binding. Planning neither reads data nor invokes UDFs.
    pub fn query(
        &self,
        source: LogicalPlan,
        build: impl FnOnce(LogicalPlan) -> DFResult<LogicalPlan>,
    ) -> Result<SelectionQuery> {
        static NEXT_SITE: AtomicU64 = AtomicU64::new(1);
        let site = NEXT_SITE.fetch_add(1, Ordering::Relaxed);
        let rows = LogicalPlan::Extension(Extension {
            node: Arc::new(SelectionSite {
                id: site,
                input: source,
            }),
        });
        let plan = build(rows)?;
        let mut found = false;
        plan.apply_with_subqueries(|plan| {
            if let Some(marker) = selection_site(plan) {
                if marker.id != site {
                    return datafusion::common::plan_err!(
                        "query contains another query's unbound selection-filter site"
                    );
                }
                found = true;
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        if !found {
            return Err(Error::InvalidQuery(
                "query callback must consume the supplied selection-filtered relation".into(),
            ));
        }
        Ok(SelectionQuery(Arc::new(QueryDefinition {
            filter: self.clone(),
            plan,
            site,
        })))
    }
}

impl SelectionQuery {
    /// Bind the complete current predicate into every occurrence of the owned site.
    /// Dataflow references, if present, still require their owning runtime.
    pub fn logical_plan(&self, selections: &SelectionSet) -> Result<LogicalPlan> {
        self.with_predicate(self.predicate(selections)?)
    }

    /// Start planning a reusable family. The builder borrows this snapshot only
    /// for validation. The completed family does not retain it.
    pub fn plan<'a>(&self, selections: &'a SelectionSet) -> QueryFamilyBuilder<'a> {
        QueryFamilyBuilder {
            query: self.clone(),
            selections,
            focus: None,
            policy: QueryPolicy::Auto,
        }
    }

    pub(crate) fn predicate(&self, selections: &SelectionSet) -> Result<Expr> {
        self.0.filter.predicate(selections)
    }

    pub(crate) fn with_predicate(&self, predicate: Expr) -> Result<LogicalPlan> {
        Ok(self
            .0
            .plan
            .clone()
            .transform_up_with_subqueries(|plan| {
                if let Some(site) = selection_site(&plan) {
                    if site.id == self.0.site {
                        return Ok(Transformed::yes(
                            LogicalPlanBuilder::from(site.input.clone())
                                .filter(predicate.clone())?
                                .build()?,
                        ));
                    }
                }
                Ok(Transformed::no(plan))
            })?
            .data)
    }
}

/// Configure a family without executing its source or retaining current values.
#[derive(Debug)]
pub struct QueryFamilyBuilder<'a> {
    query: SelectionQuery,
    selections: &'a SelectionSet,
    focus: Option<ProducerDefinition>,
    policy: QueryPolicy,
}
impl QueryFamilyBuilder<'_> {
    /// Identify the changing producer, including its view instance and pixel grid.
    /// Its named selection must exist, but the producer may be inactive.
    pub fn focus(mut self, producer: &ProducerDefinition) -> Self {
        self.focus = Some(producer.clone());
        self
    }
    /// Set the default policy for native and installed bindings.
    pub fn policy(mut self, policy: QueryPolicy) -> Self {
        self.policy = policy;
        self
    }
    /// Validate named selections and mappings, then retain the reusable recipe.
    /// Automatic planning currently selects direct execution for every query.
    pub fn build(self) -> Result<QueryFamily> {
        self.query.0.filter.resolve(self.selections)?;
        if let Some(focus) = &self.focus {
            self.selections.get(&focus.address().selection)?;
        }
        Ok(QueryFamily {
            query: self.query,
            focus: self.focus,
            policy: self.policy,
        })
    }
}

/// A reusable query definition with a planning focus and default execution policy.
#[derive(Clone, Debug)]
pub struct QueryFamily {
    pub(crate) query: SelectionQuery,
    focus: Option<ProducerDefinition>,
    policy: QueryPolicy,
}
impl QueryFamily {
    /// Return the planning hint. Current contributions determine actual membership.
    pub fn focus(&self) -> Option<&ProducerDefinition> {
        self.focus.as_ref()
    }
    /// Return the policy used by bind unless that request supplies an override.
    pub fn policy(&self) -> QueryPolicy {
        self.policy
    }
    /// Describe the family's default strategy without binding or executing data.
    pub fn explain(&self) -> QueryDiagnostics {
        Self::diagnostics(self.policy)
    }
    /// Bind a snapshot using the family's default policy.
    pub fn bind(&self, selections: &SelectionSet) -> Result<BoundQuery> {
        self.bind_with_policy(selections, self.policy)
    }
    /// Override the policy for one request, preserving this family's default.
    /// ForceDirect remains subject to normal selection and mapping validation.
    pub fn bind_with_policy(
        &self,
        selections: &SelectionSet,
        policy: QueryPolicy,
    ) -> Result<BoundQuery> {
        Ok(BoundQuery::Direct {
            plan: self.query.logical_plan(selections)?,
            reason: Self::reason(policy),
        })
    }
    pub(crate) fn diagnostics(policy: QueryPolicy) -> QueryDiagnostics {
        QueryDiagnostics::direct(Self::reason(policy))
    }
    fn reason(policy: QueryPolicy) -> DirectReason {
        match policy {
            QueryPolicy::Auto => DirectReason::PreaggregationNotImplemented,
            QueryPolicy::ForceDirect => DirectReason::Forced,
        }
    }
}

/// Native plans for a single snapshot. All bindings currently use Direct.
/// Additional strategies will expose their materialization and aggregate stages.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum BoundQuery {
    Direct {
        plan: LogicalPlan,
        reason: DirectReason,
    },
}
impl BoundQuery {
    /// Explain the strategy chosen for this binding.
    pub fn explain(&self) -> QueryDiagnostics {
        match self {
            Self::Direct { reason, .. } => QueryDiagnostics::direct(*reason),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Hash)]
struct SelectionSite {
    id: u64,
    input: LogicalPlan,
}
fn selection_site(plan: &LogicalPlan) -> Option<&SelectionSite> {
    match plan {
        LogicalPlan::Extension(e) => e.node.as_any().downcast_ref(),
        _ => None,
    }
}
impl UserDefinedLogicalNodeCore for SelectionSite {
    fn name(&self) -> &str {
        "SelectionFilterSite"
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
        write!(f, "SelectionFilterSite: {}", self.id)
    }
    fn with_exprs_and_inputs(
        &self,
        exprs: Vec<Expr>,
        mut inputs: Vec<LogicalPlan>,
    ) -> DFResult<Self> {
        if !exprs.is_empty() || inputs.len() != 1 {
            return datafusion::common::internal_err!(
                "SelectionFilterSite requires one input and no expressions"
            );
        }
        Ok(Self {
            id: self.id,
            input: inputs.remove(0),
        })
    }
}
