use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use avenger_chart_core::{
    EvaluationInvalidationReason, EvaluationInvalidationRequest, EvaluationInvalidationSink,
    MaterializationIdentity, MaterializationKey, MaterializationKind, MaterializationRequest,
    MaterializationResult,
};

pub(crate) type MaterializationCacheHandle = Arc<Mutex<MaterializationCache>>;

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum MaterializationStatus {
    Missing,
    Queued,
    Running,
    Ready,
    Error(String),
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum MaterializationCacheEntry {
    Queued(MaterializationRequest),
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
    completion_invalidation_pending: bool,
}

impl MaterializationCache {
    #[allow(dead_code)]
    pub(crate) fn status(&self, key: &MaterializationKey) -> MaterializationStatus {
        match self.entries.get(key) {
            None => MaterializationStatus::Missing,
            Some(MaterializationCacheEntry::Queued(_)) => MaterializationStatus::Queued,
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
                MaterializationCacheEntry::Queued(_) | MaterializationCacheEntry::Running(_)
            )
        })
    }

    #[allow(dead_code)]
    pub(crate) fn enqueue(&mut self, request: MaterializationRequest) -> MaterializationStatus {
        match self.entries.get_mut(&request.key) {
            None => {
                self.entries.insert(
                    request.key.clone(),
                    MaterializationCacheEntry::Queued(request),
                );
                MaterializationStatus::Queued
            }
            Some(MaterializationCacheEntry::Queued(existing)) => {
                if request.priority > existing.priority {
                    *existing = request;
                }
                MaterializationStatus::Queued
            }
            Some(MaterializationCacheEntry::Running(_)) => MaterializationStatus::Running,
            Some(MaterializationCacheEntry::Ready { .. }) => MaterializationStatus::Ready,
            Some(MaterializationCacheEntry::Error { .. }) => {
                self.entries.insert(
                    request.key.clone(),
                    MaterializationCacheEntry::Queued(request),
                );
                MaterializationStatus::Queued
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn mark_running(&mut self, key: &MaterializationKey) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if let MaterializationCacheEntry::Queued(request) = entry {
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

    fn empty_result() -> MaterializationResult {
        MaterializationResult::RecordBatch(RecordBatch::new_empty(Arc::new(Schema::empty())))
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
            MaterializationCacheEntry::Queued(request) => {
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
}
