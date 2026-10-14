//! Reuse of optimized logical plans, physical-plan prototypes, and exact
//! outputs across repeated DataFusion evaluations.
//!
//! [`PreparedPlanCache`] retains the unresolved, optimized logical plan. Each
//! call first checks for an exact output keyed by only the parameters the plan
//! references. On a result miss, eligible placeholders are rewritten to a
//! typed execution-time relation and the generic physical plan is retained as
//! a reusable prototype. Every execution uses DataFusion's public state-reset
//! API and its own task context. Ineligible plans use the authoritative
//! literal-bound planning path.

use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use arrow::{
    datatypes::{Field, FieldRef, Schema, SchemaRef},
    record_batch::{RecordBatch, RecordBatchOptions},
};
use datafusion::{
    dataframe::DataFrame,
    datasource::{MemTable, empty::EmptyTable, provider_as_source, source_as_provider},
    logical_expr::{
        Expr, LogicalPlan, LogicalPlanBuilder, Subquery, Volatility, expr_rewriter::NamePreserver,
    },
    physical_plan::{ExecutionPlan, collect, execution_plan::reset_plan_states},
};
use datafusion_common::{
    DataFusionError, ParamValues, Result, ScalarValue, TableReference,
    metadata::FieldMetadata,
    tree_node::{Transformed, TreeNode, TreeNodeRecursion},
};
use futures::lock::Mutex as AsyncMutex;
use tracing::Instrument;

use crate::clock::{CacheClock, StdClock};
use crate::fingerprint::{
    ContentHashMemo, ReusablePrototypeNodeRejection, reusable_prototype_node_rejection,
};
use crate::reusable_planning::ReusablePlanPlanning;
use crate::runtime_parameters::{RuntimeParameterBindings, RuntimeParameterTable};

/// Configuration for optimized logical-plan reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPlanCacheConfig {
    /// Whether new calls may use the cache.
    pub enabled: bool,
    /// Maximum number of optimized logical templates retained per cache.
    pub max_logical_entries: usize,
    /// Maximum number of reusable physical prototypes retained per cache.
    pub max_physical_entries: usize,
    /// Maximum number of exact query outputs retained per cache.
    pub max_result_entries: usize,
    /// Maximum approximate Arrow allocation bytes retained by exact outputs.
    pub max_result_bytes: usize,
    /// Maximum approximate Arrow allocation bytes admitted for one output.
    pub max_result_entry_bytes: usize,
}

impl Default for PreparedPlanCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_logical_entries: 128,
            max_physical_entries: 128,
            max_result_entries: 128,
            max_result_bytes: 64 * 1024 * 1024,
            max_result_entry_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Why a call bypassed optimized logical-plan reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedPlanBypassReason {
    /// The cache was disabled through its configuration.
    Disabled,
    /// The cache capacity is zero.
    ZeroCapacity,
    /// The plan contains a stable or volatile function whose value may change
    /// between query executions.
    NonImmutableFunction,
    /// The plan is a statement, an extension, or another non-query operation.
    UnsupportedPlan,
    /// A source cannot be captured as an immutable memory snapshot.
    UnversionedSource,
    /// A supplied value needs type coercion relative to an inferred placeholder.
    ParameterTypeMismatch,
}

/// Cache disposition for one collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedPlanDisposition {
    /// An existing optimized logical template was reused.
    Hit,
    /// The unresolved logical plan was optimized and inserted.
    Miss,
    /// The ordinary `DataFrame::collect` path was used.
    Bypass(PreparedPlanBypassReason),
}

/// Why a call could not use a reusable physical-plan prototype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalPlanBypassReason {
    /// Physical-prototype capacity is zero.
    ZeroCapacity,
    /// One or more referenced placeholders have no binding.
    UnresolvedBindings,
    /// The generic runtime-parameter rewrite could not be constructed.
    RuntimeParameterRewrite,
    /// The finished plan contains an operator that is not safe to reset.
    UnsupportedPlan,
    /// The plan contains a dynamic-filter consumer that cannot be reset safely.
    DynamicFilter,
    /// The plan contains a volatile physical expression.
    VolatileExpression,
    /// The plan contains recursive-query or work-table execution.
    RecursivePlan,
    /// Prototype planning unexpectedly included physical result-cache nodes.
    ResultCacheNode,
    /// Planning the generic prototype failed; literal-bound planning remains authoritative.
    PrototypePlanning,
    /// The prototype could not be reset safely, or its previous execution
    /// did not finish.
    ResetFailure,
}

/// Reusable physical-plan disposition for one collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalPlanDisposition {
    /// An existing physical prototype was reset and executed.
    Hit,
    /// A physical prototype was planned, inserted, reset, and executed.
    Miss,
    /// The call continued through literal-bound physical planning.
    Bypass(PhysicalPlanBypassReason),
}

/// Why a call could not use exact final-output reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactResultBypassReason {
    /// The exact-result entry or total capacity is zero.
    ZeroCapacity,
    /// At least one referenced placeholder has no supplied binding.
    UnresolvedBindings,
}

/// Exact final-output cache disposition for one collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactResultDisposition {
    /// An existing final output was returned before physical planning.
    Hit,
    /// No final output existed, so this call computed it.
    Miss,
    /// Exact output reuse was not eligible for this call.
    Bypass(ExactResultBypassReason),
}

/// Stage timings and cache disposition for one collection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreparedPlanCollectMetrics {
    /// Logical-template disposition for this call. This is `None` when an
    /// exact-result hit returns before logical-template lookup.
    pub disposition: Option<PreparedPlanDisposition>,
    /// Reusable physical-prototype disposition for this call.
    pub physical_plan_disposition: Option<PhysicalPlanDisposition>,
    /// Exact final-output cache disposition for this call.
    pub exact_result_disposition: Option<ExactResultDisposition>,
    /// Time spent constructing and looking up the exact-result key.
    pub exact_result_lookup: Duration,
    /// Time spent waiting for another caller computing the same exact result.
    pub exact_result_wait: Duration,
    /// Time spent looking up the logical cache key.
    pub lookup: Duration,
    /// Time spent waiting for another caller optimizing the same logical plan.
    pub logical_wait: Duration,
    /// Time spent optimizing an unresolved logical plan on a miss.
    pub logical_optimization: Duration,
    /// Time spent substituting current parameter values.
    pub parameter_binding: Duration,
    /// Time spent looking up a reusable physical prototype.
    pub physical_plan_lookup: Duration,
    /// Time spent waiting for another caller to create the same prototype.
    pub physical_plan_wait: Duration,
    /// Time spent resetting the cached prototype for this execution.
    pub physical_plan_reset: Duration,
    /// Time spent constructing a task context with this execution's bindings.
    pub task_context_creation: Duration,
    /// Time spent creating a physical plan from the bound optimized plan.
    pub physical_planning: Duration,
    /// Time spent executing and collecting the physical plan.
    pub execution: Duration,
    /// Time spent in the complete ordinary collection path after a bypass.
    pub bypass_collect: Duration,
}

/// Record batches and diagnostics produced by a prepared-plan collection.
#[derive(Debug)]
pub struct PreparedPlanCollectOutput {
    /// Collected record batches.
    pub batches: Vec<RecordBatch>,
    /// Cache and timing diagnostics for this call.
    pub metrics: PreparedPlanCollectMetrics,
}

/// Aggregate prepared-plan and exact-result cache counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparedPlanCacheMetricsSnapshot {
    /// Successful logical-template lookups.
    pub logical_hits: u64,
    /// Logical templates optimized and inserted.
    pub logical_misses: u64,
    /// Calls that used the ordinary collection path.
    pub bypasses: u64,
    /// Logical templates removed to enforce the configured bound.
    pub logical_evictions: u64,
    /// Logical templates currently retained.
    pub logical_entries: usize,
    /// Successful reusable physical-prototype lookups.
    pub physical_hits: u64,
    /// Reusable physical prototypes planned and inserted.
    pub physical_misses: u64,
    /// Calls that used literal-bound physical planning.
    pub physical_bypasses: u64,
    /// Physical prototypes removed to enforce the configured bound.
    pub physical_evictions: u64,
    /// Physical prototypes currently retained.
    pub physical_entries: usize,
    /// Successful exact final-output lookups.
    pub exact_result_hits: u64,
    /// Exact outputs computed after a cache miss.
    pub exact_result_misses: u64,
    /// Calls that could not use exact final-output reuse.
    pub exact_result_bypasses: u64,
    /// Successful exact-output admissions.
    pub exact_result_admissions: u64,
    /// Outputs declined because one result exceeded its byte limit.
    pub exact_result_oversize_declines: u64,
    /// Exact outputs removed to enforce configured bounds.
    pub exact_result_evictions: u64,
    /// Exact outputs currently retained.
    pub exact_result_entries: usize,
    /// Approximate Arrow allocation bytes currently retained.
    pub exact_result_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LogicalTemplateKey {
    session_id: String,
    generation: u64,
    planning_profile: u64,
    plan: LogicalPlan,
    source_identities: Vec<LogicalSourceIdentity>,
}

/// Avoid repeatedly walking and hashing a complete logical plan during map
/// and gate lookups. Equality remains semantic, so collisions are harmless.
#[derive(Clone, Debug)]
struct PrehashedLogicalTemplateKey {
    hash: u64,
    key: LogicalTemplateKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PhysicalTemplateKey {
    logical: Arc<PrehashedLogicalTemplateKey>,
    parameter_schema: SchemaRef,
}

#[derive(Clone, Debug)]
struct RuntimeParameterSlot {
    id: String,
    field: FieldRef,
}

#[derive(Debug)]
struct ReusablePhysicalPrototype {
    prototype: Arc<dyn ExecutionPlan>,
    execution_gate: AsyncMutex<bool>,
}

enum PhysicalPrototypeAttemptError {
    Fallback(PhysicalPlanBypassReason),
    Execution(DataFusionError),
}

impl ReusablePhysicalPrototype {
    fn new(prototype: Arc<dyn ExecutionPlan>) -> Self {
        Self {
            prototype,
            execution_gate: AsyncMutex::new(true),
        }
    }

