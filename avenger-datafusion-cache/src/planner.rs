//! The cache planner: fingerprints a physical plan and rewrites it with
//! cache read and write nodes.
//!
//! [`EvaluationCachePlanner`] must run AFTER every built-in physical
//! optimizer rule: register it via
//! [`SessionStateBuilder::with_physical_optimizer_rule`], which appends
//! custom rules after the defaults (including `SanityCheckPlan`), and never
//! re-run other rules on its output. Substitution is property-compatible by
//! construction — `CacheReadExec` reports exactly the `PlanProperties`
//! captured from the subtree it replaces — and nothing re-validates the
//! rewritten plan afterwards.
//!
//! The rewrite is two passes:
//!
//! - **Pass A** (pure, bottom-up): fingerprint every subtree
//!   ([`fingerprint_tree`]), producing a side tree addressed by position —
//!   never by node pointer, since `ExecutionPlan` `Arc`s can be shared
//!   between tree positions.
//! - **Pass B** (top-down): at each cacheable node, look the key up. A
//!   committed hit substitutes [`CacheReadExec`] and stops descending (the
//!   *hit frontier*: shadowed descendants are neither matched nor observed).
//!   A pending entry is a miss-without-write (the synchronous rule never
//!   waits). A miss is observed and, outside an enclosing admitted write,
//!   may be admitted and wrapped with [`CacheWriteExec`] (the *write
//!   frontier*: descendants of an admitted write are not admitted, but they
//!   keep being observed and can still serve hits — substitution preserves
//!   output, and the parent's fingerprint is source-based, not
//!   plan-shape-based).
//!
//! [`SessionStateBuilder::with_physical_optimizer_rule`]: datafusion::execution::session_state::SessionStateBuilder::with_physical_optimizer_rule

use std::sync::Arc;

use datafusion::config::ConfigOptions;
use datafusion::physical_optimizer::PhysicalOptimizerRule;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::physical_plan::execution_plan::ExecutionPlanProperties;
use datafusion_common::Result;
use datafusion_common::stats::Precision;

use crate::exec::{CacheReadExec, CacheWriteExec};
use crate::fingerprint::{
    ExclusionReason, FingerprintContext, FingerprintOutcome, FingerprintedNode,
    PhysicalPlanFingerprinter, ProtoFingerprinter, cache_key_for, config_fingerprint,
    fingerprint_tree,
};
use crate::store::{
    AdmissionDecision, CacheCandidate, CacheLookup, EvaluationCache, PlanObservation,
};

/// Physical optimizer rule / wrapper-mode rewriter that substitutes cached
/// results into physical plans.
///
/// Two wirings are supported:
///
/// - as a [`PhysicalOptimizerRule`] registered last on the session state
///   (the usual mode);
/// - wrapper-style, calling [`rewrite`](Self::rewrite) on the output of
///   `create_physical_plan` directly.
pub struct EvaluationCachePlanner {
    cache: Arc<EvaluationCache>,
    fingerprinter: Arc<dyn PhysicalPlanFingerprinter>,
}

impl std::fmt::Debug for EvaluationCachePlanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvaluationCachePlanner").finish()
    }
}

impl EvaluationCachePlanner {
    /// Create a planner over a shared cache with the default
    /// [`ProtoFingerprinter`].
    pub fn new(cache: Arc<EvaluationCache>) -> Self {
        Self::with_fingerprinter(cache, Arc::new(ProtoFingerprinter))
    }

    /// Create a planner with a custom fingerprinter (for extension
    /// `ExecutionPlan` nodes the proto codec cannot encode).
    pub fn with_fingerprinter(
        cache: Arc<EvaluationCache>,
        fingerprinter: Arc<dyn PhysicalPlanFingerprinter>,
    ) -> Self {
        Self {
            cache,
            fingerprinter,
        }
    }

    /// The cache this planner reads and writes.
    pub fn cache(&self) -> &Arc<EvaluationCache> {
        &self.cache
    }

    /// Wrapper-mode entry point using default configuration options for the
    /// execution-config fingerprint subset. Prefer
    /// [`rewrite_with_config`](Self::rewrite_with_config) when the session's
    /// `ConfigOptions` are available.
    pub fn rewrite(&self, plan: Arc<dyn ExecutionPlan>) -> Result<Arc<dyn ExecutionPlan>> {
        self.rewrite_with_config(plan, &ConfigOptions::default())
    }

