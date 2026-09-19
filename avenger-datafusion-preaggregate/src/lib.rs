#![doc = include_str!("../README.md")]

mod aggregates;
mod expressions;
mod markers;
mod rewrite;
pub mod runtime;

use datafusion::{
    common::{tree_node::TreeNodeRecursion, DFSchema, DFSchemaRef, Result},
    logical_expr::{Expr, LogicalPlan},
};
pub use expressions::{ExpressionProperties, ScalarFunctionProperties};
use rewrite::Preaggregation;
pub use rewrite::RollupQuery;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

pub(crate) type Eligibility<T> = std::result::Result<T, DirectReason>;
fn fresh_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .expect("preaggregation identity space exhausted")
}

/// Why a valid query uses direct execution. Invalid expressions return errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectReason {
    /// The caller requested the original query.
    Forced,
    /// The marker has no supported unique unary path to an aggregate.
    UnsupportedQueryShape,
    /// The nearest aggregate needs an unsupported state recipe or modifier.
    UnsupportedAggregate,
    /// A grouping expression is not proved safe before the changing filter.
    UnsupportedGroupingExpression,
    /// A visible expression is stable or volatile.
    NonImmutableQuery,
    /// A moved measure argument or fixed filter may fail on newly included rows.
    UnsafeMovedExpression,
    /// The predicate needs row information absent from the stored dimensions.
    PredicateNeedsUnretainedExpression,
    /// The predicate can distinguish values that DataFusion groups together.
    UnsupportedPredicate,
}

/// Controls whether a binding may use the prepared materialization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueryPolicy {
    /// Use the rewrite when supported under the documented numerical contract.
    #[default]
    Auto,
    /// Preserve the original query. This does not disable external caches.
    ForceDirect,
}
/// The two query shapes returned by binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryStrategy {
    /// Apply the changing predicate to source rows and execute the original query.
    Direct,
    /// Filter stored cells, reconstruct target groups, and execute finishing operators.
    Preaggregated,
}
/// Binding-specific eligibility, independent of execution and cache reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryDiagnostics {
    /// Query shape selected for this binding.
    pub strategy: QueryStrategy,
    /// Why this binding uses direct execution, if applicable.
    pub direct_reason: Option<DirectReason>,
}
/// Preparation details shared by every binding.
#[derive(Clone, Debug)]
pub struct PreparationReport {
    /// Why no materialization could be prepared, if applicable.
    pub direct_reason: Option<DirectReason>,
    /// Original aggregate grouping expressions, including hidden output groups.
    pub grouping_dimensions: Vec<Expr>,
    /// Requested additional dimensions. Duplicate storage keys are eliminated.
    pub retained_dimensions: Vec<Expr>,
    /// Generated state expressions, including per-measure filters.
    pub aggregate_states: Vec<Expr>,
    /// Complete input schema expected by the rollup, when preparation succeeds.
    pub materialization_schema: Option<DFSchemaRef>,
    /// Aggregate states inherit native overflow and floating regrouping behavior.
    pub numerical_contract: &'static str,
}