    /// Produce the only plan instance callers may execute. The cached
    /// prototype itself never leaves this adapter.
    fn new_execution(&self) -> Result<Arc<dyn ExecutionPlan>> {
        reset_plan_states(Arc::clone(&self.prototype))
    }
}

impl PrehashedLogicalTemplateKey {
    fn new(key: LogicalTemplateKey) -> Self {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        Self {
            hash: hasher.finish(),
            key,
        }
    }
}

impl PartialEq for PrehashedLogicalTemplateKey {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for PrehashedLogicalTemplateKey {}

impl Hash for PrehashedLogicalTemplateKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ReferencedBinding {
    id: String,
    value: ScalarValue,
    metadata: Option<FieldMetadata>,
}

#[derive(Clone, Debug)]
struct ExactResultKey {
    hash: u64,
    logical: Arc<PrehashedLogicalTemplateKey>,
    bindings: Vec<ReferencedBinding>,
}

impl ExactResultKey {
    fn new(logical: Arc<PrehashedLogicalTemplateKey>, bindings: Vec<ReferencedBinding>) -> Self {
        let mut hasher = DefaultHasher::new();
        logical.hash(&mut hasher);
        bindings.hash(&mut hasher);
        Self {
            hash: hasher.finish(),
            logical,
            bindings,
        }
    }
}

impl PartialEq for ExactResultKey {
    fn eq(&self, other: &Self) -> bool {
        self.logical == other.logical && self.bindings == other.bindings
    }
}

impl Eq for ExactResultKey {}

impl Hash for ExactResultKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LogicalSourceIdentity {
    EmptySchema(u64),
    MemoryContent(u128, u64),
}

#[derive(Debug)]
struct LogicalTemplateEntry {
    plan: Arc<LogicalPlan>,
    last_access: u64,
}

#[derive(Debug)]
struct ExactResultEntry {
    batches: Arc<[RecordBatch]>,
    bytes: usize,
    last_access: u64,
}

#[derive(Debug)]
struct PhysicalTemplateEntry {
    prototype: Arc<ReusablePhysicalPrototype>,
    last_access: u64,
}

#[derive(Debug, Default)]
struct PreparedPlanCacheState {
    generation: u64,
    logical_entries: HashMap<Arc<PrehashedLogicalTemplateKey>, LogicalTemplateEntry>,
    physical_entries: HashMap<PhysicalTemplateKey, PhysicalTemplateEntry>,
    exact_results: HashMap<ExactResultKey, ExactResultEntry>,
    access_clock: u64,
    logical_hits: u64,
    logical_misses: u64,
    bypasses: u64,
    logical_evictions: u64,
    physical_hits: u64,
    physical_misses: u64,
    physical_bypasses: u64,
    physical_evictions: u64,
    exact_result_hits: u64,
    exact_result_misses: u64,
    exact_result_bypasses: u64,
    exact_result_admissions: u64,
    exact_result_oversize_declines: u64,
    exact_result_evictions: u64,
    exact_result_bytes: usize,
}

/// A bounded cache of unresolved logical templates, reusable physical-plan
/// prototypes, and exact query outputs.
///
/// A cache is intended to live for one application/session generation. Cache
/// keys include session configuration and captured table contents because
/// DataFusion omits `TableSource` from `LogicalPlan` equality and hashing.
#[derive(Debug)]
pub struct PreparedPlanCache {
    config: PreparedPlanCacheConfig,
    clock: Arc<dyn CacheClock>,
    content_hash_memo: Arc<ContentHashMemo>,
    state: Mutex<PreparedPlanCacheState>,
    logical_gates: Mutex<HashMap<u64, Weak<AsyncMutex<()>>>>,
    physical_gates: Mutex<HashMap<u64, Weak<AsyncMutex<()>>>>,
    exact_result_gates: Mutex<HashMap<u64, Weak<AsyncMutex<()>>>>,
}

impl PreparedPlanCache {
    /// Create a prepared-plan cache with the supplied configuration.
    pub fn new(config: PreparedPlanCacheConfig) -> Arc<Self> {
        Self::with_clock(config, Arc::new(StdClock::new()))
    }

    /// Create a cache with an injected monotonic clock.
    pub fn with_clock(config: PreparedPlanCacheConfig, clock: Arc<dyn CacheClock>) -> Arc<Self> {
        Arc::new(Self {
            config,
            clock,
            content_hash_memo: Arc::new(ContentHashMemo::default()),
            state: Mutex::new(PreparedPlanCacheState::default()),
            logical_gates: Mutex::new(HashMap::new()),
            physical_gates: Mutex::new(HashMap::new()),
            exact_result_gates: Mutex::new(HashMap::new()),
        })
    }

    /// Return the configured policy.
    pub fn config(&self) -> &PreparedPlanCacheConfig {
        &self.config
    }

    /// Return cumulative cache counters and current entry count.
    pub fn metrics(&self) -> PreparedPlanCacheMetricsSnapshot {
        let state = self.state.lock().unwrap();
        PreparedPlanCacheMetricsSnapshot {
            logical_hits: state.logical_hits,
            logical_misses: state.logical_misses,
            bypasses: state.bypasses,
            logical_evictions: state.logical_evictions,
            logical_entries: state.logical_entries.len(),
            physical_hits: state.physical_hits,
            physical_misses: state.physical_misses,
            physical_bypasses: state.physical_bypasses,
            physical_evictions: state.physical_evictions,
            physical_entries: state.physical_entries.len(),
            exact_result_hits: state.exact_result_hits,
            exact_result_misses: state.exact_result_misses,
            exact_result_bypasses: state.exact_result_bypasses,
            exact_result_admissions: state.exact_result_admissions,
            exact_result_oversize_declines: state.exact_result_oversize_declines,
            exact_result_evictions: state.exact_result_evictions,
            exact_result_entries: state.exact_results.len(),
            exact_result_bytes: state.exact_result_bytes,
        }
    }

    /// Remove all retained logical templates and exact outputs without
    /// changing cumulative counters.
    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.generation += 1;
        state.logical_entries.clear();
        state.physical_entries.clear();
        state.exact_results.clear();
        state.exact_result_bytes = 0;
    }

    /// Collect a DataFrame through the deepest safe reusable layer.
    ///
    /// Exact-output hits return before parameter binding and physical planning.
    /// Otherwise, eligible queries execute a fresh reset clone of a retained
    /// physical prototype with typed runtime bindings. Ineligible queries fall
    /// back to literal binding and specialized physical planning from the
    /// retained optimized logical template.
    pub async fn collect(
        &self,
        dataframe: DataFrame,
        params: Option<ParamValues>,
    ) -> Result<PreparedPlanCollectOutput> {
        let bypass_reason = if !self.config.enabled {
            Some(PreparedPlanBypassReason::Disabled)
        } else if self.config.max_logical_entries == 0 {
            Some(PreparedPlanBypassReason::ZeroCapacity)
        } else if !is_reusable_query(dataframe.logical_plan())? {
            Some(PreparedPlanBypassReason::UnsupportedPlan)
        } else if !logical_plan_is_immutable(dataframe.logical_plan())? {
            Some(PreparedPlanBypassReason::NonImmutableFunction)
        } else if parameter_types_change(dataframe.logical_plan(), params.as_ref())? {
            Some(PreparedPlanBypassReason::ParameterTypeMismatch)
        } else {
            None
        };

        if let Some(reason) = bypass_reason {
            return self.collect_uncached(dataframe, params, reason).await;
        }

        let (session_state, unresolved_plan) = dataframe.into_parts();
        let generation = self.state.lock().unwrap().generation;
        let exact_lookup_started = self.clock.now();
        let Some((unresolved_plan, source_identities)) =
            snapshot_sources(&unresolved_plan, self.content_hash_memo.as_ref())?
        else {
            return self
                .collect_uncached(
                    DataFrame::new(session_state, unresolved_plan),
                    params,
                    PreparedPlanBypassReason::UnversionedSource,
                )
                .await;
        };
        let logical_key = Arc::new(PrehashedLogicalTemplateKey::new(LogicalTemplateKey {
            session_id: session_state.session_id().to_string(),
            generation,
            planning_profile: planning_profile(&session_state),
            source_identities,
            plan: unresolved_plan.clone(),
        }));

        let exact_cache_enabled = self.config.max_result_entries > 0
            && self.config.max_result_bytes > 0
            && self.config.max_result_entry_bytes > 0;
        let exact_key = if exact_cache_enabled {
            referenced_bindings(&unresolved_plan, params.as_ref())?
                .map(|bindings| ExactResultKey::new(Arc::clone(&logical_key), bindings))
        } else {
            None
        };
        let mut metrics = PreparedPlanCollectMetrics {
            exact_result_lookup: elapsed(self.clock.as_ref(), exact_lookup_started),
            ..Default::default()
        };

        if !exact_cache_enabled {
            self.record_exact_bypass();
            metrics.exact_result_disposition = Some(ExactResultDisposition::Bypass(
                ExactResultBypassReason::ZeroCapacity,
            ));
        } else if exact_key.is_none() {
            self.record_exact_bypass();
            metrics.exact_result_disposition = Some(ExactResultDisposition::Bypass(
                ExactResultBypassReason::UnresolvedBindings,
            ));
        }

        if let Some(key) = exact_key.as_ref() {
            let lookup_started = self.clock.now();
            if let Some(batches) = self.lookup_exact_result(key) {
                metrics.exact_result_lookup += elapsed(self.clock.as_ref(), lookup_started);
                metrics.exact_result_disposition = Some(ExactResultDisposition::Hit);
                return Ok(PreparedPlanCollectOutput { batches, metrics });
            }
            metrics.exact_result_lookup += elapsed(self.clock.as_ref(), lookup_started);
        }

        // Only misses take a gate. A second lookup after acquiring it makes
        // both result execution and logical optimization single-flight without
        // keeping the cache-wide state mutex across expensive work.
        let exact_gate = exact_key
            .as_ref()
            .map(|key| gate_for(&self.exact_result_gates, key.hash));
        let exact_wait_started = self.clock.now();
        let _exact_guard = if let Some(gate) = exact_gate.as_ref() {
            Some(
                gate.lock()
                    .instrument(tracing::trace_span!("prepared_exact_result_wait"))
                    .await,
            )
        } else {
            None
        };
        if exact_gate.is_some() {
            metrics.exact_result_wait = elapsed(self.clock.as_ref(), exact_wait_started);
        }
        if let Some(key) = exact_key.as_ref() {
            let lookup_started = self.clock.now();
            if let Some(batches) = self.lookup_exact_result(key) {
                metrics.exact_result_lookup += elapsed(self.clock.as_ref(), lookup_started);
                metrics.exact_result_disposition = Some(ExactResultDisposition::Hit);
                return Ok(PreparedPlanCollectOutput { batches, metrics });
            }
            metrics.exact_result_lookup += elapsed(self.clock.as_ref(), lookup_started);
            self.record_exact_miss();
            metrics.exact_result_disposition = Some(ExactResultDisposition::Miss);
        }

        let logical_lookup_started = self.clock.now();
        let initial_logical_hit = {
            let _span =
                tracing::trace_span!("prepared_logical_template_lookup", key = logical_key.hash)
                    .entered();
            self.lookup_logical_template(&logical_key)
        };
        metrics.lookup += elapsed(self.clock.as_ref(), logical_lookup_started);

        let (optimized_plan, disposition) = if let Some(plan) = initial_logical_hit {
            (plan, PreparedPlanDisposition::Hit)
        } else {
            let logical_gate = gate_for(&self.logical_gates, logical_key.hash);
            let wait_started = self.clock.now();
            let _logical_guard = logical_gate
                .lock()
                .instrument(tracing::trace_span!(
                    "prepared_logical_template_wait",
                    key = logical_key.hash
                ))
                .await;
            metrics.logical_wait = elapsed(self.clock.as_ref(), wait_started);

            let lookup_started = self.clock.now();
            let after_wait = self.lookup_logical_template(&logical_key);
            metrics.lookup += elapsed(self.clock.as_ref(), lookup_started);
            if let Some(plan) = after_wait {
                (plan, PreparedPlanDisposition::Hit)
            } else {
                let optimize_started = self.clock.now();
                let plan = {
                    let _span = tracing::trace_span!(
                        "prepared_logical_optimization",
                        key = logical_key.hash
                    )
                    .entered();
                    Arc::new(session_state.optimize(&unresolved_plan)?)
                };
                metrics.logical_optimization = elapsed(self.clock.as_ref(), optimize_started);
                let bypass = if !logical_plan_is_immutable(&plan)? {
                    Some(PreparedPlanBypassReason::NonImmutableFunction)
                } else if parameter_types_change(&plan, params.as_ref())? {
                    Some(PreparedPlanBypassReason::ParameterTypeMismatch)
                } else {
                    None
                };
                if let Some(reason) = bypass {
                    return self
                        .collect_uncached(
                            DataFrame::new(session_state, unresolved_plan),
                            params,
                            reason,
                        )
                        .await;
                }
                self.insert_logical_template(Arc::clone(&logical_key), Arc::clone(&plan));
                (plan, PreparedPlanDisposition::Miss)
            }
        };
        metrics.disposition = Some(disposition);
        if parameter_types_change(&optimized_plan, params.as_ref())? {
            return self
                .collect_uncached(
                    DataFrame::new(session_state, unresolved_plan),
                    params,
                    PreparedPlanBypassReason::ParameterTypeMismatch,
                )
                .await;
        }

        let batches = if self.config.max_physical_entries == 0 {
            self.record_physical_bypass();
            metrics.physical_plan_disposition = Some(PhysicalPlanDisposition::Bypass(
                PhysicalPlanBypassReason::ZeroCapacity,
            ));
            self.collect_with_specialized_plan(
                optimized_plan.as_ref(),
                params,
                &session_state,
                &mut metrics,
            )
            .await?
        } else {
            let runtime_plan = {
                let _span = tracing::trace_span!("prepared_runtime_parameter_binding").entered();
                prepare_runtime_parameter_plan(optimized_plan.as_ref(), params.as_ref())
            };
            match runtime_plan {
                Ok(Some(runtime)) => match self
                    .collect_with_physical_prototype(
                        Arc::clone(&logical_key),
                        runtime,
                        &session_state,
                        &mut metrics,
                    )
                    .await
                {
                    Ok(batches) => batches,
                    Err(PhysicalPrototypeAttemptError::Fallback(reason)) => {
                        self.record_physical_bypass();
                        metrics.physical_plan_disposition =
                            Some(PhysicalPlanDisposition::Bypass(reason));
                        self.collect_with_specialized_plan(
                            optimized_plan.as_ref(),
                            params,
                            &session_state,
                            &mut metrics,
                        )
                        .await?
                    }
                    Err(PhysicalPrototypeAttemptError::Execution(error)) => return Err(error),
                },
                Ok(None) => {
                    self.record_physical_bypass();
                    metrics.physical_plan_disposition = Some(PhysicalPlanDisposition::Bypass(
                        PhysicalPlanBypassReason::UnresolvedBindings,
                    ));
                    self.collect_with_specialized_plan(
                        optimized_plan.as_ref(),
                        params,
                        &session_state,
                        &mut metrics,
                    )
                    .await?
                }
                Err(_) => {
                    self.record_physical_bypass();
                    metrics.physical_plan_disposition = Some(PhysicalPlanDisposition::Bypass(
                        PhysicalPlanBypassReason::RuntimeParameterRewrite,
                    ));
                    self.collect_with_specialized_plan(
                        optimized_plan.as_ref(),
                        params,
                        &session_state,
                        &mut metrics,
                    )
                    .await?
                }
            }
        };

        if let Some(key) = exact_key {
            self.insert_exact_result(key, &batches);
        }

        Ok(PreparedPlanCollectOutput { batches, metrics })
    }

