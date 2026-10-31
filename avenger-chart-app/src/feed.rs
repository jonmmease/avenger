//! `ChartFeed`, through which a host sends new charts, the interaction mode, and selections to a
//! chart app.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use avenger_app::waker::{AppWaker, Refresh};
use avenger_chart::{Chart, Contribution};

use crate::state::Mode;

/// A handle through which a host sends new charts, the interaction mode, and selections to a
/// chart app, from any thread, such as when a query answers a settled view. Clones share one
/// slot, which the app takes at its next build, so the charts that arrive faster than the app
/// builds collapse to the newest one.
#[derive(Clone, Default)]
pub struct ChartFeed {
    slot: Arc<Mutex<Slot>>,
}

/// What waits in a feed for the next build of its app.
#[derive(Default)]
pub(crate) struct Waiting {
    pub(crate) chart: Option<Chart>,
    pub(crate) mode: Option<Mode>,
    /// The contributions of selections, in the order of their changes, with `None` for a
    /// selection that the host clears.
    pub(crate) selections: Vec<(String, Option<Vec<Contribution>>)>,
}

#[derive(Default)]
struct Slot {
    waiting: Waiting,
    /// The waker of the app, once a chart app takes the feed.
    waker: Option<AppWaker>,
}

impl ChartFeed {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends `chart`, which the app shows at its next build in place of the chart that it shows
    /// and of a chart that it has not shown.
    pub fn send(&self, chart: Chart) {
        self.wait(|waiting| waiting.chart = Some(chart));
    }

    /// Sets the interaction mode at the next build.
    pub fn set_mode(&self, mode: Mode) {
        self.wait(|waiting| waiting.mode = Some(mode));
    }

    /// Sets the contributions of the selection `name` at the next build, such as to restore a
    /// saved selection. The app draws its brushes and styles its rows, and reports no change.
    pub fn set_selection(&self, name: &str, contributions: Vec<Contribution>) {
        self.wait(|waiting| (waiting.selections).push((name.to_string(), Some(contributions))));
    }

    /// Clears the selection `name` at the next build.
    pub fn clear_selection(&self, name: &str) {
        self.wait(|waiting| waiting.selections.push((name.to_string(), None)));
    }

    /// Changes what waits for the next build, and wakes the app.
    fn wait(&self, change: impl FnOnce(&mut Waiting)) {
        let waker = {
            let mut slot = self.slot();
            change(&mut slot.waiting);
            slot.waker.clone()
        };
        if let Some(waker) = waker {
            waker.request(Refresh::Rebuild);
        }
    }

    /// Binds the feed to the waker of its app, which a change wakes.
    pub(crate) fn bind(&self, waker: AppWaker) {
        let waiting = {
            let mut slot = self.slot();
            slot.waker = Some(waker.clone());
            let waiting = &slot.waiting;
            waiting.chart.is_some() || waiting.mode.is_some() || !waiting.selections.is_empty()
        };
        if waiting {
            waker.request(Refresh::Rebuild);
        }
    }

    /// What waits for the next build.
    pub(crate) fn take(&self) -> Waiting {
        std::mem::take(&mut self.slot().waiting)
    }

    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
