//! Where the work of the charts runs, and a host's other work, such as queries. Natively, work
//! runs on a multithreaded tokio runtime and chart cells evaluate on its blocking threads,
//! beside the window. A wasm page has one thread, so on
//! the web each piece of work runs to its end when it starts, on a tokio runtime of that thread,
//! which DataFusion's tasks need.

use std::future::Future;

/// The runtime of a host's work, such as queries, and of the evaluations and the apps of its charts.
pub struct Exec {
    runtime: tokio::runtime::Runtime,
}

/// A piece of work that runs in the background, which newer work can stop. On the web, the work
/// has ended by the time its task returns.
pub struct Task {
    /// The signal that stops the work, which drops its future.
    #[cfg(not(target_arch = "wasm32"))]
    cancel: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl Task {
    /// Stops the work, when it still runs.
    pub fn abort(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(cancel) = self.cancel.lock().ok().and_then(|mut cancel| cancel.take()) {
            let _ = cancel.send(());
        }
    }
}

impl Exec {
    pub fn new() -> std::io::Result<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        // The work needs neither timers nor I/O, whose drivers the web does not have.
        #[cfg(target_arch = "wasm32")]
        let runtime = tokio::runtime::Builder::new_current_thread().build()?;
        Ok(Self { runtime })
    }

    /// Runs the future that `work` makes in the background: natively on a blocking thread, which
    /// runs a future that holds the syntax of a query and so cannot move between threads, and to
    /// its end now on the web. `abort` drops the future at its next point of waiting.
    pub fn spawn<F, W>(&self, work: W) -> Task
    where
        W: FnOnce() -> F + Send + 'static,
        F: Future<Output = ()> + 'static,
    {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
            let handle = self.runtime.handle().clone();
            drop(self.runtime.spawn_blocking(move || {
                handle.block_on(async move {
                    // A dropped task's sender closes the channel, which stops nothing: only a
                    // sent signal stops the work.
                    tokio::select! {
                        _ = work() => {}
                        Ok(()) = cancelled => {}
                    }
                })
            }));
            Task {
                cancel: std::sync::Mutex::new(Some(cancel)),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.runtime.block_on(work());
            Task {}
        }
    }

    /// Runs the blocking `work`: on a blocking thread natively, and now on the web.
    pub fn spawn_blocking(&self, work: impl FnOnce() + Send + 'static) {
        #[cfg(not(target_arch = "wasm32"))]
        drop(self.runtime.spawn_blocking(work));
        #[cfg(target_arch = "wasm32")]
        work();
    }

    /// The handle of the multithreaded runtime, on which a chart app runs its tasks natively. On
    /// the web, a chart app runs its tasks on the page's thread by itself.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    /// Runs `future` to its end on this thread.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}
