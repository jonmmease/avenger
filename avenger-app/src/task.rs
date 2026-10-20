//! Background tasks: jobs that compute values off the event loop, for the scene to draw.
//!
//! A [`Task`] runs one job at a time. A request replaces the job that waits or runs, and the
//! latest result stays readable until a newer request succeeds, so the scene builder can draw it,
//! through the current scales, while the next one computes. An app attaches a task with
//! [`AvengerApp::with_task`](crate::app::AvengerApp::with_task), and each finished or failed
//! request then asks the host for a rebuild through the app's waker.
//!
//! On native targets, jobs run on the multi-thread Tokio runtime that
//! [`AvengerApp::with_runtime`](crate::app::AvengerApp::with_runtime) sets. In the browser, they
//! run on one current-thread Tokio runtime, which runs in slices of about 8 ms between browser
//! tasks while any of its tasks is alive, such as a job or a query's partition that a canceled
//! job left to clean up. A slice ends after the poll that runs past its time. A DataFusion
//! task's poll runs until its partition ends or it spends Tokio's budget of 128 batches, so a
//! browser job that runs DataFusion keeps its partitions or batches small. Browser jobs are CPU
//! work: the runtime runs while any of its tasks is alive, so a task that waits on something
//! else, such as a fetch, keeps the runtime polling, and the runtime has no timer for
//! `tokio::time`.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use futures::FutureExt;
use tokio::task::AbortHandle;

use crate::waker::{AppWaker, Refresh};

/// A job's error.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// A job, boxed, as the runtime runs it.
type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Makes a request's job, once the task can run it.
type MakeJob = Box<dyn FnOnce(Cancel) -> Job + Send>;

/// A finished request: its key, which records what the value was computed for, and its value.
#[derive(Debug)]
pub struct Done<K, T> {
    pub key: K,
    pub value: T,
}

/// Computes values off the event loop, one request at a time. Clones share the task.
pub struct Task<K, T> {
    slot: Arc<Slot<K, T>>,
}

impl<K, T> Clone for Task<K, T> {
    fn clone(&self) -> Self {
        Self {
            slot: self.slot.clone(),
        }
    }
}

impl<K, T> Default for Task<K, T> {
    fn default() -> Self {
        Self {
            slot: Arc::new(Slot {
                state: Mutex::new(SlotState::default()),
            }),
        }
    }
}

impl<K, T> fmt::Debug for Task<K, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Task").finish_non_exhaustive()
    }
}

struct Slot<K, T> {
    state: Mutex<SlotState<K, T>>,
}

/// The last clone of a task cancels its pending request, since nothing can read its result.
impl<K, T> Drop for Slot<K, T> {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let Some(pending) = state.pending.take() {
            pending.cancel();
        }
    }
}

struct SlotState<K, T> {
    latest: Option<Arc<Done<K, T>>>,
    pending: Option<Pending<K>>,
    error: Option<Arc<BoxError>>,
    /// How the task runs jobs and announces their results, once an app attaches it.
    runner: Option<Runner>,
    /// The requests so far, which tell a finishing job whether a newer request replaced it.
    requests: u64,
}

impl<K, T> Default for SlotState<K, T> {
    fn default() -> Self {
        Self {
            latest: None,
            pending: None,
            error: None,
            runner: None,
            requests: 0,
        }
    }
}

/// The request that waits or runs.
struct Pending<K> {
    key: K,
    request: u64,
    cancel: Cancel,
    /// The running job, which a newer request aborts.
    running: Option<AbortHandle>,
    /// The job that waits for the task to be attached.
    waiting: Option<MakeJob>,
}

impl<K> Pending<K> {
    fn cancel(self) {
        self.cancel.0.store(true, Ordering::Release);
        if let Some(running) = self.running {
            running.abort();
        }
    }
}

/// Where an attached task runs jobs, and the waker that hears their results.
#[derive(Clone)]
pub(crate) struct Runner {
    spawner: Spawner,
    waker: AppWaker,
}

impl Runner {
    pub(crate) fn new(spawner: Spawner, waker: AppWaker) -> Self {
        Self { spawner, waker }
    }
}

/// Runs jobs on the app's runtime on native targets, and on the browser's runtime in the browser.
#[derive(Clone)]
pub(crate) enum Spawner {
    #[cfg(not(target_arch = "wasm32"))]
    Runtime(tokio::runtime::Handle),
    #[cfg(target_arch = "wasm32")]
    Browser,
}

