use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use avenger_app::app::AvengerApp;
use avenger_common::time::{Duration, Instant};
use avenger_text::TextEngine;
use avenger_winit_wgpu::{
    HostUpdateInstallOutcome, HostUpdateSender, PreparedHostUpdate, WindowSceneSizing,
};
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Handle;

use crate::{
    make_app,
    state::{Sample, State},
    tasks::{sleep, Executor, Job, MaybeSend},
};

type Feedback = Arc<Mutex<Option<Result<(), String>>>>;

struct Target {
    mark_epoch: Box<dyn Fn(u64) + Send + Sync>,
    submit: Box<dyn Fn(PreparedHostUpdate<State>) -> Result<(), String> + Send + Sync>,
    title: Box<dyn Fn(String) + Send + Sync>,
}

pub struct ReloadCoordinator {
    executor: Executor,
    target: Mutex<Option<Target>>,
    latest: AtomicU64,
    closed: AtomicBool,
    jobs: Mutex<Vec<Job>>,
    slow: bool,
}

impl ReloadCoordinator {
    pub fn new(#[cfg(not(target_arch = "wasm32"))] runtime: Handle, slow: bool) -> Arc<Self> {
        Arc::new(Self {
            #[cfg(not(target_arch = "wasm32"))]
            executor: Executor(runtime),
            #[cfg(target_arch = "wasm32")]
            executor: Executor,
            target: Mutex::new(None),
            latest: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            jobs: Mutex::new(Vec::new()),
            slow,
        })
    }
    pub fn attach(&self, sender: HostUpdateSender<State>) {
        let mark = sender.clone();
        let title = sender.clone();
        *self.target.lock().expect("reload target") = Some(Target {
            mark_epoch: Box::new(move |epoch| mark.mark_request_epoch(epoch)),
            submit: Box::new(move |update| {
                sender.submit(update).map(|_| ()).map_err(|e| e.to_string())
            }),
            title: Box::new(move |value| {
                let _ = title.set_window_title(value);
            }),
        });
    }
    pub fn request(
        self: &Arc<Self>,
        sample: Sample,
        size: [f32; 2],
        engine: TextEngine,
        feedback: Feedback,
    ) -> Result<(), String> {
        let weak = Arc::downgrade(self);
        let slow = self.slow;
        self.start(sample, feedback, move |epoch| async move {
            if slow {
                sleep(Duration::from_millis(if sample == Sample::A {
                    1600
                } else {
                    150
                }))
                .await;
            }
            let mut state = State::new(sample, epoch, engine);
            state.size = size;
            state.reload = weak;
            make_app(state).await.map_err(|e| e.to_string())
        })
    }
    fn start<F, Fut>(
        self: &Arc<Self>,
        sample: Sample,
        feedback: Feedback,
        prepare: F,
    ) -> Result<(), String>
    where
        F: FnOnce(u64) -> Fut,
        Fut: Future<Output = Result<AvengerApp<State>, String>> + MaybeSend + 'static,
    {
        if self.closed.load(Ordering::Acquire) {
            return Err("The window is closing".into());
        }
        let epoch = self.latest.fetch_add(1, Ordering::AcqRel) + 1;
        {
            let target = self.target.lock().expect("reload target");
            let target = target.as_ref().ok_or("No window is attached")?;
            // Advance before preparation starts, so an earlier result cannot
            // install during the gap before this request is ready.
            (target.mark_epoch)(epoch);
            (target.title)(format!(
                "Annotation editor — loading sample {}",
                sample.name()
            ));
        }
        *feedback.lock().expect("load feedback") = None;
        let this = self.clone();
        let preparation = prepare(epoch);
        let job = self.executor.spawn(async move {
            let result = preparation.await;
            if !this.current(epoch) {
                return;
            }
            let outcome = match result {
                Ok(app) => {
                    let (completion, receiver) = std::sync::mpsc::sync_channel(1);
                    let update = PreparedHostUpdate {
                        generation: epoch,
                        request_epoch: epoch,
                        app,
                        render_invalidation_hub: None,
                        image_resource_resolver: None,
                        window_title: Some(format!("Annotation editor — sample {}", sample.name())),
                        window_scene_sizing: WindowSceneSizing::SurfaceFollowsWindow,
                        canvas_frame: None,
                        completion: Some(completion),
                    };
                    let submitted = {
                        let target = this.target.lock().expect("reload target");
                        (target.as_ref().expect("attached target").submit)(update)
                    };
                    match submitted {
                        Err(error) => Err(error),
                        Ok(()) => {
                            let deadline = Instant::now() + Duration::from_secs(10);
                            loop {
                                match receiver.try_recv() {
                                    Ok(HostUpdateInstallOutcome::Installed) => break Ok(()),
                                    Ok(HostUpdateInstallOutcome::Superseded) => return,
                                    Ok(HostUpdateInstallOutcome::Failed(error)) => {
                                        break Err(error)
                                    }
                                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                        break Err("Window closed before installation".into());
                                    }
                                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                                        if Instant::now() >= deadline {
                                            break Err(
                                                "The window did not confirm installation".into()
                                            );
                                        }
                                        sleep(Duration::from_millis(10)).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(error) => Err(error),
            };
            if !this.current(epoch) {
                return;
            }
            if let Err(error) = &outcome {
                if let Some(target) = this.target.lock().expect("reload target").as_ref() {
                    (target.title)(format!("Annotation editor — load failed: {error}"));
                }
            }
            *feedback.lock().expect("load feedback") = Some(outcome);
        });
        let mut jobs = self.jobs.lock().expect("reload jobs");
        jobs.retain(|job| !job.is_finished());
        jobs.push(job);
        Ok(())
    }
    fn current(&self, epoch: u64) -> bool {
        !self.closed.load(Ordering::Acquire) && self.latest.load(Ordering::Acquire) == epoch
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        for job in self.jobs.lock().expect("reload jobs").drain(..) {
            job.abort();
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    type Installed = tokio::sync::mpsc::UnboundedReceiver<(u64, Sample)>;
    fn setup() -> (Arc<ReloadCoordinator>, Installed, Arc<AtomicU64>) {
        let coordinator = ReloadCoordinator::new(Handle::current(), false);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let marked = Arc::new(AtomicU64::new(0));
        let observed = marked.clone();
        *coordinator.target.lock().unwrap() = Some(Target {
            mark_epoch: Box::new(move |epoch| {
                observed.store(epoch, Ordering::Release);
            }),
            submit: Box::new(move |mut update| {
                sender
                    .send((update.generation, update.app.app_state_mut().sample))
                    .unwrap();
                if let Some(completion) = update.completion.take() {
                    completion
                        .send(HostUpdateInstallOutcome::Installed)
                        .unwrap();
                }
                Ok(())
            }),
            title: Box::new(|_| {}),
        });
        (coordinator, receiver, marked)
    }
    async fn finish(coordinator: &ReloadCoordinator) {
        let jobs = std::mem::take(&mut *coordinator.jobs.lock().unwrap());
        for job in jobs {
            job.await.unwrap();
        }
    }
    #[tokio::test]
    async fn request_epoch_advances_before_preparation_and_late_a_cannot_replace_b() {
        let (coordinator, mut installed, marked) = setup();
        let (a_send, a_receive) = tokio::sync::oneshot::channel::<()>();
        let (b_send, b_receive) = tokio::sync::oneshot::channel::<()>();
        let feedback: Feedback = Default::default();
        coordinator
            .start(Sample::A, feedback.clone(), move |epoch| async move {
                a_receive.await.unwrap();
                make_app(State::new(
                    Sample::A,
                    epoch,
                    avenger_text::default_text_engine(),
                ))
                .await
                .map_err(|e| e.to_string())
            })
            .unwrap();
        coordinator
            .start(Sample::B, feedback.clone(), move |epoch| async move {
                b_receive.await.unwrap();
                make_app(State::new(
                    Sample::B,
                    epoch,
                    avenger_text::default_text_engine(),
                ))
                .await
                .map_err(|e| e.to_string())
            })
            .unwrap();
        assert_eq!(marked.load(Ordering::Acquire), 2);
        b_send.send(()).unwrap();
        assert_eq!(installed.recv().await, Some((2, Sample::B)));
        a_send.send(()).unwrap();
        finish(&coordinator).await;
        assert!(installed.try_recv().is_err());
        assert_eq!(*feedback.lock().unwrap(), Some(Ok(())));
    }
    #[tokio::test]
    async fn preparation_failure_is_reported_without_submitting_a_replacement() {
        let (coordinator, mut installed, _) = setup();
        let feedback: Feedback = Default::default();
        coordinator
            .start(Sample::B, feedback.clone(), |_| async {
                Err("Could not prepare sample".into())
            })
            .unwrap();
        finish(&coordinator).await;
        assert!(installed.try_recv().is_err());
        assert_eq!(
            *feedback.lock().unwrap(),
            Some(Err("Could not prepare sample".into()))
        );
    }
    #[tokio::test]
    async fn closing_aborts_preparation_and_rejects_new_requests() {
        let (coordinator, mut installed, _) = setup();
        let feedback: Feedback = Default::default();
        let (send, receive) = tokio::sync::oneshot::channel::<()>();
        coordinator
            .start(Sample::B, feedback.clone(), |_| async move {
                receive.await.unwrap();
                Err("should be cancelled".into())
            })
            .unwrap();
        coordinator.close();
        tokio::task::yield_now().await;
        assert!(send.send(()).is_err());
        assert!(installed.try_recv().is_err());
        assert!(coordinator
            .start(Sample::A, feedback, |_| async { Err("unused".into()) })
            .is_err());
    }
}
