//! The in-memory cache store: entries, observations, admission, eviction.
//!
//! [`EvaluationCache`] is the concrete v1 cache (a trait is deferred until a
//! second implementation exists). It owns committed and pending entries
//! behind a mutex, a size-bounded observation table used by the admission
//! policy, and the cache-wide metrics. Committed entries are immutable and
//! shared with in-flight readers via `Arc`, so eviction never invalidates a
//! running query.
//!
//! Write lifecycle: [`EvaluationCache::begin_write`] inserts a *pending*
//! entry (single-flight: at most one write per key) and returns a
//! [`CacheWriteHandle`]. Batches are staged per partition; the entry commits
//! atomically when every partition completes. Any failure, oversize staging,
//! or dropping the last handle before completion discards the staged data,
//! removes the pending entry, and wakes any waiters — a discarded key can be
//! admitted again later.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use datafusion::physical_plan::PlanProperties;
use datafusion_common::Statistics;
use futures::channel::oneshot;

use crate::clock::{CacheClock, StdClock};
use crate::config::{CacheMetricsSnapshot, EvaluationCacheConfig};
use crate::fingerprint::{
    CacheVersionProvider, ContentHashMemo, PhysicalCacheKey, PlanFingerprint,
};

/// Result of a cache lookup.
pub enum CacheLookup {
    /// No entry for this key.
    Miss,
    /// A write for this key is in flight. Synchronous planners treat this as
    /// a miss without admitting a second write; asynchronous hosts may await
    /// the handle.
    Pending(CacheWaitHandle),
    /// A committed entry is available.
    Ready(CacheEntryRef),
}

impl std::fmt::Debug for CacheLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheLookup::Miss => f.write_str("Miss"),
            CacheLookup::Pending(_) => f.write_str("Pending"),
            CacheLookup::Ready(entry) => write!(f, "Ready({entry:?})"),
        }
    }
}

/// Shared reference to a committed cache entry.
///
/// Cheap to clone; keeps the entry alive independent of eviction.
#[derive(Clone)]
pub struct CacheEntryRef {
    pub(crate) entry: Arc<CommittedEntry>,
}

impl CacheEntryRef {
    /// Schema of the cached result.
    pub fn schema(&self) -> SchemaRef {
        Arc::clone(&self.entry.schema)
    }

    /// Number of cached output partitions.
    pub fn partition_count(&self) -> usize {
        self.entry.partitions.len()
    }

    /// Exact number of cached rows.
    pub fn rows(&self) -> usize {
        self.entry.actual_rows
    }

    /// Approximate retained bytes.
    pub fn bytes(&self) -> usize {
        self.entry.actual_bytes
    }
}

impl std::fmt::Debug for CacheEntryRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheEntryRef")
            .field("rows", &self.entry.actual_rows)
            .field("bytes", &self.entry.actual_bytes)
            .field("partitions", &self.entry.partitions.len())
            .finish()
    }
}

/// An immutable committed cache entry.
#[allow(dead_code)] // key/properties/statistics consumed by Phase 3 exec nodes
pub(crate) struct CommittedEntry {
    pub(crate) key: PhysicalCacheKey,
    pub(crate) schema: SchemaRef,
    pub(crate) properties: Arc<PlanProperties>,
    /// Batches per output partition, in partition order.
    pub(crate) partitions: Vec<Vec<RecordBatch>>,
    /// Exact statistics for the whole result.
    pub(crate) statistics: Arc<Statistics>,
    /// Exact statistics per partition.
    pub(crate) partition_statistics: Vec<Arc<Statistics>>,
    pub(crate) actual_rows: usize,
    pub(crate) actual_bytes: usize,
    pub(crate) created_at: Duration,
}

/// Waiter for an in-flight cache write.
///
/// Resolves when the pending entry commits *or* discards; the caller must
/// re-issue the lookup to learn which. Exists for asynchronous wrapper-mode
/// hosts; the synchronous optimizer rule never waits.
pub struct CacheWaitHandle {
    pub(crate) receiver: oneshot::Receiver<()>,
}

impl CacheWaitHandle {
    /// Wait until the in-flight write commits or discards.
    pub async fn wait(self) {
        // A dropped sender (discard path) resolves the wait as well; the
        // Err carries no information beyond "no longer pending".
        let _ = self.receiver.await;
    }
}

impl std::fmt::Debug for CacheWaitHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CacheWaitHandle")
    }
}

/// One observation of a cacheable fingerprint during planning.
#[derive(Clone, Debug)]
pub struct PlanObservation {
    /// The observed subtree fingerprint.
    pub fingerprint: PlanFingerprint,
    /// Estimated output bytes from plan statistics, when available.
    pub estimated_bytes: Option<usize>,
    /// Estimated execution time, when available. Plan statistics rarely
    /// provide one; observed times are fed back by committed writes instead.
    pub estimated_elapsed: Option<Duration>,
}

