//! `ChartFeed`, through which a host sends updates, the interaction mode, and selections to a
//! chart app.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use avenger_app::waker::{AppWaker, Refresh};
use avenger_chart::Contribution;
use avenger_chart_eval::{EvalError, Theme};
use avenger_typst_label::LabelEngine;

use crate::{
    state::Mode,
    update::{ChartUpdate, PreparedUpdate},
};

/// A handle through which a host sends updates, the interaction mode, and selections to a chart
/// app, from any thread, such as when a query answers a settled view. Clones share one slot,
/// which the app takes at its next build, so the updates that arrive faster than the app builds
/// collapse to the newest one.
#[derive(Clone, Default)]
pub struct ChartFeed {
    slot: Arc<Mutex<Slot>>,
}

/// An update that waits in a feed: prepared once an app holds the feed, and otherwise prepared at
/// the app's first build.
pub(crate) enum Pending {
    Unprepared(Box<ChartUpdate>),
    Prepared(Box<PreparedUpdate>),
}

/// What waits in a feed for the next build of its app.
#[derive(Default)]
pub(crate) struct Waiting {
    pub(crate) update: Option<Pending>,
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
    /// The label engine and the theme of the app, which prepare the updates that the feed sends.
    preparer: Option<(LabelEngine, Theme)>,
}

impl ChartFeed {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends `update`, such as a bare chart with the default options, which the app installs at
    /// its next build in place of the chart that it shows and of an update that it has not
    /// installed. Once an app holds the feed, `send` prepares the update on the calling thread,
    /// and returns the error of a chart that fails to prepare, which the app does not install.
    pub fn send(&self, update: impl Into<ChartUpdate>) -> Result<(), EvalError> {
        let update = update.into();
        let preparer = self.slot().preparer.clone();
        let pending = match preparer {
            Some((engine, theme)) => Pending::Prepared(Box::new(update.prepare(&engine, &theme)?)),
            None => Pending::Unprepared(Box::new(update)),
        };
        self.wait(|waiting| waiting.update = Some(pending));
        Ok(())
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

    /// Binds the feed to its app: the waker that a change wakes, and the label engine and the
    /// theme that prepare the updates that the feed sends.
    pub(crate) fn bind(&self, waker: AppWaker, engine: &LabelEngine, theme: &Theme) {
        let waiting = {
            let mut slot = self.slot();
            slot.waker = Some(waker.clone());
            slot.preparer = Some((engine.clone(), theme.clone()));
            let waiting = &slot.waiting;
            waiting.update.is_some() || waiting.mode.is_some() || !waiting.selections.is_empty()
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
