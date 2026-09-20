use crate::{
    dataflow::{Engine, Request},
    layout::DashboardLayout,
    scene::Rendered,
};
use anyhow::Result;
use avenger_eventstream::{
    runtime::{RuntimeWakeEvent, RuntimeWakeKey},
    window::WindowEvent,
};
use avenger_winit_wgpu::WinitWgpuEvent;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use winit::event_loop::EventLoopProxy;

pub type Completion = (u64, Result<Arc<Rendered>, String>);
pub struct Worker {
    engine: tokio::sync::Mutex<Engine>,
    latest: AtomicU64,
    pub proxy: Mutex<Option<EventLoopProxy<WinitWgpuEvent>>>,
    pub completed: Mutex<Option<Completion>>,
    foreground: Mutex<Option<tokio::task::JoinHandle<()>>>,
    warmup: Mutex<Option<tokio::task::JoinHandle<()>>>,
    runtime: tokio::runtime::Handle,
}
impl Worker {
    pub fn new(engine: Engine) -> Arc<Self> {
        Arc::new(Self {
            engine: tokio::sync::Mutex::new(engine),
            latest: AtomicU64::new(0),
            proxy: Mutex::new(None),
            completed: Mutex::new(None),
            foreground: Mutex::new(None),
            warmup: Mutex::new(None),
            runtime: tokio::runtime::Handle::current(),
        })
    }
    pub fn submit(
        self: &Arc<Self>,
        generation: u64,
        request: Request,
        layout: DashboardLayout,
        previous: Arc<Rendered>,
        label: &'static str,
        warm: bool,
    ) {
        if !warm {
            self.latest.store(generation, Ordering::SeqCst);
        }
        let slot = if warm { &self.warmup } else { &self.foreground };
        let mut slot = slot.lock().unwrap();
        if let Some(task) = slot.take() {
            task.abort();
        }
        let worker = self.clone();
        *slot = Some(self.runtime.spawn(async move {
            let result: Result<Option<Arc<Rendered>>> = async {
                let (job, metadata, diagnostics) = {
                    let mut engine = worker.engine.lock().await;
                    (
                        engine.job(&request, warm).await?,
                        engine.metadata.clone(),
                        engine.config.diagnostics,
                    )
                };
                let Some(job) = job else {
                    return Ok(None);
                };
                let evaluation = job.run().await?;
                if diagnostics {
                    evaluation.print(label);
                }
                if warm || worker.latest.load(Ordering::SeqCst) != generation {
                    return Ok(None);
                }
                Ok(Some(Arc::new(Rendered::new(
                    &evaluation,
                    &metadata,
                    layout,
                    Some(&previous),
                    request.selections.clone(),
                )?)))
            }
            .await;
            if warm {
                if let Err(error) = result {
                    eprintln!("Warm-up failed: {error:#}");
                }
                return;
            }
            let result = match result {
                Ok(Some(result)) => Ok(result),
                Ok(None) => return,
                Err(e) => Err(format!("{e:#}")),
            };
            if worker.latest.load(Ordering::SeqCst) != generation {
                return;
            }
            let mut completed = worker.completed.lock().unwrap();
            if completed
                .as_ref()
                .is_none_or(|(prior, _)| *prior <= generation)
            {
                *completed = Some((generation, result));
            }
            drop(completed);
            if let Some(proxy) = worker.proxy.lock().unwrap().as_ref() {
                let _ = proxy.send_event(WinitWgpuEvent::App(WindowEvent::RuntimeWake(
                    RuntimeWakeEvent {
                        key: completion_key(),
                        generation,
                    },
                )));
            }
        }));
    }
    pub fn cancel_warmup(&self) {
        if let Some(task) = self.warmup.lock().unwrap().take() {
            task.abort();
        }
    }
    pub fn shutdown(&self) {
        self.latest.store(u64::MAX, Ordering::SeqCst);
        if let Some(task) = self.foreground.lock().unwrap().take() {
            task.abort();
        }
        self.cancel_warmup();
        self.completed.lock().unwrap().take();
    }
}
pub fn completion_key() -> RuntimeWakeKey {
    RuntimeWakeKey::new("flights", 0, "query-result")
}
