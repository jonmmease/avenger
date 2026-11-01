//! The sources of what a chart app shows next: a `ChartFeed`, through which a host sends new
//! charts, the interaction mode, and selections, and a task whose latest chart the app shows.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use avenger_app::{
    task::{Done, Task},
    waker::{AppWaker, Refresh},
};
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

/// The latest chart of a task, which a chart app shows once, whatever the type of the task's
/// keys.
pub(crate) trait Charts: Send + Sync {
    /// The task's latest chart, when the app has not shown it.
    fn take(&mut self) -> Option<Chart>;
}

/// A task of charts, with the result that the app showed last. Holding the result keeps its
/// allocation, so a newer result is never mistaken for it.
pub(crate) struct TaskCharts<K> {
    pub(crate) task: Task<K, Chart>,
    pub(crate) shown: Option<Arc<Done<K, Chart>>>,
}

impl<K: Clone + PartialEq + Send + Sync + 'static> Charts for TaskCharts<K> {
    fn take(&mut self) -> Option<Chart> {
        let latest = self.task.latest()?;
        if (self.shown.as_ref()).is_some_and(|shown| Arc::ptr_eq(shown, &latest)) {
            return None;
        }
        let chart = latest.value.clone();
        self.shown = Some(latest);
        Some(chart)
    }
}
