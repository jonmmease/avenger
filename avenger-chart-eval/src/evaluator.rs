//! `ChartEvaluator`, which prepares a chart one time and evaluates it at any size, and the
//! `EvaluatedChart` of each evaluation.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, RecordBatch};
use avenger_geometry::data::SceneData;
use avenger_layout::{Edges, Measured, Size, SolveOptions};
use avenger_scales::scales::ConfiguredScale;
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
    resolve::{self, PlotRef, ScaleGroup, Tree, UnitRef},
    table, Chart, Contribution,
};

use crate::{
    encode::{self, at, UnitRows},
    fit::{self, FitValues, Position, Reach},
    geo::{MapUnit, Maps},
    guides::{self, GuideInputs, Guides, MapGuide},
    layout::{self, Lowered, Lowering, Measurer, SlabId},
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
}

/// A plot of an evaluation: its name, its rectangle in the scene, and its coordinate system.
#[derive(Clone, Debug)]
pub struct EvaluatedPlot {
    /// The id of its node, or its node path when it has none.
    pub name: String,
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

/// A chart prepared to evaluate at any size.
///
/// `ChartEvaluator::new` groups the chart's channels into scales, configures its non-position
/// scales, builds the summary of each position scale with the reach of its marks, prepares its
/// guides, and builds its layout. `evaluate` solves the layout at a size, fits each position scale
/// at the length of its plots, and encodes the marks, so one evaluator evaluates a chart at every
/// size.
pub struct ChartEvaluator {
    chart: Chart,
    engine: LabelEngine,
    theme: Theme,
    contributions: Contributions,
    prepared: Prepared,
    measured: Option<Measured<SlabId>>,
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
    guides: Guides,
    lowered: Lowered,
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
    /// plots, and of map plots with the `geo` feature.
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
                    fit::position(chart, group, values, &selections, &self.theme)?,
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
        let (guides, lowered) = guides_and_layout(
            chart,
            &tree,
            &prepared.groups,
            &prepared.positions,
            &prepared.scales,
            &prepared.map_guides,
            &self.theme,
            &self.engine,
        )?;
        prepared.guides = guides;
        prepared.lowered = lowered;
        self.measured = None;
        Ok(())
    }

