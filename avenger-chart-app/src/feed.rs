//! The sources of the charts that a chart app shows next: a `ChartFeed`, through which a host
//! sends new charts, and a task whose latest chart the app shows.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use avenger_app::{
    task::{Done, Task},
    waker::{AppWaker, Refresh},
};
use avenger_chart::Chart;

/// A handle through which a host sends new charts to a chart app, from any thread, such as when
/// a query answers a settled view. Clones share one slot, which holds the newest chart that the
/// app has not shown, so the charts that arrive faster than the app builds collapse to the
/// newest one.
#[derive(Clone, Default)]
pub struct ChartFeed {
    slot: Arc<Mutex<Slot>>,
}

#[derive(Default)]
struct Slot {
    chart: Option<Chart>,
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
        let waker = {
            let mut slot = self.slot();
            slot.chart = Some(chart);
            slot.waker.clone()
        };
        if let Some(waker) = waker {
            waker.request(Refresh::Rebuild);
        }
    }

    /// Binds the feed to the waker of its app, which a sent chart wakes.
    pub(crate) fn bind(&self, waker: AppWaker) {
        let waiting = {
            let mut slot = self.slot();
            slot.waker = Some(waker.clone());
            slot.chart.is_some()
        };
        if waiting {
            waker.request(Refresh::Rebuild);
        }
    }

    /// The chart that waits to be shown, if one does.
    pub(crate) fn take(&self) -> Option<Chart> {
        self.slot().chart.take()
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