impl Spawner {
    fn spawn(&self, job: Job) -> AbortHandle {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Spawner::Runtime(handle) => handle.spawn(job).abort_handle(),
            #[cfg(target_arch = "wasm32")]
            Spawner::Browser => browser::spawn(job),
        }
    }
}

impl<K, T> Task<K, T>
where
    K: Clone + PartialEq + Send + Sync + 'static,
    T: Send + Sync + 'static,
{
    /// A task that an app hasn't attached. Its requests wait until an app attaches it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `job` for `key` in place of the request that waits or runs, which is canceled. Does
    /// nothing when `key` is the pending request's, and only cancels the pending request when
    /// `key` is the latest result's. The job gets a [`Cancel`] that a newer request sets.
    pub fn request<F, Fut, E>(&self, key: K, job: F)
    where
        F: FnOnce(Cancel) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        E: Into<BoxError>,
    {
        let mut state = lock(&self.slot.state);
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.key == key)
        {
            return;
        }
        if let Some(replaced) = state.pending.take() {
            replaced.cancel();
        }
        state.error = None;
        if state
            .latest
            .as_ref()
            .is_some_and(|latest| latest.key == key)
        {
            return;
        }
        state.requests += 1;
        let request = state.requests;
        let slot = Arc::downgrade(&self.slot);
        let finished_key = key.clone();
        let make: MakeJob = Box::new(move |cancel| {
            Box::pin(async move {
                let run = async move { job(cancel).await };
                let result = match AssertUnwindSafe(run).catch_unwind().await {
                    Ok(result) => result.map_err(Into::into),
                    Err(panic) => Err(panic_error(panic)),
                };
                finish(&slot, request, finished_key, result);
            })
        });
        let cancel = Cancel::default();
        let (running, waiting) = match &state.runner {
            Some(runner) => (Some(runner.spawner.spawn(make(cancel.clone()))), None),
            None => (None, Some(make)),
        };
        state.pending = Some(Pending {
            key,
            request,
            cancel,
            running,
            waiting,
        });
    }

    /// The latest successful request, which stays until a newer request succeeds.
    pub fn latest(&self) -> Option<Arc<Done<K, T>>> {
        lock(&self.slot.state).latest.clone()
    }

    /// The key of the request that waits or runs.
    pub fn pending(&self) -> Option<K> {
        lock(&self.slot.state)
            .pending
            .as_ref()
            .map(|pending| pending.key.clone())
    }

    /// The last request's failure, until the next request. On native targets, a job that panics
    /// fails. In the browser, a panic aborts the module, as panics do on wasm32.
    pub fn error(&self) -> Option<Arc<BoxError>> {
        lock(&self.slot.state).error.clone()
    }
}

/// A task that an app can attach without knowing its key and value types.
pub(crate) trait Attach: Send + Sync {
    /// Run jobs through `runner` from now on, starting the request that waits.
    fn attach(&self, runner: Runner);
}

impl<K, T> Attach for Task<K, T>
where
    K: Send + Sync + 'static,
    T: Send + Sync + 'static,
{
    fn attach(&self, runner: Runner) {
        let mut state = lock(&self.slot.state);
        if let Some(pending) = &mut state.pending {
            if let Some(make) = pending.waiting.take() {
                pending.running = Some(runner.spawner.spawn(make(pending.cancel.clone())));
            }
        }
        state.runner = Some(runner);
    }
}

/// Record a job's result, unless a newer request replaced it, and ask the app's host for a
/// rebuild.
fn finish<K, T>(slot: &Weak<Slot<K, T>>, request: u64, key: K, result: Result<T, BoxError>) {
    let Some(slot) = slot.upgrade() else {
        return;
    };
    let mut state = lock(&slot.state);
    if state.pending.as_ref().map(|pending| pending.request) != Some(request) {
        return;
    }
    state.pending = None;
    match result {
        Ok(value) => state.latest = Some(Arc::new(Done { key, value })),
        Err(error) => state.error = Some(Arc::new(error)),
    }
    let waker = state.runner.as_ref().map(|runner| runner.waker.clone());
    drop(state);
    if let Some(waker) = waker {
        waker.request(Refresh::Rebuild);
    }
}

fn panic_error(panic: Box<dyn std::any::Any + Send>) -> BoxError {
    let message = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            panic
                .downcast_ref::<&str>()
                .map(|message| message.to_string())
        })
        .unwrap_or_else(|| "a job panicked".to_string());
    format!("the job panicked: {message}").into()
}