    /// Evaluates the chart at `size`, its width and height in pixels.
    pub fn evaluate(&mut self, size: [f32; 2]) -> Result<EvaluatedChart> {
        let layout_error = |error| eval_error("the layout of the chart", error);
        let mut measurer = Measurer {
            guides: &self.prepared.guides,
        };
        let solution = match &mut self.measured {
            Some(measured) => measured
                .remeasure(Size::new(size[0], size[1]), &mut measurer)
                .map_err(layout_error)?
                .clone(),
            None => {
                let options = SolveOptions {
                    padding: Edges::all(self.theme.padding),
                    ..SolveOptions::sized(size[0], size[1])
                };
                let measured = self
                    .prepared
                    .lowered
                    .layout
                    .measure(&options, &mut measurer)
                    .map_err(layout_error)?;
                let solution = measured.solution().clone();
                self.measured = Some(measured);
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
        let views = prepared.maps.views(&sizes, &mut warnings)?;

        // Each position scale at the length of its plots, which the layout makes equal, so one
        // fit at the length of its first plot serves every plot.
        let mut at_size: BTreeMap<usize, ConfiguredScale> = BTreeMap::new();
        for (index, position) in &prepared.positions {
            let group = &prepared.groups[*index];
            let Some(first) = group.members.first() else {
                continue;
            };
            let plot = tree.units[first.unit].plot;
            let length = match group.channel {
                Channel::Y => rects[plot].height,
                _ => rects[plot].width,
            };
            let fitted = position
                .fitting
                .at(length)
                .map_err(|error| eval_error("the fit of a position scale", error))?;
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
            let clip = plot_scales
                .keys()
                .any(|index| prepared.positions[index].clip)
                || view.is_some_and(|view| view.clips());

            let mut plot_marks: Vec<SceneMark> = match view {
                Some(view) => view.backdrop(&prepared.maps)?,
                None => Vec::new(),
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

                let channel_scales = channel_scales(prepared_unit, &prepared.scales);
                let position = |channel: Channel| {
                    prepared_unit
                        .groups
                        .get(&channel)
                        .and_then(|group| plot_scales.get(group).copied())
                };
                let unit_rows = UnitRows {
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
                let drawn_marks = match view {
                    Some(view) => view.marks(&unit_rows)?,
                    None => encode::marks(&unit_rows)?,
                };
                for drawn in drawn_marks {
                    data.insert(drawn.mark.mark_name(), Arc::new(drawn.rows));
                    plot_marks.push(drawn.mark);
                }
                check_summary_reach(&unit_rows, index, prepared, &unit.path, &mut warnings)?;
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
                rect: [plot.x, plot.y, plot.width, plot.height],
                coord: match view {
                    Some(view) => view.coord(),
                    None => PlotCoord::Cartesian {
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

#[allow(clippy::too_many_arguments)]
fn guides_and_layout(
    chart: &Chart,
    tree: &Tree,
    groups: &[ScaleGroup],
    positions: &BTreeMap<usize, Position>,
    scales: &BTreeMap<usize, ConfiguredScale>,
    maps: &BTreeMap<usize, MapGuide>,
    theme: &Theme,
    engine: &LabelEngine,
) -> Result<(Guides, Lowered)> {
    let fittings = positions
        .iter()
        .map(|(index, position)| (*index, position.fitting.clone()))
        .collect();
    let guides = guides::guides(&GuideInputs {
        chart,
        tree,
        groups,
        positions: &fittings,
        scales,
        maps,
        theme,
        engine,
    })?;
    let tables = table::tables(chart)?;
    let projections = resolve::projections(tree)?;
    let matches = table::matches(tree, groups, &projections, &tables)?;
    let lowered = layout::lower(&Lowering {
        chart,
        tree,
        groups,
        guides: &guides,
        tables: &tables,
        matches: &matches,
        theme,
    });
    Ok((guides, lowered))
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
    for plot in &tree.plots {
        if let Some(coord::Coord::Polar(_)) = plot.coord.and_then(|coord| coord.coord.as_ref()) {
            return Err(EvalError::Evaluation(format!(
                "the evaluator draws Cartesian and map plots, and `{}` is a polar plot",
                plot.path
            )));
        }
    }
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
                // The evaluator fits the domain to the reach of the marks of its own rows.
                let along = match &prepared.reach {
                    Some(reach) if values.len() == drawn.num_rows() && member.summary.is_none() => {
                        Some(fit::along(reach, group.channel, fit::reversed(group)))
                    }
                    _ => None,
                };
                Some((values, along))
            })
            .collect();
        positions.insert(
            index,
            fit::position(chart, group, &values, &selections, theme)?,
        );
        fit_values.insert(index, values);
    }

    // The units of map plots fit their projections.
    let unit_scales: Vec<BTreeMap<Channel, ConfiguredScale>> = prepared_units
        .iter()
        .map(|prepared| channel_scales(prepared, &scales))
        .collect();
    let mut map_units = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        let Some(coord::Coord::Geo(_)) = tree.plots[unit.plot]
            .coord
            .and_then(|coord| coord.coord.as_ref())
        else {
            continue;
        };
        let prepared = &prepared_units[index];
        map_units.push(MapUnit {
            rows: UnitRows {
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
            },
            reach: prepared.reach.as_deref(),
            plot: unit.plot,
        });
    }
    let maps = Maps::prepare(&map_units, &tree, theme, engine)?;
    drop(map_units);
    let map_guides = maps.guides();

    let (guides, lowered) = guides_and_layout(
        chart,
        &tree,
        &groups,
        &positions,
        &scales,
        &map_guides,
        theme,
        engine,
    )?;
    Ok(Prepared {
        groups,
        positions,
        fit_values,
        scales,
        scale_values,
        units: prepared_units,
        maps,
        map_guides,
        guides,
        lowered,
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
            Channel::Y => (rows.size[1], 2, 3),
            _ => (rows.size[0], 0, 1),
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
