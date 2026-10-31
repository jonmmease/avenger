//! The state of a chart app: its chart prepared to evaluate, the plots of its last evaluation,
//! its interaction mode, and the callback that hears its changes.

use std::sync::Arc;

use arrow::array::ArrayRef;
use avenger_app_tools::brushes::Lasso;
use avenger_chart::{
    proto::{self, Channel},
    resolve::{self, Identity},
    Chart, Contribution, TupleTest,
};
use avenger_chart_eval::{
    ChartEvaluator, EvalError, EvaluatedPlot, EvaluationWarning, ProjectionIdentity,
    ProjectionView, ScaleIdentity, Theme,
};
use avenger_scales::{scalar::Scalar, scales::ConfiguredScale};
use avenger_typst_label::LabelEngine;

use crate::{
    feed::ChartFeed,
    update::{ChartUpdate, Charts, PreparedUpdate},
};

/// What one gesture changed, which a chart app reports to its host: the domains of the scales and
/// the projections that a navigation settled, and the selections that a brush or a navigation
/// wrote. A navigation of `x` and `y` reports both of its scales in one change, with the
/// selections that its scales read, so a host can load the data of the next chart with one query.
#[derive(Clone, Debug, Default)]
pub struct ChartChange {
    /// The domain that each scale a navigation moved shows when the navigation settles: the domain
    /// of the scale's view, or its fitted domain after a double click returned it home.
    pub domains: Vec<(ScaleIdentity, ArrayRef)>,
    /// The projection that each projection a navigation moved shows when the navigation settles,
    /// at the size of its maps: the projection's view, or its fitted projection after a double
    /// click returned it home.
    pub projections: Vec<(ProjectionIdentity, ProjectionView)>,
    /// Each selection that the gesture wrote, by name, with all of its contributions. A brush
    /// reports its selection at each move of its drag.
    pub selections: Vec<(String, Vec<Contribution>)>,
    /// Whether the gesture ended, at a release, a click, a double click, or a wheel that stopped.
    pub settled: bool,
}

impl ChartChange {
    /// The contributions of the selection `name`, when the gesture wrote it.
    pub fn selection(&self, name: &str) -> Option<&[Contribution]> {
        (self.selections.iter())
            .find(|(written, _)| written == name)
            .map(|(_, contributions)| contributions.as_slice())
    }

    /// Whether the change holds no domain, no projection, and no selection.
    fn is_empty(&self) -> bool {
        self.domains.is_empty() && self.projections.is_empty() && self.selections.is_empty()
    }
}

/// What a drag does on a plot that declares both a navigation and a box or a lasso selection.
/// On other plots, a drag navigates or selects as the plot declares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// A drag pans.
    #[default]
    Navigate,
    /// A drag draws a box or a lasso.
    Select,
}

/// The callback that hears the changes of an app.
pub(crate) type OnChange = Arc<dyn Fn(&ChartChange) + Send + Sync>;

/// The state of a chart app: the chart, prepared to evaluate at any size with the views of its
/// scales and the contributions of its selections, the plots of its last evaluation, and the
/// interaction mode.
pub struct ChartAppState {
    pub(crate) evaluator: ChartEvaluator,
    engine: LabelEngine,
    theme: Theme,
    /// The tools of each plot of the chart, by plot.
    tools: Vec<Vec<proto::Tool>>,
    pub(crate) plots: Vec<EvaluatedPlot>,
    pub(crate) warnings: Vec<EvaluationWarning>,
    pub(crate) mode: Mode,
    /// The scales that navigations move in place of the scales whose domains are selection
    /// domains, by plot and channel, until the next build writes them to their selections.
    pub(crate) moved: Vec<(Identity, Channel, ConfiguredScale)>,
    pub(crate) feed: ChartFeed,
    /// The task whose latest chart the app shows, when the host gives one.
    pub(crate) charts: Option<Box<dyn Charts>>,
    on_change: Option<OnChange>,
    /// The selections written since the last report, in the order of their first writes.
    written: Vec<String>,
    /// Each lasso with a key, by its plot and its selection: its paths and the keys that they
    /// wrote, so that a lasso stays drawn while its plot's contribution holds those keys.
    pub(crate) lassos: Vec<((Identity, String), Lasso<Scalar>)>,
}

impl ChartAppState {
    pub(crate) fn new(
        chart: &Chart,
        engine: &LabelEngine,
        theme: &Theme,
        feed: ChartFeed,
        charts: Option<Box<dyn Charts>>,
        on_change: Option<OnChange>,
    ) -> Result<Self, EvalError> {
        Ok(Self {
            evaluator: ChartEvaluator::new(chart, engine, theme)?,
            engine: engine.clone(),
            theme: theme.clone(),
            tools: tools(chart),
            plots: Vec::new(),
            warnings: Vec::new(),
            mode: Mode::default(),
            moved: Vec::new(),
            feed,
            charts,
            on_change,
            written: Vec::new(),
            lassos: Vec::new(),
        })
    }

    /// The chart that the app shows.
    pub fn chart(&self) -> &Chart {
        self.evaluator.chart()
    }

    /// The evaluator of the chart, with the views of its scales and the contributions of its
    /// selections.
    pub fn evaluator(&self) -> &ChartEvaluator {
        &self.evaluator
    }