    async fn collect_uncached(
        &self,
        dataframe: DataFrame,
        params: Option<ParamValues>,
        reason: PreparedPlanBypassReason,
    ) -> Result<PreparedPlanCollectOutput> {
        self.state.lock().unwrap().bypasses += 1;
        let started = self.clock.now();
        let dataframe = if let Some(params) = params {
            dataframe.with_param_values(params)?
        } else {
            dataframe
        };
        Ok(PreparedPlanCollectOutput {
            batches: dataframe.collect().await?,
            metrics: PreparedPlanCollectMetrics {
                disposition: Some(PreparedPlanDisposition::Bypass(reason)),
                bypass_collect: elapsed(self.clock.as_ref(), started),
                ..Default::default()
            },
        })
    }

    async fn collect_with_specialized_plan(
        &self,
        optimized_plan: &LogicalPlan,
        params: Option<ParamValues>,
        session_state: &datafusion::execution::SessionState,
        metrics: &mut PreparedPlanCollectMetrics,
    ) -> Result<Vec<RecordBatch>> {
        let bind_started = self.clock.now();
        let bound_plan = {
            let _span = tracing::trace_span!("prepared_literal_parameter_binding").entered();
            if let Some(params) = params {
                optimized_plan.clone().with_param_values(params)?
            } else {
                optimized_plan.clone()
            }
        };
        metrics.parameter_binding += elapsed(self.clock.as_ref(), bind_started);

        let physical_started = self.clock.now();
        let physical_plan = session_state
            .query_planner()
            .create_physical_plan(&bound_plan, session_state)
            .await?;
        metrics.physical_planning += elapsed(self.clock.as_ref(), physical_started);

        let execution_started = self.clock.now();
        let batches = collect(physical_plan, session_state.task_ctx()).await?;
        metrics.execution += elapsed(self.clock.as_ref(), execution_started);
        Ok(batches)
    }

    async fn collect_with_physical_prototype(
        &self,
        logical_key: Arc<PrehashedLogicalTemplateKey>,
        runtime: PreparedRuntimeParameters,
        session_state: &datafusion::execution::SessionState,
        metrics: &mut PreparedPlanCollectMetrics,
    ) -> std::result::Result<Vec<RecordBatch>, PhysicalPrototypeAttemptError> {
        let key = PhysicalTemplateKey {
            logical: logical_key,
            parameter_schema: runtime.bindings.schema(),
        };
        let key_hash = hash_value(&key);

        let lookup_started = self.clock.now();
        let initial = {
            let _span = tracing::trace_span!("prepared_physical_prototype_lookup", key = key_hash)
                .entered();
            self.lookup_physical_template(&key)
        };
        metrics.physical_plan_lookup += elapsed(self.clock.as_ref(), lookup_started);

        let (prototype, disposition) = if let Some(prototype) = initial {
            (prototype, PhysicalPlanDisposition::Hit)
        } else {
            let gate = gate_for(&self.physical_gates, key_hash);
            let wait_started = self.clock.now();
            let _guard = gate
                .lock()
                .instrument(tracing::trace_span!(
                    "prepared_physical_prototype_wait",
                    key = key_hash
                ))
                .await;
            metrics.physical_plan_wait += elapsed(self.clock.as_ref(), wait_started);

            let lookup_started = self.clock.now();
            let after_wait = self.lookup_physical_template(&key);
            metrics.physical_plan_lookup += elapsed(self.clock.as_ref(), lookup_started);
            if let Some(prototype) = after_wait {
                (prototype, PhysicalPlanDisposition::Hit)
            } else {
                let physical_started = self.clock.now();
                let mut prototype_state = session_state.clone();
                configure_reusable_planning(&mut prototype_state);
                let physical = prototype_state
                    .query_planner()
                    .create_physical_plan(&runtime.plan, &prototype_state)
                    .instrument(tracing::trace_span!(
                        "prepared_physical_prototype_planning",
                        key = key_hash
                    ))
                    .await
                    .map_err(|_| {
                        PhysicalPrototypeAttemptError::Fallback(
                            PhysicalPlanBypassReason::PrototypePlanning,
                        )
                    })?;
                metrics.physical_planning += elapsed(self.clock.as_ref(), physical_started);
                if let Some(reason) = physical_plan_reuse_rejection(&physical).map_err(|_| {
                    PhysicalPrototypeAttemptError::Fallback(
                        PhysicalPlanBypassReason::UnsupportedPlan,
                    )
                })? {
                    return Err(PhysicalPrototypeAttemptError::Fallback(reason));
                }
                let prototype = Arc::new(ReusablePhysicalPrototype::new(physical));
                self.insert_physical_template(key.clone(), Arc::clone(&prototype));
                (prototype, PhysicalPlanDisposition::Miss)
            }
        };
        metrics.physical_plan_disposition = Some(disposition);

        // DataFusion's scalar-subquery reset clears a shared results
        // container rather than allocating an independent one. Hold the
        // prototype lease through execution so two callers can never reset or
        // populate that container concurrently. Different prototype keys
        // remain fully independent and may execute in parallel.
        let execution_wait_started = self.clock.now();
        let mut execution_guard = prototype
            .execution_gate
            .lock()
            .instrument(tracing::trace_span!(
                "prepared_physical_execution_wait",
                key = key_hash
            ))
            .await;
        metrics.physical_plan_wait += elapsed(self.clock.as_ref(), execution_wait_started);

        // Cancellation drops the lease while leaving it invalid. DataFusion
        // may still be aborting tasks that reference the shared subquery state.
        if !*execution_guard {
            self.remove_physical_template(&key);
            return Err(PhysicalPrototypeAttemptError::Fallback(
                PhysicalPlanBypassReason::ResetFailure,
            ));
        }
        *execution_guard = false;
        let reset_started = self.clock.now();
        let execution = {
            let _span =
                tracing::trace_span!("prepared_physical_plan_reset", key = key_hash).entered();
            prototype.new_execution().map_err(|_| {
                self.remove_physical_template(&key);
                PhysicalPrototypeAttemptError::Fallback(PhysicalPlanBypassReason::ResetFailure)
            })?
        };
        metrics.physical_plan_reset += elapsed(self.clock.as_ref(), reset_started);

        let result = self
            .collect_runtime_physical(execution, runtime.bindings, session_state, metrics)
            .await;
        if result.is_ok() {
            *execution_guard = true;
        } else {
            self.remove_physical_template(&key);
        }
        result
    }

