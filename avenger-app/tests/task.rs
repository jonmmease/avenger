//! Background tasks run their jobs on the app's runtime, keep the latest result, and wake the
//! host with a rebuild when a request finishes or fails.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use async_trait::async_trait;
use avenger_app::app::{AvengerApp, BuiltScene, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_app::task::{BoxError, Task};
use avenger_app::waker::Refresh;
use avenger_scenegraph::scene_graph::SceneGraph;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
struct State {
    task: Task<u32, &'static str>,
    /// The latest result's key, as each build saw it.
    built: Vec<Option<u32>>,
}

/// Reads the task's latest result, as an app's scene builder does.
struct Builder;

#[async_trait]
impl SceneGraphBuilder<State> for Builder {
    async fn build(&self, state: &mut State) -> Result<BuiltScene, AvengerAppError> {
        let latest = state.task.latest().map(|done| done.key);
        state.built.push(latest);
        Ok(SceneGraph {
            marks: Vec::new(),
            width: 10.0,
            height: 10.0,
            origin: [0.0; 2],
        }
        .into())
    }
}

fn runtime(workers: usize) -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .build()
        .unwrap()
}

/// An app without a runtime for its tasks, built on `runtime`.
fn bare(runtime: &Runtime) -> AvengerApp<State> {
    runtime
        .block_on(AvengerApp::try_new(
            State::default(),
            Arc::new(Builder),
            Vec::new(),
            avenger_typst_label::LabelEngine::new(Default::default()),
        ))
        .unwrap()
}

/// A channel that hears each time the app's waker notifies the host.
fn listen(app: &AvengerApp<State>) -> mpsc::Receiver<()> {
    let (notify, notified) = mpsc::channel();
    app.waker().set_notify(move || {
        let _ = notify.send(());
    });
    notified
}

/// An app on `runtime` with its state's task attached, and a channel that hears each time the
/// app's waker notifies the host.
fn app(
    runtime: &Runtime,
) -> (
    AvengerApp<State>,
    Task<u32, &'static str>,
    mpsc::Receiver<()>,
) {
    let mut app = bare(runtime)
        .with_runtime(runtime.handle().clone())
        .unwrap();
    let task = app.app_state_mut().task.clone();
    let app = app.with_task(&task);
    let notified = listen(&app);
    (app, task, notified)
}

/// Wait for the host to hear from the app's waker, and take the rebuild that it asks for.
fn rebuilt(app: &AvengerApp<State>, notified: &mpsc::Receiver<()>) {
    notified.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(app.waker().take(), Some(Refresh::Rebuild));
}

/// Sets its flag when it drops, as a job's future does when its request is canceled.
struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A request's result becomes the latest, the host hears one rebuild, and the scene that the
/// refresh builds reads it. A request for the pending or the latest key runs nothing.
#[test]
fn a_finished_request_is_the_latest_and_rebuilds_the_scene() {
    let runtime = runtime(2);
    let (mut app, task, notified) = app(&runtime);
    let (release, gate) = oneshot::channel::<()>();
    task.request(1, |_| async move {
        gate.await.unwrap();
        Ok::<_, BoxError>("one")
    });
    let ran = Arc::new(AtomicBool::new(false));
    let again = ran.clone();
    task.request(1, move |_| async move {
        again.store(true, Ordering::SeqCst);
        Ok::<_, BoxError>("again")
    });
    assert_eq!(task.pending(), Some(1));
    assert!(task.latest().is_none());

    release.send(()).unwrap();
    notified.recv_timeout(TIMEOUT).unwrap();
    let latest = task.latest().unwrap();
    assert_eq!((latest.key, latest.value), (1, "one"));
    assert_eq!(task.pending(), None);
    assert!(notified.try_recv().is_err());

    let refreshed = runtime.block_on(app.refresh()).unwrap();
    assert!(refreshed.scene.is_some());
    assert_eq!(app.app_state_mut().built.last(), Some(&Some(1)));

    let again = ran.clone();
    task.request(1, move |_| async move {
        again.store(true, Ordering::SeqCst);
        Ok::<_, BoxError>("again")
    });
    assert_eq!(task.pending(), None);
    assert!(!ran.load(Ordering::SeqCst));
}

