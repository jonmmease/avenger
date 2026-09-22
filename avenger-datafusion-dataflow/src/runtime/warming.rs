use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
};

use tokio::{runtime::Handle, sync::Notify};

use super::{PreparedDataflow, PreparedInner, RuntimeInner};
use crate::{fresh_id, Inputs};

/// One dispatcher serves FIFO turns across preparation/target groups.
#[derive(Default)]
pub(super) struct Scheduler {
    state: Mutex<State>,
    changed: Notify,
}

#[derive(Default)]
struct State {
    groups: VecDeque<Group>,
    dispatching: bool,
}

struct Group {
    namespace: u64,
    targets: Vec<usize>,
    requests: VecDeque<Request>,
}

struct Request {
    id: u64,
    // The owning query keeps the preparation alive until admission.
    prepared: Weak<PreparedInner>,
    inputs: Inputs,
}

/// Dropping this interest removes only work that has not been admitted.
pub(crate) struct Interest {
    scheduler: Weak<Scheduler>,
    id: u64,
}

impl Drop for Interest {
    fn drop(&mut self) {
        let Some(scheduler) = self.scheduler.upgrade() else {
            return;
        };
        let removed = {
            let mut state = scheduler.state.lock().expect("warming lock");
            let position = state.groups.iter().enumerate().find_map(|(group, entry)| {
                entry
                    .requests
                    .iter()
                    .position(|r| r.id == self.id)
                    .map(|request| (group, request))
            });
            let Some((group, request)) = position else {
                return;
            };
            let removed = state.groups[group].requests.remove(request);
            if state.groups[group].requests.is_empty() {
                state.groups.remove(group);
            }
            removed
        };
        scheduler.changed.notify_one();
        drop(removed);
    }
}

impl Scheduler {
    pub(super) fn enqueue(
        self: &Arc<Self>,
        prepared: &PreparedDataflow,
        inputs: Inputs,
        targets: Vec<usize>,
        executor: Handle,
    ) -> Interest {
        let id = fresh_id();
        let worker = {
            let mut state = self.state.lock().expect("warming lock");
            let group = state
                .groups
                .iter()
                .position(|group| {
                    group.namespace == prepared.inner.namespace && group.targets == targets
                })
                .unwrap_or_else(|| {
                    state.groups.push_back(Group {
                        namespace: prepared.inner.namespace,
                        targets,
                        requests: VecDeque::new(),
                    });
                    state.groups.len() - 1
                });
            state.groups[group].requests.push_back(Request {
                id,
                prepared: Arc::downgrade(&prepared.inner),
                inputs,
            });
            if state.dispatching {
                None
            } else {
                state.dispatching = true;
                Some(Dispatcher {
                    runtime: prepared.inner.runtime.clone(),
                    armed: true,
                })
            }
        };
        if let Some(worker) = worker {
            executor.spawn(worker.run());
        }
        Interest {
            scheduler: Arc::downgrade(self),
            id,
        }
    }

    fn take_next(&self) -> Option<(PreparedDataflow, Inputs, Vec<usize>)> {
        let mut state = self.state.lock().expect("warming lock");
        let mut group = state.groups.pop_front()?;
        let request = group.requests.pop_front().expect("nonempty warming group");
        let targets = group.targets.clone();
        if !group.requests.is_empty() {
            state.groups.push_back(group);
        }
        // Acquire execution ownership under the same lock that removes pending interest.
        let prepared = request.prepared.upgrade();
        drop(state);
        prepared.map(|inner| (PreparedDataflow { inner }, request.inputs, targets))
    }

    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        let state = self.state.lock().expect("warming lock");
        !state.dispatching && state.groups.is_empty()
    }
}

struct Dispatcher {
    runtime: Arc<RuntimeInner>,
    armed: bool,
}

impl Dispatcher {
    async fn run(mut self) {
        let runtime = self.runtime.clone();
        loop {
            let acquire = runtime.queries.acquire();
            tokio::pin!(acquire);
            let permit = loop {
                let changed = runtime.warming.changed.notified();
                {
                    let mut state = runtime.warming.state.lock().expect("warming lock");
                    if state.groups.is_empty() {
                        state.dispatching = false;
                        self.armed = false;
                        return;
                    }
                }
                // Queue updates preserve this waiter's execution-admission position.
                tokio::select! {
                    biased;
                    permit = &mut acquire => break permit.expect("private semaphore stays open"),
                    _ = changed => {},
                }
            };
            if let Some((prepared, inputs, targets)) = runtime.warming.take_next() {
                prepared.warm_targets(inputs, targets).await;
            }
            drop(permit);
        }
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        if self.armed {
            // An executor can drop a dispatched future before its first poll.
            self.runtime
                .warming
                .state
                .lock()
                .expect("warming lock")
                .dispatching = false;
        }
    }
}

#[cfg(test)]
mod tests;
