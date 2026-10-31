//! `ChartEvaluator`, which prepares a chart one time and evaluates it at any size, and the
//! `EvaluatedChart` of each evaluation.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use arrow::array::{ArrayRef, RecordBatch};
use avenger_coords::PlotProjection;
use avenger_geometry::data::SceneData;
use avenger_layout::{Edges, Measured, Rect, Size, SolveOptions};
use avenger_scales::scales::{
    domain_summary::{CategoryOptions, CategoryOrder, DomainOptions},
    fitting::FittingScale,
    ConfiguredScale,
};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
    },
    scene_graph::SceneGraph,
};
use avenger_typst_label::LabelEngine;

use avenger_chart::{
    proto::{self, domain, encoding, scale_navigation, Channel},
    resolve::{self, Identity, PlotRef, ScaleGroup, Tree, UnitRef},
    table::{self, Matches, Tables},
    Chart, Contribution, CoordKind, FitKind, TupleTest,
};

use crate::{
    coord::{CoordSystems, PlotUnit, ProjectionGuide, SystemInputs},
    encode::{self, at, UnitRows},
    fit::{self, FitValues, Position, Reach},
    guides::{self, GuideInputs, Guides},
    layout::{self, Lowered, Lowering, Measurer, SlabId},
    scale::{self, eval_error},
    select::{self, Contributions, Selections},
    update::{union_categories, Carry, Fit, UpdateOptions},
    EvalError, Theme,
};

type Result<T> = std::result::Result<T, EvalError>;

/// A warning of an evaluation, at the path of the part of the chart that it concerns.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationWarning {
    pub path: String,
    pub message: String,
}

/// The coordinate system of a plot at the size of an evaluation. A feature adds the coordinate
/// systems of its plots, so a match has an arm for the others.
#[derive(Clone, Debug)]
#[non_exhaustive]
// An evaluation holds one for each plot, so the size of the Cartesian scales costs no more than an
// indirection would.
#[allow(clippy::large_enum_variant)]
pub enum PlotCoord {
    /// The configured position scales of a Cartesian plot, which map data to pixels from the
    /// plot's top left. An overlay whose children have independent scales of `x` or `y` has a
    /// second scale of the channel, whose axis stands on the top or the right unless its spec
    /// sets a side. The plots of a shared scale take one length along it, so each has the same
    /// configured scale.
    Cartesian {
        x: Option<ConfiguredScale>,
        y: Option<ConfiguredScale>,
        second_x: Option<ConfiguredScale>,
        second_y: Option<ConfiguredScale>,
    },
    /// The projection of a map plot, fitted to the plot at its size, which maps longitudes and
    /// latitudes to pixels from the plot's top left. The map plots of a shared projection take
    /// one size, so each has the same projection.
    #[cfg(feature = "geo")]
    Geo { projection: avenger_geo::Projection },
    /// The scales and the circle of a polar plot at its size: `theta` maps data to degrees
    /// clockwise from the top, and `radius` maps data to pixels from the center, which is in
    /// pixels from the plot's top left. The polar plots that share a scale take one outer
    /// radius, so each has the same configured scale.
    Polar {
        theta: Option<ConfiguredScale>,
        radius: Option<ConfiguredScale>,
        center: [f32; 2],
        inner_radius: f32,
        outer_radius: f32,
    },
}

impl PlotCoord {
    /// The kind of the coordinate system.
    pub fn kind(&self) -> CoordKind {
        match self {
            PlotCoord::Cartesian { .. } => CoordKind::Cartesian,
            #[cfg(feature = "geo")]
            PlotCoord::Geo { .. } => CoordKind::Geo,
            PlotCoord::Polar { .. } => CoordKind::Polar,
        }
    }

    /// The scale of the positional channel `channel`, when the plot draws it through one.
    pub fn scale(&self, channel: Channel) -> Option<&ConfiguredScale> {
        match (self, channel) {
            (PlotCoord::Cartesian { x, .. }, Channel::X) => x.as_ref(),
            (PlotCoord::Cartesian { y, .. }, Channel::Y) => y.as_ref(),
            (PlotCoord::Polar { theta, .. }, Channel::Theta) => theta.as_ref(),
            (PlotCoord::Polar { radius, .. }, Channel::Radius) => radius.as_ref(),
            _ => None,
        }
    }
}

/// A plot of an evaluation: its name and identity, its rectangle in the scene, its coordinate
/// system, and the projection of its pixels.
#[derive(Clone, Debug)]
pub struct EvaluatedPlot {
    /// The name of its identity, or its node path when its node has no id, which names its group
    /// in the scene and, followed by `/` and the position of a unit in the plot, the unit's marks.
    pub name: String,
    /// The identity of its node, when it has an id.
    pub identity: Option<Identity>,
    /// The names of its marks with rows in `EvaluatedChart::data`, in the order that it draws
    /// them.
    pub marks: Vec<String>,
    /// Its left, top, width, and height in the scene.
    pub rect: [f32; 4],
    pub coord: PlotCoord,
    /// The projection between its pixels, from its top left, and the range positions of its two
    /// positional dimensions, the channels of the positions of its kind, whose scales map their
    /// data to ranges that span the dimensions' extents. Tools such as brushes and navigations
    /// work in its range positions. `None` for a plot whose pixels map to no positions.
    pub projection: Option<Arc<dyn PlotProjection>>,
}

/// A chart evaluated at one size: its scene, the rows behind its marks, its plots, and the
/// warnings of the evaluation.
pub struct EvaluatedChart {
    pub scene: SceneGraph,
    /// The rows of each mark's table that draw its instances, by the mark's name.
    pub data: SceneData,
    pub plots: Vec<EvaluatedPlot>,
    pub warnings: Vec<EvaluationWarning>,
}

/// The identity of a scale, which carries its state, such as the view of a pan or a zoom, to a new
/// chart.
#[derive(Clone, Debug, PartialEq)]
pub enum ScaleIdentity {
    /// The share family that the specs of the scale name for its channel.
    Family { family: String, channel: Channel },
    /// The first plot of the scale with an identity, in tree order, and the scale's channel.
    /// `second` tells the second scale of the channel of an overlay from the first.
    Plot {
        plot: Identity,
        channel: Channel,
        second: bool,
    },
}

