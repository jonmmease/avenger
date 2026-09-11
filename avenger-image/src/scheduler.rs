//! Bounded, priority-aware image fetch scheduler with cursor-focus
//! re-scoring and hover-driven prefetch retargeting.
//!
//! Uncached image requests queue here and at most `max_concurrent`
//! worker threads (spawned lazily, exiting after an idle timeout)
//! execute them. Pop order: `Required`
//! before `Prefetch`; within `Prefetch`, nearest to the current focus
//! hint when one is set (falling back to plan-time priority); FIFO on
//! ties. `replace_prefetch_set` atomically swaps a scope's queued
//! prefetch entries — cancelled entries never reach the network.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use avenger_resource::{
    PrefetchRetargetPlanner, PrefetchScope, ResourceKey, ResourceRequest, ResourceRequestPurpose,
};

pub(crate) const DEFAULT_MAX_CONCURRENT_IMAGE_FETCHES: usize = 16;
pub(crate) const DEFAULT_FOCUS_DEBOUNCE: Duration = Duration::from_millis(150);
const WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const DEBOUNCER_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

type CancelPending = Box<dyn Fn(&ResourceKey, u64) + Send + Sync>;
type BeginRequest = Box<dyn Fn(&ResourceRequest) -> Option<u64> + Send + Sync>;

/// Cache-side callbacks so the scheduler stays storage-agnostic (and unit
/// testable). All hooks are invoked WITHOUT the scheduler lock held.
pub(crate) struct SchedulerHooks {
    /// Run the fetch/decode for an admitted request (worker thread).
    pub execute: Box<dyn Fn(ResourceRequest, u64) + Send + Sync>,
    /// A queued entry was cancelled before admission; clear its pending
    /// cache slot iff it still belongs to `request_id`.
    pub cancel_pending: CancelPending,
    /// Gate + register a brand-new request (retarget delta): returns the
    /// request id to enqueue with, or `None` when the key is already
    /// ready/pending.
    pub begin_request: BeginRequest,
}

struct QueueEntry {
    request: ResourceRequest,
    request_id: u64,
    seq: u64,
}

struct InflightEntry {
    key: ResourceKey,
    request_id: u64,
    purpose: ResourceRequestPurpose,
    prefetch_scope: Option<PrefetchScope>,
}

#[derive(Default)]
struct SchedulerState {
    queue: Vec<QueueEntry>,
    inflight: Vec<InflightEntry>,
    next_seq: u64,
    workers: usize,
    idle_workers: usize,
    focus: Option<[f32; 2]>,
    focus_seq: u64,
    focus_handled_seq: u64,
    last_focus_at: Option<Instant>,
    gesture_active: bool,
    planners: Vec<Arc<dyn PrefetchRetargetPlanner>>,
    last_retarget_keys: HashMap<PrefetchScope, HashSet<ResourceKey>>,
    debouncer_running: bool,
}

pub(crate) struct FetchScheduler {
    state: Mutex<SchedulerState>,
    work: Condvar,
    focus_signal: Condvar,
    max_concurrent: usize,
    focus_debounce: Duration,
    hooks: SchedulerHooks,
}

impl FetchScheduler {
    pub(crate) fn new(hooks: SchedulerHooks) -> Arc<Self> {
        Self::with_limits(
            hooks,
            DEFAULT_MAX_CONCURRENT_IMAGE_FETCHES,
            DEFAULT_FOCUS_DEBOUNCE,
        )
    }

