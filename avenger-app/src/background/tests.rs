use super::*;
use host::{Attachment, Executor};
use std::{convert::Infallible, sync::atomic::AtomicUsize};

#[derive(Default)]
struct ManualExecutor {
    jobs: Mutex<Vec<BoxFuture<'static, ()>>>,
    wakes: Mutex<Vec<RuntimeWakeEvent>>,
}

impl Executor for ManualExecutor {
    fn spawn(&self, future: BoxFuture<'static, ()>) {
        self.jobs.lock().unwrap().push(future);
    }

    fn wake(&self, event: RuntimeWakeEvent) {
        self.wakes.lock().unwrap().push(event);
    }
}

impl ManualExecutor {
    fn run(&self) {
        let jobs = std::mem::take(&mut *self.jobs.lock().unwrap());
        for job in jobs {
            assert!(job.now_or_never().is_some());
        }
    }

    fn pop_wake(&self) -> RuntimeWakeEvent {
        self.wakes.lock().unwrap().pop().expect("completion wake")
    }
}

fn attached() -> (BackgroundTasks, Arc<ManualExecutor>, Attachment) {
    let tasks = BackgroundTasks::new();
    let executor = Arc::new(ManualExecutor::default());
    let attachment = Attachment::new(&tasks, executor.clone()).unwrap();
    attachment.activate();
    (tasks, executor, attachment)
}

#[test]
fn non_clone_results_are_shared_but_delivery_is_local_to_state_clones() {
    struct Value(u32);
    let (tasks, executor, _attachment) = attached();
    let mut task = tasks.task::<Value>();
    task.submit(async { Ok::<_, Infallible>(Value(7)) })
        .unwrap();
    let mut candidate = task.clone();
    let mut unrelated = tasks.task::<Value>();
    executor.run();
    let wake = executor.pop_wake();
    assert!(task.is_pending());
    assert!(unrelated.handle_wake(&wake).is_none());
    let result = candidate.handle_wake(&wake).unwrap().unwrap();
    assert_eq!(result.0, 7);
    assert!(!candidate.is_pending());
    assert!(candidate.handle_wake(&wake).is_none());
    assert!(task.is_pending());
    let retried = task.handle_wake(&wake).unwrap().unwrap();
    assert!(Arc::ptr_eq(&result, &retried));
    assert!(!task.is_pending());
}

#[test]
fn replacement_rejects_late_publication_and_already_queued_events() {
    let (tasks, executor, _attachment) = attached();
    let mut task = tasks.task::<u32>();
    task.submit(async { Ok::<_, Infallible>(1) }).unwrap();
    let mut old_state = task.clone();
    executor.run();
    let old_wake = executor.pop_wake();
    let mut late = Publisher {
        slot: Arc::downgrade(&task.slot),
        generation: old_wake.generation,
        finished: false,
    };
    task.submit(async { Ok::<_, Infallible>(2) }).unwrap();
    late.finish(Ok(Arc::new(99)));
    assert!(executor.wakes.lock().unwrap().is_empty());
    assert!(task.handle_wake(&old_wake).is_none());
    assert!(old_state.handle_wake(&old_wake).is_none());
    executor.run();
    let new_wake = executor.pop_wake();
    assert!(old_state.handle_wake(&new_wake).is_none());
    assert_eq!(*task.handle_wake(&new_wake).unwrap().unwrap(), 2);
}

#[test]
fn cancel_invalidates_ready_values_and_queued_wakes() {
    let (tasks, executor, _attachment) = attached();
    let mut task = tasks.task::<u32>();
    task.submit(async { Ok::<_, Infallible>(1) }).unwrap();
    executor.run();
    let wake = executor.pop_wake();
    let mut clone = task.clone();
    task.cancel();
    assert!(!task.is_pending());
    assert!(!clone.is_pending());
    assert!(clone.handle_wake(&wake).is_none());
    assert!(task.handle_wake(&wake).is_none());
}

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn pending(
    counter: &Arc<AtomicUsize>,
) -> impl Future<Output = Result<(), Infallible>> + Send + 'static {
    let probe = DropProbe(counter.clone());
    async move {
        let _probe = probe;
        futures::future::pending().await
    }
}

#[test]
fn only_the_latest_queued_request_starts_on_activation() {
    let tasks = BackgroundTasks::new();
    let executor = Arc::new(ManualExecutor::default());
    let drops = Arc::new(AtomicUsize::new(0));
    let mut task = tasks.task::<()>();
    task.submit(pending(&drops)).unwrap();
    task.submit(pending(&drops)).unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let attachment = Attachment::new(&tasks, executor.clone()).unwrap();
    assert!(executor.jobs.lock().unwrap().is_empty());
    attachment.activate();
    attachment.activate();
    assert_eq!(executor.jobs.lock().unwrap().len(), 1);
    task.cancel();
    executor.run();
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert!(executor.wakes.lock().unwrap().is_empty());
}