/// A chart prepared to evaluate at any size.
///
/// `ChartEvaluator::new` prepares a chart, as [`PreparedChart::new`] does: it groups the chart's
/// channels into scales, configures its non-position scales, builds the summary of each position
/// scale with the reach of its marks, prepares its guides, and builds its layout. `install`
/// replaces the chart with another prepared chart, and carries the state of the old one by its
/// options. `evaluate` solves the layout at a size, fits each position scale at the length of its
/// plots, and encodes the marks, so one evaluator evaluates a chart at every size.
///
/// A continuous scale of `x` or `y` can have a view, such as a pan or a zoom of the scale: a
/// configured scale whose domain replaces the fitted domain at every length. Every plot of the
/// scale shows the view and clips its marks, and the axes of the scale show the view's domain.
pub struct ChartEvaluator {
    chart: Chart,
    engine: LabelEngine,
    theme: Theme,
    contributions: Contributions,
    prepared: Prepared,
    measured: Option<Measured<SlabId>>,
    /// Whether the layout is lowered again since the last measure, so the next evaluation
    /// measures it with the answers of the last measure that still hold.
    relowered: bool,
    /// The view of each position group that has one, by group.
    views: ScaleViews,
    /// The last revision of an axis, which each new view of an axis takes the next of.
    revision: u64,
    /// The rectangles of the plots at the last evaluation.
    rects: Vec<Rect>,
}

/// The view of each position group that has one, by group.
pub(crate) type ScaleViews = BTreeMap<usize, ConfiguredScale>;

/// The navigation of a scale at the size of its plot: the bounds that its domain stays inside, and
/// its smallest width, in the numeric values of the scale, such as epoch milliseconds for a time
/// scale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScaleNavigation {
    pub bounds: Option<[f64; 2]>,
    pub min_span: Option<f64>,
}

/// What an evaluator prepares from its chart, its theme, and its contributions.
struct Prepared {
    groups: Vec<ScaleGroup>,
    /// The position scale of each position group, by group.
    positions: BTreeMap<usize, Position>,
    /// The values of each position group with the reach of their marks, for its fit.
    fit_values: BTreeMap<usize, FitValues>,
    /// The configured scale of each non-position group, by group.
    scales: BTreeMap<usize, ConfiguredScale>,
    /// The values of each non-position group, for its fit.
    scale_values: BTreeMap<usize, Vec<Option<ArrayRef>>>,
    /// What the evaluator prepares for each unit, in tree order.
    units: Vec<PreparedUnit>,
    /// The coordinate systems of the plots.
    systems: CoordSystems,
    guides: Guides,
    tables: Tables,
    matches: Matches,
    lowered: Lowered,
    /// The first group of each channel of each plot, by plot.
    plot_groups: Vec<BTreeMap<Channel, usize>>,
    /// The identity of each group that has one, by group.
    identities: BTreeMap<usize, ScaleIdentity>,
}

impl Prepared {
    /// Lowers the layout again from the guides.
    fn lower(&mut self, chart: &Chart, theme: &Theme) {
        let tree = resolve::tree(chart);
        self.lowered = layout::lower(&Lowering {
            chart,
            tree: &tree,
            groups: &self.groups,
            guides: &self.guides,
            tables: &self.tables,
            matches: &self.matches,
            theme,
        });
    }

    /// The scale of the position group `group` of `plot` fitted at `size`, the plot's size, by the
    /// coordinate system of the plot.
    fn fitted(&self, plot: usize, group: usize, size: [f32; 2]) -> Option<ConfiguredScale> {
        (self.systems).home_scale(&self.positions, plot, group, size)
    }

    /// Whether the position group `group` can have a view: a continuous scale of a navigated
    /// channel, such as `x` or `y` of a Cartesian plot, whose domain does not read a selection.
    fn viewable(&self, group: usize) -> bool {
        let group = &self.groups[group];
        group.is_navigated()
            && FitKind::of(&group.spec.r#type) == FitKind::Interval
            && !selection_domain(&group.spec)
    }
}

/// What an evaluator prepares for a unit: the rows that draw instances, their reach, and the
/// group of each of its channels that goes through a scale.
struct PreparedUnit {
    drawn: RecordBatch,
    reach: Option<Vec<Reach>>,
    groups: BTreeMap<Channel, usize>,
}

/// The values of the encoding of `channel` in `unit`: its column in `table`, or its scaled
/// constant.
fn member_values(
    chart: &Chart,
    unit: &proto::Unit,
    channel: Channel,
    table: &RecordBatch,
) -> Option<ArrayRef> {
    let encoding = unit
        .encodings
        .iter()
        .find(|encoding| encoding.channel == channel as i32)?;
    match encoding.value.as_ref()? {
        encoding::Value::Column(name) => table.column_by_name(name).cloned(),
        encoding::Value::ScaledConstant(index) => Some(chart.value(*index).clone()),
        encoding::Value::Constant(_) => None,
    }
}

fn mark(unit: &proto::Unit) -> Result<&proto::mark::Mark> {
    unit.mark
        .as_ref()
        .and_then(|mark| mark.mark.as_ref())
        .ok_or_else(|| EvalError::Evaluation("a unit has no mark".to_string()))
}

/// The name of the marks of a unit: the name of its plot and its position in the plot.
fn mark_name(plot: &PlotRef, unit: &UnitRef) -> String {
    format!("{}/{}", plot.name, unit.position)
}

/// The mark of the first member of a group, whose kind gives the theme's range of a size.
fn group_mark<'a>(units: &[UnitRef<'a>], group: &ScaleGroup) -> Result<&'a proto::mark::Mark> {
    let member = group
        .members
        .first()
        .ok_or_else(|| EvalError::Evaluation("a scale has no encodings".to_string()))?;
    mark(units[member.unit].unit)
}

