use avenger_app::background::{
    host::{Attachment, Executor},
    BackgroundTaskError, BackgroundTasks,
};
use avenger_eventstream::runtime::RuntimeWakeEvent;
use futures::future::BoxFuture;
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;

use crate::WinitWgpuEvent;

struct WinitExecutor {
    proxy: EventLoopProxy<WinitWgpuEvent>,
    generation: u64,
    #[cfg(not(target_arch = "wasm32"))]
    runtime: tokio::runtime::Handle,
}

impl Executor for WinitExecutor {
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        #[cfg(not(target_arch = "wasm32"))]
        self.runtime.spawn(future);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(future);
    }

    fn wake(&self, event: RuntimeWakeEvent) {
        let _ = self.proxy.send_event(WinitWgpuEvent::BackgroundTaskReady {
            host_generation: self.generation,
            event,
        });
    }
}

pub(super) fn attach(
    tasks: Option<&BackgroundTasks>,
    proxy: EventLoopProxy<WinitWgpuEvent>,
    generation: u64,
    #[cfg(not(target_arch = "wasm32"))] runtime: tokio::runtime::Handle,
) -> Result<Option<Attachment>, BackgroundTaskError> {
    tasks
        .map(|tasks| {
            Attachment::new(
                tasks,
                Arc::new(WinitExecutor {
                    proxy,
                    generation,
                    #[cfg(not(target_arch = "wasm32"))]
                    runtime,
                }),
            )
        })
        .transpose()
}

/// The installed attachment follows the canvas lifecycle, including async startup.
#[derive(Default)]
pub(super) struct BackgroundHost {
    current: Option<(u64, Attachment)>,
    ready: bool,
    closed: bool,
}

impl BackgroundHost {
    pub fn install(&mut self, generation: u64, attachment: Option<Attachment>) {
        self.current = None;
        if !self.closed {
            self.current = attachment.map(|a| (generation, a));
            if self.ready {
                self.activate();
            }
        }
    }

    pub fn activate(&mut self) {
        if !self.closed {
            self.ready = true;
            if let Some((_, attachment)) = &self.current {
                attachment.activate();
            }
        }
    }

    pub fn accepts(&self, generation: u64) -> bool {
        self.ready
            && self
                .current
                .as_ref()
                .is_some_and(|(id, _)| *id == generation)
    }

    pub fn shutdown(&mut self) {
        self.closed = true;
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use std::{convert::Infallible, sync::Mutex};

    #[derive(Default)]
    struct ManualExecutor(Mutex<Vec<BoxFuture<'static, ()>>>);

    impl Executor for ManualExecutor {
        fn spawn(&self, future: BoxFuture<'static, ()>) {
            self.0.lock().unwrap().push(future);
        }
        fn wake(&self, _: RuntimeWakeEvent) {}
    }

    #[test]
    fn startup_replacement_and_shutdown_control_the_installed_tasks() {
        let executor = Arc::new(ManualExecutor::default());
        let tasks = BackgroundTasks::new();
        let mut task = tasks.task::<()>();
        task.submit(async { Ok::<_, Infallible>(()) }).unwrap();
        let attachment = Attachment::new(&tasks, executor.clone()).unwrap();
        let mut host = BackgroundHost::default();
        host.install(0, Some(attachment));
        assert!(executor.0.lock().unwrap().is_empty());
        assert!(!host.accepts(0));
        host.activate();
        assert!(host.accepts(0));
        assert_eq!(executor.0.lock().unwrap().len(), 1);

        // Failed attachment and discarded candidates leave the installed group live.
        assert!(Attachment::new(&tasks, executor.clone()).is_err());
        let rejected = BackgroundTasks::new();
        drop(Attachment::new(&rejected, executor.clone()).unwrap());
        assert!(task.is_pending());
        assert!(host.accepts(0));

        let replacement = BackgroundTasks::new();
        let mut next = replacement.task::<()>();
        next.submit(async { Ok::<_, Infallible>(()) }).unwrap();
        let attachment = Attachment::new(&replacement, executor.clone()).unwrap();
        host.install(1, Some(attachment));
        assert!(!host.accepts(0));
        assert!(host.accepts(1));
        assert!(!task.is_pending());
        assert!(next.is_pending());
        assert_eq!(executor.0.lock().unwrap().len(), 2);
        host.shutdown();
        host.activate();
        assert!(!host.accepts(1));
        assert!(!next.is_pending());
        assert!(matches!(
            next.submit(async { Ok::<_, Infallible>(()) }),
            Err(BackgroundTaskError::Closed)
        ));
        let jobs = std::mem::take(&mut *executor.0.lock().unwrap());
        assert!(jobs.into_iter().all(|job| job.now_or_never().is_some()));
    }
}