    pub(crate) fn with_limits(
        hooks: SchedulerHooks,
        max_concurrent: usize,
        focus_debounce: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(SchedulerState::default()),
            work: Condvar::new(),
            focus_signal: Condvar::new(),
            max_concurrent: max_concurrent.max(1),
            focus_debounce,
            hooks,
        })
    }

    /// Queue a request whose pending cache slot is already registered
    /// under `request_id`.
    pub(crate) fn enqueue(self: &Arc<Self>, request: ResourceRequest, request_id: u64) {
        let mut state = self.state.lock().expect("scheduler lock poisoned");
        let seq = state.next_seq;
        state.next_seq += 1;
        state.queue.push(QueueEntry {
            request,
            request_id,
            seq,
        });
        self.ensure_worker(&mut state);
        drop(state);
        self.work.notify_one();
    }

    /// Upgrade a queued entry to `Required` ordering (the tile became
    /// visible while still waiting in the queue).
    pub(crate) fn promote_to_required(&self, key: &ResourceKey) {
        let mut state = self.state.lock().expect("scheduler lock poisoned");
        for entry in &mut state.queue {
            if &entry.request.key == key {
                entry.request.purpose = ResourceRequestPurpose::Required;
                entry.request.priority = entry.request.priority.max(0.0);
            }
        }
    }

    /// Atomically make `requests` the queued prefetch working set for
    /// `scope`: queued-not-admitted `Prefetch` entries in the scope whose
    /// key is absent from `requests` are cancelled (the network never
    /// sees them); the delta is registered + enqueued. `Required` entries
    /// are never touched. In-flight entries finish by default (their
    /// bytes remain cache-useful); `abort_inflight` instead clears their
    /// pending slots so their results are discarded on completion.
    pub(crate) fn replace_prefetch_set(
        self: &Arc<Self>,
        scope: &PrefetchScope,
        requests: Vec<ResourceRequest>,
        abort_inflight: bool,
    ) {
        let new_keys = requests
            .iter()
            .map(|request| request.key.clone())
            .collect::<HashSet<_>>();

        let (cancelled, already_queued) = {
            let mut state = self.state.lock().expect("scheduler lock poisoned");
            let mut cancelled = Vec::new();
            let mut already_queued = HashSet::new();
            state.queue.retain(|entry| {
                let in_scope = entry.request.purpose == ResourceRequestPurpose::Prefetch
                    && entry.request.prefetch_scope.as_ref() == Some(scope);
                let stale = in_scope && !new_keys.contains(&entry.request.key);
                if stale {
                    cancelled.push((entry.request.key.clone(), entry.request_id));
                } else if in_scope {
                    already_queued.insert(entry.request.key.clone());
                }
                !stale
            });
            if abort_inflight {
                for entry in &state.inflight {
                    if entry.purpose == ResourceRequestPurpose::Prefetch
                        && entry.prefetch_scope.as_ref() == Some(scope)
                        && !new_keys.contains(&entry.key)
                    {
                        cancelled.push((entry.key.clone(), entry.request_id));
                    }
                }
            }
            (cancelled, already_queued)
        };
        for (key, request_id) in &cancelled {
            (self.hooks.cancel_pending)(key, *request_id);
        }

        for request in requests {
            if already_queued.contains(&request.key) {
                continue;
            }
            if let Some(request_id) = (self.hooks.begin_request)(&request) {
                self.enqueue(request, request_id);
            }
        }
    }

    /// Record the hover cursor position (canvas px). Cheap: an atomic
    /// store + debouncer wakeup; the retarget runs only after the cursor
    /// rests for the debounce window.
    pub(crate) fn update_focus(self: &Arc<Self>, cursor_canvas_px: [f32; 2]) {
        let mut state = self.state.lock().expect("scheduler lock poisoned");
        state.focus = Some(cursor_canvas_px);
        state.focus_seq = state.focus_seq.wrapping_add(1);
        state.last_focus_at = Some(Instant::now());
        if !state.debouncer_running && !state.planners.is_empty() {
            state.debouncer_running = true;
            let scheduler = Arc::clone(self);
            std::thread::spawn(move || scheduler.debounce_loop());
        }
        drop(state);
        self.focus_signal.notify_all();
    }

    /// While a pan/zoom gesture is active, evaluations own the prefetch
    /// set; hover retargeting is suppressed (ordering still tracks focus).
    pub(crate) fn set_gesture_active(&self, active: bool) {
        let mut state = self.state.lock().expect("scheduler lock poisoned");
        state.gesture_active = active;
        drop(state);
        self.focus_signal.notify_all();
    }

    /// Replace the installed planners (called after each evaluation).
    /// Resets retarget hysteresis: the new evaluation's plan is the new
    /// baseline.
    pub(crate) fn install_retarget_planners(
        &self,
        planners: Vec<Arc<dyn PrefetchRetargetPlanner>>,
    ) {
        let mut state = self.state.lock().expect("scheduler lock poisoned");
        state.planners = planners;
        state.last_retarget_keys.clear();
    }

    #[cfg(test)]
    fn queued_keys(&self) -> Vec<ResourceKey> {
        let state = self.state.lock().expect("scheduler lock poisoned");
        state
            .queue
            .iter()
            .map(|entry| entry.request.key.clone())
            .collect()
    }

    fn ensure_worker(self: &Arc<Self>, state: &mut SchedulerState) {
        if state.idle_workers == 0 && state.workers < self.max_concurrent {
            state.workers += 1;
            let scheduler = Arc::clone(self);
            std::thread::spawn(move || scheduler.worker_loop());
        }
    }

    fn worker_loop(self: Arc<Self>) {
        loop {
            let entry = {
                let mut state = self.state.lock().expect("scheduler lock poisoned");
                loop {
                    if let Some(index) = best_entry_index(&state) {
                        let entry = state.queue.swap_remove(index);
                        state.inflight.push(InflightEntry {
                            key: entry.request.key.clone(),
                            request_id: entry.request_id,
                            purpose: entry.request.purpose,
                            prefetch_scope: entry.request.prefetch_scope.clone(),
                        });
                        break entry;
                    }
                    state.idle_workers += 1;
                    let (next, timeout) = self
                        .work
                        .wait_timeout(state, WORKER_IDLE_TIMEOUT)
                        .expect("scheduler lock poisoned");
                    state = next;
                    state.idle_workers -= 1;
                    if timeout.timed_out() && state.queue.is_empty() {
                        state.workers -= 1;
                        return;
                    }
                }
            };
            let (key, request_id) = (entry.request.key.clone(), entry.request_id);
            (self.hooks.execute)(entry.request, request_id);
            let mut state = self.state.lock().expect("scheduler lock poisoned");
            state
                .inflight
                .retain(|entry| !(entry.key == key && entry.request_id == request_id));
        }
    }

    fn debounce_loop(self: Arc<Self>) {
        let mut state = self.state.lock().expect("scheduler lock poisoned");
        loop {
            // Wait for an unhandled focus update (or exit after idling).
            while state.focus_seq == state.focus_handled_seq {
                let (next, timeout) = self
                    .focus_signal
                    .wait_timeout(state, DEBOUNCER_IDLE_TIMEOUT)
                    .expect("scheduler lock poisoned");
                state = next;
                if timeout.timed_out() && state.focus_seq == state.focus_handled_seq {
                    state.debouncer_running = false;
                    return;
                }
            }
            // Wait until the cursor has rested for the debounce window.
            while let Some(last) = state.last_focus_at {
                let deadline = last + self.focus_debounce;
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let (next, _) = self
                    .focus_signal
                    .wait_timeout(state, deadline - now)
                    .expect("scheduler lock poisoned");
                state = next;
            }
            state.focus_handled_seq = state.focus_seq;
            if state.gesture_active || state.planners.is_empty() {
                continue;
            }
            let Some(focus) = state.focus else { continue };
            let planners = state.planners.clone();
            drop(state);

            for planner in planners {
                if let Some(requests) = planner.plan(focus) {
                    let keys = requests
                        .iter()
                        .map(|request| request.key.clone())
                        .collect::<HashSet<_>>();
                    let unchanged = {
                        let mut locked = self.state.lock().expect("scheduler lock poisoned");
                        if locked.last_retarget_keys.get(planner.scope()) == Some(&keys) {
                            true
                        } else {
                            locked
                                .last_retarget_keys
                                .insert(planner.scope().clone(), keys);
                            false
                        }
                    };
                    if !unchanged {
                        // In-flight fetches finish: their bytes are
                        // near the cursor's recent path and land usefully
                        // in the cache.
                        self.replace_prefetch_set(planner.scope(), requests, false);
                    }
                }
            }

            state = self.state.lock().expect("scheduler lock poisoned");
        }
    }
}

