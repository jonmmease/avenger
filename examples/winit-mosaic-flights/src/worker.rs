use crate::{
    dataflow::{Engine, Evaluation},
    selection::Selections,
};
use avenger_eventstream::{
    runtime::{RuntimeWakeEvent, RuntimeWakeKey},
    window::WindowEvent,
};
use avenger_winit_wgpu::WinitWgpuEvent;
use std::sync::{Arc, Mutex};
use winit::event_loop::EventLoopProxy;

type Completion = (u64, Result<Evaluation, String>);
pub struct Worker {
    engine: Arc<Engine>,
    pub proxy: Mutex<Option<EventLoopProxy<WinitWgpuEvent>>>,
    completed: Mutex<[Option<Completion>; 2]>,
    tasks: Mutex<[Option<tokio::task::JoinHandle<()>>; 2]>,
    runtime: tokio::runtime::Handle,
}
impl Worker {
    pub fn new(engine: Arc<Engine>) -> Arc<Self> {
        Arc::new(Self {
            engine,
            proxy: Mutex::new(None),
            completed: Mutex::new([None, None]),
            tasks: Mutex::new([None, None]),
            runtime: tokio::runtime::Handle::current(),
        })
    }
    pub fn submit(
        self: &Arc<Self>,
        generation: u64,
        selections: Selections,
        focus: Option<usize>,
        warm: bool,
    ) {
        self.cancel(warm);
        let worker = self.clone();
        self.tasks.lock().unwrap()[usize::from(warm)] = Some(self.runtime.spawn(async move {
            let result = worker
                .engine
                .query(&selections, focus, warm)
                .await
                .map_err(|e| format!("{e:#}"));
            let mut completed = worker.completed.lock().unwrap();
            let slot = &mut completed[usize::from(warm)];
            if slot.as_ref().is_none_or(|(prior, _)| *prior <= generation) {
                *slot = Some((generation, result));
            }
            drop(completed);
            if let Some(proxy) = worker.proxy.lock().unwrap().as_ref() {
                let _ = proxy.send_event(WinitWgpuEvent::App(WindowEvent::RuntimeWake(
                    RuntimeWakeEvent {
                        key: completion_key(warm),
                        generation,
                    },
                )));
            }
        }));
    }
    pub fn take(&self, warm: bool) -> Option<Completion> {
        self.completed.lock().unwrap()[usize::from(warm)].take()
    }
    pub fn cancel(&self, warm: bool) {
        if let Some(task) = self.tasks.lock().unwrap()[usize::from(warm)].take() {
            task.abort();
        }
        self.take(warm);
    }
    pub fn shutdown(&self) {
        self.cancel(false);
        self.cancel(true);
    }
}
pub fn completion_key(warm: bool) -> RuntimeWakeKey {
    RuntimeWakeKey::new("mosaic-flights", 0, if warm { "warm-up" } else { "query" })
}
