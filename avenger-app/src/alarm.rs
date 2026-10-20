//! The app's timer, which wakes the host when the app's next wake is due.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use avenger_common::time::Instant;

use crate::waker::{AppWaker, Refresh};

/// Requests [`Refresh::Wakes`] on the app's waker at the time that it's set for: on a thread of
/// its own on native targets, and with the browser's `setTimeout`. It's set for one time, the
/// earliest that it was set for since it last rang, so a deadline that moves later rings at the
/// old time, and the app sets it again then.
#[derive(Clone)]
pub(crate) struct Alarm {
    shared: Arc<Shared>,
}

struct Shared {
    waker: AppWaker,
    /// When the alarm rings, while it's set.
    at: Mutex<Option<Instant>>,
    #[cfg(not(target_arch = "wasm32"))]
    changed: std::sync::Condvar,
    #[cfg(not(target_arch = "wasm32"))]
    thread: std::sync::OnceLock<()>,
}

impl Alarm {
    pub(crate) fn new(waker: AppWaker) -> Self {
        Self {
            shared: Arc::new(Shared {
                waker,
                at: Mutex::new(None),
                #[cfg(not(target_arch = "wasm32"))]
                changed: std::sync::Condvar::new(),
                #[cfg(not(target_arch = "wasm32"))]
                thread: std::sync::OnceLock::new(),
            }),
        }
    }

    /// Ring at `at`, unless the alarm is set to ring earlier.
    pub(crate) fn set(&self, at: Instant) {
        let mut set = lock(&self.shared.at);
        if set.is_some_and(|set| set <= at) {
            return;
        }
        *set = Some(at);
        drop(set);
        self.start(at);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start(&self, _at: Instant) {
        self.shared.changed.notify_one();
        self.shared.thread.get_or_init(|| {
            let shared = self.shared.clone();
            let _ = std::thread::Builder::new()
                .name("avenger-app-alarm".to_string())
                .spawn(move || ring(&shared));
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn start(&self, at: Instant) {
        use wasm_bindgen::{closure::Closure, JsCast};
        let shared = self.shared.clone();
        // A timeout whose time an earlier one replaced finds the alarm set for another time.
        let fire = Closure::once_into_js(move || {
            let mut set = lock(&shared.at);
            if *set == Some(at) {
                *set = None;
                drop(set);
                shared.waker.request(Refresh::Wakes);
            }
        });
        let delay = at.saturating_duration_since(Instant::now()).as_millis();
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                fire.unchecked_ref(),
                delay.min(i32::MAX as u128) as i32,
            );
        }
    }
}

/// Ring the alarm each time that it's set for passes, outside its lock.
#[cfg(not(target_arch = "wasm32"))]
fn ring(shared: &Shared) {
    let mut set = lock(&shared.at);
    loop {
        match *set {
            None => {
                set = shared
                    .changed
                    .wait(set)
                    .unwrap_or_else(PoisonError::into_inner)
            }
            Some(at) => {
                let now = Instant::now();
                if now >= at {
                    *set = None;
                    drop(set);
                    shared.waker.request(Refresh::Wakes);
                    set = lock(&shared.at);
                } else {
                    set = shared
                        .changed
                        .wait_timeout(set, at - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