/// A new key cancels the running job and drops a waiting one. A replaced job that finishes
/// anyway never becomes the latest, and the latest result stays until the new request finishes.
#[test]
fn a_new_key_replaces_the_pending_request() {
    // One worker runs the jobs in turn, so the replaced job finishes before the next one starts.
    let runtime = runtime(1);
    let (app, task, notified) = app(&runtime);

    // Requests wait while the task is unattached, and a replaced one drops its job unrun.
    let waiting = Task::<u32, &'static str>::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Dropped(dropped.clone());
    waiting.request(1, move |_| async move {
        drop(guard);
        Ok::<_, BoxError>("one")
    });
    waiting.request(2, |_| async { Ok::<_, BoxError>("two") });
    assert!(dropped.load(Ordering::SeqCst));
    let app = app.with_task(&waiting);
    rebuilt(&app, &notified);
    assert_eq!(waiting.latest().unwrap().key, 2);

    // A job that ignores its cancellation finishes after a newer request replaced it.
    let (started, job_started) = mpsc::channel();
    let (release, gate) = mpsc::channel::<()>();
    task.request(1, move |_| async move {
        started.send(()).unwrap();
        gate.recv().unwrap();
        Ok::<_, BoxError>("one")
    });
    job_started.recv_timeout(TIMEOUT).unwrap();
    let (started, job_started) = mpsc::channel();
    let (release_two, gate_two) = oneshot::channel::<()>();
    task.request(2, move |_| async move {
        started.send(()).unwrap();
        gate_two.await.unwrap();
        Ok::<_, BoxError>("two")
    });
    release.send(()).unwrap();
    job_started.recv_timeout(TIMEOUT).unwrap();
    assert!(task.latest().is_none());
    assert!(notified.try_recv().is_err());
    release_two.send(()).unwrap();
    rebuilt(&app, &notified);
    assert_eq!(task.latest().unwrap().key, 2);

    // A running job that awaits drops when a newer request replaces it.
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Dropped(dropped.clone());
    let (started, job_started) = mpsc::channel();
    task.request(3, move |_| async move {
        let _guard = guard;
        started.send(()).unwrap();
        std::future::pending::<()>().await;
        Ok::<_, BoxError>("three")
    });
    job_started.recv_timeout(TIMEOUT).unwrap();
    let (release, gate) = oneshot::channel::<()>();
    task.request(4, |_| async move {
        gate.await.unwrap();
        Ok::<_, BoxError>("four")
    });
    assert_eq!(task.latest().unwrap().key, 2);
    release.send(()).unwrap();
    rebuilt(&app, &notified);
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(task.latest().unwrap().key, 4);
}

/// A job's error, or its panic, is the task's error until the next request, and the latest
/// result stays.
#[test]
fn a_failed_request_keeps_the_latest_result() {
    let runtime = runtime(2);
    let (app, task, notified) = app(&runtime);
    task.request(1, |_| async { Ok::<_, BoxError>("one") });
    rebuilt(&app, &notified);

    task.request(2, |_| async { Err::<&str, _>("no rows") });
    rebuilt(&app, &notified);
    assert_eq!(task.error().unwrap().to_string(), "no rows");
    assert_eq!(task.latest().unwrap().key, 1);

    task.request::<_, _, BoxError>(3, |_| async { panic!("out of range") });
    rebuilt(&app, &notified);
    assert!(task.error().unwrap().to_string().contains("out of range"));
    assert_eq!((task.latest().unwrap().key, task.pending()), (1, None));

    task.request(1, |_| async { Ok::<_, BoxError>("one") });
    assert!(task.error().is_none());
}

/// An app takes only a multi-thread runtime, and a task that it attaches before it has one runs
/// its requests once the runtime arrives.
#[test]
fn attached_tasks_wait_for_a_multi_thread_runtime() {
    let current = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert!(bare(&current)
        .with_runtime(current.handle().clone())
        .is_err());

    let runtime = runtime(2);
    let mut app = bare(&runtime);
    let task = app.app_state_mut().task.clone();
    let app = app.with_task(&task);
    let notified = listen(&app);
    task.request(1, |_| async { Ok::<_, BoxError>("one") });
    assert_eq!((task.pending(), task.latest().is_none()), (Some(1), true));
    let app = app.with_runtime(runtime.handle().clone()).unwrap();
    rebuilt(&app, &notified);
    assert_eq!(task.latest().unwrap().key, 1);
}
