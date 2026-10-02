//! Replaceable background work with typed results delivered through app events.
//!
//! Each task has one current request. Clones share execution but retain their own
//! request and delivery cursors, so discarded app-state candidates do not consume
//! another state's completion. Submission and cancellation are immediate external
//! effects and are not rolled back when a later scene build fails.

pub mod host;

use avenger_eventstream::runtime::{RuntimeWakeEvent, RuntimeWakeKey};
use futures::{
    future::{AbortHandle, Abortable, BoxFuture},
    FutureExt,
};
use std::{
    error::Error,
    fmt,
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};

type BoxError = Box<dyn Error + Send + Sync + 'static>;
type Completion<T> = Result<Arc<T>, BackgroundTaskError>;

/// A task failure or an unavailable task group. Application errors retain their source.
#[derive(Clone, Debug)]
pub enum BackgroundTaskError {
    /// The submitted operation returned an error.
    Failed(Arc<dyn Error + Send + Sync>),
    /// The future panicked on a target that supports unwinding.
    Panicked(String),
    /// The executor dropped the current future without returning a result.
    Interrupted,
    /// The owning app or host has shut down.
    Closed,
    /// Another host already owns this group.
    AlreadyAttached,
}

impl fmt::Display for BackgroundTaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(error) => error.fmt(f),
            Self::Panicked(message) => write!(f, "background task panicked: {message}"),
            Self::Interrupted => f.write_str("background task was interrupted"),
            Self::Closed => f.write_str("background task group is closed"),
            Self::AlreadyAttached => f.write_str("background task group already has a host"),
        }
    }
}

impl Error for BackgroundTaskError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Failed(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

/// A task factory whose requests share one app lifetime.
///
/// Attach this group with [`crate::app::AvengerApp::with_background_tasks`].
/// Requests submitted before host activation stay queued. Cloning the group does
/// not create another lifetime. A replacement app needs a fresh group.
#[derive(Clone, Default)]
pub struct BackgroundTasks {
    group: Arc<Group>,
}

impl BackgroundTasks {
    /// Create an unattached group. It starts work when its host activates it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an independent sequence of replaceable requests.
    pub fn task<T: Send + Sync + 'static>(&self) -> BackgroundTask<T> {
        let slot = Arc::new(Slot {
            group: Arc::downgrade(&self.group),
            key: RuntimeWakeKey::new("avenger-app/background", fresh_id(), "completion"),
            request: Mutex::new(None),
        });
        let erased: Arc<dyn TaskSlot> = slot.clone();
        let mut state = self.group.state.lock().unwrap();
        state.slots.retain(|slot| slot.strong_count() != 0);
        state.slots.push(Arc::downgrade(&erased));
        BackgroundTask {
            slot,
            generation: None,
            delivered: false,
        }
    }
}

/// A typed task whose newest submission replaces its previous request.
///
/// Clones share the task slot, but each clone remembers its expected request and
/// whether it handled that completion. Use [`BackgroundTasks::task`] for work
/// that must run independently. Dropping the last handle cancels its request.
pub struct BackgroundTask<T> {
    slot: Arc<Slot<T>>,
    generation: Option<u64>,
    delivered: bool,
}

impl<T> Clone for BackgroundTask<T> {
    fn clone(&self) -> Self {
        Self {
            slot: self.slot.clone(),
            generation: self.generation,
            delivered: self.delivered,
        }
    }
}

