use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct RenderInvalidation {
    pub epoch: u64,
    pub reason: RenderInvalidationReason,
    pub schedule: RenderInvalidationSchedule,
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderInvalidationReason {
    ResourceChanged { kind: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderInvalidationSchedule {
    Now,
    After(Duration),
}

pub trait RenderInvalidationSink: Send + Sync {
    fn request_render(&self, request: RenderInvalidationRequest);
}

#[derive(Clone, Debug)]
pub struct RenderInvalidationRequest {
    pub reason: RenderInvalidationReason,
    pub schedule: RenderInvalidationSchedule,
}

impl RenderInvalidationRequest {
    pub fn now(reason: RenderInvalidationReason) -> Self {
        Self {
            reason,
            schedule: RenderInvalidationSchedule::Now,
        }
    }
}

pub type RenderInvalidationCallback = Arc<dyn Fn(RenderInvalidation) + Send + Sync + 'static>;

pub struct RenderInvalidationSubscription {
    id: usize,
    inner: Weak<Mutex<RenderInvalidationHubInner>>,
}

impl Drop for RenderInvalidationSubscription {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let mut inner = inner.lock().expect("render invalidation hub lock poisoned");
        inner.callbacks.retain(|(id, _)| *id != self.id);
    }
}

#[derive(Clone, Default)]
pub struct RenderInvalidationHub {
    inner: Arc<Mutex<RenderInvalidationHubInner>>,
}

#[derive(Default)]
struct RenderInvalidationHubInner {
    epoch: u64,
    next_callback_id: usize,
    callbacks: Vec<(usize, RenderInvalidationCallback)>,
}

impl RenderInvalidationHub {
    pub fn epoch(&self) -> u64 {
        self.inner
            .lock()
            .expect("render invalidation hub lock poisoned")
            .epoch
    }

    pub fn subscribe(
        &self,
        callback: RenderInvalidationCallback,
    ) -> RenderInvalidationSubscription {
        let mut inner = self
            .inner
            .lock()
            .expect("render invalidation hub lock poisoned");
        let id = inner.next_callback_id;
        inner.next_callback_id = inner.next_callback_id.wrapping_add(1);
        inner.callbacks.push((id, callback));
        RenderInvalidationSubscription {
            id,
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl RenderInvalidationSink for RenderInvalidationHub {
    fn request_render(&self, request: RenderInvalidationRequest) {
        let (invalidation, callbacks) = {
            let mut inner = self
                .inner
                .lock()
                .expect("render invalidation hub lock poisoned");
            inner.epoch = inner.epoch.wrapping_add(1);
            let invalidation = RenderInvalidation {
                epoch: inner.epoch,
                reason: request.reason,
                schedule: request.schedule,
            };
            let callbacks = inner
                .callbacks
                .iter()
                .map(|(_, callback)| callback.clone())
                .collect::<Vec<_>>();
            (invalidation, callbacks)
        };

        for callback in callbacks {
            callback(invalidation.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn test_request() -> RenderInvalidationRequest {
        RenderInvalidationRequest::now(RenderInvalidationReason::ResourceChanged { kind: "test" })
    }

    #[test]
    fn subscribers_receive_monotonic_epochs() {
        let hub = RenderInvalidationHub::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_callback = seen.clone();
        let _subscription = hub.subscribe(Arc::new(move |invalidation| {
            seen_callback
                .lock()
                .expect("seen lock poisoned")
                .push(invalidation.epoch);
        }));

        hub.request_render(test_request());
        hub.request_render(test_request());

        assert_eq!(*seen.lock().expect("seen lock poisoned"), vec![1, 2]);
        assert_eq!(hub.epoch(), 2);
    }

    #[test]
    fn callbacks_are_invoked_without_holding_hub_lock() {
        let hub = RenderInvalidationHub::default();
        let hub_for_callback = hub.clone();
        let _subscription = hub.subscribe(Arc::new(move |_| {
            let _epoch = hub_for_callback.epoch();
        }));

        hub.request_render(test_request());
    }

    #[test]
    fn dropping_subscription_suppresses_later_callbacks() {
        let hub = RenderInvalidationHub::default();
        let count = Arc::new(AtomicUsize::new(0));
        let count_callback = count.clone();
        let subscription = hub.subscribe(Arc::new(move |_| {
            count_callback.fetch_add(1, Ordering::SeqCst);
        }));

        hub.request_render(test_request());
        drop(subscription);
        hub.request_render(test_request());

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn multiple_subscribers_receive_same_epoch() {
        let hub = RenderInvalidationHub::default();
        let first = Arc::new(Mutex::new(Vec::new()));
        let second = Arc::new(Mutex::new(Vec::new()));
        let first_callback = first.clone();
        let second_callback = second.clone();
        let _first_subscription = hub.subscribe(Arc::new(move |invalidation| {
            first_callback
                .lock()
                .expect("first lock poisoned")
                .push(invalidation.epoch);
        }));
        let _second_subscription = hub.subscribe(Arc::new(move |invalidation| {
            second_callback
                .lock()
                .expect("second lock poisoned")
                .push(invalidation.epoch);
        }));

        hub.request_render(test_request());

        assert_eq!(*first.lock().expect("first lock poisoned"), vec![1]);
        assert_eq!(*second.lock().expect("second lock poisoned"), vec![1]);
    }

    #[test]
    fn after_schedule_is_preserved() {
        let hub = RenderInvalidationHub::default();
        let seen = Arc::new(Mutex::new(None));
        let seen_callback = seen.clone();
        let _subscription = hub.subscribe(Arc::new(move |invalidation| {
            *seen_callback.lock().expect("seen lock poisoned") = Some(invalidation);
        }));

        hub.request_render(RenderInvalidationRequest {
            reason: RenderInvalidationReason::ResourceChanged { kind: "test" },
            schedule: RenderInvalidationSchedule::After(Duration::from_millis(16)),
        });

        let invalidation = seen
            .lock()
            .expect("seen lock poisoned")
            .clone()
            .expect("invalidation");
        assert_eq!(
            invalidation.schedule,
            RenderInvalidationSchedule::After(Duration::from_millis(16))
        );
    }
}