/// Index of the entry to admit next: `Required` first; within `Prefetch`,
/// nearest to the focus hint (entries without a screen center sort last);
/// otherwise plan-time priority; FIFO on ties.
fn best_entry_index(state: &SchedulerState) -> Option<usize> {
    let focus = state.focus;
    let mut best: Option<(usize, PopKey)> = None;
    for (index, entry) in state.queue.iter().enumerate() {
        let key = PopKey::for_entry(entry, focus);
        match &best {
            Some((_, current)) if !key.beats(current) => {}
            _ => best = Some((index, key)),
        }
    }
    best.map(|(index, _)| index)
}

struct PopKey {
    required: bool,
    score: f64,
    seq: u64,
}

impl PopKey {
    fn for_entry(entry: &QueueEntry, focus: Option<[f32; 2]>) -> Self {
        let required = entry.request.purpose == ResourceRequestPurpose::Required;
        // Higher score pops first. With a live focus hint, prefetch
        // entries re-score by proximity to it (admission-time re-scoring);
        // otherwise plan-time priority applies.
        let score = if !required {
            match (focus, entry.request.screen_center) {
                (Some(focus), Some(center)) => {
                    let dx = f64::from(center[0]) - f64::from(focus[0]);
                    let dy = f64::from(center[1]) - f64::from(focus[1]);
                    -dx.hypot(dy)
                }
                (Some(_), None) => f64::MIN,
                (None, _) => f64::from(entry.request.priority),
            }
        } else {
            f64::from(entry.request.priority)
        };
        Self {
            required,
            score,
            seq: entry.seq,
        }
    }