    async fn collect_runtime_physical(
        &self,
        execution: Arc<dyn ExecutionPlan>,
        bindings: RuntimeParameterBindings,
        session_state: &datafusion::execution::SessionState,
        metrics: &mut PreparedPlanCollectMetrics,
    ) -> std::result::Result<Vec<RecordBatch>, PhysicalPrototypeAttemptError> {
        let context_started = self.clock.now();
        let mut execution_state = session_state.clone();
        execution_state
            .config_mut()
            .set_extension(Arc::new(bindings));
        let task_context = execution_state.task_ctx();
        metrics.task_context_creation += elapsed(self.clock.as_ref(), context_started);

        let execution_started = self.clock.now();
        let batches = collect(execution, task_context)
            .instrument(tracing::trace_span!("prepared_physical_execution"))
            .await
            .map_err(PhysicalPrototypeAttemptError::Execution)?;
        metrics.execution += elapsed(self.clock.as_ref(), execution_started);
        Ok(batches)
    }

    fn lookup_logical_template(
        &self,
        key: &Arc<PrehashedLogicalTemplateKey>,
    ) -> Option<Arc<LogicalPlan>> {
        let mut state = self.state.lock().unwrap();
        let access = next_access(&mut state);
        let plan = state.logical_entries.get_mut(key).map(|entry| {
            entry.last_access = access;
            Arc::clone(&entry.plan)
        });
        if plan.is_some() {
            state.logical_hits += 1;
        }
        plan
    }

    fn insert_logical_template(
        &self,
        key: Arc<PrehashedLogicalTemplateKey>,
        plan: Arc<LogicalPlan>,
    ) {
        let mut state = self.state.lock().unwrap();
        if key.key.generation != state.generation {
            return;
        }
        let access = next_access(&mut state);
        state.logical_misses += 1;
        state.logical_entries.insert(
            key,
            LogicalTemplateEntry {
                plan,
                last_access: access,
            },
        );
        while state.logical_entries.len() > self.config.max_logical_entries {
            let oldest = state
                .logical_entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(key, _)| Arc::clone(key));
            if let Some(oldest) = oldest {
                state.logical_entries.remove(&oldest);
                state.logical_evictions += 1;
            }
        }
    }

    fn lookup_physical_template(
        &self,
        key: &PhysicalTemplateKey,
    ) -> Option<Arc<ReusablePhysicalPrototype>> {
        let mut state = self.state.lock().unwrap();
        let access = next_access(&mut state);
        let prototype = state.physical_entries.get_mut(key).map(|entry| {
            entry.last_access = access;
            Arc::clone(&entry.prototype)
        });
        if prototype.is_some() {
            state.physical_hits += 1;
        }
        prototype
    }

    fn insert_physical_template(
        &self,
        key: PhysicalTemplateKey,
        prototype: Arc<ReusablePhysicalPrototype>,
    ) {
        let mut state = self.state.lock().unwrap();
        if key.logical.key.generation != state.generation {
            return;
        }
        let access = next_access(&mut state);
        state.physical_misses += 1;
        state.physical_entries.insert(
            key,
            PhysicalTemplateEntry {
                prototype,
                last_access: access,
            },
        );
        while state.physical_entries.len() > self.config.max_physical_entries {
            let oldest = state
                .physical_entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                state.physical_entries.remove(&oldest);
                state.physical_evictions += 1;
            }
        }
    }

    fn remove_physical_template(&self, key: &PhysicalTemplateKey) {
        self.state.lock().unwrap().physical_entries.remove(key);
    }

    fn lookup_exact_result(&self, key: &ExactResultKey) -> Option<Vec<RecordBatch>> {
        let mut state = self.state.lock().unwrap();
        let access = next_access(&mut state);
        let batches = state.exact_results.get_mut(key).map(|entry| {
            entry.last_access = access;
            entry.batches.as_ref().to_vec()
        });
        if batches.is_some() {
            state.exact_result_hits += 1;
        }
        batches
    }

    fn insert_exact_result(&self, key: ExactResultKey, batches: &[RecordBatch]) {
        let bytes = batches.iter().fold(0usize, |total, batch| {
            total.saturating_add(batch.get_array_memory_size())
        });
        let mut state = self.state.lock().unwrap();
        if key.logical.key.generation != state.generation {
            return;
        }
        if bytes > self.config.max_result_entry_bytes || bytes > self.config.max_result_bytes {
            state.exact_result_oversize_declines += 1;
            return;
        }

        let access = next_access(&mut state);
        if let Some(replaced) = state.exact_results.insert(
            key,
            ExactResultEntry {
                batches: Arc::from(batches.to_vec()),
                bytes,
                last_access: access,
            },
        ) {
            state.exact_result_bytes = state.exact_result_bytes.saturating_sub(replaced.bytes);
        }
        state.exact_result_bytes = state.exact_result_bytes.saturating_add(bytes);
        state.exact_result_admissions += 1;

        while state.exact_results.len() > self.config.max_result_entries
            || state.exact_result_bytes > self.config.max_result_bytes
        {
            let oldest = state
                .exact_results
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                if let Some(removed) = state.exact_results.remove(&oldest) {
                    state.exact_result_bytes =
                        state.exact_result_bytes.saturating_sub(removed.bytes);
                    state.exact_result_evictions += 1;
                }
            } else {
                break;
            }
        }
    }

    fn record_exact_miss(&self) {
        self.state.lock().unwrap().exact_result_misses += 1;
    }

    fn record_exact_bypass(&self) {
        self.state.lock().unwrap().exact_result_bypasses += 1;
    }

    fn record_physical_bypass(&self) {
        self.state.lock().unwrap().physical_bypasses += 1;
    }
}

fn gate_for(gates: &Mutex<HashMap<u64, Weak<AsyncMutex<()>>>>, hash: u64) -> Arc<AsyncMutex<()>> {
    let mut gates = gates.lock().unwrap();
    gates.retain(|_, gate| gate.strong_count() > 0);
    if let Some(gate) = gates.get(&hash).and_then(Weak::upgrade) {
        gate
    } else {
        let gate = Arc::new(AsyncMutex::new(()));
        gates.insert(hash, Arc::downgrade(&gate));
        gate
    }
}

fn next_access(state: &mut PreparedPlanCacheState) -> u64 {
    state.access_clock = state.access_clock.wrapping_add(1);
    if state.access_clock == 0 {
        for entry in state.logical_entries.values_mut() {
            entry.last_access = 0;
        }
        for entry in state.exact_results.values_mut() {
            entry.last_access = 0;
        }
        for entry in state.physical_entries.values_mut() {
            entry.last_access = 0;
        }
        state.access_clock = 1;
    }
    state.access_clock
}

fn elapsed(clock: &dyn CacheClock, started: Duration) -> Duration {
    clock.now().saturating_sub(started)
}

#[derive(Debug)]
struct PreparedRuntimeParameters {
    plan: LogicalPlan,
    bindings: RuntimeParameterBindings,
}

fn prepare_runtime_parameter_plan(
    plan: &LogicalPlan,
    params: Option<&ParamValues>,
) -> Result<Option<PreparedRuntimeParameters>> {
    let Some(bindings) = referenced_bindings(plan, params)? else {
        return Ok(None);
    };
    if bindings.iter().any(|binding| {
        binding
            .metadata
            .as_ref()
            .is_some_and(|metadata| !metadata.inner().is_empty())
    }) {
        // DataFusion 54 drops field metadata when lowering scalar subqueries.
        return datafusion_common::not_impl_err!(
            "runtime parameter subqueries cannot preserve field metadata"
        );
    }
    let binding_by_id = bindings
        .into_iter()
        .map(|binding| (binding.id.clone(), binding))
        .collect::<HashMap<_, _>>();

    let mut slots = Vec::<RuntimeParameterSlot>::new();
    let mut slot_by_id = HashMap::<String, usize>::new();
    plan.apply_with_subqueries(|node| {
        for expression in node.expressions() {
            expression.apply(|candidate| {
                if let Expr::Placeholder(placeholder) = candidate
                    && !slot_by_id.contains_key(&placeholder.id)
                {
                    let binding = binding_by_id.get(&placeholder.id).ok_or_else(|| {
                        datafusion_common::DataFusionError::Plan(format!(
                            "no value provided for runtime parameter {}",
                            placeholder.id
                        ))
                    })?;
                    let data_type = placeholder
                        .field
                        .as_ref()
                        .map(|field| field.data_type().clone())
                        .unwrap_or_else(|| binding.value.data_type());
                    let index = slots.len();
                    slot_by_id.insert(placeholder.id.clone(), index);
                    slots.push(RuntimeParameterSlot {
                        id: placeholder.id.clone(),
                        field: Arc::new(Field::new(
                            format!("__avenger_runtime_parameter_{index}"),
                            data_type,
                            true,
                        )),
                    });
                }
                Ok(TreeNodeRecursion::Continue)
            })?;
        }
        Ok(TreeNodeRecursion::Continue)
    })?;

    let schema = Arc::new(Schema::new(
        slots
            .iter()
            .map(|slot| Arc::clone(&slot.field))
            .collect::<Vec<_>>(),
    ));
    let values = slots
        .iter()
        .map(|slot| binding_by_id[&slot.id].value.to_array())
        .collect::<Result<Vec<_>>>()?;
    let batch = RecordBatch::try_new_with_options(
        Arc::clone(&schema),
        values,
        &RecordBatchOptions::new().with_row_count(Some(1)),
    )?;
    let runtime_bindings = RuntimeParameterBindings::new(batch);

    if slots.is_empty() {
        return Ok(Some(PreparedRuntimeParameters {
            plan: plan.clone(),
            bindings: runtime_bindings,
        }));
    }

    let provider = Arc::new(RuntimeParameterTable::new(Arc::clone(&schema)));
    let replacements = slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let scan = LogicalPlanBuilder::scan(
                TableReference::bare("__avenger_runtime_parameters"),
                provider_as_source(
                    Arc::clone(&provider) as Arc<dyn datafusion::catalog::TableProvider>
                ),
                Some(vec![index]),
            )?
            .build()?;
            let expression = Expr::ScalarSubquery(Subquery {
                subquery: Arc::new(scan),
                outer_ref_columns: Vec::new(),
                spans: Default::default(),
            });
            Ok((slot.id.clone(), expression))
        })
        .collect::<Result<HashMap<_, _>>>()?;

    let plan = plan
        .clone()
        .transform_up_with_subqueries(|plan| {
            let name_preserver = NamePreserver::new(&plan);
            plan.map_expressions(|expression| {
                let original_name = name_preserver.save(&expression);
                expression
                    .transform_up(|candidate| {
                        if let Expr::Placeholder(placeholder) = &candidate {
                            Ok(Transformed::yes(replacements[&placeholder.id].clone()))
                        } else {
                            Ok(Transformed::no(candidate))
                        }
                    })
                    .map(|transformed| {
                        transformed.update_data(|expression| original_name.restore(expression))
                    })
            })?
            .map_data(|plan| match plan {
                LogicalPlan::Values(values) => LogicalPlanBuilder::values(values.values)?.build(),
                plan => plan.recompute_schema(),
            })
        })?
        .data;

    Ok(Some(PreparedRuntimeParameters {
        plan,
        bindings: runtime_bindings,
    }))
}

