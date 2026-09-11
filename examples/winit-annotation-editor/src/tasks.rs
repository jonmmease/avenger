//! Platform tasks for preparing replacement samples without blocking input.
use std::future::Future;

// Native jobs move to Tokio workers. Browser jobs stay on the JS event loop.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

#[cfg(not(target_arch = "wasm32"))]
pub use tokio::time::sleep;
#[cfg(not(target_arch = "wasm32"))]
pub type Job = tokio::task::JoinHandle<()>;

#[cfg(not(target_arch = "wasm32"))]
pub struct Executor(pub tokio::runtime::Handle);

#[cfg(not(target_arch = "wasm32"))]
impl Executor {
    pub fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) -> Job {
        self.0.spawn(future)
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{sleep, Executor, Job};

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use futures::future::{AbortHandle, Abortable};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    pub struct Executor;

    pub struct Job {
        abort: AbortHandle,
        finished: Arc<AtomicBool>,
    }

    impl Job {
        pub fn is_finished(&self) -> bool {
            self.finished.load(Ordering::Acquire)
        }

        pub fn abort(&self) {
            self.abort.abort();
        }
    }

    impl Executor {
        pub fn spawn(&self, future: impl Future<Output = ()> + 'static) -> Job {
            let (abort, registration) = AbortHandle::new_pair();
            let finished = Arc::new(AtomicBool::new(false));
            let done = finished.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = Abortable::new(future, registration).await;
                done.store(true, Ordering::Release);
            });
            Job { abort, finished }
        }
    }

    pub async fn sleep(duration: std::time::Duration) {
        gloo_timers::future::TimeoutFuture::new(duration.as_millis().min(i32::MAX as u128) as u32)
            .await;
    }
}
