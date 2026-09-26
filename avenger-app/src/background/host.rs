//! Adapter contract for hosts that execute app background work.

use super::{BackgroundTaskError, BackgroundTasks, Group, Lifecycle};
use avenger_eventstream::runtime::RuntimeWakeEvent;
use futures::future::BoxFuture;
use std::sync::Arc;

/// Platform execution and event delivery. Methods run without task-group locks.
pub trait Executor: Send + Sync + 'static {
    /// Schedule the future without blocking the app event handler.
    fn spawn(&self, future: BoxFuture<'static, ()>);
    /// Queue an immediate completion event for the owning app.
    fn wake(&self, event: RuntimeWakeEvent);
}

/// The sole host attachment for a task group. Drop shuts down all its work.
pub struct Attachment {
    group: Arc<Group>,
}

impl Attachment {
    /// Attach an executor without starting queued work.
    ///
    /// Returns an error if the group is closed or already attached. Keep this
    /// guard until the host exits or successfully installs a replacement app.
    pub fn new(
        tasks: &BackgroundTasks,
        executor: Arc<dyn Executor>,
    ) -> Result<Self, BackgroundTaskError> {
        let mut state = tasks.group.state.lock().unwrap();
        match state.lifecycle {
            Lifecycle::Unattached => {}
            Lifecycle::Attached { .. } => return Err(BackgroundTaskError::AlreadyAttached),
            Lifecycle::Closed => return Err(BackgroundTaskError::Closed),
        }
        state.lifecycle = Lifecycle::Attached {
            executor,
            active: false,
        };
        Ok(Self {
            group: tasks.group.clone(),
        })
    }

    /// Start queued work once the host can receive completion events.
    pub fn activate(&self) {
        let (executor, slots) = {
            let mut state = self.group.state.lock().unwrap();
            let Lifecycle::Attached { executor, active } = &mut state.lifecycle else {
                return;
            };
            if *active {
                return;
            }
            *active = true;
            (executor.clone(), state.slots.clone())
        };
        for slot in slots.into_iter().filter_map(|slot| slot.upgrade()) {
            slot.start(&executor);
        }
    }

    /// Invalidate queued results and cancel every request. Repeated calls are harmless.
    pub fn shutdown(&self) {
        self.group.shutdown();
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.shutdown();
    }
}
