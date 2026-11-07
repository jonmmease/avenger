//! The state of a chart app: its chart prepared to evaluate, the plots of its last evaluation, the
//! registry of the tools that its plots and layers declare, its interaction mode, and the callbacks
//! that hear its changes.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::ArrayRef;
use avenger_app_tools::brushes::Lasso;
use avenger_chart::{
    proto::{self, layer_tool, tool, Channel},
    resolve::Identity,
    Chart,
};
use avenger_chart_eval::{
    ChartEvaluator, EvalError, EvalWarning, EvaluatedPlot, ProjectionIdentity, ProjectionView,
    Registry, ScaleIdentity,
};
use avenger_coords::PlotCoords;
use avenger_scales::{scalar::Scalar, scales::ConfiguredScale};
use avenger_selection::{Clause, Condition, PixelGrid, Resolution, SelectionState};
use avenger_style::Cascade;
use avenger_typst_label::LabelEngine;

use crate::{
    updates::{ChartUpdates, PreparedUpdate},
    widgets::WidgetControls,
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
    /// at the size of its geo plots: the projection's view, or its fitted projection after a double
    /// click returned it home.
    pub projections: Vec<(ProjectionIdentity, ProjectionView)>,
    /// Each selection that the gesture wrote, by name, with all of its clauses.
    pub selections: Vec<(String, Vec<Clause>)>,
    /// Whether the gesture ended, at a release, a click, a double click, or a wheel that stopped.
    pub settled: bool,
}

impl ChartChange {
    /// The clauses of the selection `name`, when the gesture wrote it.
    pub fn selection(&self, name: &str) -> Option<&[Clause]> {
        (self.selections.iter())
            .find(|(written, _)| written == name)
            .map(|(_, clauses)| clauses.as_slice())
    }

    /// Whether the change holds no domain, no projection, and no selection.
    fn is_empty(&self) -> bool {
        self.domains.is_empty() && self.projections.is_empty() && self.selections.is_empty()
    }
}

/// Which changes a chart app reports to its host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reports {
    /// Every change, such as each move of a brush's drag, so the host can follow a gesture.
    #[default]
    Every,
    /// The changes when a gesture settles, with the selections that the gesture wrote before it,
    /// so brushes and navigations report alike.
    Settled,
}

/// A plot whose selection can pre-aggregate, under the pointer before a gesture, which a host can
/// prepare data for, such as a table of the other views' aggregates by the selection's keys: its
/// selection, the owner of its clauses with its partition values, and each dimension that its
/// clauses check. A box that selects whole cells of every dimension that it covers and a point
/// selection can pre-aggregate.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionFocus {
    pub selection: String,
    pub owner: Identity,
    pub dimensions: Vec<FocusDimension>,
}

/// A dimension of a focused selection: the column and the channel of the conditions that its
/// clauses write, and the grid of a box's cells, or `None` for the sets of a point selection's
/// keys and of the selection's scope, whose channel is unspecified.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusDimension {
    pub column: String,
    pub channel: Channel,
    pub grid: Option<PixelGrid>,
}

/// What a drag does on a plot that declares both a navigation and a box or a lasso selection. On
/// other plots, a drag navigates or selects as the plot declares.
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

/// The callback that hears the focus of an app's selections.
pub(crate) type OnFocus = Arc<dyn Fn(Option<&SelectionFocus>) + Send + Sync>;

/// The owner of a declaration's clauses and the name of a selection that it declares, which name
/// an area of a brush.
pub(crate) type SelectionArea = (Identity, String);

/// The tools that a plot or a layer declares, in the order of their declarations. A layer's tools
/// act on its marks alone, and its clauses are its own.
pub(crate) struct Declaration {
    /// The position of the plot in `Tree::plots`, and in the plots of an evaluation.
    pub(crate) plot: usize,
    /// The position of the layer among the plot's layers, for a layer's tools.
    pub(crate) layer: Option<usize>,
    pub(crate) tools: Vec<proto::Tool>,
}

/// The tools that the plots and the layers of `chart` declare, built once when the app installs
/// the chart, so each tool finds its plots by a lookup.
fn declarations(chart: &Chart) -> Vec<Declaration> {
    let resolved = chart.resolve();
    let mut declarations = Vec::new();
    for (index, plot) in resolved.tree.plots.iter().enumerate() {
        if !plot.plot.tools.is_empty() {
            declarations.push(Declaration {
                plot: index,
                layer: None,
                tools: plot.plot.tools.clone(),
            });
        }
        for (position, layer) in resolved.tree.layers[plot.layers.clone()].iter().enumerate() {
            let tools: Vec<proto::Tool> = (layer.layer.tools().iter())
                .filter_map(|declared| {
                    let kind = match declared.kind.clone()? {
                        layer_tool::Kind::Tooltip(tooltip) => tool::Kind::Tooltip(tooltip),
                        layer_tool::Kind::Select(select) => tool::Kind::Select(select),
                    };
                    Some(proto::Tool { kind: Some(kind) })
                })
                .collect();
            if !tools.is_empty() {
                declarations.push(Declaration {
                    plot: index,
                    layer: Some(position),
                    tools,
                });
            }
        }
    }
    declarations
}