/// A fully described admission candidate.
#[derive(Clone)]
pub struct CacheCandidate {
    /// Full cache key for the subtree.
    pub key: PhysicalCacheKey,
    /// Output schema captured from the subtree.
    pub schema: SchemaRef,
    /// Output properties captured from the subtree; served verbatim by
    /// `CacheReadExec` on later hits.
    pub properties: Arc<PlanProperties>,
    /// Number of output partitions the write must complete.
    pub partition_count: usize,
    /// Estimated output bytes, when plan statistics provide one.
    pub estimated_bytes: Option<usize>,
}

impl std::fmt::Debug for CacheCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheCandidate")
            .field("key", &self.key)
            .field("partition_count", &self.partition_count)
            .field("estimated_bytes", &self.estimated_bytes)
            .finish()
    }
}

/// Admission verdict for a candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionDecision {
    /// Install a `CacheWriteExec` for this candidate.
    Admit,
    /// Do not write; the reason is diagnostic.
    Decline(DeclineReason),
}

/// Why a candidate was not admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclineReason {
    /// The cache is in observe-only mode (interaction gesture hint).
    ObserveOnly,
    /// The fingerprint has not been seen often enough recently.
    NotSeenEnough,
    /// The observed or estimated execution time is below the admission floor.
    TooCheap,
    /// The estimated output exceeds the per-entry byte ceiling.
    TooLarge,
    /// An entry (pending or committed) already exists for the key.
    AlreadyPresent,
}

/// State of one keyed cache slot.
pub(crate) enum EntryState {
    /// A write is in flight.
    Pending(PendingState),
    /// A committed, immutable entry.
    Committed(CommittedSlot),
}

pub(crate) struct PendingState {
    #[allow(dead_code)] // diagnostic; also read by future stretch policies
    pub(crate) started_at: Duration,
    /// Waiters notified on commit (sent) or discard (senders dropped).
    pub(crate) waiters: Vec<oneshot::Sender<()>>,
}

pub(crate) struct CommittedSlot {
    pub(crate) entry: Arc<CommittedEntry>,
    /// Logical access stamp for least-recently-used eviction.
    pub(crate) last_access: u64,
    pub(crate) hits: u64,
}

/// A recorded observation of a fingerprint.
#[derive(Clone, Debug)]
pub(crate) struct ObservationRecord {
    pub(crate) seen_count: u32,
    #[allow(dead_code)] // diagnostic
    pub(crate) first_seen: Duration,
    pub(crate) last_seen: Duration,
    pub(crate) estimated_bytes: Option<usize>,
    pub(crate) estimated_elapsed: Option<Duration>,
    /// Real elapsed time fed back by a committed write.
    pub(crate) observed_elapsed: Option<Duration>,
}

pub(crate) struct StoreInner {
    pub(crate) entries: HashMap<PhysicalCacheKey, EntryState>,
    pub(crate) bytes_total: usize,
    pub(crate) access_counter: u64,
    pub(crate) observations: HashMap<PlanFingerprint, ObservationRecord>,
    pub(crate) metrics: CacheMetricsSnapshot,
    pub(crate) observe_only: bool,
}

/// Concrete physical-plan result cache.
///
/// See the [crate docs](crate) for the overall design. Construct with
/// [`EvaluationCache::new`], register it with an
/// [`EvaluationCachePlanner`](crate::planner::EvaluationCachePlanner), and
/// install the planner as a physical optimizer rule.
pub struct EvaluationCache {
    pub(crate) config: EvaluationCacheConfig,
    pub(crate) clock: Arc<dyn CacheClock>,
    pub(crate) inner: Mutex<StoreInner>,
    pub(crate) providers: Mutex<Vec<Arc<dyn CacheVersionProvider>>>,
    pub(crate) memo: Arc<ContentHashMemo>,
}

impl std::fmt::Debug for EvaluationCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().unwrap();
        f.debug_struct("EvaluationCache")
            .field("entries", &inner.entries.len())
            .field("bytes_total", &inner.bytes_total)
            .field("observe_only", &inner.observe_only)
            .finish()
    }
}

impl EvaluationCache {
    /// Create a cache with the default [`StdClock`].
    pub fn new(config: EvaluationCacheConfig) -> Arc<Self> {
        Self::with_clock(config, Arc::new(StdClock::new()))
    }

    /// Create a cache with an injected clock (required on targets without
    /// [`std::time::Instant`], such as `wasm32-unknown-unknown`).
    pub fn with_clock(config: EvaluationCacheConfig, clock: Arc<dyn CacheClock>) -> Arc<Self> {
        Arc::new(Self {
            config,
            clock,
            inner: Mutex::new(StoreInner {
                entries: HashMap::new(),
                bytes_total: 0,
                access_counter: 0,
                observations: HashMap::new(),
                metrics: CacheMetricsSnapshot::default(),
                observe_only: false,
            }),
            providers: Mutex::new(Vec::new()),
            memo: Arc::new(ContentHashMemo::default()),
        })
    }

    /// The configuration this cache was built with.
    pub fn config(&self) -> &EvaluationCacheConfig {
        &self.config
    }