impl<T: Send + Sync + 'static> BackgroundTask<T> {
    /// Replace the current request with a fallible future.
    ///
    /// The old completion becomes unavailable immediately. Cancellation of the
    /// old future is cooperative and does not preempt synchronous or blocking work.
    /// Returns [`BackgroundTaskError::Closed`] after group shutdown.
    pub fn submit<F, E>(&mut self, future: F) -> Result<(), BackgroundTaskError>
    where
        F: Future<Output = Result<T, E>> + Send + 'static,
        E: Into<BoxError>,
    {
        let group = self
            .slot
            .group
            .upgrade()
            .ok_or(BackgroundTaskError::Closed)?;
        let state = group.state.lock().unwrap();
        if matches!(state.lifecycle, Lifecycle::Closed) {
            return Err(BackgroundTaskError::Closed);
        }
        let generation = fresh_id();
        let (abort, registration) = AbortHandle::new_pair();
        // Capture the guard before polling so executor shutdown also reports
        // futures that were accepted but never polled.
        let mut publisher = Publisher {
            slot: Arc::downgrade(&self.slot),
            generation,
            finished: false,
        };
        let execution = async move {
            future
                .await
                .map(Arc::new)
                .map_err(|error| BackgroundTaskError::Failed(Arc::from(error.into())))
        };
        let job = async move {
            let result = match AssertUnwindSafe(execution).catch_unwind().await {
                Ok(result) => result,
                Err(panic) => Err(BackgroundTaskError::Panicked(
                    panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "non-string panic payload".into()),
                )),
            };
            publisher.finish(result);
        };
        let request = Request {
            generation,
            abort,
            queued: Some(Abortable::new(job, registration).map(|_| ()).boxed()),
            completion: None,
        };
        let old = self.slot.request.lock().unwrap().replace(request);
        let executor = state.active_executor();
        self.generation = Some(generation);
        self.delivered = false;
        drop(state);
        drop(old);
        if let Some(executor) = executor {
            self.slot.start(&executor);
        }
        Ok(())
    }

    /// Invalidate this slot's request and queued completion, then cancel its future.
    pub fn cancel(&mut self) {
        self.generation = None;
        self.delivered = false;
        self.slot.cancel();
    }

    /// Whether this state copy awaits its current request's completion delivery.
    pub fn is_pending(&self) -> bool {
        !self.delivered
            && self.generation.is_some()
            && self
                .slot
                .request
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|r| Some(r.generation) == self.generation)
    }

    /// Handle a current completion once for this state copy.
    ///
    /// Unrelated, stale, canceled, and already handled events return `None`.
    /// The shared result remains available to other state copies until the next
    /// submission, cancellation, or shutdown. No result data needs to implement `Clone`.
    pub fn handle_wake(&mut self, wake: &RuntimeWakeEvent) -> Option<Completion<T>> {
        if self.delivered || wake.key != self.slot.key || Some(wake.generation) != self.generation {
            return None;
        }
        let request = self.slot.request.lock().unwrap();
        let current = request
            .as_ref()
            .filter(|r| r.generation == wake.generation)?;
        let result = current.completion.clone()?;
        self.delivered = true;
        Some(result)
    }
}

fn fresh_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Default)]
struct Group {
    state: Mutex<GroupState>,
}

#[derive(Default)]
struct GroupState {
    lifecycle: Lifecycle,
    slots: Vec<Weak<dyn TaskSlot>>,
}

#[derive(Default)]
enum Lifecycle {
    #[default]
    Unattached,
    Attached {
        executor: Arc<dyn host::Executor>,
        active: bool,
    },
    Closed,
}

impl GroupState {
    fn active_executor(&self) -> Option<Arc<dyn host::Executor>> {
        match &self.lifecycle {
            Lifecycle::Attached {
                executor,
                active: true,
            } => Some(executor.clone()),
            _ => None,
        }
    }
}

impl Group {
    fn shutdown(&self) {
        let (old, slots) = {
            let mut state = self.state.lock().unwrap();
            (
                std::mem::replace(&mut state.lifecycle, Lifecycle::Closed),
                std::mem::take(&mut state.slots),
            )
        };
        for slot in slots.into_iter().filter_map(|slot| slot.upgrade()) {
            slot.cancel();
        }
        drop(old);
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.shutdown();
    }
}

trait TaskSlot: Send + Sync {
    fn start(&self, executor: &Arc<dyn host::Executor>);
    fn cancel(&self);
}

struct Slot<T> {
    group: Weak<Group>,
    key: RuntimeWakeKey,
    request: Mutex<Option<Request<T>>>,
}

impl<T: Send + Sync + 'static> TaskSlot for Slot<T> {
    fn start(&self, executor: &Arc<dyn host::Executor>) {
        let future = self
            .request
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|r| r.queued.take());
        if let Some(future) = future {
            executor.spawn(future);
        }
    }

    fn cancel(&self) {
        let old = self.request.lock().unwrap().take();
        drop(old);
    }
}

struct Request<T> {
    generation: u64,
    abort: AbortHandle,
    queued: Option<BoxFuture<'static, ()>>,
    completion: Option<Completion<T>>,
}

impl<T> Drop for Request<T> {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

struct Publisher<T: Send + Sync + 'static> {
    slot: Weak<Slot<T>>,
    generation: u64,
    finished: bool,
}

impl<T: Send + Sync + 'static> Publisher<T> {
    fn finish(&mut self, result: Completion<T>) {
        self.finished = true;
        let Some(slot) = self.slot.upgrade() else {
            return;
        };
        let mut request = slot.request.lock().unwrap();
        let Some(current) = request.as_mut().filter(|r| r.generation == self.generation) else {
            // User values and errors can have destructors that reenter this API.
            drop(request);
            drop(result);
            return;
        };
        current.completion = Some(result);
        drop(request);
        if let Some(group) = slot.group.upgrade() {
            let executor = group.state.lock().unwrap().active_executor();
            if let Some(executor) = executor {
                executor.wake(RuntimeWakeEvent {
                    key: slot.key.clone(),
                    generation: self.generation,
                });
            }
        }
    }
}

impl<T: Send + Sync + 'static> Drop for Publisher<T> {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(Err(BackgroundTaskError::Interrupted));
        }
    }
}

#[cfg(test)]
mod tests;