/// A declaration of the last evaluation: its plot, the owner of its clauses, the marks that its
/// tools act on, the coordinates of its positions, and its tools.
pub(crate) struct Declared<'a> {
    pub(crate) index: usize,
    pub(crate) plot: &'a EvaluatedPlot,
    pub(crate) owner: &'a Identity,
    pub(crate) marks: Vec<String>,
    pub(crate) coords: &'a PlotCoords,
    pub(crate) tools: &'a [proto::Tool],
    /// Whether the declaration is a plot's, whose navigation and selections share its area.
    pub(crate) plot_tools: bool,
}

/// Whether `clause` is the clause of `owner`: its id and its partition values.
pub(crate) fn wrote(owner: &Identity, clause: &Clause) -> bool {
    clause.owner == owner.id
        && clause.partition.len() == owner.partition.len()
        && (owner.partition.iter())
            .all(|(column, value)| clause.partition.get(column) == Some(&Scalar(value.clone())))
}

/// The state of a chart app: the chart, prepared to evaluate at any size with the views of its
/// scales and the clauses of its selections, the plots of its last evaluation, and the
/// interaction mode.
pub struct ChartAppState {
    pub(crate) evaluator: ChartEvaluator,
    /// The tools of the chart's plots and layers.
    declarations: Vec<Declaration>,
    pub(crate) plots: Vec<EvaluatedPlot>,
    pub(crate) warnings: Vec<EvalWarning>,
    pub(crate) mode: Mode,
    /// The scales that navigations move in place of the scales whose domains read a selection,
    /// by plot and channel, until the next build writes them to their selections.
    pub(crate) moved: Vec<(Identity, Channel, ConfiguredScale)>,
    pub(crate) updates: ChartUpdates,
    on_change: Option<OnChange>,
    reports: Reports,
    on_focus: Option<OnFocus>,
    /// The selection under the pointer, as the app last reported it.
    pub(crate) focused: Option<SelectionFocus>,
    /// The selections written since the last report, in the order of their first writes.
    written: Vec<String>,
    /// Each lasso with keys, by its area: its paths and the keys that they wrote, so a lasso
    /// stays drawn while its owner's clause holds those keys.
    pub(crate) lassos: Vec<(SelectionArea, Lasso<Vec<Scalar>>)>,
    /// The widgets of the last evaluation and their controls.
    pub(crate) controls: WidgetControls,
}

