//! Waking an app's host from background work.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// What a refresh asks of the host, from least to most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refresh {
    /// Draw the scene again, as when an image that it shows arrives.
    Redraw = 1,
    /// Build the scene again and draw it, as when background work changes the app's state.
    Rebuild = 2,
}

/// Wakes an app's host from background work, such as fetches and timers. A request raises the
/// pending refresh, and the host hears only when it rises, so a burst of requests becomes one
/// event. The host takes the pending refresh with [`crate::app::AvengerApp::refresh`]. Clones
/// share the pending refresh.
#[derive(Clone, Default)]
pub struct AppWaker {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    /// The pending refresh: 0 for none, else a [`Refresh`].
    level: AtomicU8,
    notify: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl AppWaker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the host to refresh at `level`, or at a higher pending level.
    pub fn request(&self, level: Refresh) {
        let previous = self.shared.level.fetch_max(level as u8, Ordering::AcqRel);
        if previous < level as u8 {
            let notify = self
                .shared
                .notify
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(notify) = notify {
                notify();
            }
        }
    }

    /// Take the pending refresh, leaving none.
    pub fn take(&self) -> Option<Refresh> {
        match self.shared.level.swap(0, Ordering::AcqRel) {
            0 => None,
            1 => Some(Refresh::Redraw),
            _ => Some(Refresh::Rebuild),
        }
    }

    /// Set the host's notifier, which runs when the pending refresh rises, on the thread that
    /// requests it. This replaces the previous notifier.
    pub fn set_notify(&self, notify: impl Fn() + Send + Sync + 'static) {
        *self
            .shared
            .notify
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(notify));
    }
}

impl std::fmt::Debug for AppWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppWaker")
            .field("level", &self.shared.level.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}
