//! The one path by which a host changes a chart app: `ChartUpdates`, a handle that any thread clones
//! and sends to, which carries updates of the chart and of the style inputs, and the commands of the
//! app's state. The app prepares each new chart as a job of its runtime, never on the sending
//! thread and never on the render loop, and it applies the updates and the commands in the order
//! of the calls.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use avenger_app::{
    task::{BoxError, Task},
    waker::{AppWaker, Refresh},
};
use avenger_chart::Chart;
use avenger_chart_eval::{
    Carry, EvalError, Fit, Fitted, PreparedChart, Registry, UpdateOptions, Views,
};
use avenger_selection::SelectionState;
use avenger_style::Cascade;
use avenger_typst_label::LabelEngine;

/// An update of a chart app: a new chart with what the app carries over from the chart that it
/// replaces, new style inputs as the cascade of a style table, or both. The chart carries the
/// fitted domain of each scale and the fit of each projection ([`Fit`]), the views of pans and
/// zooms ([`Views`]), and the clauses of each selection ([`Carry`]). A chart converts to an update
/// with the defaults, which fit each scale and each projection to the new rows and keep the views
/// and the clauses.
#[derive(Clone, Default)]
#[must_use]
pub struct ChartUpdate {
    chart: Option<Chart>,
    styles: Option<Arc<Cascade>>,
    options: UpdateOptions,
}

impl ChartUpdate {
    /// An update to `chart`, with the defaults.
    pub fn new(chart: Chart) -> Self {
        Self {
            chart: Some(chart),
            ..Self::default()
        }
    }

    /// An update of the style inputs alone: the cascade of a new style table, such as after an
    /// edit of a stylesheet, which keeps the chart and its encoded rows.
    pub fn styles(cascade: Arc<Cascade>) -> Self {
        Self {
            styles: Some(cascade),
            ..Self::default()
        }
    }

    /// The update with new style inputs too.
    pub fn with_styles(mut self, cascade: Arc<Cascade>) -> Self {
        self.styles = Some(cascade);
        self
    }

    /// The fit of every scale and every projection without an override, `Fit::Refit` by default.
    pub fn fit(mut self, fit: Fit) -> Self {
        self.options = self.options.fit(fit);
        self
    }

    /// The fit of the scale or the projection with `identity`, in place of the fit of every scale
    /// and every projection.
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
}

impl From<Chart> for ChartUpdate {
    fn from(chart: Chart) -> Self {
        Self::new(chart)
    }
}

/// A new chart prepared to install, with the options of its update.
pub(crate) struct PreparedUpdate {
    pub(crate) chart: PreparedChart,
    pub(crate) options: UpdateOptions,
}

/// A command that waits for the app's next build, in the order of the calls.
enum Command {
    /// An update: the sequence number of its chart, which the task of the handle prepares, and its
    /// style inputs.
    Update {
        chart: Option<u64>,
        styles: Option<Arc<Cascade>>,
    },
    Mode(String, bool),
    /// The state of a selection, or `None` to clear it.
    Selection(String, Option<SelectionState>),
}

/// What the app applies at a build: a prepared chart, new style inputs, a mode on or off, or the
/// state of a
/// selection, in the order of the calls.
pub(crate) enum Applied {
    Chart(Box<PreparedUpdate>),
    Styles(Arc<Cascade>),
    Mode(String, bool),
    Selection(String, Option<SelectionState>),
}

/// A chart whose preparation waits for an app to bind the handle.
struct Unprepared {
    sequence: u64,
    chart: Chart,
    options: UpdateOptions,
}

#[derive(Default)]
struct Queue {
    commands: VecDeque<Command>,
    /// The sequence number of the next chart.
    next: u64,
    /// The sequence number of the newest chart, which overtakes the charts before it.
    newest: Option<u64>,
    /// The newest chart sent before an app bound the handle.
    unprepared: Option<Unprepared>,
    waker: Option<AppWaker>,
    /// The label engine and the registry of the app, which prepare the charts.
    preparer: Option<(LabelEngine, Registry)>,
}

/// The one handle through which a host changes a chart app, from any thread: updates of the chart
/// and of the style inputs, the chart's modes, and the states of selections. The app applies
/// them in the order of the calls at its builds. It prepares each new chart as a job of its
/// runtime, so a native host gives the built app a multi-thread runtime with
/// `AvengerApp::with_runtime`. Updates that wait together merge: a newer chart overtakes the
/// preparation of an older one, whose options it replaces, and the commands between them still
/// apply in order. A chart that fails to prepare keeps the app's chart, and [`ChartUpdates::error`]
/// reports its error. Clones share the handle, which serves one app.
#[derive(Clone, Default)]
pub struct ChartUpdates {
    queue: Arc<Mutex<Queue>>,
    task: Task<u64, Arc<Mutex<Option<PreparedUpdate>>>>,
}