    /// Register a source version provider. Providers are consulted in
    /// registration order; the first `Some` wins.
    pub fn register_version_provider(&self, provider: Arc<dyn CacheVersionProvider>) {
        self.providers.lock().unwrap().push(provider);
    }

    /// Snapshot of the registered version providers.
    pub(crate) fn version_providers(&self) -> Vec<Arc<dyn CacheVersionProvider>> {
        self.providers.lock().unwrap().clone()
    }

    /// Toggle observe-only mode (the interaction/gesture hint): lookups are
    /// still served and fingerprints still observed, but no new writes are
    /// admitted while enabled.
    pub fn set_observe_only(&self, observe_only: bool) {
        self.inner.lock().unwrap().observe_only = observe_only;
    }

    /// Look up a key. Updates hit/miss/pending metrics and the entry's
    /// recency stamp. A `Pending` result carries a wait handle registered
    /// with the in-flight write.
    pub fn lookup(&self, key: &PhysicalCacheKey) -> CacheLookup {
        let mut inner = self.inner.lock().unwrap();
        inner.access_counter += 1;
        let stamp = inner.access_counter;
        match inner.entries.get_mut(key) {
            Some(EntryState::Committed(slot)) => {
                slot.last_access = stamp;
                slot.hits += 1;
                let entry = Arc::clone(&slot.entry);
                inner.metrics.hits += 1;
                CacheLookup::Ready(CacheEntryRef { entry })
            }
            Some(EntryState::Pending(pending)) => {
                let (sender, receiver) = oneshot::channel();
                pending.waiters.push(sender);
                inner.metrics.pending_fallbacks += 1;
                CacheLookup::Pending(CacheWaitHandle { receiver })
            }
            None => {
                inner.metrics.misses += 1;
                CacheLookup::Miss
            }
        }
    }

    /// Record an observation of a cacheable fingerprint.
    ///
    /// Repeated observations within the recency window raise the seen-count;
    /// a stale record restarts at one. The table is capacity-bounded; when
    /// full, the stalest record is evicted.
    pub fn observe(&self, observation: PlanObservation) {
        let now = self.clock.now();
        let window = self.config.observation_window;
        let capacity = self.config.observation_capacity.max(1);
        let mut inner = self.inner.lock().unwrap();

        match inner.observations.get_mut(&observation.fingerprint) {
            Some(record) => {
                if now.saturating_sub(record.last_seen) > window {
                    record.seen_count = 1;
                } else {
                    record.seen_count = record.seen_count.saturating_add(1);
                }
                record.last_seen = now;
                if observation.estimated_bytes.is_some() {
                    record.estimated_bytes = observation.estimated_bytes;
                }
                if observation.estimated_elapsed.is_some() {
                    record.estimated_elapsed = observation.estimated_elapsed;
                }
            }
            None => {
                if inner.observations.len() >= capacity {
                    if let Some(&stalest) = inner
                        .observations
                        .iter()
                        .min_by_key(|(_, record)| record.last_seen)
                        .map(|(fingerprint, _)| fingerprint)
                    {
                        inner.observations.remove(&stalest);
                    }
                }
                inner.observations.insert(
                    observation.fingerprint,
                    ObservationRecord {
                        seen_count: 1,
                        first_seen: now,
                        last_seen: now,
                        estimated_bytes: observation.estimated_bytes,
                        estimated_elapsed: observation.estimated_elapsed,
                        observed_elapsed: None,
                    },
                );
            }
        }
    }

    /// Evaluate the admission policy for a candidate.
    ///
    /// Admits when the fingerprint has been seen at least
    /// `min_seen_count` times within the recency window, the observed (or
    /// estimated) elapsed time clears the admission floor — passing
    /// optimistically when neither exists — the estimated bytes fit the
    /// per-entry ceiling, the cache is not observe-only, and no entry
    /// already exists for the key.
    pub fn should_admit(&self, candidate: &CacheCandidate) -> AdmissionDecision {
        let now = self.clock.now();
        let window = self.config.observation_window;
        let inner = self.inner.lock().unwrap();

        if inner.observe_only {
            return AdmissionDecision::Decline(DeclineReason::ObserveOnly);
        }
        if inner.entries.contains_key(&candidate.key) {
            return AdmissionDecision::Decline(DeclineReason::AlreadyPresent);
        }

        let record = inner.observations.get(&candidate.key.subtree_fingerprint);
        let recent_seen = match record {
            Some(record) if now.saturating_sub(record.last_seen) <= window => record.seen_count,
            _ => 0,
        };
        if recent_seen < self.config.min_seen_count {
            return AdmissionDecision::Decline(DeclineReason::NotSeenEnough);
        }

        let elapsed = record.and_then(|r| r.observed_elapsed.or(r.estimated_elapsed));
        if let Some(elapsed) = elapsed {
            if elapsed < self.config.min_elapsed_for_admission {
                return AdmissionDecision::Decline(DeclineReason::TooCheap);
            }
        }

        let estimated_bytes = candidate
            .estimated_bytes
            .or_else(|| record.and_then(|r| r.estimated_bytes));
        if let Some(bytes) = estimated_bytes {
            if bytes > self.config.max_entry_bytes {
                return AdmissionDecision::Decline(DeclineReason::TooLarge);
            }
        }

        AdmissionDecision::Admit
    }

