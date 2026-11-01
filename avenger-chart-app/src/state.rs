//! The state of a chart app: its chart prepared to evaluate, the plots of its last evaluation,
//! and the callback that hears its changes.

use std::sync::Arc;

use arrow::array::ArrayRef;
use avenger_chart::{proto, resolve, Chart};
use avenger_chart_eval::{
    ChartEvaluator, EvalError, EvaluatedPlot, EvaluationWarning, ScaleIdentity, Theme,
};
use avenger_typst_label::LabelEngine;

use crate::feed::{ChartFeed, Charts};

/// What a settled navigation changed, which a chart app reports to its host: the domain of each
/// scale that the navigation moves. A navigation of `x` and `y` reports both of its scales in one
/// change, so a host can load the data of the next chart with one query.
#[derive(Clone, Debug, Default)]
pub struct ChartChange {
    /// The domain that each scale the navigation moved shows when it settles: the domain of the
    /// scale's view, or its fitted domain after a double click returned it home.
    pub domains: Vec<(ScaleIdentity, ArrayRef)>,
}

/// The callback that hears the changes of an app.
pub(crate) type OnChange = Arc<dyn Fn(&ChartChange) + Send + Sync>;

/// The state of a chart app: the chart, prepared to evaluate at any size with the views of its
/// scales, and the plots of its last evaluation.
pub struct ChartAppState {
    pub(crate) evaluator: ChartEvaluator,
    engine: LabelEngine,
    theme: Theme,
    /// The tools of each plot of the chart, by plot.
    tools: Vec<Vec<proto::Tool>>,
    pub(crate) plots: Vec<EvaluatedPlot>,
    pub(crate) warnings: Vec<EvaluationWarning>,
    pub(crate) feed: ChartFeed,
    /// The task whose latest chart the app shows, when the host gives one.
    pub(crate) charts: Option<Box<dyn Charts>>,
    on_change: Option<OnChange>,
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
            feed,
            charts,
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

    pub(crate) fn report(&self, change: ChartChange) {
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