/// A chart prepared to install in a [`ChartEvaluator`]: its scale groups, its scales fitted to its
/// rows, its guides, and its layout. They depend only on the chart, the label engine, and the
/// theme, so a chart prepares on any thread, and the render loop only installs it. A scale whose
/// domain reads a selection fits to its fallback until an installation gives it the selection.
pub struct PreparedChart {
    chart: Chart,
    prepared: Prepared,
}

impl PreparedChart {
    /// Prepares `chart` with `engine` and `theme`, the label engine and the theme of the
    /// evaluator that installs it.
    pub fn new(chart: &Chart, engine: &LabelEngine, theme: &Theme) -> Result<Self> {
        Ok(Self {
            chart: chart.clone(),
            prepared: prepare(chart, &Contributions::new(), engine, theme)?,
        })
    }

    /// The chart that the preparation prepared.
    pub fn chart(&self) -> &Chart {
        &self.chart
    }
}

impl ChartEvaluator {
    /// Prepares `chart` to evaluate with `engine` and `theme`, with the initial contributions of
    /// its selections. The evaluator draws units, overlays, rows, columns, and wraps of Cartesian
    /// and polar plots, and of map plots with the `geo` feature.
    pub fn new(chart: &Chart, engine: &LabelEngine, theme: &Theme) -> Result<Self> {
        let PreparedChart { chart, prepared } = PreparedChart::new(chart, engine, theme)?;
        let mut evaluator = Self {
            contributions: chart.initial_contributions(),
            chart,
            engine: engine.clone(),
            theme: theme.clone(),
            prepared,
            measured: None,
            relowered: false,
            views: BTreeMap::new(),
            revision: 0,
            rects: Vec::new(),
        };
        evaluator.fit_selection_domains()?;
        Ok(evaluator)
    }