    /// Begin a single-flight write for a candidate: inserts a pending entry
    /// and returns a staging handle, or `None` when an entry (pending or
    /// committed) already exists for the key.
    pub fn begin_write(self: &Arc<Self>, candidate: CacheCandidate) -> Option<CacheWriteHandle> {
        let now = self.clock.now();
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.entries.contains_key(&candidate.key) {
                return None;
            }
            inner.entries.insert(
                candidate.key,
                EntryState::Pending(PendingState {
                    started_at: now,
                    waiters: Vec::new(),
                }),
            );
            inner.metrics.admitted_writes += 1;
        }
        Some(CacheWriteHandle {
            inner: Arc::new(WriteHandleInner {
                key: candidate.key,
                schema: candidate.schema,
                properties: candidate.properties,
                partition_count: candidate.partition_count,
                max_entry_bytes: self.config.max_entry_bytes,
                clock: Arc::clone(&self.clock),
                cache: Arc::downgrade(self),
                staging: Mutex::new(StagingState::new(candidate.partition_count, now)),
            }),
        })
    }

    /// Point-in-time copy of the metrics counters.
    pub fn metrics(&self) -> CacheMetricsSnapshot {
        let inner = self.inner.lock().unwrap();
        let mut snapshot = inner.metrics.clone();
        snapshot.entries = inner
            .entries
            .values()
            .filter(|state| matches!(state, EntryState::Committed(_)))
            .count();
        snapshot.bytes = inner.bytes_total;
        snapshot
    }

    /// Drop every entry and observation, returning the cache to a cold
    /// start. Cumulative counters (hits, misses, writes, evictions) are
    /// retained; `entries` and `bytes` reset with the content. In-flight
    /// write handles whose pending entries are cleared discard harmlessly.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.entries.clear();
        inner.observations.clear();
        inner.bytes_total = 0;
    }

    /// Accumulate exclusion counts from a fingerprint pass.
    pub(crate) fn record_exclusions(
        &self,
        dynamic: u64,
        volatile: u64,
        unbounded: u64,
        unsupported: u64,
    ) {
        let mut inner = self.inner.lock().unwrap();
        inner.metrics.excluded_dynamic += dynamic;
        inner.metrics.excluded_volatile += volatile;
        inner.metrics.excluded_unbounded += unbounded;
        inner.metrics.excluded_unsupported += unsupported;
    }

    /// Accumulate planner timing.
    pub(crate) fn record_planner_timing(&self, fingerprint_nanos: u64, planner_nanos: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.metrics.fingerprint_nanos_total += fingerprint_nanos;
        inner.metrics.planner_nanos_total += planner_nanos;
    }

    /// Commit a completed write: replace the pending entry, account bytes,
    /// feed observed elapsed back into the observation table, notify
    /// waiters, and run eviction.
    fn commit(&self, entry: CommittedEntry, observed_elapsed: Option<Duration>) {
        let key = entry.key;
        let fingerprint = key.subtree_fingerprint;
        let bytes = entry.actual_bytes;
        let mut waiters = Vec::new();

        let mut inner = self.inner.lock().unwrap();
        if let Some(EntryState::Pending(pending)) = inner.entries.get_mut(&key) {
            waiters = std::mem::take(&mut pending.waiters);
        }
        // Insert regardless of whether the pending entry survived (a clear()
        // between begin and commit is not a correctness event; the data is
        // complete and valid).
        inner.access_counter += 1;
        let stamp = inner.access_counter;
        inner.entries.insert(
            key,
            EntryState::Committed(CommittedSlot {
                entry: Arc::new(entry),
                last_access: stamp,
                hits: 0,
            }),
        );
        inner.bytes_total += bytes;
        inner.metrics.committed_writes += 1;

        if let Some(elapsed) = observed_elapsed {
            if let Some(record) = inner.observations.get_mut(&fingerprint) {
                record.observed_elapsed = Some(elapsed);
            }
        }

        // Evict least-recently-accessed committed entries until back under
        // budget. Pending entries are never evicted. O(n) scans are
        // acceptable: the map is small by construction (byte-budgeted
        // entries, bounded admission).
        while inner.bytes_total > self.config.max_memory_bytes {
            let victim = inner
                .entries
                .iter()
                .filter_map(|(key, state)| match state {
                    EntryState::Committed(slot) => Some((*key, slot.last_access)),
                    EntryState::Pending(_) => None,
                })
                .min_by_key(|(_, last_access)| *last_access)
                .map(|(key, _)| key);
            match victim {
                Some(victim) => {
                    if let Some(EntryState::Committed(slot)) = inner.entries.remove(&victim) {
                        inner.bytes_total =
                            inner.bytes_total.saturating_sub(slot.entry.actual_bytes);
                        inner.metrics.evictions += 1;
                    }
                }
                None => break,
            }
        }
        drop(inner);

        for waiter in waiters {
            let _ = waiter.send(());
        }
    }

    /// Discard a pending entry (write failed, oversized, terminated early,
    /// or the handle was dropped). Waiters resolve via dropped senders.
    fn discard(&self, key: &PhysicalCacheKey) {
        let mut inner = self.inner.lock().unwrap();
        if matches!(inner.entries.get(key), Some(EntryState::Pending(_))) {
            inner.entries.remove(key);
        }
        inner.metrics.discarded_writes += 1;
    }
}