/// Set when a newer request replaces the job's request, or the task's last clone drops.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Yield to the runtime's other tasks, such as a query's partitions, and in the browser let
    /// the runtime's slice end on time, so a job that calls it between chunks of work keeps the
    /// app responsive. Err once the job is canceled.
    pub async fn yield_now(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            return Err(Cancelled);
        }
        tokio::task::yield_now().await;
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// The error of a job that stopped because it was canceled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the request was canceled")
    }
}

impl Error for Cancelled {}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A runtime that runs in slices: a current-thread runtime that returns to its driver after each
/// task's poll, so a slice ends soon after its time is up.
#[cfg(any(target_arch = "wasm32", test))]
fn slice_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .event_interval(1)
        .build()
        .expect("the slice runtime builds")
}

/// Run `runtime`'s tasks a poll at a time while any task is alive and `time_left` says the slice
/// has time. Returns whether a task is still alive.
///
/// A task counts as alive until the runtime drops it, so a canceled job's child tasks, such as a
/// query's partitions, finish their cleanup before the runtime stops.
#[cfg(any(target_arch = "wasm32", test))]
fn run_slice(runtime: &tokio::runtime::Runtime, mut time_left: impl FnMut() -> bool) -> bool {
    let alive = || runtime.metrics().num_alive_tasks() > 0;
    while alive() && time_left() {
        runtime.block_on(tokio::task::yield_now());
    }
    alive()
}

/// The browser's runtime: one current-thread Tokio runtime, which runs in slices of about 8 ms
/// between browser tasks while any of its tasks is alive.
#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::Cell;

    use tokio::task::AbortHandle;
    use wasm_bindgen::JsCast;

    use super::{run_slice, slice_runtime, Job};

    /// How long the runtime runs before the page gets the thread back.
    const SLICE_MS: f64 = 8.0;

    thread_local! {
        static RUNTIME: tokio::runtime::Runtime = slice_runtime();
        static DRIVING: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn spawn(job: Job) -> AbortHandle {
        let handle = RUNTIME.with(|runtime| runtime.spawn(job).abort_handle());
        if !DRIVING.with(|driving| driving.replace(true)) {
            wasm_bindgen_futures::spawn_local(drive());
        }
        handle
    }

    /// Run the runtime in slices between browser tasks until none of its tasks is alive.
    async fn drive() {
        loop {
            let start = js_sys::Date::now();
            let alive = RUNTIME
                .with(|runtime| run_slice(runtime, || js_sys::Date::now() - start < SLICE_MS));
            if !alive {
                DRIVING.with(|driving| driving.set(false));
                return;
            }
            next_task().await;
        }
    }

    /// Wait for the browser's next task, through a zero-delay timeout.
    async fn next_task() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    resolve.unchecked_ref(),
                    0,
                );
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use tokio::task::JoinHandle;

    use super::{run_slice, slice_runtime};

    /// Counts its drop.
    struct Dropped(Arc<AtomicUsize>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Aborts its task when it drops, as DataFusion's `SpawnedTask` does.
    struct AbortOnDrop(JoinHandle<()>);

    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    /// A slice whose time never runs out within `turns` polls.
    fn turns(mut turns: usize) -> impl FnMut() -> bool {
        move || {
            turns = turns.saturating_sub(1);
            turns > 0
        }
    }

    /// A canceled job's child and grandchild tasks, which wait as a query's partitions do, drop
    /// before the runtime runs out of alive tasks, though no other job starts.
    #[test]
    fn a_slice_runs_until_a_canceled_jobs_children_drop() {
        let runtime = slice_runtime();
        let dropped = Arc::new(AtomicUsize::new(0));
        let guard = || Dropped(dropped.clone());
        let (job_guard, child_guard, grandchild_guard) = (guard(), guard(), guard());
        let job = runtime.spawn(async move {
            let _guard = job_guard;
            let _child = AbortOnDrop(tokio::spawn(async move {
                let _guard = child_guard;
                let _grandchild = AbortOnDrop(tokio::spawn(async move {
                    let _guard = grandchild_guard;
                    pending::<()>().await
                }));
                pending::<()>().await
            }));
            pending::<()>().await
        });
        assert!(run_slice(&runtime, turns(10)));
        assert_eq!(runtime.metrics().num_alive_tasks(), 3);

        job.abort();
        assert!(!run_slice(&runtime, turns(100)));
        assert_eq!(dropped.load(Ordering::SeqCst), 3);
    }
}
