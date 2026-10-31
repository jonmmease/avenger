//! The updates of a chart app: `ChartUpdate`, a new chart with what the app carries over from
//! the chart that it replaces, and `ChartTask`, which loads updates on the app's runtime and
//! prepares them there.

use std::{
    future::Future,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

use avenger_app::{
    app::AvengerApp,
    task::{BoxError, Cancel, Done, Task},
};
use avenger_chart::Chart;
use avenger_chart_eval::{
    Carry, EvalError, Fit, Fitted, PreparedChart, Theme, UpdateOptions, Views,
};
use avenger_typst_label::LabelEngine;

use crate::state::ChartAppState;

/// A new chart for a chart app, and what the app carries over from the chart that it replaces:
/// the fitted domain of each scale, the fit of each projection, the views of pans and zooms, and
/// the contributions of each selection. A chart converts to an update with the defaults, which
/// fit each scale and each projection to the new rows and keep the views and the contributions.
#[derive(Clone)]
#[must_use]
pub struct ChartUpdate {
    chart: Chart,
    options: UpdateOptions,
}

impl ChartUpdate {
    /// An update to `chart`, with the defaults.
    pub fn new(chart: Chart) -> Self {
        Self {
            chart,
            options: UpdateOptions::new(),
        }
    }

    /// The fit of every scale and every projection without an override, `Fit::Refit` by default.
    pub fn fit(mut self, fit: Fit) -> Self {
        self.options = self.options.fit(fit);
        self
    }

    /// The fit of the scale or the projection with `identity`, in place of the fit of every
    /// scale and every projection.
    pub fn fit_of(mut self, identity: impl Into<Fitted>, fit: Fit) -> Self {
        self.options = self.options.fit_of(identity, fit);
        self
    }

    /// Whether the scales keep the views of pans and zooms, `Views::Keep` by default.
    pub fn views(mut self, views: Views) -> Self {
        self.options = self.options.views(views);
        self
    }

    /// The carry of every selection without an override, `Carry::Keep` by default.
    pub fn selections(mut self, carry: Carry) -> Self {
        self.options = self.options.selections(carry);
        self
    }

    /// The carry of the selection `name`, in place of the carry of every selection.
    pub fn selection(mut self, name: impl Into<String>, carry: Carry) -> Self {
        self.options = self.options.selection(name, carry);
        self
    }

    /// The update prepared with the label engine and the theme of its app.
    pub(crate) fn prepare(
        self,
        engine: &LabelEngine,
        theme: &Theme,
    ) -> Result<PreparedUpdate, EvalError> {
        Ok(PreparedUpdate {
            chart: PreparedChart::new(&self.chart, engine, theme)?,
            options: self.options,
        })
    }
}

impl From<Chart> for ChartUpdate {
    fn from(chart: Chart) -> Self {
        Self::new(chart)
    }
}

/// An update prepared to install.
pub(crate) struct PreparedUpdate {
    pub(crate) chart: PreparedChart,
    pub(crate) options: UpdateOptions,
}

/// The prepared update of a finished request, which the app takes once to install it.
pub(crate) struct Finished(Mutex<Option<PreparedUpdate>>);

/// The label engine and the theme of the app that a task serves.
type Preparer = Arc<OnceLock<(LabelEngine, Theme)>>;

/// Loads the updates of a chart app on the app's runtime. A request runs the host's work, which
/// returns an update, and prepares the update in the same job with the label engine and the
/// theme of the app, so the render loop only installs it. The app shows the update of each
/// request that finishes at its next build. A newer request replaces the request that waits or
/// runs and cancels it, so a host requests an update for each settled change and the app shows
/// the newest one that finishes. A task serves one app, which attaches it with
/// [`ChartApp::charts`](crate::ChartApp::charts). Clones share the task.
pub struct ChartTask<K> {
    task: Task<K, Finished>,
    preparer: Preparer,
}

impl<K> Clone for ChartTask<K> {
    fn clone(&self) -> Self {
        Self {
            task: self.task.clone(),
            preparer: self.preparer.clone(),
        }
    }
}

impl<K: Clone + PartialEq + Send + Sync + 'static> Default for ChartTask<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + PartialEq + Send + Sync + 'static> ChartTask<K> {
    /// A task that waits for an app to attach it.
    pub fn new() -> Self {
        Self {
            task: Task::new(),
            preparer: Arc::default(),
        }
    }

    /// Runs `work` for `key` in place of the request that waits or runs, which the task
    /// cancels, and prepares the update that `work` returns. A request for the key of the request
    /// that waits or runs does nothing, and a request for the key of the latest update only
    /// cancels the request that waits or runs, as [`Task::request`] does, so a key names
    /// everything that the update depends on.
    pub fn request<F, Fut, U, E>(&self, key: K, work: F)
    where
        F: FnOnce(Cancel) -> Fut + Send + 'static,
        Fut: Future<Output = Result<U, E>> + Send + 'static,
        U: Into<ChartUpdate>,
        E: Into<BoxError>,
    {
        let preparer = self.preparer.clone();
        self.task.request(key, move |cancel| async move {
            let update: ChartUpdate = work(cancel).await.map_err(Into::into)?.into();
            let (engine, theme) = preparer
                .get()
                .ok_or("the chart task prepares only for an app that attached it")?;
            let update = update.prepare(engine, theme).map_err(Box::new)?;
            Ok::<_, BoxError>(Finished(Mutex::new(Some(update))))
        });
    }

    /// The key of the request that waits or runs.
    pub fn pending(&self) -> Option<K> {
        self.task.pending()
    }

    /// The error of the last request that failed, until a newer request.
    pub fn error(&self) -> Option<Arc<BoxError>> {
        self.task.error()
    }

    /// The updates of the task, for the app, and how the built app attaches the task.
    pub(crate) fn source(&self) -> TaskSource {
        let attached = self.clone();
        TaskSource {
            charts: Box::new(TaskCharts {
                task: self.task.clone(),
                shown: None,
            }),
            attach: Box::new(move |app, engine, theme| {
                // A second app keeps the engine and the theme of the first.
                let _ = attached.preparer.set((engine.clone(), theme.clone()));
                app.with_task(&attached.task)
            }),
        }
    }
}

/// The attachment of a task to a built app, with the app's label engine and theme.
type Attach = Box<
    dyn FnOnce(AvengerApp<ChartAppState>, &LabelEngine, &Theme) -> AvengerApp<ChartAppState> + Send,
>;

/// The updates of a task, and how the built app attaches the task.
pub(crate) struct TaskSource {
    pub(crate) charts: Box<dyn Charts>,
    pub(crate) attach: Attach,
}

/// The latest update of a task, which a chart app installs once, whatever the type of the task's
/// keys.
pub(crate) trait Charts: Send + Sync {
    /// The task's latest update, when the app has not installed it.
    fn take(&mut self) -> Option<PreparedUpdate>;
}

/// A task of updates, with the result that the app installed last. Holding the result keeps its
/// allocation, so a newer result is never mistaken for it.
struct TaskCharts<K> {
    task: Task<K, Finished>,
    shown: Option<Arc<Done<K, Finished>>>,
}

impl<K: Clone + PartialEq + Send + Sync + 'static> Charts for TaskCharts<K> {
    fn take(&mut self) -> Option<PreparedUpdate> {
        let latest = self.task.latest()?;
        if (self.shown.as_ref()).is_some_and(|shown| Arc::ptr_eq(shown, &latest)) {
            return None;
        }
        let update = (latest.value.0.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.shown = Some(latest);
        update
    }
}