/// Handle for staging an admitted cache write.
///
/// Obtained from [`EvaluationCache::begin_write`]; cheap to clone and shared
/// by the write node's per-partition streams. The entry commits atomically
/// when every partition completes; any failure, oversize staging, or
/// dropping the last handle before completion discards the staged data and
/// removes the pending entry.
#[derive(Clone)]
pub struct CacheWriteHandle {
    pub(crate) inner: Arc<WriteHandleInner>,
}

impl std::fmt::Debug for CacheWriteHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CacheWriteHandle")
    }
}

impl CacheWriteHandle {
    /// Stage one batch for a partition. Batches are `Arc`-cheap to clone.
    /// Staging on an already-complete partition resets that partition
    /// (last-write-wins re-execution semantics).
    pub fn stage(&self, partition: usize, batch: RecordBatch) {
        self.inner.stage(partition, batch);
    }

    /// Mark a partition's stream complete. When every partition is
    /// complete, the entry commits atomically.
    pub fn complete_partition(&self, partition: usize) {
        self.inner.complete_partition(partition);
    }

    /// Report a failure on a partition: the whole entry is discarded (the
    /// query itself is unaffected — the cache fails open).
    pub fn fail(&self, partition: usize) {
        self.inner.fail(partition);
    }

    /// Record first activity for elapsed-time measurement (idempotent).
    pub fn note_first_poll(&self) {
        self.inner.note_first_activity();
    }

    /// The key this write will commit under.
    #[allow(dead_code)] // consumed by the Phase 3 write node display
    pub(crate) fn key(&self) -> PhysicalCacheKey {
        self.inner.key
    }
}

pub(crate) struct WriteHandleInner {
    key: PhysicalCacheKey,
    schema: SchemaRef,
    properties: Arc<PlanProperties>,
    partition_count: usize,
    max_entry_bytes: usize,
    clock: Arc<dyn CacheClock>,
    cache: Weak<EvaluationCache>,
    staging: Mutex<StagingState>,
}

struct StagingState {
    partitions: Vec<PartitionStaging>,
    completed: usize,
    bytes: usize,
    started_at: Duration,
    first_activity: Option<Duration>,
    /// Terminal: committed or discarded. All further calls no-op.
    finished: bool,
}

#[derive(Default)]
struct PartitionStaging {
    batches: Vec<RecordBatch>,
    bytes: usize,
    complete: bool,
}

impl StagingState {
    fn new(partition_count: usize, started_at: Duration) -> Self {
        Self {
            partitions: (0..partition_count)
                .map(|_| PartitionStaging::default())
                .collect(),
            completed: 0,
            bytes: 0,
            started_at,
            first_activity: None,
            finished: false,
        }
    }

    /// Reset a completed partition for re-execution (last-write-wins).
    fn reset_partition(&mut self, partition: usize) {
        let slot = &mut self.partitions[partition];
        if slot.complete {
            self.completed -= 1;
            slot.complete = false;
        }
        self.bytes -= slot.bytes;
        slot.bytes = 0;
        slot.batches.clear();
    }
}

impl WriteHandleInner {
    fn note_first_activity(&self) {
        let now = self.clock.now();
        let mut staging = self.staging.lock().unwrap();
        if staging.first_activity.is_none() {
            staging.first_activity = Some(now);
        }
    }

    fn stage(&self, partition: usize, batch: RecordBatch) {
        let now = self.clock.now();
        let mut staging = self.staging.lock().unwrap();
        if staging.finished || partition >= self.partition_count {
            return;
        }
        if staging.first_activity.is_none() {
            staging.first_activity = Some(now);
        }
        if staging.partitions[partition].complete {
            staging.reset_partition(partition);
        }
        let batch_bytes = batch.get_array_memory_size();
        if staging.bytes + batch_bytes > self.max_entry_bytes {
            // Oversize: abort the whole entry but keep the query streaming.
            staging.finished = true;
            drop(staging);
            if let Some(cache) = self.cache.upgrade() {
                cache.discard(&self.key);
            }
            return;
        }
        let slot = &mut staging.partitions[partition];
        slot.batches.push(batch);
        slot.bytes += batch_bytes;
        staging.bytes += batch_bytes;
    }

