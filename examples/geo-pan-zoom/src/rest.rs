//! A timer that runs a job once the cursor rests, such as a prefetch of the tiles that zooming at
//! the cursor would show, without rebuilding the scene.

/// How long the cursor stays still before it rests.
const REST_MS: u64 = 150;

/// The job that runs when the cursor rests.
type Job = Box<dyn FnOnce() + Send>;

/// Runs the last move's job once the cursor stops moving for 150 ms: once a rest, however many
/// moves came before it. On native targets, a thread waits for the rests and runs the jobs.
#[derive(Clone)]
pub struct RestTimer {
    #[cfg(not(target_arch = "wasm32"))]
    shared: std::sync::Arc<(std::sync::Mutex<Rest>, std::sync::Condvar)>,
    /// The moves so far, so that a browser timer runs only the last move's job.
    #[cfg(target_arch = "wasm32")]
    moves: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// The rest that a native timer waits for: when it starts, and the job it runs then.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Rest {
    at: Option<std::time::Instant>,
    job: Option<Job>,
}

impl RestTimer {
    pub fn new() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                Self { moves: Default::default() }
            } else {
                let shared: std::sync::Arc<(std::sync::Mutex<Rest>, std::sync::Condvar)> =
                    Default::default();
                let waiting = shared.clone();
                std::thread::Builder::new()
                    .name("cursor-rest".to_string())
                    .spawn(move || run_rests(&waiting))
                    .expect("the cursor rest thread starts");
                Self { shared }
            }
        }
    }

    /// Start the rest over: `job` runs when the cursor next rests, in place of the job that
    /// waited.
    pub fn restart(&self, job: impl FnOnce() + Send + 'static) {
        let job: Job = Box::new(job);
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                use std::sync::atomic::Ordering;
                use wasm_bindgen::{closure::Closure, JsCast};
                let generation = self.moves.fetch_add(1, Ordering::AcqRel) + 1;
                let moves = self.moves.clone();
                let fire = Closure::once_into_js(move || {
                    if moves.load(Ordering::Acquire) == generation {
                        job();
                    }
                });
                if let Some(window) = web_sys::window() {
                    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                        fire.unchecked_ref(),
                        REST_MS as i32,
                    );
                }
            } else {
                let (rest, changed) = &*self.shared;
                let mut rest = rest.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                rest.at = Some(std::time::Instant::now() + std::time::Duration::from_millis(REST_MS));
                rest.job = Some(job);
                changed.notify_one();
            }
        }
    }
}

impl Default for RestTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Wait for each rest, a deadline that passes without a newer move, and run its job outside the
/// lock.
#[cfg(not(target_arch = "wasm32"))]
fn run_rests(shared: &(std::sync::Mutex<Rest>, std::sync::Condvar)) {
    use std::sync::PoisonError;
    let (rest, changed) = shared;
    let mut next = rest.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        match next.at {
            None => next = changed.wait(next).unwrap_or_else(PoisonError::into_inner),
            Some(at) => {
                let now = std::time::Instant::now();
                if now >= at {
                    next.at = None;
                    let job = next.job.take();
                    drop(next);
                    if let Some(job) = job {
                        job();
                    }
                    next = rest.lock().unwrap_or_else(PoisonError::into_inner);
                } else {
                    next = changed
                        .wait_timeout(next, at - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            }
        }
    }
}