    /// The plots of the last evaluation.
    pub fn plots(&self) -> &[EvaluatedPlot] {
        &self.plots
    }

    /// The warnings of the last evaluation.
    pub fn warnings(&self) -> &[EvaluationWarning] {
        &self.warnings
    }

    /// The interaction mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Installs `update` in place of the current chart, which carries the views of its scales
    /// and the contributions of its selections by the update's options.
    pub(crate) fn install(&mut self, update: PreparedUpdate) -> Result<(), EvalError> {
        self.evaluator.install(update.chart, &update.options)?;
        self.tools = tools(self.evaluator.chart());
        self.moved.clear();
        Ok(())
    }

    /// `update` prepared with the label engine and the theme of the app.
    pub(crate) fn prepare(&self, update: ChartUpdate) -> Result<PreparedUpdate, EvalError> {
        update.prepare(&self.engine, &self.theme)
    }

    /// The plots of the last evaluation that declare tools, by their indices, with the tools.
    pub(crate) fn declaring(
        &self,
    ) -> impl Iterator<Item = (usize, &EvaluatedPlot, &[proto::Tool])> {
        (self.plots.iter().zip(&self.tools))
            .enumerate()
            .filter(|(_, (plot, tools))| plot.identity.is_some() && !tools.is_empty())
            .map(|(index, (plot, tools))| (index, plot, &tools[..]))
    }

    /// The index and the plot of the last evaluation with `identity`.
    pub(crate) fn plot(&self, identity: &Identity) -> Option<(usize, &EvaluatedPlot)> {
        (self.plots.iter().enumerate()).find(|(_, plot)| plot.identity.as_ref() == Some(identity))
    }

    /// The resolution of the selection `name`.
    pub(crate) fn resolution(&self, name: &str) -> proto::SelectionResolution {
        (self.chart().selections().iter())
            .find(|selection| selection.name == name)
            .and_then(|selection| proto::SelectionResolution::try_from(selection.resolution).ok())
            .unwrap_or(proto::SelectionResolution::Union)
    }

    /// The contribution of the selection `name` that the plot with `owner` shows: the one
    /// contribution of a global selection, or else the plot's own.
    pub(crate) fn contribution(&self, name: &str, owner: &Identity) -> Option<&Contribution> {
        let contributions = self.evaluator.selections().get(name)?;
        match self.resolution(name) {
            proto::SelectionResolution::Global => contributions.first(),
            _ => (contributions.iter()).find(|contribution| owner.wrote(contribution)),
        }
    }

    /// Writes the contribution of the plot with `owner` to the selection `name`: `tuples`, or no
    /// contribution without tuples. It replaces every contribution of a global selection, and
    /// otherwise the plot's own, and the next report holds the selection.
    pub(crate) fn write(&mut self, name: &str, owner: &Identity, tuples: Vec<Vec<TupleTest>>) {
        let global = self.resolution(name) == proto::SelectionResolution::Global;
        let mut selections = self.evaluator.selections().clone();
        let contributions = selections.entry(name.to_string()).or_default();
        if global {
            contributions.clear();
        } else {
            contributions.retain(|contribution| !owner.wrote(contribution));
        }
        if !tuples.is_empty() {
            contributions.push(Contribution {
                owner: owner.id.clone(),
                partition: (owner.partition.iter())
                    .map(|(column, value)| (column.clone(), Scalar(value.clone())))
                    .collect(),
                tuples,
            });
        }
        self.set_selection(name, Some(selections[name].clone()));
        self.touch(name);
    }

    /// Holds the selection `name` in the next report, such as a selection that a navigation
    /// wrote before it settled.
    pub(crate) fn touch(&mut self, name: &str) {
        if !self.written.iter().any(|written| written == name) {
            self.written.push(name.to_string());
        }
    }

    /// Sets the contributions of the selection `name`, or removes them for `None`.
    pub(crate) fn set_selection(&mut self, name: &str, contributions: Option<Vec<Contribution>>) {
        let mut selections = self.evaluator.selections().clone();
        match contributions {
            Some(contributions) => selections.insert(name.to_string(), contributions),
            None => selections.remove(name),
        };
        if let Err(error) = self.evaluator.set_selections(selections) {
            eprintln!("The chart app couldn't set the selection {name:?}: {error}");
        }
    }

    /// Reports `change` with the selections written since the last report and their
    /// contributions. It reports nothing without a domain, a projection, or a selection.
    pub(crate) fn report(&mut self, mut change: ChartChange) {
        change.selections = std::mem::take(&mut self.written)
            .into_iter()
            .map(|name| {
                let contributions = self.evaluator.selections().get(&name).cloned();
                (name, contributions.unwrap_or_default())
            })
            .collect();
        if change.is_empty() {
            return;
        }
        if let Some(on_change) = &self.on_change {
            on_change(&change);
        }
    }
}

/// The tools of each plot of `chart`, by plot.
fn tools(chart: &Chart) -> Vec<Vec<proto::Tool>> {
    (resolve::tree(chart).plots.iter())
        .map(|plot| plot.tools.iter().map(|&tool| tool.clone()).collect())
        .collect()
}
