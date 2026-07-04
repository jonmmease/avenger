use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use avenger_chart_core::{
    EvaluationInvalidationReason, EvaluationInvalidationRequest, EvaluationInvalidationSink,
    MaterializationIdentity, MaterializationKey, MaterializationKind, MaterializationRequest,
    MaterializationResult,
};

pub(crate) type MaterializationCacheHandle = Arc<Mutex<MaterializationCache>>;

const MAX_UNSCOPED_READY_ENTRIES: usize = 64;
const MAX_TOTAL_READY_ENTRIES: usize = 512;

/// How long a preview-desired materialization key must stay unchanged before
/// a ready result is consumed (rebuilding data marks) instead of retargeting
/// the cached scene. This keeps mid-gesture frames on the cheap retarget path
/// even when micro-pauses let a raster complete under the pointer, while a
/// deliberate hold or release still swaps the fresh result in shortly after
/// the view stops moving. Exact/settled evaluations are unaffected.
pub(crate) const PREVIEW_CONSUME_STABILITY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum MaterializationStatus {
    Missing,
    Queued,
    Running,
    Ready,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum MaterializationDeferReason {
    Debounce,
    Throttle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum MaterializationStart {
    Started,
    Deferred {
        remaining: Duration,
        reason: MaterializationDeferReason,
    },
    NotStarted,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum MaterializationCacheEntry {
    Queued {
        request: MaterializationRequest,
        queued_at: Instant,
    },
    Running(MaterializationRequest),
    Ready {
        kind: MaterializationKind,
        identity: Option<MaterializationIdentity>,
        result: MaterializationResult,
    },
    Error {
        message: String,
    },
}

#[derive(Default)]
#[allow(dead_code)]
pub(crate) struct MaterializationCache {
    entries: HashMap<MaterializationKey, MaterializationCacheEntry>,
    last_ready_by_identity: HashMap<MaterializationIdentity, MaterializationKey>,
    last_settled_ready_by_identity: HashMap<MaterializationIdentity, MaterializationKey>,
    last_started_by_identity: HashMap<MaterializationIdentity, Instant>,
    preview_desired_key_motion: HashMap<MaterializationIdentity, (MaterializationKey, Instant)>,
    preview_schedule_last_run: HashMap<MaterializationIdentity, Instant>,
    pending_consume_wakeup: Option<(Duration, MaterializationKind)>,
    ready_order: VecDeque<MaterializationKey>,
    completion_invalidation_pending: bool,
}

fn throttle_identity_for_request(request: &MaterializationRequest) -> MaterializationIdentity {
    request
        .identity
        .clone()
        .unwrap_or_else(|| MaterializationIdentity::new(format!("key:{}", request.key.as_ref())))
}

impl MaterializationCache {
    #[allow(dead_code)]
    pub(crate) fn status(&self, key: &MaterializationKey) -> MaterializationStatus {
        match self.entries.get(key) {
            None => MaterializationStatus::Missing,
            Some(MaterializationCacheEntry::Queued { .. }) => MaterializationStatus::Queued,
            Some(MaterializationCacheEntry::Running(_)) => MaterializationStatus::Running,
            Some(MaterializationCacheEntry::Ready { .. }) => MaterializationStatus::Ready,
            Some(MaterializationCacheEntry::Error { message }) => {
                MaterializationStatus::Error(message.clone())
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn has_pending(&self) -> bool {
        self.entries.values().any(|entry| {
            matches!(
                entry,
                MaterializationCacheEntry::Queued { .. } | MaterializationCacheEntry::Running(_)
            )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn running_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry, MaterializationCacheEntry::Running(_)))
            .count()
    }

    #[allow(dead_code)]
    pub(crate) fn enqueue(&mut self, request: MaterializationRequest) -> MaterializationStatus {
        let dropped = self.drop_stale_queued_for_request(&request);
        if dropped > 0 {
            tracing::debug!(
                target: "avenger_chart::materialization",
                key = %request.key,
                dropped,
                "dropped stale queued materialization requests"
            );
        }

        match self.entries.get_mut(&request.key) {
            None => {
                self.entries.insert(
                    request.key.clone(),
                    MaterializationCacheEntry::Queued {
                        request,
                        queued_at: Instant::now(),
                    },
                );
                MaterializationStatus::Queued
            }
            Some(MaterializationCacheEntry::Queued {
                request: existing,
                queued_at,
            }) => {
                if request.priority > existing.priority {
                    *existing = request;
                    *queued_at = Instant::now();
                }
                MaterializationStatus::Queued
            }
            Some(MaterializationCacheEntry::Running(_)) => MaterializationStatus::Running,
            Some(MaterializationCacheEntry::Ready { .. }) => MaterializationStatus::Ready,
            Some(MaterializationCacheEntry::Error { .. }) => {
                self.entries.insert(
                    request.key.clone(),
                    MaterializationCacheEntry::Queued {
                        request,
                        queued_at: Instant::now(),
                    },
                );
                MaterializationStatus::Queued
            }
        }
    }

    fn drop_stale_queued_for_request(&mut self, request: &MaterializationRequest) -> usize {
        let Some(identity) = request.identity.as_ref() else {
            return 0;
        };
        let before = self.entries.len();
        self.entries.retain(|key, entry| {
            if key == &request.key {
                return true;
            }
            !matches!(
                entry,
                MaterializationCacheEntry::Queued {
                    request: existing,
                    ..
                }
                    if existing.identity.as_ref() == Some(identity)
            )
        });
        before.saturating_sub(self.entries.len())
    }

    #[allow(dead_code)]
    pub(crate) fn mark_running_if_ready(
        &mut self,
        key: &MaterializationKey,
        now: Instant,
    ) -> MaterializationStart {
        let (request, queued_at) = match self.entries.get(key) {
            Some(MaterializationCacheEntry::Queued { request, queued_at }) => {
                (request.clone(), *queued_at)
            }
            _ => return MaterializationStart::NotStarted,
        };

        if request.priority < 0.0 {
            let mut deferred: Option<(Duration, MaterializationDeferReason)> = None;
            if let Some(debounce) = request.policy.debounce {
                let elapsed = now.saturating_duration_since(queued_at);
                if elapsed < debounce {
                    deferred = Some((debounce - elapsed, MaterializationDeferReason::Debounce));
                }
            }

            if let Some(throttle) = request.policy.throttle {
                let identity = throttle_identity_for_request(&request);
                if let Some(last_started) = self.last_started_by_identity.get(&identity) {
                    let elapsed = now.saturating_duration_since(*last_started);
                    if elapsed < throttle {
                        let remaining = throttle - elapsed;
                        deferred = Some(match deferred {
                            Some((existing, reason)) if existing >= remaining => (existing, reason),
                            _ => (remaining, MaterializationDeferReason::Throttle),
                        });
                    }
                }
            }

            if let Some((remaining, reason)) = deferred {
                return MaterializationStart::Deferred { remaining, reason };
            }
        }

        if let Some(entry) = self.entries.get_mut(key) {
            *entry = MaterializationCacheEntry::Running(request.clone());
            self.last_started_by_identity
                .insert(throttle_identity_for_request(&request), now);
            return MaterializationStart::Started;
        }

        MaterializationStart::NotStarted
    }

    /// Record the key a preview evaluation currently desires for this
    /// request's identity and return how long that key has been unchanged.
    /// A changed key resets the clock to zero.
    pub(crate) fn note_preview_desired_key(
        &mut self,
        request: &MaterializationRequest,
        now: Instant,
    ) -> Duration {
        let identity = throttle_identity_for_request(request);
        match self.preview_desired_key_motion.get_mut(&identity) {
            Some((key, changed_at)) if *key == request.key => {
                now.saturating_duration_since(*changed_at)
            }
            Some(entry) => {
                *entry = (request.key.clone(), now);
                Duration::ZERO
            }
            None => {
                self.preview_desired_key_motion
                    .insert(identity, (request.key.clone(), now));
                Duration::ZERO
            }
        }
    }

    /// Note that a ready preview result was NOT consumed because its key has
    /// not been stable long enough; the session drains this after evaluation
    /// and schedules a delayed re-evaluation so a hold still swaps the result
    /// in without another interaction event.
    pub(crate) fn defer_preview_consume(
        &mut self,
        request: &MaterializationRequest,
        remaining: Duration,
    ) {
        self.defer_preview_wakeup(request.kind.clone(), remaining);
    }

    fn defer_preview_wakeup(&mut self, kind: MaterializationKind, remaining: Duration) {
        self.pending_consume_wakeup = Some(match self.pending_consume_wakeup.take() {
            Some((existing, kind)) if existing <= remaining => (existing, kind),
            _ => (remaining, kind),
        });
    }

    /// Rate-limit the preview schedule-only pass for a view. The pass runs
    /// the view's transform chain (including any eager scalar aggregations)
    /// just to compute materialization keys, so during a gesture it is
    /// throttled alongside the requests themselves: returns true (recording
    /// the run) when the throttle window has passed, otherwise records a
    /// delayed wakeup so the settled view state still gets scheduled without
    /// another interaction event.
    pub(crate) fn should_run_preview_schedule(
        &mut self,
        identity: &MaterializationIdentity,
        throttle: Duration,
        now: Instant,
    ) -> bool {
        if let Some(last_run) = self.preview_schedule_last_run.get(identity) {
            let elapsed = now.saturating_duration_since(*last_run);
            if elapsed < throttle {
                self.defer_preview_wakeup(
                    MaterializationKind::new("preview-schedule-throttle"),
                    throttle - elapsed,
                );
                return false;
            }
        }
        self.preview_schedule_last_run.insert(identity.clone(), now);
        true
    }

    pub(crate) fn take_pending_consume_wakeup(
        &mut self,
    ) -> Option<(Duration, MaterializationKind)> {
        self.pending_consume_wakeup.take()
    }

    #[allow(dead_code)]
    pub(crate) fn mark_running(&mut self, key: &MaterializationKey) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if let MaterializationCacheEntry::Queued { request, .. } = entry {
            *entry = MaterializationCacheEntry::Running(request.clone());
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub(crate) fn get_ready(&self, key: &MaterializationKey) -> Option<MaterializationResult> {
        match self.entries.get(key) {
            Some(MaterializationCacheEntry::Ready { result, .. }) => Some(result.clone()),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn last_ready(
        &self,
        identity: &MaterializationIdentity,
    ) -> Option<(MaterializationKey, MaterializationResult)> {
        let key = self.last_ready_by_identity.get(identity)?;
        self.get_ready(key).map(|result| (key.clone(), result))
    }

    #[allow(dead_code)]
    /// Latest usable stale result for `identity`.
    ///
    /// `prefer_settled` pins the fallback to the last gesture-settled result
    /// — correct for the preview RETARGET path, where the cached scene keeps
    /// re-displaying one stable raster until the consume-stability window
    /// swaps in a fresh one. Paths that re-render from scratch every frame
    /// (no retargeted scene to hold steady) must pass `false` and take the
    /// newest ready result instead: alternating between "settled" fallback
    /// frames and exact-key hits on just-completed mid-gesture results
    /// visibly flashes between old and new rasters.
    pub(crate) fn stale_fallback_ready(
        &self,
        identity: &MaterializationIdentity,
        prefer_settled: bool,
    ) -> Option<(MaterializationKey, MaterializationResult)> {
        if prefer_settled
            && let Some(key) = self.last_settled_ready_by_identity.get(identity)
            && let Some(result) = self.get_ready(key)
        {
            return Some((key.clone(), result));
        }
        self.last_ready(identity)
    }

    #[allow(dead_code)]
    pub(crate) fn mark_ready(
        &mut self,
        request: &MaterializationRequest,
        result: MaterializationResult,
    ) {
        self.entries.insert(
            request.key.clone(),
            MaterializationCacheEntry::Ready {
                kind: request.kind.clone(),
                identity: request.identity.clone(),
                result,
            },
        );
        if let Some(identity) = &request.identity {
            self.last_ready_by_identity
                .insert(identity.clone(), request.key.clone());
            if request.priority >= 0.0 {
                self.last_settled_ready_by_identity
                    .insert(identity.clone(), request.key.clone());
            }
        }
        self.remember_ready_key(&request.key);
        self.prune_ready_entries_for_request(request);
    }

    fn remember_ready_key(&mut self, key: &MaterializationKey) {
        self.ready_order.retain(|existing| existing != key);
        self.ready_order.push_back(key.clone());
    }

    fn remove_ready_key_from_order(&mut self, key: &MaterializationKey) {
        self.ready_order.retain(|existing| existing != key);
    }

    fn ready_entry_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry, MaterializationCacheEntry::Ready { .. }))
            .count()
    }

    fn is_ready_key_protected(&self, key: &MaterializationKey) -> bool {
        self.last_ready_by_identity
            .values()
            .any(|protected| protected == key)
            || self
                .last_settled_ready_by_identity
                .values()
                .any(|protected| protected == key)
    }

    fn evict_ready_key(&mut self, key: &MaterializationKey, reason: &'static str) -> bool {
        if matches!(
            self.entries.get(key),
            Some(MaterializationCacheEntry::Ready { .. })
        ) {
            tracing::debug!(
                target: "avenger_chart::materialization",
                key = %key,
                reason,
                "evicting ready materialization"
            );
            self.entries.remove(key);
            self.remove_ready_key_from_order(key);
            true
        } else {
            self.remove_ready_key_from_order(key);
            false
        }
    }

    fn prune_ready_entries_for_request(&mut self, request: &MaterializationRequest) {
        if let Some(identity) = &request.identity {
            self.prune_ready_entries_for_identity(identity);
        } else {
            self.prune_unscoped_ready_entries();
        }
        self.prune_total_ready_entries();
    }

    fn prune_ready_entries_for_identity(&mut self, identity: &MaterializationIdentity) {
        let latest_ready = self.last_ready_by_identity.get(identity);
        let latest_settled_ready = self.last_settled_ready_by_identity.get(identity);
        let evict = self
            .entries
            .iter()
            .filter_map(|(key, entry)| match entry {
                MaterializationCacheEntry::Ready {
                    identity: Some(entry_identity),
                    ..
                } if entry_identity == identity
                    && Some(key) != latest_ready
                    && Some(key) != latest_settled_ready =>
                {
                    Some(key.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        for key in evict {
            self.evict_ready_key(&key, "stale_identity_ready");
        }
    }

    fn prune_unscoped_ready_entries(&mut self) {
        let unscoped_ready = self
            .ready_order
            .iter()
            .filter_map(|key| match self.entries.get(key) {
                Some(MaterializationCacheEntry::Ready { identity: None, .. }) => Some(key.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let excess = unscoped_ready
            .len()
            .saturating_sub(MAX_UNSCOPED_READY_ENTRIES);
        for key in unscoped_ready.into_iter().take(excess) {
            self.evict_ready_key(&key, "unscoped_ready_lru");
        }
    }

    fn prune_total_ready_entries(&mut self) {
        let mut ready_count = self.ready_entry_count();
        if ready_count <= MAX_TOTAL_READY_ENTRIES {
            return;
        }

        let ready_order = self.ready_order.iter().cloned().collect::<Vec<_>>();
        for key in ready_order {
            if ready_count <= MAX_TOTAL_READY_ENTRIES {
                break;
            }
            if self.is_ready_key_protected(&key) {
                continue;
            }
            if self.evict_ready_key(&key, "global_ready_cap") {
                ready_count -= 1;
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn mark_ready_and_request_invalidation(
        &mut self,
        request: &MaterializationRequest,
        result: MaterializationResult,
        sink: &dyn EvaluationInvalidationSink,
    ) {
        self.mark_ready(request, result);
        if !self.completion_invalidation_pending {
            sink.request_evaluation(EvaluationInvalidationRequest::now(
                EvaluationInvalidationReason::MaterializationCompleted {
                    kind: request.kind.clone(),
                },
            ));
            self.completion_invalidation_pending = true;
        }
    }

    #[allow(dead_code)]
    pub(crate) fn mark_error(&mut self, request: &MaterializationRequest, message: String) {
        self.entries.insert(
            request.key.clone(),
            MaterializationCacheEntry::Error { message },
        );
    }

    #[allow(dead_code)]
    pub(crate) fn clear_completion_invalidation_pending(&mut self) {
        self.completion_invalidation_pending = false;
    }

    #[allow(dead_code)]
    pub(crate) fn completion_invalidation_pending(&self) -> bool {
        self.completion_invalidation_pending
    }

    #[cfg(test)]
    fn ready_count_for_testing(&self) -> usize {
        self.ready_entry_count()
    }

    #[cfg(test)]
    fn ready_keys_for_identity_for_testing(
        &self,
        identity: &MaterializationIdentity,
    ) -> Vec<MaterializationKey> {
        self.ready_order
            .iter()
            .filter_map(|key| match self.entries.get(key) {
                Some(MaterializationCacheEntry::Ready {
                    identity: Some(entry_identity),
                    ..
                }) if entry_identity == identity => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    fn contains_ready_for_testing(&self, key: &MaterializationKey) -> bool {
        matches!(
            self.entries.get(key),
            Some(MaterializationCacheEntry::Ready { .. })
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use datafusion::arrow::{datatypes::Schema, record_batch::RecordBatch};

    use avenger_chart_core::{
        EvaluationInvalidation, EvaluationInvalidationHub, MaterializationOutputKind,
    };

    use super::*;

    fn request(key: &str, identity: &str, priority: f32) -> MaterializationRequest {
        MaterializationRequest::new(key, "test", MaterializationOutputKind::RecordBatch)
            .identity(identity)
            .priority(priority)
    }

    fn unscoped_request(key: &str, priority: f32) -> MaterializationRequest {
        MaterializationRequest::new(key, "test", MaterializationOutputKind::RecordBatch)
            .priority(priority)
    }

    fn empty_result() -> MaterializationResult {
        MaterializationResult::RecordBatch(RecordBatch::new_empty(Arc::new(Schema::empty())))
    }

    #[test]
    fn preview_desired_key_stability_resets_on_key_change() {
        let mut cache = MaterializationCache::default();
        let key_a = request("key/a", "scope/a", -1.0);
        let key_b = request("key/b", "scope/a", -1.0);
        let start = Instant::now();

        assert_eq!(
            cache.note_preview_desired_key(&key_a, start),
            Duration::ZERO
        );
        assert_eq!(
            cache.note_preview_desired_key(&key_a, start + Duration::from_millis(80)),
            Duration::from_millis(80)
        );

        // A different desired key for the same identity resets the clock.
        assert_eq!(
            cache.note_preview_desired_key(&key_b, start + Duration::from_millis(100)),
            Duration::ZERO
        );
        assert_eq!(
            cache.note_preview_desired_key(&key_b, start + Duration::from_millis(400)),
            Duration::from_millis(300)
        );

        // Identities track independently.
        let other = request("key/z", "scope/z", -1.0);
        assert_eq!(
            cache.note_preview_desired_key(&other, start + Duration::from_millis(400)),
            Duration::ZERO
        );
    }

    #[test]
    fn deferred_preview_consume_wakeup_keeps_earliest_and_drains_once() {
        let mut cache = MaterializationCache::default();
        let first = request("key/a", "scope/a", -1.0);
        let second = request("key/b", "scope/b", -1.0);

        cache.defer_preview_consume(&first, Duration::from_millis(200));
        cache.defer_preview_consume(&second, Duration::from_millis(90));
        cache.defer_preview_consume(&first, Duration::from_millis(150));

        let (remaining, _kind) = cache
            .take_pending_consume_wakeup()
            .expect("a deferred consume must request a wakeup");
        assert_eq!(remaining, Duration::from_millis(90));
        assert!(cache.take_pending_consume_wakeup().is_none());
    }

    #[test]
    fn preview_schedule_throttle_rate_limits_and_requests_wakeup() {
        let mut cache = MaterializationCache::default();
        let identity = MaterializationIdentity::new("preview-schedule:pickups:0");
        let throttle = Duration::from_millis(100);
        let start = Instant::now();

        assert!(cache.should_run_preview_schedule(&identity, throttle, start));
        assert!(!cache.should_run_preview_schedule(
            &identity,
            throttle,
            start + Duration::from_millis(40)
        ));
        let (remaining, _kind) = cache
            .take_pending_consume_wakeup()
            .expect("throttled schedule pass must request a wakeup");
        assert_eq!(remaining, Duration::from_millis(60));

        // Other identities are unaffected; the window reopens after elapse.
        let other = MaterializationIdentity::new("preview-schedule:other:1");
        assert!(cache.should_run_preview_schedule(
            &other,
            throttle,
            start + Duration::from_millis(40)
        ));
        assert!(cache.should_run_preview_schedule(
            &identity,
            throttle,
            start + Duration::from_millis(150)
        ));
    }

    #[test]
    fn cache_tracks_ready_and_last_ready_by_identity() {
        let mut cache = MaterializationCache::default();
        let first = request("key/a", "scope/a", 0.0);
        let second = request("key/b", "scope/a", 0.0);

        cache.enqueue(first.clone());
        cache.mark_ready(&first, empty_result());
        assert_eq!(cache.status(&first.key), MaterializationStatus::Ready);
        assert!(cache.get_ready(&first.key).is_some());
        assert_eq!(
            cache
                .last_ready(first.identity.as_ref().expect("identity"))
                .expect("last ready")
                .0,
            first.key
        );

        cache.enqueue(second.clone());
        cache.mark_ready(&second, empty_result());
        assert_eq!(
            cache
                .last_ready(second.identity.as_ref().expect("identity"))
                .expect("last ready")
                .0,
            second.key
        );
    }

    #[test]
    fn enqueue_keeps_highest_priority_request() {
        let mut cache = MaterializationCache::default();
        let low = request("key/a", "scope/a", 0.25);
        let high = request("key/a", "scope/a", 2.0);

        assert_eq!(cache.enqueue(low), MaterializationStatus::Queued);
        assert_eq!(cache.enqueue(high.clone()), MaterializationStatus::Queued);
        match cache.entries.get(&high.key).expect("entry") {
            MaterializationCacheEntry::Queued { request, .. } => {
                assert_eq!(request.priority, 2.0);
            }
            other => panic!("unexpected entry: {other:?}"),
        }
    }

    #[test]
    fn cache_reports_pending_queued_and_running_entries() {
        let mut cache = MaterializationCache::default();
        let pending = request("key/a", "scope/a", 0.0);
        let ready = request("key/b", "scope/b", 0.0);

        assert!(!cache.has_pending());

        cache.enqueue(pending.clone());
        assert!(cache.has_pending());

        assert!(cache.mark_running(&pending.key));
        assert!(cache.has_pending());

        cache.mark_ready(&pending, empty_result());
        assert!(!cache.has_pending());

        cache.enqueue(ready.clone());
        cache.mark_ready(&ready, empty_result());
        assert!(!cache.has_pending());
    }

    #[test]
    fn enqueue_drops_stale_queued_requests_for_same_identity() {
        let mut cache = MaterializationCache::default();
        let queued = request("key/a", "scope/a", 0.0);
        let replacement = request("key/b", "scope/a", 0.0);
        let other_scope = request("key/c", "scope/c", 0.0);
        let running = request("key/d", "scope/a", 0.0);
        let ready = request("key/e", "scope/a", 0.0);

        cache.enqueue(queued.clone());
        cache.enqueue(other_scope.clone());
        cache.enqueue(running.clone());
        assert!(cache.mark_running(&running.key));
        cache.enqueue(ready.clone());
        cache.mark_ready(&ready, empty_result());

        assert_eq!(
            cache.enqueue(replacement.clone()),
            MaterializationStatus::Queued
        );

        assert_eq!(cache.status(&queued.key), MaterializationStatus::Missing);
        assert_eq!(
            cache.status(&replacement.key),
            MaterializationStatus::Queued
        );
        assert_eq!(
            cache.status(&other_scope.key),
            MaterializationStatus::Queued
        );
        assert_eq!(cache.status(&running.key), MaterializationStatus::Running);
        assert_eq!(cache.status(&ready.key), MaterializationStatus::Ready);
    }

    #[test]
    fn running_count_tracks_running_entries() {
        let mut cache = MaterializationCache::default();
        let first = request("key/a", "scope/a", 0.0);
        let second = request("key/b", "scope/b", 0.0);

        assert_eq!(cache.running_count(), 0);
        cache.enqueue(first.clone());
        cache.enqueue(second.clone());
        assert_eq!(cache.running_count(), 0);

        assert!(cache.mark_running(&first.key));
        assert_eq!(cache.running_count(), 1);
        assert!(cache.mark_running(&second.key));
        assert_eq!(cache.running_count(), 2);

        cache.mark_ready(&first, empty_result());
        assert_eq!(cache.running_count(), 1);
        cache.mark_error(&second, "failed".to_string());
        assert_eq!(cache.running_count(), 0);
    }

    #[test]
    fn mark_running_if_ready_defers_debounced_preview_requests() {
        let mut cache = MaterializationCache::default();
        let mut debounced = request("key/a", "scope/a", -1.0);
        debounced.policy.debounce = Some(Duration::from_millis(50));
        let now = Instant::now();

        cache.enqueue(debounced.clone());
        match cache.mark_running_if_ready(&debounced.key, now + Duration::from_millis(10)) {
            MaterializationStart::Deferred { remaining, reason } => {
                assert_eq!(reason, MaterializationDeferReason::Debounce);
                assert!(remaining <= Duration::from_millis(50));
                assert!(remaining >= Duration::from_millis(30));
            }
            other => panic!("expected debounced request to defer, got {other:?}"),
        }
        assert_eq!(cache.status(&debounced.key), MaterializationStatus::Queued);

        assert_eq!(
            cache.mark_running_if_ready(&debounced.key, now + Duration::from_millis(60)),
            MaterializationStart::Started
        );
        assert_eq!(cache.status(&debounced.key), MaterializationStatus::Running);
    }

    #[test]
    fn mark_running_if_ready_starts_settled_requests_despite_debounce() {
        let mut cache = MaterializationCache::default();
        let mut settled = request("key/a", "scope/a", 1.0);
        settled.policy.debounce = Some(Duration::from_secs(60));

        cache.enqueue(settled.clone());
        assert_eq!(
            cache.mark_running_if_ready(&settled.key, Instant::now()),
            MaterializationStart::Started
        );
        assert_eq!(cache.status(&settled.key), MaterializationStatus::Running);
    }

    #[test]
    fn mark_running_if_ready_throttles_preview_requests_by_identity() {
        let mut cache = MaterializationCache::default();
        let mut first = request("key/a", "scope/a", -1.0);
        first.policy.throttle = Some(Duration::from_millis(100));
        let now = Instant::now();

        cache.enqueue(first.clone());
        assert_eq!(
            cache.mark_running_if_ready(&first.key, now),
            MaterializationStart::Started
        );

        let mut second = request("key/b", "scope/a", -1.0);
        second.policy.throttle = Some(Duration::from_millis(100));
        cache.enqueue(second.clone());
        match cache.mark_running_if_ready(&second.key, now + Duration::from_millis(25)) {
            MaterializationStart::Deferred { remaining, reason } => {
                assert_eq!(reason, MaterializationDeferReason::Throttle);
                assert!(remaining <= Duration::from_millis(100));
                assert!(remaining >= Duration::from_millis(50));
            }
            other => panic!("expected throttled request to defer, got {other:?}"),
        }
        assert_eq!(cache.status(&second.key), MaterializationStatus::Queued);

        assert_eq!(
            cache.mark_running_if_ready(&second.key, now + Duration::from_millis(125)),
            MaterializationStart::Started
        );
    }

    #[test]
    fn mark_running_if_ready_throttles_unscoped_preview_requests_by_key() {
        let mut cache = MaterializationCache::default();
        let mut first = unscoped_request("key/a", -1.0);
        first.policy.throttle = Some(Duration::from_millis(100));
        let now = Instant::now();

        cache.enqueue(first.clone());
        assert_eq!(
            cache.mark_running_if_ready(&first.key, now),
            MaterializationStart::Started
        );

        let mut same_key = unscoped_request("key/a", -1.0);
        same_key.policy.throttle = Some(Duration::from_millis(100));
        cache.mark_error(&same_key, "retry".to_string());
        cache.enqueue(same_key.clone());
        match cache.mark_running_if_ready(&same_key.key, now + Duration::from_millis(25)) {
            MaterializationStart::Deferred { remaining, reason } => {
                assert_eq!(reason, MaterializationDeferReason::Throttle);
                assert!(remaining >= Duration::from_millis(50));
            }
            other => panic!("expected key-fallback throttle defer, got {other:?}"),
        }
    }

    #[test]
    fn mark_running_if_ready_starts_settled_requests_despite_throttle() {
        let mut cache = MaterializationCache::default();
        let mut preview = request("key/a", "scope/a", -1.0);
        preview.policy.throttle = Some(Duration::from_secs(60));
        let now = Instant::now();
        cache.enqueue(preview.clone());
        assert_eq!(
            cache.mark_running_if_ready(&preview.key, now),
            MaterializationStart::Started
        );

        let mut settled = request("key/b", "scope/a", 1.0);
        settled.policy.throttle = Some(Duration::from_secs(60));
        cache.enqueue(settled.clone());
        assert_eq!(
            cache.mark_running_if_ready(&settled.key, now + Duration::from_millis(1)),
            MaterializationStart::Started
        );
    }

    #[test]
    fn mark_running_if_ready_uses_larger_debounce_or_throttle_wait() {
        let mut cache = MaterializationCache::default();
        let mut first = request("key/a", "scope/a", -1.0);
        first.policy.throttle = Some(Duration::from_millis(100));
        let now = Instant::now();
        cache.enqueue(first.clone());
        assert_eq!(
            cache.mark_running_if_ready(&first.key, now),
            MaterializationStart::Started
        );

        let mut second = request("key/b", "scope/a", -1.0);
        second.policy.debounce = Some(Duration::from_millis(50));
        second.policy.throttle = Some(Duration::from_millis(100));
        cache.enqueue(second.clone());
        match cache.mark_running_if_ready(&second.key, now + Duration::from_millis(10)) {
            MaterializationStart::Deferred { remaining, reason } => {
                assert_eq!(reason, MaterializationDeferReason::Throttle);
                assert!(remaining >= Duration::from_millis(80));
            }
            other => panic!("expected larger throttle defer, got {other:?}"),
        }
    }

    #[test]
    fn mark_ready_coalesces_completion_invalidations_until_cleared() {
        let mut cache = MaterializationCache::default();
        let hub = EvaluationInvalidationHub::default();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::<EvaluationInvalidation>::new()));
        let count_callback = count.clone();
        let seen_callback = seen.clone();
        let _subscription = hub.subscribe(Arc::new(move |invalidation| {
            count_callback.fetch_add(1, Ordering::SeqCst);
            seen_callback
                .lock()
                .expect("seen lock poisoned")
                .push(invalidation);
        }));

        let first = request("key/a", "scope/a", 0.0);
        let second = request("key/b", "scope/a", 0.0);
        cache.mark_ready_and_request_invalidation(&first, empty_result(), &hub);
        cache.mark_ready_and_request_invalidation(&second, empty_result(), &hub);

        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(cache.completion_invalidation_pending());
        let invalidations = seen.lock().expect("seen lock poisoned");
        assert_eq!(invalidations[0].epoch, 1);

        drop(invalidations);
        cache.clear_completion_invalidation_pending();
        cache.mark_ready_and_request_invalidation(&second, empty_result(), &hub);
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn ready_cache_keeps_latest_preview_and_latest_settled_per_identity() {
        let mut cache = MaterializationCache::default();
        let exact = request("key/exact", "scope/a", 1.0);
        cache.mark_ready(&exact, empty_result());

        let preview_a = request("key/preview-a", "scope/a", -1.0);
        cache.mark_ready(&preview_a, empty_result());

        let preview_b = request("key/preview-b", "scope/a", -1.0);
        cache.mark_ready(&preview_b, empty_result());

        assert!(cache.contains_ready_for_testing(&exact.key));
        assert!(!cache.contains_ready_for_testing(&preview_a.key));
        assert!(cache.contains_ready_for_testing(&preview_b.key));
        assert_eq!(
            cache.ready_keys_for_identity_for_testing(exact.identity.as_ref().unwrap()),
            vec![exact.key.clone(), preview_b.key.clone()]
        );
        assert_eq!(
            cache
                .last_ready(exact.identity.as_ref().unwrap())
                .expect("last ready")
                .0,
            preview_b.key
        );
        assert_eq!(
            cache
                .stale_fallback_ready(exact.identity.as_ref().unwrap(), true)
                .expect("stale fallback")
                .0,
            exact.key
        );
    }

    #[test]
    fn exact_ready_replaces_previous_settled_ready_for_identity() {
        let mut cache = MaterializationCache::default();
        let first = request("key/exact-a", "scope/a", 1.0);
        let preview = request("key/preview", "scope/a", -1.0);
        let second = request("key/exact-b", "scope/a", 1.0);

        cache.mark_ready(&first, empty_result());
        cache.mark_ready(&preview, empty_result());
        cache.mark_ready(&second, empty_result());

        assert!(!cache.contains_ready_for_testing(&first.key));
        assert!(!cache.contains_ready_for_testing(&preview.key));
        assert!(cache.contains_ready_for_testing(&second.key));
        assert_eq!(
            cache
                .stale_fallback_ready(first.identity.as_ref().unwrap(), true)
                .expect("stale fallback")
                .0,
            second.key
        );
    }

    #[test]
    fn unscoped_ready_entries_are_bounded() {
        let mut cache = MaterializationCache::default();
        for index in 0..(MAX_UNSCOPED_READY_ENTRIES + 5) {
            let request = unscoped_request(&format!("key/{index}"), 0.0);
            cache.mark_ready(&request, empty_result());
        }

        assert_eq!(cache.ready_count_for_testing(), MAX_UNSCOPED_READY_ENTRIES);
        assert!(!cache.contains_ready_for_testing(&MaterializationKey::new("key/0")));
        assert!(
            cache.contains_ready_for_testing(&MaterializationKey::new(format!(
                "key/{}",
                MAX_UNSCOPED_READY_ENTRIES + 4
            )))
        );
    }

    #[test]
    fn global_ready_cap_preserves_protected_identity_ready_entries() {
        let mut cache = MaterializationCache::default();
        for index in 0..(MAX_TOTAL_READY_ENTRIES + 5) {
            let request = request(&format!("key/{index}"), &format!("scope/{index}"), 1.0);
            cache.mark_ready(&request, empty_result());
        }

        // Every identity's latest settled result is protected, so the defensive
        // cap cannot evict below the number of protected identities.
        assert_eq!(cache.ready_count_for_testing(), MAX_TOTAL_READY_ENTRIES + 5);

        for index in 0..5 {
            let preview = request(
                &format!("key/{index}/preview"),
                &format!("scope/{index}"),
                -1.0,
            );
            cache.mark_ready(&preview, empty_result());
        }

        assert_eq!(
            cache.ready_count_for_testing(),
            MAX_TOTAL_READY_ENTRIES + 10
        );
        for index in 0..5 {
            assert!(
                cache.contains_ready_for_testing(&MaterializationKey::new(format!("key/{index}")))
            );
            assert!(
                cache.contains_ready_for_testing(&MaterializationKey::new(format!(
                    "key/{index}/preview"
                )))
            );
        }
    }
}
