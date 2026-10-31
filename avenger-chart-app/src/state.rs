//! The state of a chart app: its chart prepared to evaluate, the plots of its last evaluation,
//! its interaction mode, and the callback that hears its changes.

use std::sync::Arc;

use arrow::array::ArrayRef;
use avenger_chart::{
    proto::{self, Channel},
    resolve::{self, Identity},
    Chart, Contribution, TupleTest,
};
use avenger_chart_eval::{
    ChartEvaluator, EvalError, EvaluatedPlot, EvaluationWarning, PlotCoord, ScaleIdentity, Theme,
};
use avenger_scales::{scalar::Scalar, scales::ConfiguredScale};
use avenger_typst_label::LabelEngine;

use crate::feed::ChartFeed;

/// A change that a chart app reports to its host.
#[derive(Clone, Debug)]
pub enum ChartChange {
    /// A navigation settled the domain of a scale: the domain of its view, or its fitted domain
    /// when a double click returned it home.
    View {
        scale: ScaleIdentity,
        domain: ArrayRef,
    },
    /// A brush, or a navigation of a scale whose domain is a selection domain, changed a
    /// selection, whose contributions are now `contributions`. `settled` is true at the end of
    /// the gesture.
    Selection {
        name: String,
        contributions: Vec<Contribution>,
        settled: bool,
    },
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
    on_change: Option<OnChange>,
}

impl ChartAppState {
    pub(crate) fn new(
        chart: &Chart,
        engine: &LabelEngine,
        theme: &Theme,
        feed: ChartFeed,
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
            on_change,
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

    /// Shows `chart` in place of the current chart. Its scales take the views of the current
    /// scales with their identities and scale types, and its selections keep the contributions
    /// of the current selections with their names.
    pub(crate) fn swap(&mut self, chart: &Chart) -> Result<(), EvalError> {
        let mut evaluator = ChartEvaluator::new(chart, &self.engine, &self.theme)?;
        evaluator.set_views(&self.evaluator.views());
        let mut selections = evaluator.selections().clone();
        selections.extend(self.evaluator.selections().clone());
        evaluator.set_selections(selections)?;
        self.evaluator = evaluator;
        self.tools = tools(chart);
        self.moved.clear();
        Ok(())
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
    /// otherwise the plot's own, and the app reports the change.
    pub(crate) fn write(
        &mut self,
        name: &str,
        owner: &Identity,
        tuples: Vec<Vec<TupleTest>>,
        settled: bool,
    ) {
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
        self.report_selection(name, settled);
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

    /// Reports the contributions of the selection `name`.
    pub(crate) fn report_selection(&self, name: &str, settled: bool) {
        let contributions = (self.evaluator.selections().get(name))
            .cloned()
            .unwrap_or_default();
        self.report(ChartChange::Selection {
            name: name.to_string(),
            contributions,
            settled,
        });
    }

    pub(crate) fn report(&self, change: ChartChange) {
        if let Some(on_change) = &self.on_change {
            on_change(&change);
        }
    }
}

/// The scale of `channel`, `x` or `y`, of a Cartesian plot.
pub(crate) fn plot_scale(plot: &EvaluatedPlot, channel: Channel) -> Option<&ConfiguredScale> {
    let PlotCoord::Cartesian { x, y, .. } = &plot.coord else {
        return None;
    };
    match channel {
        Channel::X => x.as_ref(),
        Channel::Y => y.as_ref(),
        _ => None,
    }
}

/// The tools of each plot of `chart`, by plot.
fn tools(chart: &Chart) -> Vec<Vec<proto::Tool>> {
    (resolve::tree(chart).plots.iter())
        .map(|plot| plot.tools.iter().map(|&tool| tool.clone()).collect())
        .collect()
}
