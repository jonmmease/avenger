//! `ChartFeed`, through which a host sends new charts to a chart app.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use avenger_app::waker::{AppWaker, Refresh};
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