#[test]
fn independent_tasks_keep_their_own_consumer_interest() {
    let (tasks, executor, _attachment) = attached();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut hover = tasks.task::<()>();
    let mut foreground = tasks.task::<()>();
    let (sender, receiver) = futures::channel::oneshot::channel();
    let shared = receiver.map(|r| r.unwrap()).shared();
    let consumer = |shared: futures::future::Shared<_>| {
        let probe = DropProbe(drops.clone());
        async move {
            let _probe = probe;
            shared.await;
            Ok::<_, Infallible>(())
        }
    };
    hover.submit(consumer(shared.clone())).unwrap();
    foreground.submit(consumer(shared)).unwrap();
    let mut jobs = std::mem::take(&mut *executor.jobs.lock().unwrap());
    assert!(jobs[0].as_mut().now_or_never().is_none());
    assert!(jobs[1].as_mut().now_or_never().is_none());
    hover.cancel();
    assert!(jobs[0].as_mut().now_or_never().is_some());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(foreground.is_pending());
    sender.send(()).unwrap();
    assert!(jobs[1].as_mut().now_or_never().is_some());
    let wake = executor.pop_wake();
    assert!(foreground.handle_wake(&wake).unwrap().is_ok());
    assert!(hover.handle_wake(&wake).is_none());
    assert!(executor.wakes.lock().unwrap().is_empty());
}

#[test]
fn last_handle_drop_cancels_without_a_strong_publisher_cycle() {
    let (tasks, executor, _attachment) = attached();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut task = tasks.task::<()>();
    task.submit(pending(&drops)).unwrap();
    let clone = task.clone();
    let weak = Arc::downgrade(&task.slot);
    drop(task);
    assert!(weak.upgrade().is_some());
    drop(clone);
    assert!(weak.upgrade().is_none());
    executor.run();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(executor.wakes.lock().unwrap().is_empty());
}

#[test]
fn shutdown_closes_all_handles_and_releases_queued_and_running_requests() {
    let (tasks, executor, attachment) = attached();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut task = tasks.task::<()>();
    task.submit(pending(&drops)).unwrap();
    attachment.shutdown();
    attachment.shutdown();
    assert!(!task.is_pending());
    assert!(matches!(
        task.submit(async { Ok::<_, Infallible>(()) }),
        Err(BackgroundTaskError::Closed)
    ));
    executor.run();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(executor.wakes.lock().unwrap().is_empty());

    let queued_group = BackgroundTasks::new();
    let mut queued = queued_group.task::<()>();
    queued.submit(pending(&drops)).unwrap();
    drop(queued_group);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert!(!queued.is_pending());
}

#[test]
fn failures_preserve_sources_and_report_executor_interruption_and_panics() {
    let (tasks, executor, _attachment) = attached();
    let mut task = tasks.task::<()>();
    task.submit(async { Err(std::io::Error::other("query failed")) })
        .unwrap();
    executor.run();
    let error = task.handle_wake(&executor.pop_wake()).unwrap().unwrap_err();
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .to_string(),
        "query failed"
    );

    task.submit(async {
        panic!("poll panic");
        #[allow(unreachable_code)]
        Ok::<_, Infallible>(())
    })
    .unwrap();
    executor.run();
    assert!(
        matches!(task.handle_wake(&executor.pop_wake()), Some(Err(BackgroundTaskError::Panicked(message))) if message == "poll panic")
    );

    task.submit(async { Ok::<_, Infallible>(()) }).unwrap();
    // An executor can shut down before ever polling an accepted future.
    let jobs = std::mem::take(&mut *executor.jobs.lock().unwrap());
    drop(jobs);
    assert!(matches!(
        task.handle_wake(&executor.pop_wake()),
        Some(Err(BackgroundTaskError::Interrupted))
    ));
    assert!(!task.is_pending());
}

#[test]
fn user_value_destructors_run_outside_task_and_group_locks() {
    struct Reenter {
        tasks: BackgroundTasks,
        other: BackgroundTask<()>,
    }
    impl Drop for Reenter {
        fn drop(&mut self) {
            self.other.cancel();
            let _ = self.tasks.task::<()>();
        }
    }
    let (tasks, executor, _attachment) = attached();
    let mut task = tasks.task::<Reenter>();
    let value = Reenter {
        tasks: tasks.clone(),
        other: tasks.task(),
    };
    task.submit(async { Ok::<_, Infallible>(value) }).unwrap();
    executor.run();
    task.cancel();
    assert!(!task.is_pending());
}