impl ChartAppState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        engine: &LabelEngine,
        registry: &Registry,
        updates: ChartUpdates,
        on_change: Option<OnChange>,
        reports: Reports,
        on_focus: Option<OnFocus>,
    ) -> Result<Self, EvalError> {
        let declarations = declarations(&chart);
        Ok(Self {
            evaluator: ChartEvaluator::new(chart, cascade, engine.clone(), registry.clone())?,
            declarations,
            plots: Vec::new(),
            warnings: Vec::new(),
            mode: Mode::default(),
            moved: Vec::new(),
            updates,
            on_change,
            reports,
            on_focus,
            focused: None,
            written: Vec::new(),
            lassos: Vec::new(),
            controls: WidgetControls::default(),
        })
    }

    /// The chart that the app shows.
    pub fn chart(&self) -> &Chart {
        self.evaluator.chart()
    }

    /// The evaluator of the chart, with the views of its scales and the clauses of its
    /// selections.
    pub fn evaluator(&self) -> &ChartEvaluator {
        &self.evaluator
    }

    /// The plots of the last evaluation.
    pub fn plots(&self) -> &[EvaluatedPlot] {
        &self.plots
    }

    /// The warnings of the last evaluation.
    pub fn warnings(&self) -> &[EvalWarning] {
        &self.warnings
    }

    /// The interaction mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Installs `update` in place of the current chart, which carries the views of its scales and
    /// the clauses of its selections by the update's options.
    pub(crate) fn install(&mut self, update: PreparedUpdate) -> Result<(), EvalError> {
        let PreparedUpdate { chart, options } = update;
        let declarations = declarations(chart.chart());
        self.evaluator.install(chart, &options)?;
        self.declarations = declarations;
        self.moved.clear();
        Ok(())
    }

    /// The declarations of the last evaluation whose owners have identities, which their tools
    /// key their state by.
    pub(crate) fn declaring(&self) -> impl Iterator<Item = Declared<'_>> {
        self.declarations.iter().filter_map(|declaration| {
            let plot = self.plots.get(declaration.plot)?;
            let (owner, marks, coords) = match declaration.layer {
                None => {
                    let marks = (plot.layers.iter())
                        .flat_map(|layer| layer.marks.iter().cloned())
                        .collect();
                    (plot.identity.as_ref()?, marks, &plot.coords)
                }
                Some(position) => {
                    let layer = plot.layers.get(position)?;
                    (layer.identity.as_ref()?, layer.marks.clone(), &layer.coords)
                }
            };
            Some(Declared {
                index: declaration.plot,
                plot,
                owner,
                marks,
                coords,
                tools: &declaration.tools,
                plot_tools: declaration.layer.is_none(),
            })
        })
    }

    /// The index and the plot of the last evaluation with `identity`.
    pub(crate) fn plot(&self, identity: &Identity) -> Option<(usize, &EvaluatedPlot)> {
        (self.plots.iter().enumerate()).find(|(_, plot)| plot.identity.as_ref() == Some(identity))
    }

    /// The declaration of the last evaluation whose owner is `owner`.
    pub(crate) fn declared(&self, owner: &Identity) -> Option<Declared<'_>> {
        self.declaring().find(|declared| declared.owner == owner)
    }

    /// The resolution of the selection `name`.
    pub(crate) fn resolution(&self, name: &str) -> Resolution {
        (self.evaluator.selections().get(name)).map_or(Resolution::Union, |state| state.resolution)
    }

    /// The clause of the selection `name` that `owner` shows: the one clause of a global
    /// selection, or else its own.
    pub(crate) fn clause(&self, name: &str, owner: &Identity) -> Option<&Clause> {
        let state = self.evaluator.selections().get(name)?;
        match state.resolution {
            Resolution::Global => state.clauses.first(),
            _ => (state.clauses.iter()).find(|clause| wrote(owner, clause)),
        }
    }

    /// Writes the clause of `owner` to the selection `name`: `conditions`, or no clause without
    /// conditions. It replaces every clause of a global selection, and otherwise the owner's own,
    /// and the next report holds the selection.
    pub(crate) fn write(&mut self, name: &str, owner: &Identity, conditions: Vec<Condition>) {
        let resolution = self.resolution(name);
        let mut selections = self.evaluator.selections().clone();
        let state = (selections.entry(name.to_string()))
            .or_insert_with(|| SelectionState::new(resolution, Vec::new()));
        match resolution {
            Resolution::Global => state.clauses.clear(),
            _ => state.clauses.retain(|clause| !wrote(owner, clause)),
        }
        if !conditions.is_empty() {
            let mut clause = Clause::new(owner.id.clone());
            clause.partition = (owner.partition.iter())
                .map(|(column, value)| (column.clone(), Scalar(value.clone())))
                .collect();
            clause.conditions = conditions;
            state.clauses.push(clause);
        }
        self.evaluator.set_selections(selections);
        self.touch(name);
    }

    /// Holds the selection `name` in the next report, such as a selection that a navigation
    /// wrote before it settled.
    pub(crate) fn touch(&mut self, name: &str) {
        if !self.written.iter().any(|written| written == name) {
            self.written.push(name.to_string());
        }
    }

    /// Sets the state of the selection `name`, or removes it for `None`.
    pub(crate) fn set_selection(&mut self, name: &str, state: Option<SelectionState>) {
        let mut selections: BTreeMap<String, SelectionState> = self.evaluator.selections().clone();
        match state {
            Some(state) => selections.insert(name.to_string(), state),
            None => selections.remove(name),
        };
        self.evaluator.set_selections(selections);
    }

    /// Reports `focus` when it differs from the focus that the app last reported.
    pub(crate) fn focus(&mut self, focus: Option<SelectionFocus>) {
        if self.focused == focus {
            return;
        }
        self.focused = focus;
        if let Some(on_focus) = &self.on_focus {
            on_focus(self.focused.as_ref());
        }
    }

    /// Reports `change` with the selections written since the last report and their clauses. It
    /// reports nothing without a domain, a projection, or a selection, and with
    /// [`Reports::Settled`], nothing before the gesture settles, whose report holds the selections
    /// written until then.
    pub(crate) fn report(&mut self, mut change: ChartChange) {
        if self.reports == Reports::Settled && !change.settled {
            return;
        }
        change.selections = std::mem::take(&mut self.written)
            .into_iter()
            .map(|name| {
                let clauses =
                    (self.evaluator.selections().get(&name)).map(|state| state.clauses.clone());
                (name, clauses.unwrap_or_default())
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