    /// Fingerprint `plan` bottom-up and rewrite it with `CacheReadExec` /
    /// `CacheWriteExec` nodes. Returns the input `Arc` unchanged when the
    /// cache is disabled or nothing in the plan is cacheable.
    pub fn rewrite_with_config(
        &self,
        plan: Arc<dyn ExecutionPlan>,
        config: &ConfigOptions,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !self.cache.config().enabled {
            return Ok(plan);
        }
        let clock = &self.cache.clock;
        let started = clock.now();

        // Pass A: bottom-up fingerprints, positionally addressed.
        let ctx = FingerprintContext::new(
            config_fingerprint(config),
            self.cache.version_providers(),
            Arc::clone(&self.cache.memo),
        );
        let tree = fingerprint_tree(&plan, self.fingerprinter.as_ref(), &ctx);
        let fingerprinted = clock.now();

        let mut exclusions = ExclusionCounts::default();
        count_exclusions(&tree, &mut exclusions);
        self.cache.record_exclusions(
            exclusions.dynamic,
            exclusions.volatile,
            exclusions.unbounded,
            exclusions.unsupported,
        );

        let rewritten = if any_cacheable(&tree) {
            // Pass B: top-down rewrite.
            self.rewrite_node(&plan, &tree, false)?
        } else {
            plan
        };

        let finished = clock.now();
        self.cache.record_planner_timing(
            fingerprinted.saturating_sub(started).as_nanos() as u64,
            finished.saturating_sub(started).as_nanos() as u64,
        );
        Ok(rewritten)
    }

    fn rewrite_node(
        &self,
        plan: &Arc<dyn ExecutionPlan>,
        node: &FingerprintedNode,
        in_write_frontier: bool,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if let FingerprintOutcome::Cacheable(fingerprint) = node.outcome {
            let key = cache_key_for(plan, fingerprint);
            match self.cache.lookup(&key) {
                CacheLookup::Ready(entry) => {
                    if entry.schema() == plan.schema() {
                        // Hit frontier: substitute and stop descending —
                        // shadowed descendants are neither matched nor
                        // observed.
                        return Ok(Arc::new(CacheReadExec::new(&entry)));
                    }
                    // Schema mismatch (defensive; the key hashes the schema,
                    // so this indicates a hash collision): treat as a miss
                    // without observing or admitting, and keep descending.
                }
                CacheLookup::Pending(_) => {
                    // Miss-without-write: no observation (the in-flight
                    // write's planner already observed it), no second write.
                    // Descend so children can still serve hits — but inside
                    // the write frontier: the pending ancestor IS an
                    // in-flight write, and a descendant admitted beneath it
                    // would commit an entry permanently shadowed by the
                    // ancestor's.
                    return self.rewrite_children(plan, node, true);
                }
                CacheLookup::Miss => {
                    let estimated_bytes = estimated_bytes(plan);
                    self.cache.observe(PlanObservation {
                        fingerprint,
                        estimated_bytes,
                        estimated_elapsed: None,
                    });
                    if !in_write_frontier {
                        let candidate = CacheCandidate {
                            key,
                            schema: plan.schema(),
                            properties: Arc::clone(plan.properties()),
                            partition_count: plan.output_partitioning().partition_count(),
                            estimated_bytes,
                        };
                        if self.cache.should_admit(&candidate) == AdmissionDecision::Admit {
                            if let Some(handle) = self.cache.begin_write(candidate) {
                                // Write frontier: descendants are not
                                // admitted, but keep descending so they are
                                // observed and can serve hits.
                                let rebuilt = self.rewrite_children(plan, node, true)?;
                                return Ok(Arc::new(CacheWriteExec::new(rebuilt, handle)));
                            }
                        }
                    }
                }
            }
        }
        self.rewrite_children(plan, node, in_write_frontier)
    }