impl ChartUpdates {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends `update`, such as a bare chart with the default options.
    pub fn send(&self, update: impl Into<ChartUpdate>) {
        let ChartUpdate {
            chart,
            styles,
            options,
        } = update.into();
        let (waker, request) = {
            let mut queue = self.queue();
            let sequence = chart.as_ref().map(|_| {
                let sequence = queue.next;
                queue.next += 1;
                queue.newest = Some(sequence);
                sequence
            });
            queue.commands.push_back(Command::Update {
                chart: sequence,
                styles,
            });
            let request = match (sequence.zip(chart), &queue.preparer) {
                (Some((sequence, chart)), Some((engine, registry))) => {
                    Some((sequence, chart, options, engine.clone(), registry.clone()))
                }
                (Some((sequence, chart)), None) => {
                    queue.unprepared = Some(Unprepared {
                        sequence,
                        chart,
                        options,
                    });
                    None
                }
                (None, _) => None,
            };
            (queue.waker.clone(), request)
        };
        if let Some((sequence, chart, options, engine, registry)) = request {
            self.prepare(sequence, chart, options, engine, registry);
        }
        if let Some(waker) = waker {
            waker.request(Refresh::Rebuild);
        }
    }

    /// Turns the chart's mode `name` on or off, as a mode widget does: turning on a mode of a group
    /// turns off the group's other modes, and turning one off does nothing. A mode that the chart
    /// does not declare is off until a chart that declares it is installed. The app reports no
    /// change.
    pub fn set_mode(&self, name: impl Into<String>, on: bool) {
        self.command(Command::Mode(name.into(), on));
    }

    /// Sets the state of the selection `name`, such as to restore a saved selection. The app
    /// draws its brushes and styles its rows, and reports no change.
    pub fn set_selection(&self, name: &str, state: SelectionState) {
        self.command(Command::Selection(name.to_string(), Some(state)));
    }

    /// Clears the selection `name`.
    pub fn clear_selection(&self, name: &str) {
        self.command(Command::Selection(name.to_string(), None));
    }

    /// The error of the last chart that failed to prepare, until a newer chart.
    pub fn error(&self) -> Option<Arc<BoxError>> {
        self.task.error()
    }

    /// Whether a chart waits to prepare or prepares.
    pub fn pending(&self) -> bool {
        self.task.pending().is_some() || self.queue().unprepared.is_some()
    }

    fn command(&self, command: Command) {
        let waker = {
            let mut queue = self.queue();
            queue.commands.push_back(command);
            queue.waker.clone()
        };
        if let Some(waker) = waker {
            waker.request(Refresh::Rebuild);
        }
    }

    /// Prepares the chart of the update `sequence` as a job of the app's runtime.
    fn prepare(
        &self,
        sequence: u64,
        chart: Chart,
        options: UpdateOptions,
        engine: LabelEngine,
        registry: Registry,
    ) {
        self.task.request(sequence, move |_cancel| async move {
            let chart = PreparedChart::new(chart, &engine, &registry)?;
            let update = PreparedUpdate { chart, options };
            Ok::<_, EvalError>(Arc::new(Mutex::new(Some(update))))
        });
    }

    /// The task that prepares the charts, which the app attaches.
    pub(crate) fn task(&self) -> &Task<u64, Arc<Mutex<Option<PreparedUpdate>>>> {
        &self.task
    }

    /// Binds the handle to its app: the waker that a command wakes, and the label engine and the
    /// registry that prepare the charts. A chart sent before prepares now.
    pub(crate) fn bind(&self, waker: AppWaker, engine: &LabelEngine, registry: &Registry) {
        let (waiting, unprepared) = {
            let mut queue = self.queue();
            queue.waker = Some(waker.clone());
            queue.preparer = Some((engine.clone(), registry.clone()));
            (!queue.commands.is_empty(), queue.unprepared.take())
        };
        if let Some(Unprepared {
            sequence,
            chart,
            options,
        }) = unprepared
        {
            self.prepare(sequence, chart, options, engine.clone(), registry.clone());
        }
        if waiting {
            waker.request(Refresh::Rebuild);
        }
    }

    /// What the app applies at a build, in the order of the calls: every command up to the first
    /// update whose chart still prepares. A chart that a newer chart overtook applies nothing, and
    /// a chart that failed to prepare applies nothing, with its error in [`ChartUpdates::error`].
    pub(crate) fn take(&self) -> Vec<Applied> {
        let mut queue = self.queue();
        let mut applied = Vec::new();
        while let Some(command) = queue.commands.pop_front() {
            match command {
                Command::Mode(mode, on) => applied.push(Applied::Mode(mode, on)),
                Command::Selection(name, state) => applied.push(Applied::Selection(name, state)),
                Command::Update { chart, styles } => {
                    if let Some(styles) = styles {
                        applied.push(Applied::Styles(styles));
                    }
                    let Some(sequence) = chart else {
                        continue;
                    };
                    if queue.newest.is_some_and(|newest| newest > sequence) {
                        continue;
                    }
                    let latest = self.task.latest();
                    if let Some(done) = latest.filter(|done| done.key == sequence) {
                        let update = (done.value.lock())
                            .unwrap_or_else(PoisonError::into_inner)
                            .take();
                        applied.extend(update.map(|update| Applied::Chart(Box::new(update))));
                        continue;
                    }
                    let preparing = self.task.pending() == Some(sequence)
                        || queue.unprepared.as_ref().map(|chart| chart.sequence) == Some(sequence);
                    // The later commands wait for a chart that prepares, and a chart that failed
                    // applies nothing.
                    if preparing {
                        queue.commands.push_front(Command::Update {
                            chart: Some(sequence),
                            styles: None,
                        });
                        break;
                    }
                }
            }
        }
        applied
    }

    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