    fn beats(&self, other: &Self) -> bool {
        if self.required != other.required {
            return self.required;
        }
        if self.score != other.score {
            return self.score > other.score;
        }
        self.seq < other.seq
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use avenger_resource::{ResourceKind, ResourceSource};

    use super::*;

    fn request(key: &str, purpose: ResourceRequestPurpose, priority: f32) -> ResourceRequest {
        ResourceRequest {
            priority,
            purpose,
            ..ResourceRequest::new(
                ResourceKey::new(key),
                ResourceKind::new("image"),
                ResourceSource::Url {
                    url: format!("https://tiles.example/{key}.png"),
                },
            )
        }
    }

    fn prefetch(
        key: &str,
        priority: f32,
        scope: &str,
        center: Option<[f32; 2]>,
    ) -> ResourceRequest {
        let mut request = request(key, ResourceRequestPurpose::Prefetch, priority);
        request.prefetch_scope = Some(PrefetchScope::new(scope));
        request.screen_center = center;
        request
    }

    /// Scheduler whose single worker blocks on a gate until released, so
    /// enqueued entries stay queued; executed keys are recorded in order.
    #[allow(
        clippy::type_complexity,
        reason = "The tuple describes the input or output of this test fixture."
    )]
    fn gated_scheduler() -> (
        Arc<FetchScheduler>,
        mpsc::Receiver<String>,
        mpsc::Sender<()>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let (executed_tx, executed_rx) = mpsc::channel::<String>();
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let gate_rx = Mutex::new(gate_rx);
        let cancelled = Arc::new(Mutex::new(Vec::new()));
        let cancelled_hook = cancelled.clone();
        let hooks = SchedulerHooks {
            execute: Box::new(move |request, _| {
                gate_rx
                    .lock()
                    .expect("gate lock")
                    .recv()
                    .expect("gate closed");
                executed_tx.send(request.key.0.clone()).ok();
            }),
            cancel_pending: Box::new(move |key, _| {
                cancelled_hook
                    .lock()
                    .expect("cancel lock")
                    .push(key.0.clone());
            }),
            begin_request: Box::new(|_| Some(0)),
        };
        let scheduler = FetchScheduler::with_limits(hooks, 1, Duration::from_millis(25));
        (scheduler, executed_rx, gate_tx, cancelled)
    }

    fn drain(rx: &mpsc::Receiver<String>, gate: &mpsc::Sender<()>, count: usize) -> Vec<String> {
        let mut out = Vec::new();
        for _ in 0..count {
            gate.send(()).expect("gate send");
            out.push(
                rx.recv_timeout(Duration::from_secs(5))
                    .expect("executed key"),
            );
        }
        out
    }

    #[test]
    fn pops_required_before_prefetch_then_priority_then_fifo() {
        let (scheduler, executed, gate, _) = gated_scheduler();
        // First entry occupies the single worker (blocked on the gate).
        scheduler.enqueue(request("head", ResourceRequestPurpose::Required, 0.0), 1);
        std::thread::sleep(Duration::from_millis(50));
        scheduler.enqueue(prefetch("far", -0.9, "s", None), 2);
        scheduler.enqueue(prefetch("near", -0.1, "s", None), 3);
        scheduler.enqueue(request("visible", ResourceRequestPurpose::Required, 0.0), 4);
        scheduler.enqueue(prefetch("mid", -0.5, "s", None), 5);

        let order = drain(&executed, &gate, 5);
        assert_eq!(order, vec!["head", "visible", "near", "mid", "far"]);
    }

    #[test]
    fn focus_hint_rescores_queued_prefetch_on_admission() {
        let (scheduler, executed, gate, _) = gated_scheduler();
        scheduler.enqueue(request("head", ResourceRequestPurpose::Required, 0.0), 1);
        std::thread::sleep(Duration::from_millis(50));
        // Plan-time priorities favor "a", but the focus hint sits next to
        // "b"'s screen center.
        scheduler.enqueue(prefetch("a", -0.1, "s", Some([0.0, 0.0])), 2);
        scheduler.enqueue(prefetch("b", -0.9, "s", Some([500.0, 500.0])), 3);
        scheduler.update_focus([490.0, 505.0]);

        let order = drain(&executed, &gate, 3);
        assert_eq!(order, vec!["head", "b", "a"]);
    }

    #[test]
    fn replace_prefetch_set_cancels_only_stale_in_scope_prefetch() {
        let (scheduler, executed, gate, cancelled) = gated_scheduler();
        scheduler.enqueue(request("head", ResourceRequestPurpose::Required, 0.0), 1);
        std::thread::sleep(Duration::from_millis(50));
        scheduler.enqueue(prefetch("keep", -0.2, "geo/map/base", None), 2);
        scheduler.enqueue(prefetch("stale", -0.3, "geo/map/base", None), 3);
        scheduler.enqueue(prefetch("other-scope", -0.4, "geo/map/labels", None), 4);
        scheduler.enqueue(
            request("required", ResourceRequestPurpose::Required, 0.0),
            5,
        );

        scheduler.replace_prefetch_set(
            &PrefetchScope::new("geo/map/base"),
            vec![
                prefetch("keep", -0.2, "geo/map/base", None),
                prefetch("fresh", -0.1, "geo/map/base", None),
            ],
            false,
        );

        assert_eq!(*cancelled.lock().expect("cancel lock"), vec!["stale"]);
        let mut queued = scheduler.queued_keys();
        queued.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            queued.iter().map(|key| key.0.as_str()).collect::<Vec<_>>(),
            vec!["fresh", "keep", "other-scope", "required"]
        );
        let order = drain(&executed, &gate, 5);
        assert_eq!(order[0], "head");
        assert_eq!(order[1], "required");
    }

    #[test]
    fn abort_inflight_cancels_running_prefetch_out_of_set() {
        let (scheduler, executed, gate, cancelled) = gated_scheduler();
        // The single worker admits this prefetch and blocks on the gate —
        // it is in flight, not queued.
        scheduler.enqueue(prefetch("inflight", -0.2, "geo/map/base", None), 7);
        std::thread::sleep(Duration::from_millis(50));

        // Default: in-flight entries are left to finish.
        scheduler.replace_prefetch_set(&PrefetchScope::new("geo/map/base"), Vec::new(), false);
        assert!(cancelled.lock().expect("cancel lock").is_empty());

        // abort_inflight discards the result via the pending-slot clear.
        scheduler.replace_prefetch_set(&PrefetchScope::new("geo/map/base"), Vec::new(), true);
        assert_eq!(*cancelled.lock().expect("cancel lock"), vec!["inflight"]);

        gate.send(()).expect("gate send");
        assert_eq!(
            executed.recv_timeout(Duration::from_secs(5)).expect("done"),
            "inflight"
        );
    }

    #[test]
    fn promote_to_required_reorders_a_queued_prefetch() {
        let (scheduler, executed, gate, _) = gated_scheduler();
        scheduler.enqueue(request("head", ResourceRequestPurpose::Required, 0.0), 1);
        std::thread::sleep(Duration::from_millis(50));
        scheduler.enqueue(prefetch("promoted", -0.9, "s", None), 2);
        scheduler.enqueue(prefetch("plain", -0.1, "s", None), 3);
        scheduler.promote_to_required(&ResourceKey::new("promoted"));

        let order = drain(&executed, &gate, 3);
        assert_eq!(order, vec!["head", "promoted", "plain"]);
    }

    struct TestPlanner {
        scope: PrefetchScope,
        calls: Arc<Mutex<Vec<[f32; 2]>>>,
        requests: Vec<ResourceRequest>,
    }

    impl PrefetchRetargetPlanner for TestPlanner {
        fn scope(&self) -> &PrefetchScope {
            &self.scope
        }

        fn plan(&self, cursor_canvas_px: [f32; 2]) -> Option<Vec<ResourceRequest>> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(cursor_canvas_px);
            Some(self.requests.clone())
        }
    }

    #[test]
    fn hover_debounce_fires_once_after_rest_and_skips_gestures() {
        let (scheduler, _executed, _gate, _) = gated_scheduler();
        let calls = Arc::new(Mutex::new(Vec::new()));
        scheduler.install_retarget_planners(vec![Arc::new(TestPlanner {
            scope: PrefetchScope::new("geo/map/base"),
            calls: calls.clone(),
            requests: vec![prefetch("t", -0.1, "geo/map/base", None)],
        })]);

        // A burst of moves: nothing fires until the cursor rests.
        for i in 0..10 {
            scheduler.update_focus([i as f32, 0.0]);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(calls.lock().expect("calls lock").is_empty());
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(calls.lock().expect("calls lock").as_slice(), &[[9.0, 0.0]]);

        // Same focus set again: handled seq advances, no duplicate call
        // (and the identical key set is a hysteresis no-op anyway).
        scheduler.update_focus([9.0, 0.0]);
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(calls.lock().expect("calls lock").len(), 2);

        // Gesture-active suppresses retargeting entirely.
        scheduler.set_gesture_active(true);
        scheduler.update_focus([50.0, 50.0]);
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(calls.lock().expect("calls lock").len(), 2);
        scheduler.set_gesture_active(false);
    }
}
