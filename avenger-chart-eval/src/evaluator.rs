//! `ChartEvaluator`, which prepares a chart one time and evaluates it at any size, and the
//! `EvaluatedChart` of each evaluation.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, RecordBatch};
use avenger_geometry::data::SceneData;
use avenger_layout::{Edges, Measured, Rect, Size, SolveOptions};
use avenger_scales::scales::{fitting::FittingScale, ConfiguredScale};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
    },
    scene_graph::SceneGraph,
};
use avenger_typst_label::LabelEngine;

use avenger_chart::{
    proto::{self, coord, domain, encoding, Channel},
    resolve::{self, Identity, PlotRef, ScaleGroup, Tree, UnitRef},
    table::{self, Matches, Tables},
    Chart, Contribution, FitKind, TupleTest,
};

use crate::{
    encode::{self, at, UnitRows},
    fit::{self, FitValues, Position, Reach},
    geo::{MapUnit, Maps},
    guides::{self, GuideInputs, Guides, MapGuide},
    layout::{self, Lowered, Lowering, Measurer, SlabId},
    polar::{self, PolarUnit, Polars},
    scale::{self, eval_error},
    select::{self, Contributions, Selections},
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

/// A plot of an evaluation: its name and identity, its rectangle in the scene, and its coordinate
/// system.
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

/// The identity of a scale of `x` or `y`, which carries its view to a new chart.
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
/// `ChartEvaluator::new` groups the chart's channels into scales, configures its non-position
/// scales, builds the summary of each position scale with the reach of its marks, prepares its
/// guides, and builds its layout. `evaluate` solves the layout at a size, fits each position scale
/// at the length of its plots, and encodes the marks, so one evaluator evaluates a chart at every
/// size.
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
    views: BTreeMap<usize, ConfiguredScale>,
    /// The last revision of an axis, which each new view of an axis takes the next of.
    revision: u64,
    /// The rectangles of the plots at the last evaluation.
    rects: Vec<Rect>,
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
    /// The map plots, and the guide of each, by plot.
    maps: Maps,
    map_guides: BTreeMap<usize, MapGuide>,
    /// The polar plots and their axes.
    polars: Polars,
    guides: Guides,
    tables: Tables,
    matches: Matches,
    lowered: Lowered,
    /// The first position group of `x` and of `y` of each plot, by plot.
    plot_groups: Vec<[Option<usize>; 2]>,
    /// The identity of each position group of `x` and `y` that has one, by group.
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

    /// The scale of the position group `group` of `plot` fitted at the plot's size in `rects`.
    fn fitted(&self, rects: &[Rect], plot: usize, group: usize) -> Option<ConfiguredScale> {
        let rect = rects.get(plot)?;
        let length = match self.groups[group].channel {
            Channel::X => rect.width,
            _ => rect.height,
        };
        let fitted = self.positions.get(&group)?.fitting.at(length).ok()?;
        Some(fitted.scale)
    }

    /// Whether the position group `group` can have a view: a continuous scale of `x` or `y` whose
    /// domain does not read a selection.
    fn viewable(&self, group: usize) -> bool {
        let group = &self.groups[group];
        matches!(group.channel, Channel::X | Channel::Y)
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

impl ChartEvaluator {
    /// Prepares `chart` to evaluate with `engine` and `theme`, with the initial contributions of
    /// its selections. The evaluator draws units, overlays, rows, columns, and wraps of Cartesian
    /// and polar plots, and of map plots with the `geo` feature.
    pub fn new(chart: &Chart, engine: &LabelEngine, theme: &Theme) -> Result<Self> {
        let contributions = chart.initial_contributions();
        let prepared = prepare(chart, &contributions, engine, theme)?;
        Ok(Self {
            chart: chart.clone(),
            engine: engine.clone(),
            theme: theme.clone(),
            contributions,
            prepared,
            measured: None,
            relowered: false,
            views: BTreeMap::new(),
            revision: 0,
            rects: Vec::new(),
        })
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
        let refits = self
            .prepared
            .groups
            .iter()
            .any(|group| selection_domain(&group.spec));
        if refits {
            self.refit()?;
        }
        Ok(())
    }

    /// The view of the scale of `channel`, `x` or `y`, of the plot at index `plot` of the
    /// evaluation, to move, as a pan or a zoom does. A scale without a view takes its fitted scale
    /// at the plot's size at the last evaluation as its view. A plot's first scale of the channel
    /// has a view when it is continuous and its domain does not read a selection, and the plot has
    /// been evaluated, and otherwise this is `None`.
    pub fn view_mut(&mut self, plot: usize, channel: Channel) -> Option<&mut ConfiguredScale> {
        let group = self.plot_group(plot, channel)?;
        if !self.prepared.viewable(group) {
            return None;
        }
        if !self.views.contains_key(&group) {
            let fitted = self.prepared.fitted(&self.rects, plot, group)?;
            self.views.insert(group, fitted);
        }
        self.views.get_mut(&group)
    }

    /// Removes the view of the scale of `channel` of the plot at index `plot`, so that the scale
    /// fits its domain again.
    pub fn clear_view(&mut self, plot: usize, channel: Channel) {
        if let Some(group) = self.plot_group(plot, channel) {
            self.views.remove(&group);
        }
    }

    /// The view of the scale of `channel` of the plot at index `plot`, when it has one.
    pub fn view(&self, plot: usize, channel: Channel) -> Option<&ConfiguredScale> {
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

    /// The fitted scales of `x` and `y` of the plot at index `plot` at its size at the last
    /// evaluation, without their views, which a navigation returns to.
    pub fn home_scales(&self, plot: usize) -> [Option<ConfiguredScale>; 2] {
        [Channel::X, Channel::Y].map(|channel| {
            let group = self.plot_group(plot, channel)?;
            self.prepared.fitted(&self.rects, plot, group)
        })
    }

    /// The views of the scales that have identities.
    pub fn views(&self) -> Vec<(ScaleIdentity, ConfiguredScale)> {
        (self.views.iter())
            .filter_map(|(group, view)| {
                let identity = self.prepared.identities.get(group)?;
                Some((identity.clone(), view.clone()))
            })
            .collect()
    }

    /// Replaces the views with `views`, such as those of an earlier chart: each view goes to the
    /// scale with its identity, when the scale has the type of the view's scale and can have a
    /// view, as the domain of the scale.
    pub fn set_views(&mut self, views: &[(ScaleIdentity, ConfiguredScale)]) {
        self.views.clear();
        for (identity, view) in views {
            let Some(group) = (self.prepared.identities.iter())
                .find(|(_, own)| *own == identity)
                .map(|(group, _)| *group)
            else {
                continue;
            };
            let scale = self.prepared.positions[&group].fitting.scale();
            if self.prepared.viewable(group)
                && scale.scale_impl.scale_type() == view.scale_impl.scale_type()
            {
                let view = scale.clone().with_domain(view.domain().clone());
                self.views.insert(group, view);
            }
        }
    }

    /// The first position group of `channel` of the plot at index `plot`.
    fn plot_group(&self, plot: usize, channel: Channel) -> Option<usize> {
        let groups = self.prepared.plot_groups.get(plot)?;
        match channel {
            Channel::X => groups[0],
            Channel::Y => groups[1],
            _ => None,
        }
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
        self.prepared.maps.set_image_resolver(resolver)
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
                        polar::wraps(&tree, group),
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
        prepared.guides = guides(
            chart,
            &tree,
            &prepared.groups,
            &prepared.positions,
            &prepared.scales,
            &prepared.map_guides,
            &self.theme,
            &self.engine,
        )?;
        prepared.lower(chart, &self.theme);
        prepared.polars.set_axes(
            chart,
            &tree,
            &prepared.groups,
            &prepared.positions,
            &self.theme,
            &self.engine,
        )?;
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
        let prepared = &self.prepared;
        let tree = resolve::tree(chart);
        let selections = Selections {
            chart,
            contributions: &self.contributions,
        };
        let mut warnings = Vec::new();
        let mut marks: Vec<SceneMark> = Vec::new();
        let mut plots = Vec::with_capacity(tree.plots.len());
        let mut data = SceneData::new();
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
        let views = prepared.maps.views(&sizes, &mut warnings)?;
        let polar_views = prepared.polars.views(&sizes, &mut warnings)?;

        // Each position scale of `x` and `y` at the length of its plots, which the layout makes
        // equal, so one fit at the length of its first plot serves every plot, and a view keeps its
        // domain at that length. The polar views fit the scales of `theta` and `radius`.
        let mut at_size: BTreeMap<usize, ConfiguredScale> = BTreeMap::new();
        for (index, position) in &prepared.positions {
            let group = &prepared.groups[*index];
            let Some(first) = group.members.first() else {
                continue;
            };
            let plot = tree.units[first.unit].plot;
            let length = match group.channel {
                Channel::X => rects[plot].width,
                Channel::Y => rects[plot].height,
                _ => continue,
            };
            let view = self.views.get_mut(index);
            let fitted = match &view {
                Some(view) => FittingScale::from(&**view).at(length),
                None => position.fitting.at(length),
            }
            .map_err(|error| eval_error("the fit of a position scale", error))?;
            if let Some(view) = view {
                view.clone_from(&fitted.scale);
            }
            if !fitted.marks_fit {
                warnings.push(EvaluationWarning {
                    path: tree.plots[plot].path.clone(),
                    message: format!(
                        "the plot is too small for the marks along `{}`, so they reach past its \
                         edges",
                        group.channel.as_str_name()
                    ),
                });
            }
            at_size.insert(*index, fitted.scale);
        }

        for (plot_index, plot_ref) in tree.plots.iter().enumerate() {
            let plot = rects[plot_index];
            let plot_size = [plot.width, plot.height];
            // The position scales of the plot, and whether the plot clips its marks, because one
            // of them has an explicit domain.
            let plot_scales: BTreeMap<usize, &ConfiguredScale> = at_size
                .iter()
                .filter(|(index, _)| {
                    prepared.groups[**index]
                        .members
                        .iter()
                        .any(|member| tree.units[member.unit].plot == plot_index)
                })
                .map(|(index, scale)| (*index, scale))
                .collect();
            let view = views.get(&plot_index);
            let polar_view = polar_views.get(&plot_index);
            let clip = plot_scales
                .keys()
                .any(|index| prepared.positions[index].clip || self.views.contains_key(index))
                || view.is_some_and(|view| view.clips())
                || polar_view.is_some_and(|view| view.clips());

            let mut mark_names = Vec::new();
            let mut plot_marks: Vec<SceneMark> = match (view, polar_view) {
                (Some(view), _) => view.backdrop(&prepared.maps)?,
                (None, Some(view)) => view.backdrop(&prepared.polars),
                (None, None) => Vec::new(),
            };
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
                // In a map plot, the rows of a mark at a place whose place the projection keeps.
                let (table, projected) = match view {
                    Some(view) => view.locate(chart, unit.unit, &prepared_unit.drawn)?,
                    None => (prepared_unit.drawn.clone(), None),
                };
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

                let mut channel_scales = channel_scales(prepared_unit, &prepared.scales);
                if let Some(view) = polar_view {
                    view.add_scales(&mut channel_scales);
                }
                let position = |channel: Channel| {
                    prepared_unit
                        .groups
                        .get(&channel)
                        .and_then(|group| plot_scales.get(group).copied())
                };
                let mut unit_rows = UnitRows {
                    chart,
                    unit: unit.unit,
                    mark: mark(unit.unit)?,
                    table,
                    x: position(Channel::X),
                    y: position(Channel::Y),
                    projected,
                    scales: &channel_scales,
                    masks,
                    theme: &self.theme,
                    engine: &self.engine,
                    size: plot_size,
                    name: mark_name(plot_ref, unit),
                };
                let drawn_marks = match (view, polar_view) {
                    (Some(view), _) => view.marks(&unit_rows)?,
                    (None, Some(view)) => view.marks(&mut unit_rows)?,
                    (None, None) => encode::marks(&unit_rows)?,
                };
                for drawn in drawn_marks {
                    let name = drawn.mark.mark_name().to_string();
                    data.insert(&name, Arc::new(drawn.rows));
                    mark_names.push(name);
                    plot_marks.push(drawn.mark);
                }
                check_summary_reach(&unit_rows, index, prepared, &unit.path, &mut warnings)?;
            }
            // A polar plot draws its axes inside its box, over its data.
            if let Some(view) = polar_view {
                plot_marks.extend(view.axes(&prepared.polars));
            }

            marks.push(
                SceneGroup {
                    name: plot_ref.name.clone(),
                    origin: [plot.x, plot.y],
                    clip: if clip {
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

            // The scales of each direction, in the order of their groups.
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (index, scale) in plot_scales {
                match prepared.groups[index].channel {
                    Channel::X => xs.push(scale.clone()),
                    Channel::Y => ys.push(scale.clone()),
                    _ => {}
                }
            }
            let mut xs = xs.into_iter();
            let mut ys = ys.into_iter();
            plots.push(EvaluatedPlot {
                name: plot_ref.name.clone(),
                identity: plot_ref.identity.clone(),
                marks: mark_names,
                rect: [plot.x, plot.y, plot.width, plot.height],
                coord: match (view, polar_view) {
                    (Some(view), _) => view.coord(),
                    (None, Some(view)) => view.coord(),
                    (None, None) => PlotCoord::Cartesian {
                        x: xs.next(),
                        y: ys.next(),
                        second_x: xs.next(),
                        second_y: ys.next(),
                    },
                },
            });
        }
        marks.extend(layout::draw(&prepared.guides, &prepared.lowered, &solution));
        // The labels of each graticule, through the projector of its plot.
        for id in &prepared.lowered.slabs {
            let SlabId::Graticule { plot, side } = id else {
                continue;
            };
            let (Some(view), Some(region)) = (views.get(plot), solution.region(id)) else {
                continue;
            };
            let rect = region.content;
            let slot = [rect.x, rect.y, rect.width, rect.height];
            if let Some(group) = view.labels(&prepared.maps, *side, slot) {
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
    maps: &BTreeMap<usize, MapGuide>,
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
        maps,
        theme,
        engine,
    })
}

/// The first position group of `x` and of `y` of each plot of `tree`.
fn plot_groups(tree: &Tree, groups: &[ScaleGroup]) -> Vec<[Option<usize>; 2]> {
    let mut plot_groups = vec![[None; 2]; tree.plots.len()];
    for (index, group) in groups.iter().enumerate() {
        let axis = match group.channel {
            Channel::X => 0,
            Channel::Y => 1,
            _ => continue,
        };
        for member in &group.members {
            let first = &mut plot_groups[tree.units[member.unit].plot][axis];
            first.get_or_insert(index);
        }
    }
    plot_groups
}

/// The identity of each position group of `x` and `y` of `tree` that has one: its share family,
/// or else its first plot with an identity, in tree order, with its channel, and whether an
/// earlier group of the channel is in that plot.
fn identities(tree: &Tree, groups: &[ScaleGroup]) -> BTreeMap<usize, ScaleIdentity> {
    let in_plot = |group: &ScaleGroup, plot: usize| {
        (group.members.iter()).any(|member| tree.units[member.unit].plot == plot)
    };
    let mut identities = BTreeMap::new();
    for (index, group) in groups.iter().enumerate() {
        let channel = group.channel;
        if !matches!(channel, Channel::X | Channel::Y) {
            continue;
        }
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
    let groups = resolve::groups(chart, units)?;
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
            x: None,
            y: None,
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
                // The evaluator fits the domain of `x` and `y` to the reach of the marks of its own
                // rows. A polar plot fits its circle to them instead.
                let along = match &prepared.reach {
                    Some(reach)
                        if values.len() == drawn.num_rows()
                            && member.summary.is_none()
                            && matches!(group.channel, Channel::X | Channel::Y) =>
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
                polar::wraps(&tree, group),
            )?,
        );
        fit_values.insert(index, values);
    }

    // The units of map plots fit their projections, and those of polar plots their circles.
    let unit_scales: Vec<BTreeMap<Channel, ConfiguredScale>> = prepared_units
        .iter()
        .map(|prepared| channel_scales(prepared, &scales))
        .collect();
    let mut map_units = Vec::new();
    let mut polar_units = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        let prepared = &prepared_units[index];
        let rows = || -> Result<UnitRows> {
            Ok(UnitRows {
                chart,
                unit: unit.unit,
                mark: mark(unit.unit)?,
                table: prepared.drawn.clone(),
                x: None,
                y: None,
                projected: None,
                scales: &unit_scales[index],
                masks: BTreeMap::new(),
                theme,
                engine,
                size: [0.0, 0.0],
                name: String::new(),
            })
        };
        match tree.plots[unit.plot]
            .coord
            .and_then(|coord| coord.coord.as_ref())
        {
            Some(coord::Coord::Geo(_)) => map_units.push(MapUnit {
                rows: rows()?,
                reach: prepared.reach.as_deref(),
                plot: unit.plot,
            }),
            Some(coord::Coord::Polar(_)) => polar_units.push(PolarUnit {
                rows: rows()?,
                reach: prepared.reach.as_deref(),
                plot: unit.plot,
            }),
            _ => {}
        }
    }
    let maps = Maps::prepare(&map_units, &tree, theme, engine)?;
    drop(map_units);
    let map_guides = maps.guides();
    let polars = Polars::prepare(
        chart,
        &polar_units,
        &tree,
        &groups,
        &positions,
        theme,
        engine,
    )?;
    drop(polar_units);

    let guides = guides(
        chart,
        &tree,
        &groups,
        &positions,
        &scales,
        &map_guides,
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
        maps,
        map_guides,
        polars,
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