/// A reusable logical query with an owned changing-filter location.
#[derive(Clone, Debug)]
pub struct FilterQuery {
    pub(crate) plan: LogicalPlan,
    pub(crate) source_schema: DFSchemaRef,
    pub(crate) site: u64,
}
impl FilterQuery {
    /// Start construction when the plan will be built asynchronously or in stages.
    pub fn builder(source: LogicalPlan) -> FilterQueryBuilder {
        FilterQueryBuilder {
            site: fresh_id(),
            source,
        }
    }
    /// Build the final query from rows containing the owned filter marker.
    pub fn new(
        source: LogicalPlan,
        build: impl FnOnce(LogicalPlan) -> Result<LogicalPlan>,
    ) -> Result<Self> {
        let builder = Self::builder(source);
        let plan = build(builder.rows())?;
        builder.finish(plan)
    }
    /// Return the schema in which changing predicates and retained dimensions resolve.
    pub fn source_schema(&self) -> &DFSchemaRef {
        &self.source_schema
    }
    /// Insert a concrete Boolean predicate into every occurrence of this filter.
    pub fn direct(&self, predicate: Expr) -> Result<LogicalPlan> {
        let predicate = expressions::boolean(predicate, &self.source_schema, true)?;
        markers::substitute(&self.plan, self.site, Some(predicate))
    }
}
/// Owns a filter marker until the completed query is validated.
#[derive(Debug)]
pub struct FilterQueryBuilder {
    site: u64,
    source: LogicalPlan,
}
impl FilterQueryBuilder {
    /// Return a relation containing the marker. Repeated uses retain the same identity.
    pub fn rows(&self) -> LogicalPlan {
        markers::rows(self.site, self.source.clone())
    }
    /// Finish construction. Missing and foreign markers are errors.
    /// Use an unoptimized logical plan so optimization cannot remove the marker.
    pub fn finish(self, plan: LogicalPlan) -> Result<FilterQuery> {
        let mut found = false;
        plan.apply_with_subqueries(|p| {
            if let Some(s) = markers::site(p) {
                if s.id != self.site {
                    return datafusion::common::plan_err!(
                        "query contains another builder's filter marker"
                    );
                }
                found = true;
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        if !found {
            return datafusion::common::plan_err!("query must consume the builder's rows");
        }
        Ok(FilterQuery {
            plan,
            source_schema: self.source.schema().clone(),
            site: self.site,
        })
    }
}

/// Synchronous logical planning with conservative scalar safety checks.
#[derive(Clone, Debug)]
pub struct PreaggregatePlanner {
    properties: Arc<dyn ExpressionProperties>,
}
impl Default for PreaggregatePlanner {
    fn default() -> Self {
        Self {
            properties: Arc::new(expressions::ConservativeProperties),
        }
    }
}
impl PreaggregatePlanner {
    /// Supply trusted properties for configured scalar UDFs. Volatility and children
    /// remain independently checked. The hook must not evaluate functions.
    pub fn with_expression_properties(mut self, properties: Arc<dyn ExpressionProperties>) -> Self {
        self.properties = properties;
        self
    }
    /// Check a deferred expression's properties after native coercion. Runtime
    /// adapters use this before binding inputs whose wrappers carry trusted properties.
    pub fn expression_properties(
        &self,
        expr: Expr,
        schema: &DFSchema,
    ) -> Result<ScalarFunctionProperties> {
        let expr = expressions::coerce(expr, schema)?;
        Ok(expressions::properties(&expr, schema, &self.properties))
    }
    /// Analyze once without reading sources, evaluating functions, or planning execution.
    /// Explicit dimensions describe coverage for warm-up before a predicate exists.
    pub fn prepare(
        &self,
        query: FilterQuery,
        retained_dimensions: Vec<Expr>,
    ) -> Result<PreparedQuery> {
        let retained_dimensions = retained_dimensions
            .into_iter()
            .map(|e| expressions::coerce(e, &query.source_schema))
            .collect::<Result<Vec<_>>>()?;
        // Validate dimensions even if query shape analysis will choose direct execution.
        for e in &retained_dimensions {
            use datafusion::logical_expr::ExprSchemable;
            e.get_type(&query.source_schema)?;
        }
        let analyzed = Preaggregation::analyze(&query, &retained_dimensions, &self.properties)?;
        let report = match &analyzed {
            Ok(p) => PreparationReport {
                direct_reason: None,
                grouping_dimensions: p.grouping.clone(),
                retained_dimensions,
                aggregate_states: p.states.clone(),
                materialization_schema: Some(p.materialization.schema().clone()),
                numerical_contract: NUMERICAL_CONTRACT,
            },
            Err(reason) => PreparationReport {
                direct_reason: Some(*reason),
                grouping_dimensions: vec![],
                retained_dimensions,
                aggregate_states: vec![],
                materialization_schema: None,
                numerical_contract: NUMERICAL_CONTRACT,
            },
        };
        Ok(PreparedQuery {
            id: fresh_id(),
            query,
            analyzed,
            report,
            properties: self.properties.clone(),
        })
    }
}
const NUMERICAL_CONTRACT: &str = "State merging changes accumulation order. Floating results, overflow, and boundary-sensitive HAVING/ranking/top-k can differ from direct execution. No execution-time retry is performed.";

/// Immutable preparation shared by concrete bindings and runtime templates.
#[derive(Clone, Debug)]
pub struct PreparedQuery {
    id: u64,
    query: FilterQuery,
    analyzed: Eligibility<Preaggregation>,
    report: PreparationReport,
    properties: Arc<dyn ExpressionProperties>,
}
impl PreparedQuery {
    /// Inspect eligibility, dimensions, states, and the numerical contract.
    pub fn explain(&self) -> &PreparationReport {
        &self.report
    }
    /// Inspect the complete warm-up plan without binding a changing predicate.
    /// The caller must validate ordinary deferred inputs before executing it.
    pub fn materialization_plan(&self) -> Option<&LogicalPlan> {
        self.analyzed.as_ref().ok().map(|p| &p.materialization)
    }
    /// Bind a concrete predicate using automatic eligibility checks.
    pub fn bind(&self, changing: Expr) -> Result<BoundQuery> {
        self.bind_with_policy(changing, QueryPolicy::Auto)
    }
    /// Bind a predicate with an explicit policy. Invalid predicates remain errors.
    pub fn bind_with_policy(&self, changing: Expr, policy: QueryPolicy) -> Result<BoundQuery> {
        let source = expressions::boolean(changing, &self.query.source_schema, true)?;
        let eligible = if policy == QueryPolicy::ForceDirect {
            Err(DirectReason::Forced)
        } else {
            match &self.analyzed {
                Ok(p) => p.predicate(&source, &self.query.source_schema, &self.properties)?,
                Err(reason) => Err(*reason),
            }
        };
        match eligible {
            Ok(retained) => {
                let p = self.analyzed.as_ref().expect("eligible preparation");
                Ok(BoundQuery::Preaggregated {
                    materialization: p.materialization.clone(),
                    rollup: p.rollup(retained.clone())?,
                    diagnostics: QueryDiagnostics {
                        strategy: QueryStrategy::Preaggregated,
                        direct_reason: None,
                    },
                    predicates: runtime::BoundPredicates {
                        id: self.id,
                        source,
                        retained: Some(retained),
                    },
                })
            }
            Err(reason) => Ok(BoundQuery::Direct {
                plan: markers::substitute(&self.query.plan, self.query.site, Some(source.clone()))?,
                diagnostics: QueryDiagnostics {
                    strategy: QueryStrategy::Direct,
                    direct_reason: Some(reason),
                },
                predicates: runtime::BoundPredicates {
                    id: self.id,
                    source,
                    retained: None,
                },
            }),
        }
    }
}
/// Native plans for a single binding. Materialize state data before using the rollup.
#[derive(Clone, Debug)]
pub enum BoundQuery {
    /// The original query with the changing predicate inserted.
    Direct {
        /// Executable logical plan, subject to any ordinary deferred inputs.
        plan: LogicalPlan,
        /// Binding-specific strategy and fallback reason.
        diagnostics: QueryDiagnostics,
        /// Validated runtime predicate forms and preparation ownership.
        predicates: runtime::BoundPredicates,
    },
    /// Separate state construction and finishing plans.
    Preaggregated {
        /// State construction independent of the changing predicate.
        materialization: LogicalPlan,
        /// Finishing query requiring a compatible stored relation.
        rollup: RollupQuery,
        /// Binding-specific strategy.
        diagnostics: QueryDiagnostics,
        /// Validated runtime predicate forms and preparation ownership.
        predicates: runtime::BoundPredicates,
    },
}
impl BoundQuery {
    /// Inspect the strategy selected for this binding.
    pub fn diagnostics(&self) -> &QueryDiagnostics {
        match self {
            Self::Direct { diagnostics, .. } | Self::Preaggregated { diagnostics, .. } => {
                diagnostics
            }
        }
    }
    /// Obtain validated predicate forms for this preparation's runtime templates.
    pub fn predicates(&self) -> &runtime::BoundPredicates {
        match self {
            Self::Direct { predicates, .. } | Self::Preaggregated { predicates, .. } => predicates,
        }
    }
}