fn configure_reusable_planning(state: &mut datafusion::execution::SessionState) {
    let options = state.config_mut().options_mut();
    options.optimizer.enable_dynamic_filter_pushdown = false;
    options.optimizer.enable_topk_dynamic_filter_pushdown = false;
    options.optimizer.enable_join_dynamic_filter_pushdown = false;
    options.optimizer.enable_aggregate_dynamic_filter_pushdown = false;
    options
        .extensions
        .insert(ReusablePlanPlanning { enabled: true });
}

fn planning_profile(state: &datafusion::execution::SessionState) -> u64 {
    let mut hasher = DefaultHasher::new();
    1_u32.hash(&mut hasher);
    datafusion::DATAFUSION_VERSION.hash(&mut hasher);

    let mut entries = state.config_options().entries();
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    entries.hash(&mut hasher);
    state
        .physical_optimizers()
        .iter()
        .map(|optimizer| Arc::as_ptr(optimizer) as *const () as usize)
        .collect::<Vec<_>>()
        .hash(&mut hasher);
    for rule in &state.analyzer().rules {
        (Arc::as_ptr(rule) as *const () as usize).hash(&mut hasher);
    }
    for rule in state.optimizers() {
        (Arc::as_ptr(rule) as *const () as usize).hash(&mut hasher);
    }
    (Arc::as_ptr(state.query_planner()) as *const () as usize).hash(&mut hasher);
    (Arc::as_ptr(state.runtime_env()) as usize).hash(&mut hasher);

    let mut scalar = state.scalar_functions().iter().collect::<Vec<_>>();
    scalar.sort_by_key(|(name, _)| *name);
    for (name, function) in scalar {
        name.hash(&mut hasher);
        (Arc::as_ptr(function) as *const () as usize).hash(&mut hasher);
    }
    let mut higher = state.higher_order_functions().iter().collect::<Vec<_>>();
    higher.sort_by_key(|(name, _)| *name);
    for (name, function) in higher {
        name.hash(&mut hasher);
        (Arc::as_ptr(function) as *const () as usize).hash(&mut hasher);
    }
    let mut aggregate = state.aggregate_functions().iter().collect::<Vec<_>>();
    aggregate.sort_by_key(|(name, _)| *name);
    for (name, function) in aggregate {
        name.hash(&mut hasher);
        (Arc::as_ptr(function) as *const () as usize).hash(&mut hasher);
    }
    let mut window = state.window_functions().iter().collect::<Vec<_>>();
    window.sort_by_key(|(name, _)| *name);
    for (name, function) in window {
        name.hash(&mut hasher);
        (Arc::as_ptr(function) as *const () as usize).hash(&mut hasher);
    }
    hasher.finish()
}

fn physical_plan_reuse_rejection(
    plan: &Arc<dyn ExecutionPlan>,
) -> Result<Option<PhysicalPlanBypassReason>> {
    let mut rejection = None;
    plan.apply(|node| {
        let name = node.name();
        rejection = if matches!(name, "RecursiveQueryExec" | "WorkTableExec") {
            Some(PhysicalPlanBypassReason::RecursivePlan)
        } else if matches!(name, "CacheReadExec" | "CacheWriteExec") {
            Some(PhysicalPlanBypassReason::ResultCacheNode)
        } else {
            reusable_prototype_node_rejection(node).map(|reason| match reason {
                ReusablePrototypeNodeRejection::DynamicExpression => {
                    PhysicalPlanBypassReason::DynamicFilter
                }
                ReusablePrototypeNodeRejection::VolatileExpression => {
                    PhysicalPlanBypassReason::VolatileExpression
                }
                ReusablePrototypeNodeRejection::UnsupportedNode => {
                    PhysicalPlanBypassReason::UnsupportedPlan
                }
            })
        };
        if rejection.is_some() {
            Ok(TreeNodeRecursion::Stop)
        } else {
            Ok(TreeNodeRecursion::Continue)
        }
    })?;
    Ok(rejection)
}

fn hash_value(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn referenced_bindings(
    plan: &LogicalPlan,
    params: Option<&ParamValues>,
) -> Result<Option<Vec<ReferencedBinding>>> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    plan.apply_with_subqueries(|node| {
        for expression in node.expressions() {
            expression.apply(|candidate| {
                if let Expr::Placeholder(placeholder) = candidate
                    && seen.insert(placeholder.id.clone())
                {
                    ids.push(placeholder.id.clone());
                }
                Ok(TreeNodeRecursion::Continue)
            })?;
        }
        Ok(TreeNodeRecursion::Continue)
    })?;

    if ids.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let Some(params) = params else {
        return Ok(None);
    };
    let mut bindings = Vec::with_capacity(ids.len());
    for id in ids {
        let Ok(binding) = params.get_placeholders_with_values(&id) else {
            return Ok(None);
        };
        bindings.push(ReferencedBinding {
            id,
            value: binding.value,
            metadata: binding.metadata,
        });
    }
    Ok(Some(bindings))
}

