//! Bounded image fetch scheduling: required requests, descending priority, then FIFO.
//! Workers start on demand and exit after an idle timeout.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use avenger_resource::{ResourceRequest, ResourceRequestPurpose};

const DEFAULT_MAX_CONCURRENT_IMAGE_FETCHES: usize = 16;
const WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
type Execute = Box<dyn Fn(ResourceRequest, u64) + Send + Sync>;

struct QueueEntry {
    request: ResourceRequest,
    request_id: u64,
    seq: u64,
}

#[derive(Default)]
struct SchedulerState {
    queue: Vec<QueueEntry>,
    next_seq: u64,
    workers: usize,
    idle_workers: usize,
}

pub(crate) struct FetchScheduler {
    state: Mutex<SchedulerState>,
    work: Condvar,
    max_concurrent: usize,
    execute: Execute,
}

impl FetchScheduler {
    pub(crate) fn new(execute: Execute) -> Arc<Self> {
        Self::with_limit(execute, DEFAULT_MAX_CONCURRENT_IMAGE_FETCHES)
    }

    fn with_limit(execute: Execute, max_concurrent: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(SchedulerState::default()),
            work: Condvar::new(),
            max_concurrent,
            execute,
        })
    }

    /// Queue work already registered in the cache under `request_id`.
    pub(crate) fn enqueue(self: &Arc<Self>, request: ResourceRequest, request_id: u64) {
        let mut state = self.state.lock().expect("scheduler lock poisoned");
        let seq = state.next_seq;
        state.next_seq += 1;
        state.queue.push(QueueEntry {
            request,
            request_id,
            seq,
        });
        if state.queue.len() > state.idle_workers && state.workers < self.max_concurrent {
            state.workers += 1;
            let scheduler = Arc::clone(self);
            std::thread::spawn(move || scheduler.worker_loop());
        }
        drop(state);
        self.work.notify_one();
    }

    /// A queued prefetch becomes required when its image enters the viewport.
    pub(crate) fn promote_to_required(&self, request: &ResourceRequest) {
        let mut state = self.state.lock().expect("scheduler lock poisoned");
        for entry in &mut state.queue {
            if entry.request.key == request.key {
                entry.request.purpose = ResourceRequestPurpose::Required;
                entry.request.priority = request.priority;
            }
        }
    }

    fn worker_loop(self: Arc<Self>) {
        loop {
            let entry = {
                let mut state = self.state.lock().expect("scheduler lock poisoned");
                loop {
                    let best = state
                        .queue
                        .iter()
                        .enumerate()
                        .reduce(|best, candidate| {
                            if crate::image_request_precedes(
                                &candidate.1.request,
                                candidate.1.seq,
                                &best.1.request,
                                best.1.seq,
                            ) {
                                candidate
                            } else {
                                best
                            }
                        })
                        .map(|(index, _)| index);
                    if let Some(index) = best {
                        break state.queue.swap_remove(index);
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
            // Fetching and cache callbacks can acquire locks independently.
            (self.execute)(entry.request, entry.request_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_resource::{ResourceKey, ResourceKind, ResourceSource};
    use std::sync::mpsc;

    fn request(key: &str, purpose: ResourceRequestPurpose, priority: f32) -> ResourceRequest {
        ResourceRequest {
            purpose,
            priority,
            ..ResourceRequest::new(
                ResourceKey::new(key),
                ResourceKind::new("image"),
                ResourceSource::Url {
                    url: format!("https://tiles.example/{key}.png"),
                },
            )
        }
    }

    struct GatedScheduler {
        scheduler: Arc<FetchScheduler>,
        admitted: mpsc::Receiver<(String, mpsc::Sender<()>)>,
    }

    impl GatedScheduler {
        fn new(limit: usize) -> Self {
            let (admitted_tx, admitted) = mpsc::channel();
            let scheduler = FetchScheduler::with_limit(
                Box::new(move |request, _| {
                    let (release, gate) = mpsc::channel();
                    admitted_tx.send((request.key.0, release)).unwrap();
                    gate.recv_timeout(Duration::from_secs(5)).unwrap();
                }),
                limit,
            );
            Self {
                scheduler,
                admitted,
            }
        }

        fn admit(&self) -> (String, mpsc::Sender<()>) {
            self.admitted.recv_timeout(Duration::from_secs(5)).unwrap()
        }

        fn enqueue(&self, key: &str, purpose: ResourceRequestPurpose, priority: f32) {
            self.scheduler.enqueue(request(key, purpose, priority), 0);
        }
    }

    #[test]
    fn required_priority_fifo_and_promotion_order_actual_fetches() {
        use ResourceRequestPurpose::{Prefetch, Required};
        let fixture = GatedScheduler::new(1);
        fixture.enqueue("head", Required, 0.0);
        let (key, release) = fixture.admit();
        assert_eq!(key, "head");
        fixture.enqueue("first", Prefetch, 2.0);
        fixture.enqueue("second", Prefetch, 2.0);
        fixture.enqueue("low", Prefetch, 1.0);
        fixture.enqueue("visible", Required, 0.0);
        fixture.enqueue("promoted", Prefetch, -10.0);
        fixture
            .scheduler
            .promote_to_required(&request("promoted", Required, 3.0));
        release.send(()).unwrap();
        for expected in ["promoted", "visible", "first", "second", "low"] {
            let (key, release) = fixture.admit();
            assert_eq!(key, expected);
            release.send(()).unwrap();
        }
    }

    #[test]
    fn fetches_fill_but_do_not_exceed_concurrency_limit() {
        let fixture = GatedScheduler::new(2);
        for key in ["a", "b", "c"] {
            fixture.enqueue(key, ResourceRequestPurpose::Required, 0.0);
        }
        let (_, first) = fixture.admit();
        let (_, second) = fixture.admit();
        {
            let state = fixture.scheduler.state.lock().unwrap();
            assert_eq!(state.workers, 2);
            assert_eq!(state.queue.len(), 1);
        }
        first.send(()).unwrap();
        let (key, third) = fixture.admit();
        assert_eq!(key, "c");
        second.send(()).unwrap();
        third.send(()).unwrap();
    }
}