    /// Installs `prepared` in place of the chart, and carries the state of the old chart by
    /// `options`: the fitted domains of its scales, the views of pans and zooms, and the
    /// contributions of its selections, as they stand at the installation. A scale carries over
    /// when the new chart has a scale with its identity and its scale type. The first evaluation
    /// measures the new chart's layout.
    pub fn install(&mut self, prepared: PreparedChart, options: &UpdateOptions) -> Result<()> {
        let PreparedChart {
            chart,
            prepared: mut new,
        } = prepared;
        let old = &self.prepared;
        let mut views = BTreeMap::new();
        let mut refitted = false;
        for (index, identity) in new.identities.clone() {
            let Some(old_index) = (old.identities.iter())
                .find(|(_, own)| **own == identity)
                .map(|(group, _)| *group)
            else {
                continue;
            };
            let group = &new.groups[index];
            if old.groups[old_index].spec.r#type != group.spec.r#type {
                continue;
            }
            if let (true, Some(view)) = (options.keeps_views(), self.views.get(&old_index)) {
                if new.viewable(index) {
                    let scale = new.positions[&index].fitting.scale();
                    views.insert(index, scale.clone().with_domain(view.domain().clone()));
                }
            }
            let fit = options.fit_for(&identity);
            if fit == Fit::Refit || selection_domain(&group.spec) || explicit_domain(&group.spec) {
                continue;
            }
            if group.is_position() {
                let (Some(old_position), Some(new_position)) =
                    (old.positions.get(&old_index), new.positions.get(&index))
                else {
                    continue;
                };
                let position = carry_position(&chart, group, fit, old_position, new_position)?;
                new.positions.insert(index, position);
            } else {
                let (Some(old_scale), Some(new_scale)) =
                    (old.scales.get(&old_index), new.scales.get(&index))
                else {
                    continue;
                };
                new.scales
                    .insert(index, carry_scale(group, fit, old_scale, new_scale)?);
            }
            refitted = true;
        }
        let contributions = self.carry_contributions(&chart, options);
        self.chart = chart;
        self.prepared = new;
        self.contributions = contributions;
        self.views = views;
        self.measured = None;
        self.relowered = false;
        self.rects.clear();
        if self.has_selection_domains() {
            self.refit()
        } else if refitted {
            self.reguide()
        } else {
            Ok(())
        }
    }

    /// The contributions of the selections of the new chart `chart` by `options`, from the
    /// contributions of the evaluator and the initial contributions of `chart`.
    fn carry_contributions(&self, chart: &Chart, options: &UpdateOptions) -> Contributions {
        let tree = resolve::tree(chart);
        let initial = chart.initial_contributions();
        let global = |name: &str| {
            (chart.selections().iter())
                .chain(self.chart.selections())
                .find(|selection| selection.name == name)
                .is_some_and(|selection| {
                    selection.resolution == proto::SelectionResolution::Global as i32
                })
        };
        // A contribution whose owner is a plot of the new chart, which can draw and clear it.
        let owned = |contribution: &Contribution| {
            (tree.plots.iter())
                .any(|plot| (plot.identity.as_ref()).is_some_and(|plot| plot.wrote(contribution)))
        };
        let names: BTreeSet<&String> = self.contributions.keys().chain(initial.keys()).collect();
        let mut contributions = Contributions::new();
        for name in names {
            let carried = match (options.carry_for(name), self.contributions.get(name)) {
                (Carry::Keep, Some(kept)) if global(name) => Some(kept.clone()),
                (Carry::Keep, Some(kept)) => {
                    Some(kept.iter().filter(|c| owned(c)).cloned().collect())
                }
                (Carry::Keep, None) | (Carry::Reset, _) => initial.get(name).cloned(),
            };
            if let Some(carried) = carried {
                contributions.insert(name.clone(), carried);
            }
        }
        contributions
    }

    /// Whether a scale's domain reads a selection.
    fn has_selection_domains(&self) -> bool {
        (self.prepared.groups.iter()).any(|group| selection_domain(&group.spec))
    }

    /// Fits the scales whose domains are selection domains to the contributions, when the chart
    /// has such a scale.
    fn fit_selection_domains(&mut self) -> Result<()> {
        if self.has_selection_domains() {
            self.refit()?;
        }
        Ok(())
    }

    /// The chart of the evaluator.
    pub fn chart(&self) -> &Chart {
        &self.chart
    }

    /// The contributions of the evaluator's selections, by name.
    pub fn selections(&self) -> &BTreeMap<String, Vec<Contribution>> {
        &self.contributions
    }

    /// Replaces the contributions of the evaluator's selections. A selection without an entry has
    /// no contributions. The conditions of encodings take them at the next evaluation, and the
    /// scales whose domains are selection domains fit again.
    pub fn set_selections(
        &mut self,
        contributions: BTreeMap<String, Vec<Contribution>>,
    ) -> Result<()> {
        self.contributions = contributions;
        self.fit_selection_domains()
    }

    /// The view of the scale of `channel`, `x` or `y`, of the plot at index `plot` of the
    /// evaluation, to move, as a pan or a zoom does. A scale without a view takes its fitted scale
    /// at the plot's size at the last evaluation as its view. A plot's first scale of the channel
    /// has a view when it is continuous and its domain does not read a selection, and the plot has
    /// been evaluated, and otherwise this is `None`.
    pub fn scale_view_mut(
        &mut self,
        plot: usize,
        channel: Channel,
    ) -> Option<&mut ConfiguredScale> {
        let group = self.plot_group(plot, channel)?;
        if !self.prepared.viewable(group) {
            return None;
        }
        if !self.views.contains_key(&group) {
            let fitted = self.prepared.fitted(plot, group, self.size(plot)?)?;
            self.views.insert(group, fitted);
        }
        self.views.get_mut(&group)
    }

    /// Removes the view of the scale of `channel` of the plot at index `plot`, so that the scale
    /// fits its domain again.
    pub fn clear_scale_view(&mut self, plot: usize, channel: Channel) {
        if let Some(group) = self.plot_group(plot, channel) {
            self.views.remove(&group);
        }
    }

    /// The view of the scale of `channel` of the plot at index `plot`, when it has one.
    pub fn scale_view(&self, plot: usize, channel: Channel) -> Option<&ConfiguredScale> {
        self.views.get(&self.plot_group(plot, channel)?)
    }

    /// The identity of the scale of `channel` of the plot at index `plot`, when it has one.
    pub fn scale_identity(&self, plot: usize, channel: Channel) -> Option<&ScaleIdentity> {
        self.prepared
            .identities
            .get(&self.plot_group(plot, channel)?)
    }

    /// The selection whose values give the domain of the scale of `channel` of the plot at index
    /// `plot`, when its domain is a selection domain. A navigation of such a scale moves the
    /// selection, because the scale has no view.
    pub fn domain_selection(&self, plot: usize, channel: Channel) -> Option<&str> {
        let group = &self.prepared.groups[self.plot_group(plot, channel)?];
        match group.spec.domain.as_ref()?.domain.as_ref()? {
            domain::Domain::Selection(selection) => Some(&selection.selection),
            _ => None,
        }
    }

    /// The column that a selection through `channel` of the plot at index `plot` records: the
    /// selection field of the plot's first encoding of the channel, or its encoded column.
    pub fn selection_column(&self, plot: usize, channel: Channel) -> Option<&str> {
        let tree = resolve::tree(&self.chart);
        tree.units
            .iter()
            .filter(|unit| unit.plot == plot)
            .find_map(|unit| {
                let encoding = (unit.unit.encodings.iter())
                    .find(|encoding| encoding.channel == channel as i32)?;
                if !encoding.selection_field.is_empty() {
                    return Some(encoding.selection_field.as_str());
                }
                match encoding.value.as_ref()? {
                    encoding::Value::Column(column) => Some(column.as_str()),
                    _ => None,
                }
            })
    }

    /// Whether every row that the units of the plot at index `plot` draw matches `test`, with the
    /// partition values of the units, as a set of the plot's partition value does. A plot shows a
    /// tuple of a selection when it draws some of its tests and every other test holds there.
    pub fn satisfies(&self, plot: usize, test: &TupleTest) -> Result<bool> {
        let tree = resolve::tree(&self.chart);
        for (index, unit) in tree.units.iter().enumerate() {
            if unit.plot != plot {
                continue;
            }
            let rows = select::Rows {
                table: &self.prepared.units[index].drawn,
                partition: &unit.partition,
                owners: &unit.owners,
                path: &unit.path,
            };
            let mask = select::test_mask(test, &rows, "", &mut Vec::new())?;
            if !mask.into_iter().all(|selected| selected) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The fitted scale of `channel` of the plot at index `plot` at its size at the last
    /// evaluation, without its view, which a navigation returns to.
    pub fn home_scale(&self, plot: usize, channel: Channel) -> Option<ConfiguredScale> {
        let group = self.plot_group(plot, channel)?;
        self.prepared.fitted(plot, group, self.size(plot)?)
    }

    /// The width and the height of the plot at index `plot` at the last evaluation.
    fn size(&self, plot: usize) -> Option<[f32; 2]> {
        let rect = self.rects.get(plot)?;
        Some([rect.width, rect.height])
    }

    /// The navigation of the scale of `channel` of the plot at index `plot`, from the scale's
    /// spec, at the plot's size at the last evaluation: the bounds that its domain stays inside,
    /// with the fitted domain in place of data bounds, and its smallest width, in the numeric
    /// values of the scale. A scale whose domain is a selection domain has no data bounds, because
    /// its fitted domain is the selection. This is `None` for a plot without a scale of the
    /// channel.
    pub fn scale_navigation(&self, plot: usize, channel: Channel) -> Option<ScaleNavigation> {
        let group = self.plot_group(plot, channel)?;
        let spec = &self.prepared.groups[group].spec;
        let Some(navigation) = &spec.navigation else {
            return Some(ScaleNavigation::default());
        };
        let bounds = match &navigation.bounds {
            Some(scale_navigation::Bounds::Data(_)) if !selection_domain(spec) => self
                .home_scale(plot, channel)
                .and_then(|home| home.numeric_interval_domain().ok())
                .map(|(start, end)| [start.min(end), start.max(end)]),
            Some(scale_navigation::Bounds::Interval(interval)) => {
                Some([interval.low, interval.high])
            }
            Some(scale_navigation::Bounds::Data(_)) | None => None,
        };
        Some(ScaleNavigation {
            bounds,
            min_span: navigation.min_span,
        })
    }

    /// The first group of `channel` of the plot at index `plot`.
    fn plot_group(&self, plot: usize, channel: Channel) -> Option<usize> {
        self.prepared.plot_groups.get(plot)?.get(&channel).copied()
    }

    /// Shows the view of each axis whose scale has a new view or lost its view, and lowers the
    /// layout again when an axis changes.
    fn show_views(&mut self) -> Result<()> {
        let prepared = &mut self.prepared;
        let mut changed = false;
        for axis in &mut prepared.guides.axes {
            let view = self.views.get(&axis.group);
            let shown = axis.view.as_ref().map(|domain| domain.as_ref());
            if shown == view.map(|view| view.domain().as_ref()) {
                continue;
            }
            self.revision += 1;
            let fitting = &prepared.positions[&axis.group].fitting;
            axis.set_view(view, fitting, self.revision, &self.engine)?;
            changed = true;
        }
        if changed {
            prepared.lower(&self.chart, &self.theme);
            self.relowered = true;
        }
        Ok(())
    }

    /// Loads the raster tiles of the chart's map plots through `resolver`, the image resolver of
    /// the host, whose cache follows the terms of the tile servers. A map plot with tiles needs
    /// one.
    #[cfg(feature = "geo")]
    pub fn set_image_resolver(
        &mut self,
        resolver: Arc<dyn avenger_image::ImageResolver>,
    ) -> Result<()> {
        self.prepared.systems.set_image_resolver(resolver)
    }

    /// Fits the scales whose domains are selection domains again, with the stored values of
    /// their groups, and prepares the guides and the layout again.
    fn refit(&mut self) -> Result<()> {
        let chart = &self.chart;
        let tree = resolve::tree(chart);
        let selections = Selections {
            chart,
            contributions: &self.contributions,
        };
        let prepared = &mut self.prepared;
        for (index, group) in prepared.groups.iter().enumerate() {
            if !selection_domain(&group.spec) {
                continue;
            }
            if group.is_position() {
                let values = &prepared.fit_values[&index];
                prepared.positions.insert(
                    index,
                    fit::position(
                        chart,
                        group,
                        values,
                        &selections,
                        &self.theme,
                        CoordSystems::wraps(&tree, group),
                    )?,
                );
            } else {
                let values = &prepared.scale_values[&index];
                prepared.scales.insert(
                    index,
                    scale::non_position(
                        chart,
                        group,
                        group_mark(&tree.units, group)?,
                        values,
                        &selections,
                        &self.theme,
                    )?,
                );
            }
        }
        self.reguide()
    }

    /// Prepares the guides, the layout, and the axes of the polar plots again from the scales.
    fn reguide(&mut self) -> Result<()> {
        let chart = &self.chart;
        let tree = resolve::tree(chart);
        let prepared = &mut self.prepared;
        prepared.guides = guides(
            chart,
            &tree,
            &prepared.groups,
            &prepared.positions,
            &prepared.scales,
            &prepared.systems.guides(),
            &self.theme,
            &self.engine,
        )?;
        prepared.lower(chart, &self.theme);
        prepared.systems.rescale(&SystemInputs {
            chart,
            tree: &tree,
            groups: &prepared.groups,
            positions: &prepared.positions,
            theme: &self.theme,
            engine: &self.engine,
        })?;
        self.measured = None;
        Ok(())
    }

    /// Evaluates the chart at `size`, its width and height in pixels.
    pub fn evaluate(&mut self, size: [f32; 2]) -> Result<EvaluatedChart> {
        self.show_views()?;
        let layout_error = |error| eval_error("the layout of the chart", error);
        let mut measurer = Measurer {
            guides: &self.prepared.guides,
        };
        let solution = match (&mut self.measured, self.relowered) {
            (Some(measured), false) => measured
                .remeasure(Size::new(size[0], size[1]), &mut measurer)
                .map_err(layout_error)?
                .clone(),
            (previous, _) => {
                let options = SolveOptions {
                    padding: Edges::all(self.theme.padding),
                    ..SolveOptions::sized(size[0], size[1])
                };
                let layout = &self.prepared.lowered.layout;
                let measured = match previous {
                    Some(previous) => layout.measure_reusing(previous, &options, &mut measurer),
                    None => layout.measure(&options, &mut measurer),
                }
                .map_err(layout_error)?;
                let solution = measured.solution().clone();
                self.measured = Some(measured);
                self.relowered = false;
                solution
            }
        };
        let chart = &self.chart;
        let tree = resolve::tree(chart);
        let mut warnings = Vec::new();
        let rects = (0..tree.plots.len())
            .map(|plot| {
                solution
                    .region(&SlabId::Plot(plot))
                    .map(|region| region.content)
                    .ok_or_else(|| EvalError::Evaluation("the layout has no plot".to_string()))
            })
            .collect::<Result<Vec<_>>>()?;
        let sizes: Vec<[f32; 2]> = rects.iter().map(|rect| [rect.width, rect.height]).collect();
        self.rects.clone_from(&rects);

        // The coordinate systems fit the projections and the position scales of the plots at
        // their sizes, and each view of a scale keeps its domain at the length of its plots.
        let prepared = &mut self.prepared;
        (prepared.systems).fit(&prepared.positions, &mut self.views, &sizes, &mut warnings)?;
        let prepared = &self.prepared;
        let selections = Selections {
            chart,
            contributions: &self.contributions,
        };
        let mut marks: Vec<SceneMark> = Vec::new();
        let mut plots = Vec::with_capacity(tree.plots.len());
        let mut data = SceneData::new();
        let fitted_plots = (0..tree.plots.len())
            .map(|plot| prepared.systems.fitted(plot))
            .collect::<Result<Vec<_>>>()?;

        for (plot_index, plot_ref) in tree.plots.iter().enumerate() {
            let plot = rects[plot_index];
            let plot_size = [plot.width, plot.height];
            let fitted = &fitted_plots[plot_index];

            let mut mark_names = Vec::new();
            let mut plot_marks: Vec<SceneMark> = fitted.backdrop()?;
            plot_marks.extend(
                prepared.guides.plots[plot_index]
                    .axes
                    .iter()
                    .map(|axis| &prepared.guides.axes[*axis])
                    .filter(|axis| axis.grid)
                    .map(|axis| SceneMark::from(axis.axis.grid(plot_size))),
            );
            for (index, unit) in tree.units.iter().enumerate() {
                if unit.plot != plot_index {
                    continue;
                }
                let prepared_unit = &prepared.units[index];
                // The rows that the plot draws, such as the rows of a mark at a place in a map plot
                // whose place the projection keeps.
                let (table, projected) = fitted.locate(chart, unit.unit, &prepared_unit.drawn)?;
                // The rows that each condition selects.
                let rows = select::Rows {
                    table: &table,
                    partition: &unit.partition,
                    owners: &unit.owners,
                    path: &unit.path,
                };
                let mut masks = BTreeMap::new();
                for encoding in &unit.unit.encodings {
                    if encoding.conditions.is_empty() {
                        continue;
                    }
                    let channel =
                        Channel::try_from(encoding.channel).unwrap_or(Channel::Unspecified);
                    let channel_masks = encoding
                        .conditions
                        .iter()
                        .map(|condition| match &condition.predicate {
                            Some(predicate) => selections.mask(predicate, &rows, &mut warnings),
                            None => Ok(vec![false; table.num_rows()]),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    masks.insert(channel, channel_masks);
                }

                let channel_scales = channel_scales(prepared_unit, &prepared.scales);
                let positions = (prepared_unit.groups.iter())
                    .filter_map(|(channel, group)| Some((*channel, fitted.scale(*group)?)))
                    .collect();
                let mut unit_rows = UnitRows {
                    chart,
                    unit: unit.unit,
                    mark: mark(unit.unit)?,
                    table,
                    positions,
                    projected,
                    scales: &channel_scales,
                    masks,
                    theme: &self.theme,
                    engine: &self.engine,
                    size: plot_size,
                    name: mark_name(plot_ref, unit),
                };
                for drawn in fitted.marks(&mut unit_rows)? {
                    let name = drawn.mark.mark_name().to_string();
                    data.insert(&name, Arc::new(drawn.rows));
                    mark_names.push(name);
                    plot_marks.push(drawn.mark);
                }
                check_summary_reach(&unit_rows, index, prepared, &unit.path, &mut warnings)?;
            }
            // What the plot draws over its data, such as the axes of a polar plot inside its box.
            plot_marks.extend(fitted.overlay());

            marks.push(
                SceneGroup {
                    name: plot_ref.name.clone(),
                    origin: [plot.x, plot.y],
                    clip: if fitted.clips() {
                        Clip::Rect {
                            x: 0.0,
                            y: 0.0,
                            width: plot.width,
                            height: plot.height,
                        }
                    } else {
                        Clip::None
                    },
                    marks: plot_marks,
                    ..Default::default()
                }
                .into(),
            );

            plots.push(EvaluatedPlot {
                name: plot_ref.name.clone(),
                identity: plot_ref.identity.clone(),
                marks: mark_names,
                rect: [plot.x, plot.y, plot.width, plot.height],
                coord: fitted.coord(),
                projection: fitted.projection([plot.width, plot.height]),
            });
        }
        marks.extend(layout::draw(&prepared.guides, &prepared.lowered, &solution));
        // The slabs of each projection, such as the labels of a graticule, through the
        // projection of its plot.
        for id in &prepared.lowered.slabs {
            let SlabId::Projection { plot, side } = id else {
                continue;
            };
            let Some(region) = solution.region(id) else {
                continue;
            };
            let rect = region.content;
            let slot = [rect.x, rect.y, rect.width, rect.height];
            if let Some(group) = fitted_plots[*plot].slab(*side, slot) {
                marks.push(group.into());
            }
        }
        Ok(EvaluatedChart {
            scene: SceneGraph {
                marks,
                width: size[0],
                height: size[1],
                origin: [0.0, 0.0],
            },
            data,
            plots,
            warnings,
        })
    }
}

/// Whether the domain of `spec` reads a selection.
fn selection_domain(spec: &proto::ScaleSpec) -> bool {
    matches!(
        spec.domain
            .as_ref()
            .and_then(|domain| domain.domain.as_ref()),
        Some(domain::Domain::Selection(_))
    )
}

/// Whether the domain of `spec` is explicit.
fn explicit_domain(spec: &proto::ScaleSpec) -> bool {
    matches!(
        spec.domain
            .as_ref()
            .and_then(|domain| domain.domain.as_ref()),
        Some(domain::Domain::Explicit(_))
    )
}

/// The position scale of the group `group` of a new chart `chart` under `fit`, `Keep` or
/// `Extend`, from its scale `old` in the old chart and its prepared scale `new`. A kept or
/// extended domain clips the marks outside it, as an explicit domain does.
fn carry_position(
    chart: &Chart,
    group: &ScaleGroup,
    fit: Fit,
    old: &Position,
    new: &Position,
) -> Result<Position> {
    fn fit_error(error: impl std::fmt::Display) -> EvalError {
        eval_error("the fit of a scale that an update extends", error)
    }
    let fitting = match (fit, FitKind::of(&group.spec.r#type)) {
        (Fit::Keep, _) => old.fitting.clone(),
        (Fit::Extend, FitKind::Interval) => match (old.fitting.summary(), new.fitting.summary()) {
            (Some(old_summary), Some(new_summary)) => FittingScale::new(
                new.fitting.scale().clone(),
                old_summary.merge(new_summary).map_err(fit_error)?,
                scale::domain_options(chart, &group.spec)?,
            ),
            _ => return Ok(new.clone()),
        },
        (Fit::Extend, FitKind::Categories) => {
            let categories = |position: &Position| {
                (position
                    .fitting
                    .at(1.0)
                    .map(|fitted| fitted.scale.domain().clone()))
                .map_err(fit_error)
            };
            let order = union_categories(&categories(old)?, &categories(new)?)?;
            match (old.fitting.summary(), new.fitting.summary()) {
                (Some(old_summary), Some(new_summary)) => FittingScale::new(
                    new.fitting.scale().clone(),
                    old_summary.merge(new_summary).map_err(fit_error)?,
                    DomainOptions::Categories(CategoryOptions {
                        order: CategoryOrder::Explicit(order),
                    }),
                ),
                _ => FittingScale::from(new.fitting.scale().clone().with_domain(order)),
            }
        }
        _ => return Ok(new.clone()),
    };
    Ok(Position {
        fitting,
        clip: true,
        summarized: vec![false; new.summarized.len()],
    })
}

/// The non-position scale of the group `group` of a new chart under `fit`, `Keep` or `Extend`,
/// from its scale `old` in the old chart and its prepared scale `new`.
fn carry_scale(
    group: &ScaleGroup,
    fit: Fit,
    old: &ConfiguredScale,
    new: &ConfiguredScale,
) -> Result<ConfiguredScale> {
    Ok(match (fit, FitKind::of(&group.spec.r#type)) {
        (Fit::Keep, _) => new.clone().with_domain(old.domain().clone()),
        (Fit::Extend, FitKind::Interval) if old.domain().len() == 2 && new.domain().len() == 2 => {
            match (old.numeric_interval_domain(), new.numeric_interval_domain()) {
                (Ok((a, b)), Ok((c, d))) => {
                    let (low, high) = (a.min(b).min(c.min(d)), a.max(b).max(c.max(d)));
                    let ends = if c <= d { (low, high) } else { (high, low) };
                    new.clone().with_domain_interval(ends)
                }
                _ => new.clone(),
            }
        }
        (Fit::Extend, FitKind::Categories) => new
            .clone()
            .with_domain(union_categories(old.domain(), new.domain())?),
        _ => new.clone(),
    })
}

/// The non-position scales of a unit, by the channel of their group.
fn channel_scales(
    unit: &PreparedUnit,
    scales: &BTreeMap<usize, ConfiguredScale>,
) -> BTreeMap<Channel, ConfiguredScale> {
    unit.groups
        .iter()
        .filter_map(|(channel, group)| Some((*channel, scales.get(group)?.clone())))
        .collect()
}

/// The guides of a chart, whose axes show the fitted domains of their scales.
#[allow(clippy::too_many_arguments)]
fn guides(
    chart: &Chart,
    tree: &Tree,
    groups: &[ScaleGroup],
    positions: &BTreeMap<usize, Position>,
    scales: &BTreeMap<usize, ConfiguredScale>,
    projection_guides: &[ProjectionGuide],
    theme: &Theme,
    engine: &LabelEngine,
) -> Result<Guides> {
    let fittings = positions
        .iter()
        .map(|(index, position)| (*index, position.fitting.clone()))
        .collect();
    guides::guides(&GuideInputs {
        chart,
        tree,
        groups,
        positions: &fittings,
        scales,
        projections: projection_guides,
        theme,
        engine,
    })
}

/// The first group of each channel of each plot of `tree`.
fn plot_groups(tree: &Tree, groups: &[ScaleGroup]) -> Vec<BTreeMap<Channel, usize>> {
    let mut plot_groups = vec![BTreeMap::new(); tree.plots.len()];
    for (index, group) in groups.iter().enumerate() {
        for member in &group.members {
            let plot = &mut plot_groups[tree.units[member.unit].plot];
            plot.entry(group.channel).or_insert(index);
        }
    }
    plot_groups
}

/// The identity of each group of `tree` that has one: its share family, or else its first plot
/// with an identity, in tree order, with its channel, and whether an earlier group of the channel
/// is in that plot.
fn identities(tree: &Tree, groups: &[ScaleGroup]) -> BTreeMap<usize, ScaleIdentity> {
    let in_plot = |group: &ScaleGroup, plot: usize| {
        (group.members.iter()).any(|member| tree.units[member.unit].plot == plot)
    };
    let mut identities = BTreeMap::new();
    for (index, group) in groups.iter().enumerate() {
        let channel = group.channel;
        let identity = if group.spec.family.is_empty() {
            let Some((plot, identity)) = (group.members.iter()).find_map(|member| {
                let plot = tree.units[member.unit].plot;
                Some((plot, tree.plots[plot].identity.clone()?))
            }) else {
                continue;
            };
            let second = (groups[..index].iter())
                .any(|earlier| earlier.channel == channel && in_plot(earlier, plot));
            ScaleIdentity::Plot {
                plot: identity,
                channel,
                second,
            }
        } else {
            ScaleIdentity::Family {
                family: group.spec.family.clone(),
                channel,
            }
        };
        identities.insert(index, identity);
    }
    identities
}

/// Prepares a chart: its scale groups, its non-position scales, the drawn rows and the reach of
/// each unit, the values of its position groups and their fitting scales, its guides, and its
/// layout.
fn prepare(
    chart: &Chart,
    contributions: &Contributions,
    engine: &LabelEngine,
    theme: &Theme,
) -> Result<Prepared> {
    let tree = resolve::tree(chart);
    let units = &tree.units;
    let groups = resolve::groups(chart, &tree)?;
    let selections = Selections {
        chart,
        contributions,
    };
    let mut unit_groups: Vec<BTreeMap<Channel, usize>> = vec![BTreeMap::new(); units.len()];
    for (index, group) in groups.iter().enumerate() {
        for member in &group.members {
            unit_groups[member.unit].insert(member.channel, index);
        }
    }

    // Non-position scales fit first, without a size, so that the reach of each mark takes them.
    let mut scales = BTreeMap::new();
    let mut scale_values = BTreeMap::new();
    for (index, group) in groups.iter().enumerate() {
        if group.is_position() {
            continue;
        }
        let values: Vec<Option<ArrayRef>> = group
            .members
            .iter()
            .map(|member| {
                let unit = &units[member.unit];
                member_values(chart, unit.unit, member.channel, unit.table)
            })
            .collect();
        scales.insert(
            index,
            scale::non_position(
                chart,
                group,
                group_mark(units, group)?,
                &values,
                &selections,
                theme,
            )?,
        );
        scale_values.insert(index, values);
    }

    let mut prepared_units = Vec::with_capacity(units.len());
    for (unit, groups) in units.iter().zip(unit_groups) {
        let prepared = PreparedUnit {
            drawn: encode::drawn_rows(unit.unit, unit.table)?,
            reach: None,
            groups,
        };
        let channel_scales = channel_scales(&prepared, &scales);
        let rows = UnitRows {
            chart,
            unit: unit.unit,
            mark: mark(unit.unit)?,
            table: prepared.drawn.clone(),
            positions: BTreeMap::new(),
            projected: None,
            scales: &channel_scales,
            masks: BTreeMap::new(),
            theme,
            engine,
            size: [0.0, 0.0],
            name: String::new(),
        };
        let reach = fit::reach(&rows)?;
        prepared_units.push(PreparedUnit { reach, ..prepared });
    }

    let mut positions = BTreeMap::new();
    let mut fit_values = BTreeMap::new();
    for (index, group) in groups.iter().enumerate() {
        if !group.is_position() {
            continue;
        }
        let values: FitValues = group
            .members
            .iter()
            .map(|member| {
                let prepared = &prepared_units[member.unit];
                let drawn = &prepared.drawn;
                let values = member_values(chart, units[member.unit].unit, member.channel, drawn)?;
                // The evaluator fits the domain of an edge channel, `x` or `y`, to the reach of the
                // marks of its own rows. A polar plot fits its circle to them instead.
                let along = match &prepared.reach {
                    Some(reach)
                        if values.len() == drawn.num_rows()
                            && member.summary.is_none()
                            && group.is_edge() =>
                    {
                        Some(fit::along(reach, group.channel, fit::reversed(group)))
                    }
                    _ => None,
                };
                Some((values, along))
            })
            .collect();
        positions.insert(
            index,
            fit::position(
                chart,
                group,
                &values,
                &selections,
                theme,
                CoordSystems::wraps(&tree, group),
            )?,
        );
        fit_values.insert(index, values);
    }

    // The coordinate systems prepare their plots from the drawn rows of the units and the reach
    // of their marks.
    let unit_scales: Vec<BTreeMap<Channel, ConfiguredScale>> = prepared_units
        .iter()
        .map(|prepared| channel_scales(prepared, &scales))
        .collect();
    let mut plot_units = Vec::with_capacity(units.len());
    for (index, unit) in units.iter().enumerate() {
        let prepared = &prepared_units[index];
        plot_units.push(PlotUnit {
            rows: UnitRows {
                chart,
                unit: unit.unit,
                mark: mark(unit.unit)?,
                table: prepared.drawn.clone(),
                positions: BTreeMap::new(),
                projected: None,
                scales: &unit_scales[index],
                masks: BTreeMap::new(),
                theme,
                engine,
                size: [0.0, 0.0],
                name: String::new(),
            },
            reach: prepared.reach.as_deref(),
            plot: unit.plot,
        });
    }
    let inputs = SystemInputs {
        chart,
        tree: &tree,
        groups: &groups,
        positions: &positions,
        theme,
        engine,
    };
    let systems = CoordSystems::prepare(&inputs, &plot_units)?;
    drop(plot_units);

    let guides = guides(
        chart,
        &tree,
        &groups,
        &positions,
        &scales,
        &systems.guides(),
        theme,
        engine,
    )?;
    let tables = table::tables(chart)?;
    let projections = resolve::projections(&tree)?;
    let matches = table::matches(&tree, &groups, &projections, &tables)?;
    let lowered = layout::lower(&Lowering {
        chart,
        tree: &tree,
        groups: &groups,
        guides: &guides,
        tables: &tables,
        matches: &matches,
        theme,
    });
    let plot_groups = plot_groups(&tree, &groups);
    let identities = identities(&tree, &groups);
    Ok(Prepared {
        groups,
        positions,
        fit_values,
        scales,
        scale_values,
        units: prepared_units,
        systems,
        guides,
        tables,
        matches,
        lowered,
        plot_groups,
        identities,
    })
}

/// Warns when a mark of a unit whose position scale takes the producer's summary for the unit
/// reaches past the edges of its plot at the fitted domain.
fn check_summary_reach(
    rows: &UnitRows,
    unit: usize,
    prepared: &Prepared,
    path: &str,
    warnings: &mut Vec<EvaluationWarning>,
) -> Result<()> {
    let Some(reach) = &prepared.units[unit].reach else {
        return Ok(());
    };
    for (index, position) in &prepared.positions {
        let group = &prepared.groups[*index];
        let summarized =
            group
                .members
                .iter()
                .zip(&position.summarized)
                .any(|(member, summarized)| {
                    *summarized && member.unit == unit && member.channel == group.channel
                });
        if !summarized {
            continue;
        }
        let (length, low, high) = match group.channel {
            Channel::X => (rows.size[0], 0, 1),
            Channel::Y => (rows.size[1], 2, 3),
            _ => continue,
        };
        let Some(values) = rows
            .has(group.channel)
            .then(|| rows.position(group.channel))
            .transpose()?
        else {
            continue;
        };
        let outside = (0..rows.len()).any(|row| {
            let position = at(&values, row);
            position - reach[row][low] < -0.5 || position + reach[row][high] > length + 0.5
        });
        if outside {
            warnings.push(EvaluationWarning {
                path: path.to_string(),
                message: format!(
                    "a mark along `{}` reaches past the edge of its plot at the domain of the \
                     producer's summary",
                    group.channel.as_str_name()
                ),
            });
        }
    }
    Ok(())
}
