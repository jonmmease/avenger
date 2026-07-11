//! Cache configuration and metrics.

use std::time::Duration;

/// Configuration for an [`EvaluationCache`](crate::store::EvaluationCache).
#[derive(Clone, Debug)]
pub struct EvaluationCacheConfig {
    /// Master kill switch. When `false`, the planner returns every plan
    /// unchanged and records nothing. Hosts map their own switches (for
    /// example an environment variable) onto this field.
    pub enabled: bool,
    /// Hard byte budget for committed entries. Eviction runs on commit until
    /// retained bytes fall back under this ceiling. This is a soft ceiling on
    /// process memory: batch sizes are approximate and in-flight readers keep
    /// evicted entries alive until dropped.
    pub max_memory_bytes: usize,
    /// Per-entry byte ceiling. Candidates estimated above it are not
    /// admitted, and staged writes that grow past it are discarded mid-write
    /// (the query itself is unaffected).
    pub max_entry_bytes: usize,
    /// Minimum number of recent observations of a fingerprint before a write
    /// is admitted.
    pub min_seen_count: u32,
    /// Minimum observed (or, when unavailable, estimated) subtree execution
    /// time before a write is admitted. When neither an observed nor an
    /// estimated elapsed time exists the clause passes optimistically: the
    /// first admitted write feeds real timing back into the observation
    /// table for future decisions.
    pub min_elapsed_for_admission: Duration,
    /// Capacity of the observation table. When full, the stalest record is
    /// evicted.
    pub observation_capacity: usize,
    /// Recency window for observations. A fingerprint whose last observation
    /// is older than this restarts its seen-count.
    pub observation_window: Duration,
}

impl Default for EvaluationCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_memory_bytes: 256 * 1024 * 1024,
            max_entry_bytes: 32 * 1024 * 1024,
            min_seen_count: 2,
            min_elapsed_for_admission: Duration::from_millis(5),
            observation_capacity: 4096,
            observation_window: Duration::from_secs(300),
        }
    }
}

/// Point-in-time copy of the cache's counters.
///
/// Returned by [`EvaluationCache::metrics`](crate::store::EvaluationCache::metrics).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheMetricsSnapshot {
    /// Lookups that found a committed entry.
    pub hits: u64,
    /// Lookups that found nothing.
    pub misses: u64,
    /// Lookups that found a pending (in-flight) entry and fell back to
    /// executing without admitting a second write.
    pub pending_fallbacks: u64,
    /// Writes admitted (a `CacheWriteExec` was installed).
    pub admitted_writes: u64,
    /// Writes that completed every partition and committed.
    pub committed_writes: u64,
    /// Writes discarded before commit (early termination, error, oversize,
    /// or handle drop).
    pub discarded_writes: u64,
    /// Committed entries evicted by the byte budget.
    pub evictions: u64,
    /// Committed entries currently retained.
    pub entries: usize,
    /// Approximate bytes currently retained by committed entries.
    pub bytes: usize,
    /// Subtrees excluded because they contain a runtime-mutated (dynamic
    /// filter) expression.
    pub excluded_dynamic: u64,
    /// Subtrees excluded because they contain a volatile expression.
    pub excluded_volatile: u64,
    /// Subtrees excluded because they are unbounded.
    pub excluded_unbounded: u64,
    /// Subtrees excluded because a node could not be fingerprinted.
    pub excluded_unsupported: u64,
    /// Total nanoseconds spent computing fingerprints.
    pub fingerprint_nanos_total: u64,
    /// Total nanoseconds spent in the planner rewrite (including the
    /// fingerprint pass).
    pub planner_nanos_total: u64,
}
