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

use crate::{
    preaggregate::{
        predicates::{self, PredicateSplit},
        Eligibility, Preaggregation,
    },
    AggregateStep, ConsumerFilter, Error, ProducerDefinition, Result, SelectionSet,
};

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
    Preaggregated,
}

/// Why a binding uses the original query without pre-aggregation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectReason {
    /// The caller disabled pre-aggregation through QueryPolicy.
    Forced,
    NoFocus,
    FocusNotUsed,
    UnsupportedFactorization,
    UnsupportedInteraction,
    IncompatibleFocus,
    UnsupportedQueryShape,
    UnsupportedAggregate,
    UnsupportedGroupingExpression,
    NonImmutableQuery,
    UnsafeMovedExpression,
}
impl fmt::Display for DirectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Forced => "direct execution requested by QueryPolicy::ForceDirect",
            Self::NoFocus => "no changing producer was specified",
            Self::FocusNotUsed => "the consumer excludes or does not use the focused producer",
            Self::UnsupportedFactorization => {
                "the resolved predicate has no supported fixed/changing conjunction"
            }
            Self::UnsupportedInteraction => {
                "the focused producer has no supported retained interaction keys"
            }
            Self::IncompatibleFocus => "the focused producer's definition or pixel grid changed",
            Self::UnsupportedQueryShape => {
                "the query has no supported single-site aggregate and suffix"
            }
            Self::UnsupportedAggregate => "the query needs an unsupported aggregate state recipe",
            Self::UnsupportedGroupingExpression => {
                "a grouping expression is not proved safe over unselected rows"
            }
            Self::UnsafeMovedExpression => {
                "an expression is not proved safe before the changing filter"
            }
            Self::NonImmutableQuery => "the query contains stable or volatile computations",
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
    pub(crate) fn preaggregated() -> Self {
        Self {
            strategy: QueryStrategy::Preaggregated,
            direct_reason: None,
        }
    }
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
        with_predicate(&self.0.plan, self.0.site, predicate)
    }
}

pub(crate) fn with_predicate(plan: &LogicalPlan, id: u64, predicate: Expr) -> Result<LogicalPlan> {
    Ok(plan
        .clone()
        .transform_up_with_subqueries(|plan| {
            if let Some(site) = selection_site(&plan) {
                if site.id == id {
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
    /// Validate named selections and mappings, then derive sufficient state for
    /// supported built-in aggregates. Unsupported queries retain the direct recipe.
    pub fn build(self) -> Result<QueryFamily> {
        let resolved = self.query.0.filter.resolve(self.selections)?;
        if let Some(focus) = &self.focus {
            self.selections.get(&focus.address().selection)?;
        }
        let preaggregation = if let Some(focus) = &self.focus {
            if focus.identity().is_some() {
                Err(DirectReason::UnsupportedInteraction)
            } else {
                let keys = self.query.0.filter.interaction_keys(focus);
                match predicates::split(
                    &resolved,
                    focus,
                    self.query.0.filter.consumer_view(),
                    &keys,
                ) {
                    Ok(_) => Preaggregation::analyze(&self.query.0.plan, self.query.0.site, keys)?,
                    Err(reason) => Err(reason),
                }
            }
        } else {
            Err(DirectReason::NoFocus)
        };
        Ok(QueryFamily {
            query: self.query,
            focus: self.focus,
            policy: self.policy,
            preaggregation,
        })
    }
}

/// A reusable query definition with a planning focus and default execution policy.
#[derive(Clone, Debug)]
pub struct QueryFamily {
    pub(crate) query: SelectionQuery,
    focus: Option<ProducerDefinition>,
    policy: QueryPolicy,
    pub(crate) preaggregation: Eligibility<Preaggregation>,
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
        if self.policy == QueryPolicy::ForceDirect {
            return QueryDiagnostics::direct(DirectReason::Forced);
        }
        match &self.preaggregation {
            Ok(_) => QueryDiagnostics::preaggregated(),
            Err(reason) => QueryDiagnostics::direct(*reason),
        }
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
        // Resolve the complete predicate even when the optimized output wins.
        let full = self.query.predicate(selections)?;
        match self.split(selections, policy)? {
            Ok(split) => {
                let prepared = self.preaggregation.as_ref().expect("eligible template");
                Ok(BoundQuery::Preaggregated {
                    materialization: prepared.materialization(split.fixed)?,
                    aggregate: prepared.aggregate(split.changing)?,
                })
            }
            Err(reason) => Ok(BoundQuery::Direct {
                plan: self.query.with_predicate(full)?,
                reason,
            }),
        }
    }

    pub(crate) fn split(
        &self,
        selections: &SelectionSet,
        policy: QueryPolicy,
    ) -> Result<Eligibility<PredicateSplit>> {
        if policy == QueryPolicy::ForceDirect {
            return Ok(Err(DirectReason::Forced));
        }
        let prepared = match &self.preaggregation {
            Ok(prepared) => prepared,
            Err(reason) => return Ok(Err(*reason)),
        };
        let resolved = self.query.0.filter.resolve(selections)?;
        let split = predicates::split(
            &resolved,
            self.focus.as_ref().expect("planned focus"),
            self.query.0.filter.consumer_view(),
            &prepared.interaction_keys,
        );
        Ok(match split {
            Ok(split) => prepared.validate(&split)?.map(|()| split),
            Err(reason) => Err(reason),
        })
    }
}

/// Native plans for a single snapshot, with explicit materialization when supported.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum BoundQuery {
    Direct {
        plan: LogicalPlan,
        reason: DirectReason,
    },
    Preaggregated {
        materialization: LogicalPlan,
        aggregate: AggregateStep,
    },
}
impl BoundQuery {
    /// Explain the strategy chosen for this binding.
    pub fn explain(&self) -> QueryDiagnostics {
        match self {
            Self::Direct { reason, .. } => QueryDiagnostics::direct(*reason),
            Self::Preaggregated { .. } => QueryDiagnostics::preaggregated(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Hash)]
pub(crate) struct SelectionSite {
    pub id: u64,
    pub input: LogicalPlan,
}
pub(crate) fn selection_site(plan: &LogicalPlan) -> Option<&SelectionSite> {
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