    fn complete_partition(&self, partition: usize) {
        let now = self.clock.now();
        let mut staging = self.staging.lock().unwrap();
        if staging.finished || partition >= self.partition_count {
            return;
        }
        let slot = &mut staging.partitions[partition];
        if slot.complete {
            // Re-executed stream completed again without staging in between:
            // last write wins, and that write was empty.
            staging.reset_partition(partition);
        }
        let slot = &mut staging.partitions[partition];
        slot.complete = true;
        staging.completed += 1;
        if staging.completed < self.partition_count {
            return;
        }

        // All partitions complete: assemble outside the store lock (we hold
        // only the staging lock) and commit.
        staging.finished = true;
        let partitions: Vec<Vec<RecordBatch>> = staging
            .partitions
            .iter_mut()
            .map(|slot| std::mem::take(&mut slot.batches))
            .collect();
        let observed_elapsed = staging
            .first_activity
            .or(Some(staging.started_at))
            .map(|start| now.saturating_sub(start));
        drop(staging);

        let Some(cache) = self.cache.upgrade() else {
            return;
        };
        let schema = Arc::clone(&self.schema);
        let statistics = Arc::new(
            datafusion::physical_plan::common::compute_record_batch_statistics(
                &partitions,
                schema.as_ref(),
                None,
            ),
        );
        let partition_statistics: Vec<Arc<Statistics>> = partitions
            .iter()
            .map(|partition| {
                Arc::new(
                    datafusion::physical_plan::common::compute_record_batch_statistics(
                        std::slice::from_ref(partition),
                        schema.as_ref(),
                        None,
                    ),
                )
            })
            .collect();
        let actual_rows: usize = partitions
            .iter()
            .flat_map(|partition| partition.iter().map(|batch| batch.num_rows()))
            .sum();
        let actual_bytes: usize = partitions
            .iter()
            .flat_map(|partition| partition.iter().map(|batch| batch.get_array_memory_size()))
            .sum();

        cache.commit(
            CommittedEntry {
                key: self.key,
                schema,
                properties: Arc::clone(&self.properties),
                partitions,
                statistics,
                partition_statistics,
                actual_rows,
                actual_bytes,
                created_at: now,
            },
            observed_elapsed,
        );
    }

    fn fail(&self, _partition: usize) {
        let mut staging = self.staging.lock().unwrap();
        if staging.finished {
            return;
        }
        staging.finished = true;
        drop(staging);
        if let Some(cache) = self.cache.upgrade() {
            cache.discard(&self.key);
        }
    }
}

impl Drop for WriteHandleInner {
    fn drop(&mut self) {
        let finished = self.staging.get_mut().map(|s| s.finished).unwrap_or(true);
        if !finished {
            if let Some(cache) = self.cache.upgrade() {
                cache.discard(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::test_support::ManualClock;
    use crate::fingerprint::PlanFingerprint;
    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use datafusion::physical_plan::ExecutionPlan;
    use datafusion::physical_plan::empty::EmptyExec;

    fn key(n: u128) -> PhysicalCacheKey {
        PhysicalCacheKey {
            subtree_fingerprint: PlanFingerprint(n),
            schema_fingerprint: 1,
            partitioning_fingerprint: 2,
            ordering_fingerprint: 3,
            bounded: true,
        }
    }

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]))
    }

    fn batch(values: &[i64]) -> RecordBatch {
        RecordBatch::try_new(schema(), vec![Arc::new(Int64Array::from(values.to_vec()))]).unwrap()
    }

    fn candidate(n: u128, partitions: usize) -> CacheCandidate {
        let empty = EmptyExec::new(schema()).with_partitions(partitions);
        CacheCandidate {
            key: key(n),
            schema: schema(),
            properties: Arc::new(empty.properties().as_ref().clone()),
            partition_count: partitions,
            estimated_bytes: None,
        }
    }

    fn config() -> EvaluationCacheConfig {
        EvaluationCacheConfig::default()
    }

    fn commit_entry(cache: &Arc<EvaluationCache>, n: u128, values: &[i64]) {
        let handle = cache
            .begin_write(candidate(n, 1))
            .expect("single-flight write");
        handle.stage(0, batch(values));
        handle.complete_partition(0);
    }