fn parameter_types_change(plan: &LogicalPlan, params: Option<&ParamValues>) -> Result<bool> {
    let Some(params) = params else {
        return Ok(false);
    };
    for (id, field) in plan.get_parameter_fields()? {
        if let Some(field) = field
            && let Ok(binding) = params.get_placeholders_with_values(&id)
            && *field.data_type() != binding.value.data_type()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn snapshot_sources(
    plan: &LogicalPlan,
    content_hash_memo: &ContentHashMemo,
) -> Result<Option<(LogicalPlan, Vec<LogicalSourceIdentity>)>> {
    let mut identities = Vec::new();
    let mut reusable = true;
    let plan = plan
        .clone()
        .transform_up_with_subqueries(|node| {
            let LogicalPlan::TableScan(mut scan) = node else {
                return Ok(Transformed::no(node));
            };
            let Ok(provider) = source_as_provider(&scan.source) else {
                reusable = false;
                return Ok(Transformed::no(LogicalPlan::TableScan(scan)));
            };
            if provider.downcast_ref::<EmptyTable>().is_some() {
                identities.push(LogicalSourceIdentity::EmptySchema(hash_value(
                    &provider.schema(),
                )));
            } else if let Some(memory) = provider.downcast_ref::<MemTable>() {
                // Retain every partition lock until the snapshot is captured.
                let Some(guards) = memory
                    .batches
                    .iter()
                    .map(|partition| partition.try_read().ok())
                    .collect::<Option<Vec<_>>>()
                else {
                    reusable = false;
                    return Ok(Transformed::no(LogicalPlan::TableScan(scan)));
                };
                let partitions = guards
                    .iter()
                    .map(|batches| batches.to_vec())
                    .collect::<Vec<_>>();
                let sort_order = memory.sort_order.lock().clone();
                drop(guards);
                let Ok(version) = content_hash_memo.version_for(&partitions, &provider.schema())
                else {
                    reusable = false;
                    return Ok(Transformed::no(LogicalPlan::TableScan(scan)));
                };
                let constraints = provider.constraints().cloned().unwrap_or_default();
                identities.push(LogicalSourceIdentity::MemoryContent(
                    version.0,
                    hash_value(&(&constraints, &sort_order)),
                ));
                let snapshot = MemTable::try_new(provider.schema(), partitions)?
                    .with_constraints(constraints)
                    .with_sort_order(sort_order);
                scan.source = provider_as_source(Arc::new(snapshot));
            } else {
                reusable = false;
            }
            Ok(Transformed::yes(LogicalPlan::TableScan(scan)))
        })?
        .data;
    Ok(reusable.then_some((plan, identities)))
}

fn is_reusable_query(plan: &LogicalPlan) -> Result<bool> {
    let mut reusable = true;
    plan.apply_with_subqueries(|node| {
        reusable &= matches!(
            node,
            LogicalPlan::Projection(_)
                | LogicalPlan::Filter(_)
                | LogicalPlan::Window(_)
                | LogicalPlan::Aggregate(_)
                | LogicalPlan::Sort(_)
                | LogicalPlan::Join(_)
                | LogicalPlan::Repartition(_)
                | LogicalPlan::Union(_)
                | LogicalPlan::TableScan(_)
                | LogicalPlan::EmptyRelation(_)
                | LogicalPlan::Subquery(_)
                | LogicalPlan::SubqueryAlias(_)
                | LogicalPlan::Limit(_)
                | LogicalPlan::Values(_)
                | LogicalPlan::Distinct(_)
                | LogicalPlan::Unnest(_)
        );
        Ok(if reusable {
            TreeNodeRecursion::Continue
        } else {
            TreeNodeRecursion::Stop
        })
    })?;
    Ok(reusable)
}

/// Return whether a logical expression contains only immutable functions.
///
/// Callers can use this conservative gate before changing evaluation order.
/// Stable and volatile functions both return `false`. For expressions with
/// subqueries, use [`logical_plan_is_immutable`] on the containing plan to
/// inspect the subquery plans as well.
pub fn logical_expr_is_immutable(expr: &Expr) -> Result<bool> {
    let mut immutable = true;
    expr.apply(|candidate| {
        let volatility = match candidate {
            Expr::ScalarFunction(function) => Some(function.func.signature().volatility),
            Expr::AggregateFunction(function) => Some(function.func.signature().volatility),
            Expr::WindowFunction(function) => Some(function.fun.signature().volatility),
            Expr::HigherOrderFunction(function) => Some(function.func.signature().volatility),
            Expr::ScalarVariable(..) => Some(Volatility::Stable),
            _ => None,
        };
        if volatility.is_some_and(|value| value != Volatility::Immutable) {
            immutable = false;
            Ok(TreeNodeRecursion::Stop)
        } else {
            Ok(TreeNodeRecursion::Continue)
        }
    })?;
    Ok(immutable)
}

/// Return whether every expression in a logical plan and its subqueries uses
/// only immutable functions.
///
/// This is a conservative scheduling and reuse predicate. It does not assert
/// anything about source mutability; callers must separately establish that
/// the plan reads the intended input snapshot.
pub fn logical_plan_is_immutable(plan: &LogicalPlan) -> Result<bool> {
    let mut immutable = true;
    plan.apply_with_subqueries(|node| {
        for expression in node.expressions() {
            if !logical_expr_is_immutable(&expression)? {
                immutable = false;
                return Ok(TreeNodeRecursion::Stop);
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(immutable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Instant,
    };

    use arrow::{
        array::{ArrayRef, Int64Array, StringArray, StructArray},
        datatypes::{DataType, Field, Schema},
    };
    use datafusion::{
        common::{ScalarValue, metadata::ScalarAndMetadata},
        optimizer::{OptimizerConfig, OptimizerRule},
        prelude::SessionContext,
    };
    use datafusion_common::tree_node::Transformed;

    #[derive(Debug)]
    struct CountingOptimizerRule {
        calls: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        high_water: Arc<AtomicUsize>,
        barrier: Option<Arc<Barrier>>,
    }

    impl OptimizerRule for CountingOptimizerRule {
        fn name(&self) -> &str {
            "prepared_cache_counting_rule"
        }

        fn rewrite(
            &self,
            plan: LogicalPlan,
            _config: &dyn OptimizerConfig,
        ) -> Result<Transformed<LogicalPlan>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.high_water.fetch_max(active, Ordering::SeqCst);
            if let Some(barrier) = &self.barrier {
                barrier.wait();
            }
            thread::sleep(Duration::from_millis(20));
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(Transformed::no(plan))
        }
    }

    fn context_with_counting_optimizer(
        barrier: Option<Arc<Barrier>>,
    ) -> (SessionContext, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let high_water = Arc::new(AtomicUsize::new(0));
        let state = datafusion::execution::session_state::SessionStateBuilder::new()
            .with_default_features()
            .with_optimizer_rules(vec![Arc::new(CountingOptimizerRule {
                calls: Arc::clone(&calls),
                active,
                high_water: Arc::clone(&high_water),
                barrier,
            })])
            .build();
        (SessionContext::new_with_state(state), calls, high_water)
    }

    fn int_batch(values: Vec<i64>) -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Int64,
                false,
            )])),
            vec![Arc::new(Int64Array::from(values))],
        )
        .unwrap()
    }

    fn first_i64(batches: &[RecordBatch]) -> i64 {
        batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0)
    }

    #[tokio::test]
    async fn reuses_optimized_template_with_new_parameter_values() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch(vec![1, 2, 3]))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());

        let first = cache
            .collect(
                ctx.sql("SELECT MAX(value) FROM numbers WHERE value <= $limit")
                    .await
                    .unwrap(),
                Some(vec![("limit", ScalarValue::Int64(Some(2)))].into()),
            )
            .await
            .unwrap();
        let second = cache
            .collect(
                ctx.sql("SELECT MAX(value) FROM numbers WHERE value <= $limit")
                    .await
                    .unwrap(),
                Some(vec![("limit", ScalarValue::Int64(Some(3)))].into()),
            )
            .await
            .unwrap();

        assert_eq!(first_i64(&first.batches), 2);
        assert_eq!(first_i64(&second.batches), 3);
        assert_eq!(
            first.metrics.disposition,
            Some(PreparedPlanDisposition::Miss)
        );
        assert_eq!(
            second.metrics.disposition,
            Some(PreparedPlanDisposition::Hit)
        );
        assert_eq!(
            second.metrics.exact_result_disposition,
            Some(ExactResultDisposition::Miss)
        );
        assert_eq!(
            first.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Miss)
        );
        assert_eq!(
            second.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Hit)
        );
        assert_eq!(second.metrics.physical_planning, Duration::ZERO);
        let metrics = cache.metrics();
        assert_eq!(metrics.logical_misses, 1);
        assert_eq!(metrics.logical_hits, 1);
        assert_eq!(metrics.physical_misses, 1);
        assert_eq!(metrics.physical_hits, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_physical_executions_keep_bindings_isolated() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch(vec![1, 2, 3]))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entries: 0,
            ..Default::default()
        });

        cache
            .collect(
                ctx.sql("SELECT MAX(value) FROM numbers WHERE value <= $limit")
                    .await
                    .unwrap(),
                Some(vec![("limit", ScalarValue::Int64(Some(2)))].into()),
            )
            .await
            .unwrap();

        let first_df = ctx
            .sql("SELECT MAX(value) FROM numbers WHERE value <= $limit")
            .await
            .unwrap();
        let second_df = ctx
            .sql("SELECT MAX(value) FROM numbers WHERE value <= $limit")
            .await
            .unwrap();
        let first_cache = Arc::clone(&cache);
        let second_cache = Arc::clone(&cache);
        let first = tokio::spawn(async move {
            first_cache
                .collect(
                    first_df,
                    Some(vec![("limit", ScalarValue::Int64(Some(1)))].into()),
                )
                .await
        });
        let second = tokio::spawn(async move {
            second_cache
                .collect(
                    second_df,
                    Some(vec![("limit", ScalarValue::Int64(Some(3)))].into()),
                )
                .await
        });

        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();
        assert_eq!(first_i64(&first.batches), 1);
        assert_eq!(first_i64(&second.batches), 3);
        assert_eq!(
            first.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Hit)
        );
        assert_eq!(
            second.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Hit)
        );
    }

    #[tokio::test]
    async fn topk_sort_prototype_resets_with_new_parameter_values() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch(vec![1, 2, 3]))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entries: 0,
            ..Default::default()
        });
        let sql = "SELECT value FROM numbers WHERE value <= $limit ORDER BY value DESC LIMIT 1";

        let first = cache
            .collect(
                ctx.sql(sql).await.unwrap(),
                Some(vec![("limit", ScalarValue::Int64(Some(2)))].into()),
            )
            .await
            .unwrap();
        let second = cache
            .collect(
                ctx.sql(sql).await.unwrap(),
                Some(vec![("limit", ScalarValue::Int64(Some(3)))].into()),
            )
            .await
            .unwrap();

        assert_eq!(first_i64(&first.batches), 2);
        assert_eq!(first_i64(&second.batches), 3);
        assert_eq!(
            first.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Miss)
        );
        assert_eq!(
            second.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Hit)
        );
        assert_eq!(second.metrics.physical_planning, Duration::ZERO);
    }

    #[tokio::test]
    async fn physical_prototypes_cover_representative_relational_operators() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch(vec![1, 2, 3]))
            .unwrap();
        let queries = [
            "SELECT value * 2 AS doubled FROM numbers WHERE value <= $limit ORDER BY value",
            "SELECT SUM(value) AS total FROM numbers WHERE value <= $limit",
            "SELECT value, ROW_NUMBER() OVER (ORDER BY value) AS row_id FROM numbers WHERE value <= $limit ORDER BY value",
            "SELECT a.value FROM numbers a JOIN numbers b ON a.value = b.value WHERE a.value <= $limit ORDER BY a.value",
            "SELECT value FROM numbers WHERE value <= (SELECT $limit) ORDER BY value LIMIT 2",
        ];

        for sql in queries {
            let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
                max_result_entries: 0,
                ..Default::default()
            });
            let bindings = |limit| Some(vec![("limit", ScalarValue::Int64(Some(limit)))].into());
            let first = cache
                .collect(ctx.sql(sql).await.unwrap(), bindings(2))
                .await
                .unwrap();
            let second = cache
                .collect(ctx.sql(sql).await.unwrap(), bindings(3))
                .await
                .unwrap();
            let expected = ctx
                .sql(sql)
                .await
                .unwrap()
                .with_param_values(bindings(3).unwrap())
                .unwrap()
                .collect()
                .await
                .unwrap();

            assert!(!first.batches.is_empty(), "cold output for {sql}");
            assert_eq!(second.batches, expected, "reused output for {sql}");
            assert_eq!(
                first.metrics.physical_plan_disposition,
                Some(PhysicalPlanDisposition::Miss),
                "cold disposition for {sql}"
            );
            assert_eq!(
                second.metrics.physical_plan_disposition,
                Some(PhysicalPlanDisposition::Hit),
                "warm disposition for {sql}"
            );
            assert_eq!(second.metrics.physical_planning, Duration::ZERO);
        }
    }

    #[tokio::test]
    async fn physical_prototype_preserves_struct_parameter_type_and_value() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entries: 0,
            ..Default::default()
        });
        let value = ScalarValue::Struct(Arc::new(StructArray::from(vec![
            (
                Arc::new(Field::new("label", DataType::Utf8, true)),
                Arc::new(StringArray::from(vec![Some("A")])) as ArrayRef,
            ),
            (
                Arc::new(Field::new("count", DataType::Int64, true)),
                Arc::new(Int64Array::from(vec![Some(7)])) as ArrayRef,
            ),
        ])));

        let first = cache
            .collect(
                ctx.sql("SELECT $value AS value").await.unwrap(),
                Some(vec![("value", value.clone())].into()),
            )
            .await
            .unwrap();
        let second = cache
            .collect(
                ctx.sql("SELECT $value AS value").await.unwrap(),
                Some(vec![("value", value.clone())].into()),
            )
            .await
            .unwrap();

        assert_eq!(
            ScalarValue::try_from_array(first.batches[0].column(0), 0).unwrap(),
            value
        );
        assert_eq!(
            second.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Hit)
        );
    }

    #[tokio::test]
    async fn table_provider_identity_is_part_of_the_key() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        ctx.register_batch("source", int_batch(vec![1])).unwrap();
        let first_df = ctx.sql("SELECT value FROM source").await.unwrap();
        ctx.deregister_table("source").unwrap();
        ctx.register_batch("source", int_batch(vec![9])).unwrap();
        let second_df = ctx.sql("SELECT value FROM source").await.unwrap();

        let first = cache.collect(first_df, None).await.unwrap();
        let second = cache.collect(second_df, None).await.unwrap();

        assert_eq!(first_i64(&first.batches), 1);
        assert_eq!(first_i64(&second.batches), 9);
        assert_eq!(cache.metrics().logical_misses, 2);
        assert_eq!(cache.metrics().exact_result_misses, 2);
    }

    #[tokio::test]
    async fn equivalent_memory_tables_share_a_logical_template() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        ctx.register_batch("source", int_batch(vec![1, 2, 3]))
            .unwrap();
        let first_df = ctx.sql("SELECT MAX(value) FROM source").await.unwrap();
        ctx.deregister_table("source").unwrap();
        ctx.register_batch("source", int_batch(vec![1, 2, 3]))
            .unwrap();
        let second_df = ctx.sql("SELECT MAX(value) FROM source").await.unwrap();

        let first = cache.collect(first_df, None).await.unwrap();
        let second = cache.collect(second_df, None).await.unwrap();

        assert_eq!(first_i64(&first.batches), 3);
        assert_eq!(first_i64(&second.batches), 3);
        assert_eq!(
            first.metrics.disposition,
            Some(PreparedPlanDisposition::Miss)
        );
        assert_eq!(
            second.metrics.exact_result_disposition,
            Some(ExactResultDisposition::Hit)
        );
        assert_eq!(second.metrics.disposition, None);
        assert_eq!(second.metrics.parameter_binding, Duration::ZERO);
        assert_eq!(second.metrics.physical_planning, Duration::ZERO);
        assert_eq!(second.metrics.execution, Duration::ZERO);
    }

    #[tokio::test]
    async fn exact_results_ignore_unreferenced_parameter_values() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch(vec![1, 2, 3]))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());

        let first = cache
            .collect(
                ctx.sql("SELECT MAX(value) FROM numbers WHERE value <= $limit")
                    .await
                    .unwrap(),
                Some(
                    vec![
                        ("limit", ScalarValue::Int64(Some(2))),
                        ("unrelated", ScalarValue::Int64(Some(10))),
                    ]
                    .into(),
                ),
            )
            .await
            .unwrap();
        let second = cache
            .collect(
                ctx.sql("SELECT MAX(value) FROM numbers WHERE value <= $limit")
                    .await
                    .unwrap(),
                Some(
                    vec![
                        ("limit", ScalarValue::Int64(Some(2))),
                        ("unrelated", ScalarValue::Int64(Some(99))),
                    ]
                    .into(),
                ),
            )
            .await
            .unwrap();

        assert_eq!(first_i64(&first.batches), 2);
        assert_eq!(first_i64(&second.batches), 2);
        assert_eq!(
            second.metrics.exact_result_disposition,
            Some(ExactResultDisposition::Hit)
        );
        let metrics = cache.metrics();
        assert_eq!(metrics.exact_result_misses, 1);
        assert_eq!(metrics.exact_result_hits, 1);
    }

    #[tokio::test]
    async fn exact_results_include_referenced_positional_values() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch(vec![1, 2, 3]))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());

        let first = cache
            .collect(
                ctx.sql("SELECT MAX(value) FROM numbers WHERE value <= $1")
                    .await
                    .unwrap(),
                Some(vec![ScalarValue::Int64(Some(2))].into()),
            )
            .await
            .unwrap();
        let second = cache
            .collect(
                ctx.sql("SELECT MAX(value) FROM numbers WHERE value <= $1")
                    .await
                    .unwrap(),
                Some(vec![ScalarValue::Int64(Some(3))].into()),
            )
            .await
            .unwrap();

        assert_eq!(first_i64(&first.batches), 2);
        assert_eq!(first_i64(&second.batches), 3);
        assert_eq!(
            second.metrics.exact_result_disposition,
            Some(ExactResultDisposition::Miss)
        );
    }

    #[tokio::test]
    async fn prepared_binding_preserves_typed_null_metadata() {
        let ctx = SessionContext::new();
        let dataframe = ctx.sql("SELECT $value").await.unwrap();
        let metadata = FieldMetadata::new(BTreeMap::from([(
            "extension".to_string(),
            "example".to_string(),
        )]));
        let params = ParamValues::Map(HashMap::from([(
            "value".to_string(),
            ScalarAndMetadata::new(ScalarValue::Int64(None), Some(metadata)),
        )]));

        let expected = dataframe
            .clone()
            .with_param_values(params.clone())
            .unwrap()
            .collect()
            .await
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        let actual = cache.collect(dataframe, Some(params)).await.unwrap();
        assert_eq!(actual.batches, expected, "{:?}", actual.metrics);
    }

    #[tokio::test]
    async fn parameter_coercion_matches_ordinary_collection() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch(vec![1, 2, 3]))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        let dataframe = ctx
            .sql("SELECT value FROM numbers WHERE value < $limit ORDER BY value")
            .await
            .unwrap();
        for value in [ScalarValue::Int64(Some(2)), ScalarValue::Float64(Some(2.5))] {
            let bindings: ParamValues = vec![("limit", value)].into();
            let expected = dataframe
                .clone()
                .with_param_values(bindings.clone())
                .unwrap()
                .collect()
                .await
                .unwrap();
            let actual = cache
                .collect(dataframe.clone(), Some(bindings))
                .await
                .unwrap();
            assert_eq!(actual.batches, expected);
        }
    }

    #[tokio::test]
    async fn changing_function_registration_invalidates_every_prepared_layer() {
        use datafusion::logical_expr::{ColumnarValue, create_udf};
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        for value in [1, 9] {
            ctx.register_udf(create_udf(
                "constant_value",
                vec![],
                DataType::Int64,
                Volatility::Immutable,
                Arc::new(move |_| Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(value))))),
            ));
            let result = cache
                .collect(ctx.sql("SELECT constant_value()").await.unwrap(), None)
                .await
                .unwrap();
            assert_eq!(first_i64(&result.batches), value);
            assert_eq!(
                result.metrics.disposition,
                Some(PreparedPlanDisposition::Miss)
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelled_execution_discards_its_physical_prototype() {
        use datafusion::logical_expr::create_udf;
        use std::sync::{atomic::AtomicBool, mpsc};
        let ctx = SessionContext::new();
        ctx.register_table(
            "numbers",
            Arc::new(
                MemTable::try_new(
                    int_batch(vec![1]).schema(),
                    vec![vec![int_batch(vec![1])], vec![int_batch(vec![1])]],
                )
                .unwrap(),
            ),
        )
        .unwrap();
        let block = Arc::new(AtomicBool::new(false));
        let should_block = Arc::clone(&block);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        ctx.register_udf(create_udf(
            "wait_once",
            vec![DataType::Int64],
            DataType::Int64,
            Volatility::Immutable,
            Arc::new(move |args| {
                if should_block.swap(false, Ordering::SeqCst) {
                    entered_tx.send(()).unwrap();
                    release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                }
                Ok(args[0].clone())
            }),
        ));
        let dataframe = ctx
            .sql("SELECT wait_once(value + $n) FROM numbers")
            .await
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entries: 0,
            ..Default::default()
        });
        let params = || Some(vec![("n", ScalarValue::Int64(Some(1)))].into());
        let first = cache.collect(dataframe.clone(), params()).await.unwrap();
        assert_eq!(
            first.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Miss)
        );
        block.store(true, Ordering::SeqCst);
        let task_cache = Arc::clone(&cache);
        let task_df = dataframe.clone();
        let task = tokio::spawn(async move { task_cache.collect(task_df, params()).await });
        tokio::task::spawn_blocking(move || {
            entered_rx.recv_timeout(Duration::from_secs(10)).unwrap()
        })
        .await
        .unwrap();
        task.abort();
        // Release DataFusion's worker only after the collection future is dropped.
        assert!(task.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();
        let next = cache.collect(dataframe, params()).await.unwrap();
        assert_eq!(next.batches, first.batches);
        assert_eq!(
            next.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Bypass(
                PhysicalPlanBypassReason::ResetFailure
            ))
        );
    }

    #[tokio::test]
    async fn logical_templates_retain_the_captured_source_snapshot() {
        let ctx = SessionContext::new();
        let original = Arc::new(
            MemTable::try_new(
                int_batch(vec![1, 2]).schema(),
                vec![vec![int_batch(vec![1, 2])]],
            )
            .unwrap(),
        );
        ctx.register_table("source", original.clone()).unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_physical_entries: 0,
            ..Default::default()
        });
        let sql = "SELECT SUM(value) FROM source WHERE value <= $limit";
        cache
            .collect(
                ctx.sql(sql).await.unwrap(),
                Some(vec![("limit", ScalarValue::Int64(Some(1)))].into()),
            )
            .await
            .unwrap();
        ctx.deregister_table("source").unwrap();
        ctx.register_batch("source", int_batch(vec![1, 2])).unwrap();
        *original.batches[0].write().await = vec![int_batch(vec![99])];
        let result = cache
            .collect(
                ctx.sql(sql).await.unwrap(),
                Some(vec![("limit", ScalarValue::Int64(Some(2)))].into()),
            )
            .await
            .unwrap();
        assert_eq!(first_i64(&result.batches), 3);
        assert_eq!(
            result.metrics.disposition,
            Some(PreparedPlanDisposition::Hit)
        );
    }

    #[tokio::test]
    async fn repeated_inserts_are_executed_every_time() {
        let ctx = SessionContext::new();
        ctx.register_batch("destination", int_batch(vec![]))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        for _ in 0..2 {
            let plan = ctx
                .state()
                .create_logical_plan("INSERT INTO destination VALUES (7)")
                .await
                .unwrap();
            let result = cache
                .collect(DataFrame::new(ctx.state(), plan), None)
                .await
                .unwrap();
            assert_eq!(
                result.metrics.disposition,
                Some(PreparedPlanDisposition::Bypass(
                    PreparedPlanBypassReason::UnsupportedPlan
                ))
            );
        }
        let rows = ctx
            .sql("SELECT value FROM destination")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(rows.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
    }

    #[tokio::test]
    async fn empty_outputs_are_exactly_reused() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch(vec![1, 2, 3]))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());

        let first = cache
            .collect(
                ctx.sql("SELECT value FROM numbers WHERE false")
                    .await
                    .unwrap(),
                None,
            )
            .await
            .unwrap();
        let second = cache
            .collect(
                ctx.sql("SELECT value FROM numbers WHERE false")
                    .await
                    .unwrap(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            first
                .batches
                .iter()
                .map(RecordBatch::num_rows)
                .sum::<usize>(),
            0
        );
        assert_eq!(
            second.metrics.exact_result_disposition,
            Some(ExactResultDisposition::Hit)
        );
    }

    #[tokio::test]
    async fn unversioned_sources_are_scanned_again_after_updates() {
        use datafusion::catalog::{Session, TableProvider};
        #[derive(Debug)]
        struct LiveTable(Arc<MemTable>);
        #[async_trait::async_trait]
        impl TableProvider for LiveTable {
            fn schema(&self) -> SchemaRef {
                self.0.schema()
            }
            fn table_type(&self) -> datafusion::logical_expr::TableType {
                self.0.table_type()
            }
            async fn scan(
                &self,
                state: &dyn Session,
                projection: Option<&Vec<usize>>,
                filters: &[Expr],
                limit: Option<usize>,
            ) -> Result<Arc<dyn ExecutionPlan>> {
                self.0.scan(state, projection, filters, limit).await
            }
        }
        let ctx = SessionContext::new();
        let memory = Arc::new(
            MemTable::try_new(int_batch(vec![1]).schema(), vec![vec![int_batch(vec![1])]]).unwrap(),
        );
        let live = Arc::new(LiveTable(Arc::clone(&memory)));
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        for value in [1, 9] {
            *memory.batches[0].write().await = vec![int_batch(vec![value])];
            let dataframe = ctx.read_table(live.clone()).unwrap();
            let result = cache.collect(dataframe, None).await.unwrap();
            assert_eq!(first_i64(&result.batches), value);
            assert_eq!(
                result.metrics.disposition,
                Some(PreparedPlanDisposition::Bypass(
                    PreparedPlanBypassReason::UnversionedSource
                ))
            );
        }
        assert_eq!(cache.metrics().logical_entries, 0);
        assert_eq!(cache.metrics().physical_entries, 0);
        assert_eq!(cache.metrics().exact_result_entries, 0);
    }

    #[tokio::test]
    async fn exact_results_are_lru_and_byte_bounded() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entries: 1,
            max_result_bytes: usize::MAX,
            max_result_entry_bytes: usize::MAX,
            ..Default::default()
        });

        cache
            .collect(ctx.sql("SELECT 1").await.unwrap(), None)
            .await
            .unwrap();
        cache
            .collect(ctx.sql("SELECT 2").await.unwrap(), None)
            .await
            .unwrap();

        let metrics = cache.metrics();
        assert_eq!(metrics.exact_result_entries, 1);
        assert_eq!(metrics.exact_result_evictions, 1);
        assert!(metrics.exact_result_bytes > 0);
    }

    #[tokio::test]
    async fn oversize_exact_results_are_not_admitted() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entry_bytes: 1,
            ..Default::default()
        });

        let first = cache
            .collect(ctx.sql("SELECT 1").await.unwrap(), None)
            .await
            .unwrap();
        let second = cache
            .collect(ctx.sql("SELECT 1").await.unwrap(), None)
            .await
            .unwrap();

        assert_eq!(
            first.metrics.exact_result_disposition,
            Some(ExactResultDisposition::Miss)
        );
        assert_eq!(
            second.metrics.exact_result_disposition,
            Some(ExactResultDisposition::Miss)
        );
        let metrics = cache.metrics();
        assert_eq!(metrics.exact_result_entries, 0);
        assert_eq!(metrics.exact_result_oversize_declines, 2);
    }

    #[tokio::test]
    async fn clear_during_collection_does_not_republish_entries() {
        #[derive(Debug)]
        struct ClearCache(Arc<PreparedPlanCache>);
        impl OptimizerRule for ClearCache {
            fn name(&self) -> &str {
                "clear_cache"
            }
            fn rewrite(
                &self,
                plan: LogicalPlan,
                _: &dyn OptimizerConfig,
            ) -> Result<Transformed<LogicalPlan>> {
                self.0.clear();
                Ok(Transformed::no(plan))
            }
        }
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        let state = datafusion::execution::session_state::SessionStateBuilder::new()
            .with_default_features()
            .with_optimizer_rules(vec![Arc::new(ClearCache(Arc::clone(&cache)))])
            .build();
        let ctx = SessionContext::new_with_state(state);
        let output = cache
            .collect(ctx.sql("SELECT 1").await.unwrap(), None)
            .await
            .unwrap();
        assert_eq!(first_i64(&output.batches), 1);
        let metrics = cache.metrics();
        assert_eq!(metrics.logical_entries, 0);
        assert_eq!(metrics.physical_entries, 0);
        assert_eq!(metrics.exact_result_entries, 0);
        assert_eq!(metrics.exact_result_bytes, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_same_key_collection_is_single_flight() {
        let (ctx, calls, _) = context_with_counting_optimizer(None);
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entries: 0,
            ..Default::default()
        });
        let first_df = ctx.sql("SELECT 1").await.unwrap();
        let second_df = ctx.sql("SELECT 1").await.unwrap();
        let first_cache = Arc::clone(&cache);
        let second_cache = Arc::clone(&cache);
        let first = tokio::spawn(async move { first_cache.collect(first_df, None).await });
        let second = tokio::spawn(async move { second_cache.collect(second_df, None).await });

        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        let metrics = cache.metrics();
        assert_eq!(metrics.logical_misses, 1);
        assert_eq!(metrics.logical_hits, 1);
        assert_eq!(metrics.exact_result_bypasses, 2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_same_exact_result_is_single_flight() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch((1..=1_000).collect()))
            .unwrap();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        let first_df = ctx
            .sql("SELECT SUM(a.value * b.value) FROM numbers a, numbers b")
            .await
            .unwrap();
        let second_df = ctx
            .sql("SELECT SUM(a.value * b.value) FROM numbers a, numbers b")
            .await
            .unwrap();
        let first_cache = Arc::clone(&cache);
        let second_cache = Arc::clone(&cache);
        let first = tokio::spawn(async move { first_cache.collect(first_df, None).await });
        let second = tokio::spawn(async move { second_cache.collect(second_df, None).await });

        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        let metrics = cache.metrics();
        assert_eq!(metrics.logical_misses, 1);
        assert_eq!(metrics.exact_result_misses, 1);
        assert_eq!(metrics.exact_result_hits, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn different_logical_keys_optimize_concurrently() {
        let barrier = Arc::new(Barrier::new(2));
        let (ctx, calls, high_water) = context_with_counting_optimizer(Some(barrier));
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entries: 0,
            ..Default::default()
        });
        let first_df = ctx.sql("SELECT 1").await.unwrap();
        let second_df = ctx.sql("SELECT 2").await.unwrap();
        let first_cache = Arc::clone(&cache);
        let second_cache = Arc::clone(&cache);
        let first = tokio::spawn(async move { first_cache.collect(first_df, None).await });
        let second = tokio::spawn(async move { second_cache.collect(second_df, None).await });

        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(high_water.load(Ordering::SeqCst), 2);
        assert_eq!(cache.metrics().logical_misses, 2);
    }

    #[test]
    fn prehashed_key_collisions_still_use_full_equality() {
        let plan_one = LogicalPlan::EmptyRelation(datafusion::logical_expr::EmptyRelation {
            produce_one_row: true,
            schema: Arc::new(datafusion_common::DFSchema::empty()),
        });
        let plan_two = LogicalPlan::EmptyRelation(datafusion::logical_expr::EmptyRelation {
            produce_one_row: false,
            schema: Arc::new(datafusion_common::DFSchema::empty()),
        });
        let mut first = PrehashedLogicalTemplateKey::new(LogicalTemplateKey {
            session_id: "session".to_string(),
            generation: 0,
            planning_profile: 0,
            plan: plan_one,
            source_identities: Vec::new(),
        });
        let second = PrehashedLogicalTemplateKey::new(LogicalTemplateKey {
            session_id: "session".to_string(),
            generation: 0,
            planning_profile: 0,
            plan: plan_two,
            source_identities: Vec::new(),
        });
        first.hash = second.hash;
        let mut keys = HashSet::new();
        keys.insert(first);
        keys.insert(second);

        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn stable_functions_bypass_reuse() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
        let output = cache
            .collect(ctx.sql("SELECT now()").await.unwrap(), None)
            .await
            .unwrap();

        assert!(matches!(
            output.metrics.disposition,
            Some(PreparedPlanDisposition::Bypass(
                PreparedPlanBypassReason::NonImmutableFunction
            ))
        ));
        assert_eq!(cache.metrics().bypasses, 1);
    }

    #[tokio::test]
    async fn disabled_cache_uses_the_ordinary_collection_path() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            enabled: false,
            ..Default::default()
        });
        let output = cache
            .collect(ctx.sql("SELECT 1").await.unwrap(), None)
            .await
            .unwrap();

        assert!(matches!(
            output.metrics.disposition,
            Some(PreparedPlanDisposition::Bypass(
                PreparedPlanBypassReason::Disabled
            ))
        ));
        assert_eq!(output.metrics.exact_result_disposition, None);
        assert_eq!(cache.metrics().bypasses, 1);
        assert_eq!(cache.metrics().exact_result_entries, 0);
    }

    #[tokio::test]
    async fn logical_templates_are_lru_bounded() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_logical_entries: 1,
            ..Default::default()
        });

        cache
            .collect(ctx.sql("SELECT 1").await.unwrap(), None)
            .await
            .unwrap();
        cache
            .collect(ctx.sql("SELECT 2").await.unwrap(), None)
            .await
            .unwrap();

        let metrics = cache.metrics();
        assert_eq!(metrics.logical_entries, 1);
        assert_eq!(metrics.logical_evictions, 1);
    }

    #[tokio::test]
    async fn physical_prototypes_are_lru_bounded() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_physical_entries: 1,
            max_result_entries: 0,
            ..Default::default()
        });

        for sql in ["SELECT $value + 1", "SELECT $value + 2"] {
            cache
                .collect(
                    ctx.sql(sql).await.unwrap(),
                    Some(vec![("value", ScalarValue::Int64(Some(1)))].into()),
                )
                .await
                .unwrap();
        }

        let metrics = cache.metrics();
        assert_eq!(metrics.physical_entries, 1);
        assert_eq!(metrics.physical_evictions, 1);
    }

    #[tokio::test]
    #[ignore = "microbenchmark: run explicitly when changing prepared-plan policy"]
    async fn bench_ordinary_logical_and_physical_reuse_100_times() {
        let ctx = SessionContext::new();
        ctx.register_batch("numbers", int_batch((0..10_000).collect()))
            .unwrap();
        let sql = "SELECT value / 10 AS bin_start, COUNT(*) AS count \
                   FROM numbers WHERE value <= $limit \
                   GROUP BY bin_start ORDER BY bin_start";
        let dataframe = ctx.sql(sql).await.unwrap();
        let values = (0..100)
            .map(|index| 4_000 + index * 10)
            .collect::<Vec<i64>>();

        let ordinary_started = Instant::now();
        for value in &values {
            std::hint::black_box(
                dataframe
                    .clone()
                    .with_param_values(vec![("limit", ScalarValue::Int64(Some(*value)))])
                    .unwrap()
                    .collect()
                    .await
                    .unwrap(),
            );
        }
        let ordinary = ordinary_started.elapsed();

        let logical_cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_physical_entries: 0,
            max_result_entries: 0,
            ..Default::default()
        });
        logical_cache
            .collect(
                dataframe.clone(),
                Some(vec![("limit", ScalarValue::Int64(Some(values[0])))].into()),
            )
            .await
            .unwrap();
        let logical_started = Instant::now();
        for value in &values {
            std::hint::black_box(
                logical_cache
                    .collect(
                        dataframe.clone(),
                        Some(vec![("limit", ScalarValue::Int64(Some(*value)))].into()),
                    )
                    .await
                    .unwrap(),
            );
        }
        let logical = logical_started.elapsed();

        let physical_cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_result_entries: 0,
            ..Default::default()
        });
        physical_cache
            .collect(
                dataframe.clone(),
                Some(vec![("limit", ScalarValue::Int64(Some(values[0])))].into()),
            )
            .await
            .unwrap();
        let physical_started = Instant::now();
        for value in &values {
            std::hint::black_box(
                physical_cache
                    .collect(
                        dataframe.clone(),
                        Some(vec![("limit", ScalarValue::Int64(Some(*value)))].into()),
                    )
                    .await
                    .unwrap(),
            );
        }
        let physical = physical_started.elapsed();

        println!(
            "prepared plan benchmark: iterations=100 ordinary_ms={:.3} logical_ms={:.3} physical_ms={:.3}",
            ordinary.as_secs_f64() * 1_000.0,
            logical.as_secs_f64() * 1_000.0,
            physical.as_secs_f64() * 1_000.0,
        );
    }

    #[tokio::test]
    async fn zero_physical_capacity_uses_literal_bound_planning() {
        let ctx = SessionContext::new();
        let cache = PreparedPlanCache::new(PreparedPlanCacheConfig {
            max_physical_entries: 0,
            max_result_entries: 0,
            ..Default::default()
        });
        let output = cache
            .collect(
                ctx.sql("SELECT $value").await.unwrap(),
                Some(vec![("value", ScalarValue::Int64(Some(7)))].into()),
            )
            .await
            .unwrap();

        assert_eq!(first_i64(&output.batches), 7);
        assert_eq!(
            output.metrics.physical_plan_disposition,
            Some(PhysicalPlanDisposition::Bypass(
                PhysicalPlanBypassReason::ZeroCapacity
            ))
        );
        assert_eq!(cache.metrics().physical_entries, 0);
    }
}