    fn rewrite_children(
        &self,
        plan: &Arc<dyn ExecutionPlan>,
        node: &FingerprintedNode,
        in_write_frontier: bool,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let children = plan.children();
        if children.is_empty() {
            return Ok(Arc::clone(plan));
        }
        let mut rewritten = Vec::with_capacity(children.len());
        let mut changed = false;
        for (child, child_node) in children.into_iter().zip(&node.children) {
            let new_child = self.rewrite_node(child, child_node, in_write_frontier)?;
            changed |= !Arc::ptr_eq(child, &new_child);
            rewritten.push(new_child);
        }
        if changed {
            Arc::clone(plan).with_new_children(rewritten)
        } else {
            Ok(Arc::clone(plan))
        }
    }
}

impl PhysicalOptimizerRule for EvaluationCachePlanner {
    fn optimize(
        &self,
        plan: Arc<dyn ExecutionPlan>,
        config: &ConfigOptions,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        self.rewrite_with_config(plan, config)
    }

    fn name(&self) -> &str {
        "evaluation_cache"
    }

    fn schema_check(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct ExclusionCounts {
    dynamic: u64,
    volatile: u64,
    unbounded: u64,
    unsupported: u64,
}

fn count_exclusions(node: &FingerprintedNode, counts: &mut ExclusionCounts) {
    match &node.outcome {
        FingerprintOutcome::Excluded(ExclusionReason::DynamicExpr) => counts.dynamic += 1,
        FingerprintOutcome::Excluded(ExclusionReason::VolatileExpr) => counts.volatile += 1,
        FingerprintOutcome::Excluded(ExclusionReason::Unbounded) => counts.unbounded += 1,
        FingerprintOutcome::Excluded(ExclusionReason::UnsupportedNode { .. }) => {
            counts.unsupported += 1
        }
        FingerprintOutcome::Excluded(ExclusionReason::ExcludedChild)
        | FingerprintOutcome::Cacheable(_) => {}
    }
    for child in &node.children {
        count_exclusions(child, counts);
    }
}

fn any_cacheable(node: &FingerprintedNode) -> bool {
    matches!(node.outcome, FingerprintOutcome::Cacheable(_))
        || node.children.iter().any(any_cacheable)
}

/// Estimated output bytes from plan statistics, when they carry one.
fn estimated_bytes(plan: &Arc<dyn ExecutionPlan>) -> Option<usize> {
    let statistics = plan.partition_statistics(None).ok()?;
    match statistics.total_byte_size {
        Precision::Exact(bytes) | Precision::Inexact(bytes) => Some(bytes),
        Precision::Absent => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EvaluationCacheConfig;

    use arrow::array::{Int64Array, RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use datafusion::datasource::MemTable;
    use datafusion::execution::session_state::SessionStateBuilder;
    use datafusion::prelude::SessionContext;

    fn test_batch() -> (Arc<Schema>, RecordBatch) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("v", DataType::Int64, false),
            Field::new("k", DataType::Utf8, false),
        ]));
        let values: Vec<i64> = (0..100).collect();
        let keys: Vec<String> = values.iter().map(|v| format!("k{}", v % 4)).collect();
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(values)),
                Arc::new(StringArray::from(keys)),
            ],
        )
        .unwrap();
        (schema, batch)
    }

    /// Context with the cache rule installed as the last physical optimizer
    /// rule, plus a registered `MemTable`.
    fn cached_context(cache: Arc<EvaluationCache>) -> SessionContext {
        let planner = EvaluationCachePlanner::new(cache);
        let state = SessionStateBuilder::new()
            .with_default_features()
            .with_physical_optimizer_rule(Arc::new(planner))
            .build();
        let ctx = SessionContext::new_with_state(state);
        let (schema, batch) = test_batch();
        let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
        ctx.register_table("t", Arc::new(table)).unwrap();
        ctx
    }

    async fn run(ctx: &SessionContext, sql: &str) -> Vec<RecordBatch> {
        ctx.sql(sql).await.unwrap().collect().await.unwrap()
    }

    #[test]
    fn kill_switch_returns_input_unchanged() {
        let cache = EvaluationCache::new(EvaluationCacheConfig {
            enabled: false,
            ..Default::default()
        });
        let planner = EvaluationCachePlanner::new(Arc::clone(&cache));

        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
        let plan: Arc<dyn ExecutionPlan> =
            Arc::new(datafusion::physical_plan::empty::EmptyExec::new(schema));
        let rewritten = planner.rewrite(Arc::clone(&plan)).unwrap();
        assert!(
            Arc::ptr_eq(&plan, &rewritten),
            "disabled cache must return the identical Arc"
        );
        assert_eq!(
            cache.metrics(),
            crate::config::CacheMetricsSnapshot::default(),
            "disabled cache must record nothing"
        );
    }

    #[tokio::test]
    async fn kill_switch_end_to_end() {
        let cache = EvaluationCache::new(EvaluationCacheConfig {
            enabled: false,
            ..Default::default()
        });
        let ctx = cached_context(Arc::clone(&cache));
        let sql = "SELECT k, sum(v) AS s FROM t GROUP BY k ORDER BY k";
        let first = run(&ctx, sql).await;
        let second = run(&ctx, sql).await;
        assert_eq!(first, second);
        assert_eq!(
            cache.metrics(),
            crate::config::CacheMetricsSnapshot::default(),
            "disabled cache must show zero metric movement"
        );
    }

    #[tokio::test]
    async fn smoke_query_through_rule_installed_context() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let ctx = cached_context(cache);
        let batches = run(&ctx, "SELECT sum(v) AS s FROM t").await;
        let total = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0);
        assert_eq!(total, (0..100).sum::<i64>());
    }

    #[tokio::test]
    async fn repeated_aggregate_observe_admit_hit_sequence() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let ctx = cached_context(Arc::clone(&cache));
        let sql = "SELECT k, sum(v) AS s FROM t GROUP BY k ORDER BY k";

        // Run 1: every cacheable node misses and is observed; nothing is
        // admitted (seen-count 1 < 2).
        let first = run(&ctx, sql).await;
        let m1 = cache.metrics();
        assert_eq!(m1.hits, 0);
        assert!(m1.misses > 0, "cacheable nodes must be looked up");
        assert_eq!(m1.admitted_writes, 0);
        assert_eq!(m1.committed_writes, 0);
        let misses_per_run = m1.misses;

        // Run 2: the root reaches seen-count 2, is admitted, wrapped, and
        // committed during execution. Descendants stay unadmitted (write
        // frontier) but are still looked up and observed.
        let second = run(&ctx, sql).await;
        let m2 = cache.metrics();
        assert_eq!(m2.hits, 0);
        assert_eq!(m2.misses, misses_per_run * 2, "same plan, same lookups");
        assert_eq!(m2.admitted_writes, 1, "exactly the root admitted");
        assert_eq!(m2.committed_writes, 1);
        assert_eq!(m2.entries, 1);

        // Run 3: the root hits; descendants are shadowed (no new misses).
        let third = run(&ctx, sql).await;
        let m3 = cache.metrics();
        assert_eq!(m3.hits, 1);
        assert_eq!(
            m3.misses,
            misses_per_run * 2,
            "hit frontier: no descendant lookups"
        );
        assert_eq!(m3.admitted_writes, 1);

        assert_eq!(first, second);
        assert_eq!(second, third, "cached replay must equal live output");
    }

    #[tokio::test]
    async fn hit_frontier_shadows_descendant_observation() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let ctx = cached_context(Arc::clone(&cache));
        let sql = "SELECT k, sum(v) AS s FROM t GROUP BY k ORDER BY k";
        run(&ctx, sql).await;
        run(&ctx, sql).await;
        let observations_after_write = cache.inner.lock().unwrap().observations.len();
        run(&ctx, sql).await; // hit at the root
        let observations_after_hit = cache.inner.lock().unwrap().observations.len();
        assert_eq!(
            observations_after_write, observations_after_hit,
            "a root hit must not observe shadowed descendants"
        );
    }

    #[tokio::test]
    async fn literal_change_upstream_subtree_hits() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let ctx = cached_context(Arc::clone(&cache));

        // The scan subtree is shared by all three filters; the filter and
        // everything above it changes with the literal.
        run(&ctx, "SELECT v FROM t WHERE v > 5").await;
        run(&ctx, "SELECT v FROM t WHERE v > 6").await; // scan seen twice → admitted+committed
        let m2 = cache.metrics();
        assert_eq!(m2.committed_writes, 1, "shared upstream subtree committed");
        assert_eq!(m2.hits, 0);

        let results = run(&ctx, "SELECT v FROM t WHERE v > 7").await;
        let m3 = cache.metrics();
        assert!(
            m3.hits >= 1,
            "third literal reuses the committed upstream subtree"
        );
        let rows: usize = results.iter().map(|b| b.num_rows()).sum();
        assert_eq!(rows, 92, "correct filtered output through the cache read");
    }

    #[tokio::test]
    async fn dynamic_filter_topk_query_end_to_end() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let ctx = cached_context(Arc::clone(&cache));

        // Parquet-backed table so dynamic filter pushdown has a target.
        let dir = std::env::temp_dir().join(format!(
            "avenger-datafusion-cache-planner-topk-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.parquet").to_str().unwrap().to_string();
        ctx.sql("SELECT v, k FROM t")
            .await
            .unwrap()
            .write_parquet(
                &path,
                datafusion::dataframe::DataFrameWriteOptions::new(),
                None,
            )
            .await
            .unwrap();
        ctx.register_parquet("p", &path, Default::default())
            .await
            .unwrap();

        let sql = "SELECT v FROM p ORDER BY v LIMIT 5";
        let expected: Vec<i64> = (0..5).collect();
        for _ in 0..3 {
            let batches = run(&ctx, sql).await;
            let values: Vec<i64> = batches
                .iter()
                .flat_map(|b| {
                    b.column(0)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .unwrap()
                        .values()
                        .to_vec()
                })
                .collect();
            assert_eq!(
                values, expected,
                "TopK results correct with cache installed"
            );
        }
        let metrics = cache.metrics();
        assert!(metrics.excluded_dynamic > 0, "dynamic filters counted");
        assert_eq!(
            metrics.entries, 0,
            "nothing cacheable in a dynamic-filter plan"
        );
        assert_eq!(metrics.committed_writes, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn pending_entry_falls_back_without_second_write() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let ctx = cached_context(Arc::clone(&cache));
        let sql = "SELECT k, sum(v) AS s FROM t GROUP BY k ORDER BY k";

        // Plan (not execute) twice: the second planning admits the root and
        // leaves a PENDING entry (no execution has committed it yet).
        let plan_a = ctx
            .sql(sql)
            .await
            .unwrap()
            .create_physical_plan()
            .await
            .unwrap();
        let plan_b = ctx
            .sql(sql)
            .await
            .unwrap()
            .create_physical_plan()
            .await
            .unwrap();
        assert_eq!(cache.metrics().admitted_writes, 1);
        assert_eq!(cache.metrics().committed_writes, 0);

        // Planning the same query now sees Pending and falls back without a
        // second write (single-flight).
        let plan_c = ctx
            .sql(sql)
            .await
            .unwrap()
            .create_physical_plan()
            .await
            .unwrap();
        let metrics = cache.metrics();
        assert_eq!(metrics.pending_fallbacks, 1);
        assert_eq!(metrics.admitted_writes, 1, "no second write admitted");

        // All three plans execute correctly; at most one entry commits.
        let task_ctx = ctx.task_ctx();
        let a = datafusion::physical_plan::collect(plan_a, Arc::clone(&task_ctx))
            .await
            .unwrap();
        let b = datafusion::physical_plan::collect(plan_b, Arc::clone(&task_ctx))
            .await
            .unwrap();
        let c = datafusion::physical_plan::collect(plan_c, task_ctx)
            .await
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(cache.metrics().committed_writes, 1);
        assert_eq!(cache.metrics().entries, 1);
    }

    #[tokio::test]
    async fn observe_only_gesture_hint_mid_sequence() {
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let ctx = cached_context(Arc::clone(&cache));
        let sql = "SELECT k, sum(v) AS s FROM t GROUP BY k ORDER BY k";

        run(&ctx, sql).await; // observe (seen 1)
        cache.set_observe_only(true);
        run(&ctx, sql).await; // observe (seen 2) but decline admission
        assert_eq!(
            cache.metrics().admitted_writes,
            0,
            "observe-only blocks writes"
        );

        cache.set_observe_only(false);
        run(&ctx, sql).await; // seen 3, admitted, committed
        assert_eq!(cache.metrics().admitted_writes, 1);
        assert_eq!(cache.metrics().committed_writes, 1);

        run(&ctx, sql).await; // hit
        assert_eq!(cache.metrics().hits, 1);
    }
}