    #[test]
    fn lookup_transitions_miss_pending_ready() {
        let cache = EvaluationCache::new(config());
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));

        let handle = cache.begin_write(candidate(1, 1)).unwrap();
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Pending(_)));

        handle.stage(0, batch(&[1, 2, 3]));
        handle.complete_partition(0);
        match cache.lookup(&key(1)) {
            CacheLookup::Ready(entry) => {
                assert_eq!(entry.rows(), 3);
                assert_eq!(entry.partition_count(), 1);
            }
            other => panic!("expected Ready, got {other:?}"),
        }

        let metrics = cache.metrics();
        assert_eq!(metrics.misses, 1);
        assert_eq!(metrics.pending_fallbacks, 1);
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.admitted_writes, 1);
        assert_eq!(metrics.committed_writes, 1);
        assert_eq!(metrics.entries, 1);
        assert!(metrics.bytes > 0);
    }

    #[test]
    fn lru_eviction_under_tiny_budget() {
        let entry_bytes = batch(&[1, 2, 3]).get_array_memory_size();
        let cache = EvaluationCache::new(EvaluationCacheConfig {
            max_memory_bytes: entry_bytes + entry_bytes / 2, // holds one entry
            ..config()
        });
        commit_entry(&cache, 1, &[1, 2, 3]);
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Ready(_)));

        commit_entry(&cache, 2, &[4, 5, 6]);
        let metrics = cache.metrics();
        assert_eq!(metrics.evictions, 1);
        assert_eq!(metrics.entries, 1);
        // Entry 1 was least recently accessed; entry 2 survives.
        assert!(matches!(cache.lookup(&key(2)), CacheLookup::Ready(_)));
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
    }

    #[test]
    fn eviction_never_touches_pending() {
        let entry_bytes = batch(&[1, 2, 3]).get_array_memory_size();
        let cache = EvaluationCache::new(EvaluationCacheConfig {
            max_memory_bytes: entry_bytes / 2, // nothing fits
            ..config()
        });
        let pending_handle = cache.begin_write(candidate(7, 1)).unwrap();
        commit_entry(&cache, 1, &[1, 2, 3]);
        // The committed entry immediately evicts itself (over budget), but
        // the pending entry must survive.
        assert!(matches!(cache.lookup(&key(7)), CacheLookup::Pending(_)));
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        assert_eq!(cache.metrics().evictions, 1);
        drop(pending_handle);
    }

    #[test]
    fn observation_recency_window_and_capacity() {
        let clock = Arc::new(ManualClock::default());
        let cache = EvaluationCache::with_clock(
            EvaluationCacheConfig {
                observation_capacity: 2,
                observation_window: Duration::from_secs(10),
                min_seen_count: 2,
                ..config()
            },
            Arc::clone(&clock) as Arc<dyn CacheClock>,
        );
        let observe = |fp: u128| {
            cache.observe(PlanObservation {
                fingerprint: PlanFingerprint(fp),
                estimated_bytes: None,
                estimated_elapsed: None,
            })
        };

        observe(1);
        observe(1);
        assert_eq!(
            cache.should_admit(&candidate(1, 1)),
            AdmissionDecision::Admit,
            "two recent observations must admit"
        );

        // Stale: past the window, the count restarts.
        clock.advance(Duration::from_secs(11));
        observe(1);
        assert_eq!(
            cache.should_admit(&candidate(1, 1)),
            AdmissionDecision::Decline(DeclineReason::NotSeenEnough),
            "stale record restarts at one"
        );

        // Capacity: a third fingerprint evicts the stalest.
        observe(2);
        observe(3);
        let inner = cache.inner.lock().unwrap();
        assert_eq!(inner.observations.len(), 2);
        drop(inner);
    }

    #[test]
    fn admission_truth_table() {
        let clock = Arc::new(ManualClock::default());
        let cache = EvaluationCache::with_clock(
            EvaluationCacheConfig {
                min_seen_count: 2,
                min_elapsed_for_admission: Duration::from_millis(5),
                max_entry_bytes: 1000,
                ..config()
            },
            Arc::clone(&clock) as Arc<dyn CacheClock>,
        );
        let observe = |fp: u128, elapsed: Option<Duration>| {
            cache.observe(PlanObservation {
                fingerprint: PlanFingerprint(fp),
                estimated_bytes: None,
                estimated_elapsed: elapsed,
            })
        };

        // NotSeenEnough: zero and one observation.
        assert_eq!(
            cache.should_admit(&candidate(1, 1)),
            AdmissionDecision::Decline(DeclineReason::NotSeenEnough)
        );
        observe(1, None);
        assert_eq!(
            cache.should_admit(&candidate(1, 1)),
            AdmissionDecision::Decline(DeclineReason::NotSeenEnough)
        );

        // Admit: seen twice, no elapsed info (optimistic pass), no size info.
        observe(1, None);
        assert_eq!(
            cache.should_admit(&candidate(1, 1)),
            AdmissionDecision::Admit
        );

        // TooCheap: estimated elapsed below the floor.
        observe(2, Some(Duration::from_millis(1)));
        observe(2, Some(Duration::from_millis(1)));
        assert_eq!(
            cache.should_admit(&candidate(2, 1)),
            AdmissionDecision::Decline(DeclineReason::TooCheap)
        );

        // TooLarge: candidate estimate above the entry ceiling.
        observe(3, None);
        observe(3, None);
        let mut too_large = candidate(3, 1);
        too_large.estimated_bytes = Some(10_000);
        assert_eq!(
            cache.should_admit(&too_large),
            AdmissionDecision::Decline(DeclineReason::TooLarge)
        );

        // ObserveOnly gates everything.
        cache.set_observe_only(true);
        assert_eq!(
            cache.should_admit(&candidate(1, 1)),
            AdmissionDecision::Decline(DeclineReason::ObserveOnly)
        );
        cache.set_observe_only(false);

        // AlreadyPresent: pending entry for the key.
        let handle = cache.begin_write(candidate(1, 1)).unwrap();
        assert_eq!(
            cache.should_admit(&candidate(1, 1)),
            AdmissionDecision::Decline(DeclineReason::AlreadyPresent)
        );
        drop(handle);
    }

    #[test]
    fn single_flight_second_begin_write_returns_none() {
        let cache = EvaluationCache::new(config());
        let first = cache.begin_write(candidate(1, 1)).unwrap();
        assert!(cache.begin_write(candidate(1, 1)).is_none());
        drop(first);
        // After discard the key can be admitted again.
        assert!(cache.begin_write(candidate(1, 1)).is_some());
    }

    #[tokio::test]
    async fn waiter_resolves_on_commit_and_on_discard() {
        let cache = EvaluationCache::new(config());

        // Commit path.
        let handle = cache.begin_write(candidate(1, 1)).unwrap();
        let CacheLookup::Pending(waiter) = cache.lookup(&key(1)) else {
            panic!("expected pending");
        };
        handle.stage(0, batch(&[1]));
        handle.complete_partition(0);
        waiter.wait().await; // must not hang
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Ready(_)));

        // Discard path (handle dropped before completion).
        let handle = cache.begin_write(candidate(2, 1)).unwrap();
        let CacheLookup::Pending(waiter) = cache.lookup(&key(2)) else {
            panic!("expected pending");
        };
        drop(handle);
        waiter.wait().await; // resolves via dropped sender
        assert!(matches!(cache.lookup(&key(2)), CacheLookup::Miss));
        assert_eq!(cache.metrics().discarded_writes, 1);
    }

    #[test]
    fn observe_only_blocks_admission_not_lookup() {
        let cache = EvaluationCache::new(config());
        commit_entry(&cache, 1, &[1, 2, 3]);
        cache.set_observe_only(true);
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Ready(_)));
        assert_eq!(
            cache.should_admit(&candidate(9, 1)),
            AdmissionDecision::Decline(DeclineReason::ObserveOnly)
        );
    }

    #[test]
    fn byte_accounting_returns_to_zero() {
        let cache = EvaluationCache::new(config());
        commit_entry(&cache, 1, &[1, 2, 3]);
        assert!(cache.metrics().bytes > 0);
        cache.clear();
        let metrics = cache.metrics();
        assert_eq!(metrics.bytes, 0);
        assert_eq!(metrics.entries, 0);
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
    }

    #[test]
    fn re_executed_partition_commits_once_with_last_write() {
        let cache = EvaluationCache::new(config());
        let handle = cache.begin_write(candidate(1, 2)).unwrap();

        // Partition 0 executes twice; the second execution's data wins.
        handle.stage(0, batch(&[1, 1, 1]));
        handle.complete_partition(0);
        handle.stage(0, batch(&[9, 9]));
        handle.complete_partition(0);
        handle.stage(1, batch(&[5]));
        handle.complete_partition(1);

        let CacheLookup::Ready(entry) = cache.lookup(&key(1)) else {
            panic!("expected committed entry");
        };
        assert_eq!(
            entry.rows(),
            3,
            "partition 0 last write (2 rows) + partition 1 (1 row)"
        );
        assert_eq!(cache.metrics().committed_writes, 1);
    }

    #[test]
    fn incomplete_partition_never_commits_and_can_readmit() {
        let cache = EvaluationCache::new(config());
        let handle = cache.begin_write(candidate(1, 2)).unwrap();
        handle.stage(0, batch(&[1]));
        handle.complete_partition(0);
        // Partition 1 never executes.
        drop(handle);
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        assert_eq!(cache.metrics().committed_writes, 0);
        assert_eq!(cache.metrics().discarded_writes, 1);
        // The discarded key is admissible again.
        assert!(cache.begin_write(candidate(1, 2)).is_some());
    }

    #[test]
    fn oversize_staging_discards_whole_entry() {
        let cache = EvaluationCache::new(EvaluationCacheConfig {
            max_entry_bytes: 8, // smaller than any real batch
            ..config()
        });
        let handle = cache.begin_write(candidate(1, 1)).unwrap();
        handle.stage(0, batch(&[1, 2, 3, 4, 5]));
        handle.complete_partition(0); // no-op: already discarded
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        assert_eq!(cache.metrics().discarded_writes, 1);
        assert_eq!(cache.metrics().committed_writes, 0);
    }

    #[test]
    fn failed_partition_discards() {
        let cache = EvaluationCache::new(config());
        let handle = cache.begin_write(candidate(1, 2)).unwrap();
        handle.stage(0, batch(&[1]));
        handle.fail(0);
        handle.stage(1, batch(&[2])); // no-op after failure
        handle.complete_partition(1);
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        assert_eq!(cache.metrics().discarded_writes, 1);
    }

    #[test]
    fn clear_behaves_like_cold_start() {
        let cache = EvaluationCache::new(config());
        commit_entry(&cache, 1, &[1, 2, 3]);
        cache.observe(PlanObservation {
            fingerprint: PlanFingerprint(42),
            estimated_bytes: None,
            estimated_elapsed: None,
        });
        cache.clear();
        assert!(matches!(cache.lookup(&key(1)), CacheLookup::Miss));
        assert_eq!(
            cache.should_admit(&candidate(42, 1)),
            AdmissionDecision::Decline(DeclineReason::NotSeenEnough),
            "observations cleared with entries"
        );
    }
}
