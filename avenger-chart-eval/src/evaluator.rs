//! `ChartEvaluator`, which prepares a chart one time and evaluates it at any size, and the
//! `EvaluatedChart` of each evaluation.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, RecordBatch};
use avenger_coords::PlotProjection;
use avenger_geometry::data::SceneData;
use avenger_layout::{Edges, Layout, Measured, Size, SolveOptions};
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
    proto::{self, domain, encoding, Channel},
    resolve::{self, ScaleGroup, UnitRef},
    Chart, Contribution, CoordKind,
};

use crate::{
    coord::{CoordSystems, ProjectionGuide, SystemInputs},
    encode::{self, at, UnitRows},
    fit::{self, FitValues, Position, Reach},
    guides::{self, GuideInputs, Guides},
    layout::{self, Measurer, SlabId},
    scale::{self, eval_error},
    select::{self, Contributions, Owner, Selections},
    EvalError, Theme,
};

type Result<T> = std::result::Result<T, EvalError>;

/// A warning of an evaluation, at the path of the part of the chart that it concerns.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationWarning {
    pub path: String,
    pub message: String,
}

/// The coordinate system of a plot at the size of an evaluation.
#[derive(Clone, Debug)]
pub enum PlotCoord {
    /// The configured position scales of a Cartesian plot, which map data to pixels from the
    /// plot's top left.
    Cartesian {
        x: Option<ConfiguredScale>,
        y: Option<ConfiguredScale>,
    },
}

impl PlotCoord {
    /// The kind of the coordinate system.
    pub fn kind(&self) -> CoordKind {
        match self {
            PlotCoord::Cartesian { .. } => CoordKind::Cartesian,
        }
    }

    /// The scale of the positional channel `channel`, when the plot draws it through one.
    pub fn scale(&self, channel: Channel) -> Option<&ConfiguredScale> {
        match (self, channel) {
            (PlotCoord::Cartesian { x, .. }, Channel::X) => x.as_ref(),
            (PlotCoord::Cartesian { y, .. }, Channel::Y) => y.as_ref(),
            _ => None,
        }
    }
}

/// A plot of an evaluation: its name, its rectangle in the scene, its coordinate system, and the
/// projection of its pixels.
#[derive(Clone, Debug)]
pub struct EvaluatedPlot {
    /// The id of its node, or its node path when it has none.
    pub name: String,
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

/// A chart prepared to evaluate at any size.
///
/// `ChartEvaluator::new` groups the chart's channels into scales, configures its non-position
/// scales, builds the summary of each position scale with the reach of its marks, prepares its
/// guides, and builds its layout. `evaluate` solves the layout at a size, fits each position scale
/// at its length, and encodes the marks, so one evaluator evaluates a chart at every size.
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
    scale_values: BTreeMap<usize, Vec<ArrayRef>>,
    /// The unit's rows that draw instances, and their reach.
    drawn: RecordBatch,
    reach: Option<Vec<Reach>>,
    /// The coordinate systems of the plots.
    systems: CoordSystems,
    guides: Guides,
    layout: Layout<SlabId>,
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

/// The partition values of a unit: its own and its ancestors'.
fn partition(chart: &Chart, unit: &proto::Unit) -> BTreeMap<String, ArrayRef> {
    unit.partition
        .iter()
        .map(|value| (value.column.clone(), chart.value(value.value).clone()))
        .collect()
}

fn mark(unit: &proto::Unit) -> Result<&proto::mark::Mark> {
    unit.mark
        .as_ref()
        .and_then(|mark| mark.mark.as_ref())
        .ok_or_else(|| EvalError::Evaluation("a unit has no mark".to_string()))
}

/// The name of the marks of a unit: its id, or its node path, and its position in its plot.
fn mark_name(unit: &UnitRef) -> String {
    format!("{}/0", unit.name)
}

impl ChartEvaluator {
    /// Prepares `chart` to evaluate with `engine` and `theme`, with the initial contributions of
    /// its selections. The evaluator draws a chart whose root is a unit in a Cartesian plot.
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

    /// Fits the scales whose domains are selection domains again, with the stored values of
    /// their groups, and prepares the guides and the layout again.
    fn refit(&mut self) -> Result<()> {
        let chart = &self.chart;
        let units = resolve::units(chart);
        let selections = Selections {
            chart,
            contributions: &self.contributions,
        };
        let prepared = &mut self.prepared;
        let unit = &units[0];
        let unit_mark = mark(unit.unit)?;
        for (index, group) in prepared.groups.iter().enumerate() {
            if !selection_domain(&group.spec) {
                continue;
            }
            if group.plot.is_some() {
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
                        &group.spec,
                        group.channel,
                        unit_mark,
                        values,
                        &selections,
                        &self.theme,
                    )?,
                );
            }
        }
        let (guides, layout) = guides_and_layout(
            chart,
            &units,
            &prepared.groups,
            &prepared.positions,
            &prepared.scales,
            &prepared.systems.guide(0),
            &self.theme,
            &self.engine,
        )?;
        prepared.guides = guides;
        prepared.layout = layout;
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
                    .layout
                    .measure(&options, &mut measurer)
                    .map_err(layout_error)?;
                let solution = measured.solution().clone();
                self.measured = Some(measured);
                solution
            }
        };
        let chart = &self.chart;
        let units = resolve::units(chart);
        let unit = &units[0];
        let unit_mark = mark(unit.unit)?;
        let plot = solution
            .region(&SlabId::Plot(0))
            .map(|region| region.content)
            .ok_or_else(|| EvalError::Evaluation("the layout has no plot".to_string()))?;
        let plot_size = [plot.width, plot.height];
        let mut warnings = Vec::new();

        // The coordinate system fits the projection and the position scales of the plot at its
        // size.
        let prepared = &mut self.prepared;
        (prepared.systems).fit(&prepared.positions, &[plot_size], &mut warnings)?;
        let prepared = &self.prepared;
        let fitted = prepared.systems.fitted(0)?;

        // The rows that each condition selects.
        let unit_partition = partition(chart, unit.unit);
        let owners: Vec<Owner> = if unit.unit.id.is_empty() {
            Vec::new()
        } else {
            vec![Owner {
                id: unit.unit.id.clone(),
                partition: unit_partition.clone(),
            }]
        };
        let rows = select::Rows {
            table: &prepared.drawn,
            partition: &unit_partition,
            owners: &owners,
            path: &unit.path,
        };
        let selections = Selections {
            chart,
            contributions: &self.contributions,
        };
        let mut masks = BTreeMap::new();
        for encoding in &unit.unit.encodings {
            if encoding.conditions.is_empty() {
                continue;
            }
            let channel = Channel::try_from(encoding.channel).unwrap_or(Channel::Unspecified);
            let channel_masks = encoding
                .conditions
                .iter()
                .map(|condition| match &condition.predicate {
                    Some(predicate) => selections.mask(predicate, &rows, &mut warnings),
                    None => Ok(vec![false; prepared.drawn.num_rows()]),
                })
                .collect::<Result<Vec<_>>>()?;
            masks.insert(channel, channel_masks);
        }

        let channel_scales = channel_scales(&prepared.groups, &prepared.scales);
        let positions = (prepared.groups.iter().enumerate())
            .filter_map(|(index, group)| Some((group.channel, fitted.scale(index)?)))
            .collect();
        let unit_rows = UnitRows {
            chart,
            unit: unit.unit,
            mark: unit_mark,
            table: prepared.drawn.clone(),
            positions,
            scales: &channel_scales,
            masks,
            theme: &self.theme,
            engine: &self.engine,
            size: plot_size,
            name: mark_name(unit),
        };
        let drawn = fitted.marks(&unit_rows)?;
        check_summary_reach(&unit_rows, prepared, &unit.path, &mut warnings)?;

        let mut plot_marks: Vec<SceneMark> = prepared
            .guides
            .axes
            .iter()
            .filter(|axis| axis.grid)
            .map(|axis| axis.axis.grid(plot_size).into())
            .collect();
        let mut data = SceneData::new();
        for drawn in drawn {
            data.insert(drawn.mark.mark_name(), Arc::new(drawn.rows));
            plot_marks.push(drawn.mark);
        }
        let plot_group = SceneGroup {
            name: unit.name.clone(),
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
        };
        let mut marks: Vec<SceneMark> = vec![plot_group.into()];
        marks.extend(layout::draw(&prepared.guides, &solution));
        Ok(EvaluatedChart {
            scene: SceneGraph {
                marks,
                width: size[0],
                height: size[1],
                origin: [0.0, 0.0],
            },
            data,
            plots: vec![EvaluatedPlot {
                name: unit.name.clone(),
                rect: [plot.x, plot.y, plot.width, plot.height],
                coord: fitted.coord(),
                projection: fitted.projection([plot.width, plot.height]),
            }],
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
    groups: &[ScaleGroup],
    scales: &BTreeMap<usize, ConfiguredScale>,
) -> BTreeMap<Channel, ConfiguredScale> {
    scales
        .iter()
        .map(|(index, scale)| (groups[*index].channel, scale.clone()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn guides_and_layout(
    chart: &Chart,
    units: &[UnitRef],
    groups: &[ScaleGroup],
    positions: &BTreeMap<usize, Position>,
    scales: &BTreeMap<usize, ConfiguredScale>,
    projection: &ProjectionGuide,
    theme: &Theme,
    engine: &LabelEngine,
) -> Result<(Guides, Layout<SlabId>)> {
    let fittings = positions
        .iter()
        .map(|(index, position)| (*index, position.fitting.clone()))
        .collect();
    let guides = guides::guides(&GuideInputs {
        chart,
        units,
        groups,
        positions: &fittings,
        scales,
        theme,
        engine,
    })?;
    let layout = layout::lower(&guides, projection, theme);
    Ok((guides, layout))
}

/// Prepares a chart: its scale groups, its non-position scales, the values and the reach of its
/// position groups and their fitting scales, its guides, and its layout.
fn prepare(
    chart: &Chart,
    contributions: &Contributions,
    engine: &LabelEngine,
    theme: &Theme,
) -> Result<Prepared> {
    let units = resolve::units(chart);
    let [unit] = units.as_slice() else {
        return Err(EvalError::Evaluation(
            "the evaluator draws a chart whose root is a unit".to_string(),
        ));
    };
    let unit_mark = mark(unit.unit)?;
    let groups = resolve::groups(&units);
    let selections = Selections {
        chart,
        contributions,
    };

    // Non-position scales fit first, without a size, so that the reach of each mark takes them.
    let mut scales = BTreeMap::new();
    let mut scale_values = BTreeMap::new();
    for (index, group) in groups.iter().enumerate() {
        if group.plot.is_some() {
            continue;
        }
        let values: Vec<ArrayRef> = group
            .members
            .iter()
            .filter_map(|member| {
                let member_unit = &units[member.unit];
                member_values(chart, member_unit.unit, member.channel, member_unit.table)
            })
            .collect();
        scales.insert(
            index,
            scale::non_position(
                chart,
                &group.spec,
                group.channel,
                unit_mark,
                &values,
                &selections,
                theme,
            )?,
        );
        scale_values.insert(index, values);
    }

    let drawn = encode::drawn_rows(unit.unit, unit.table)?;
    let channel_scales = channel_scales(&groups, &scales);
    let rows = UnitRows {
        chart,
        unit: unit.unit,
        mark: unit_mark,
        table: drawn.clone(),
        positions: BTreeMap::new(),
        scales: &channel_scales,
        masks: BTreeMap::new(),
        theme,
        engine,
        size: [0.0, 0.0],
        name: mark_name(unit),
    };
    let reach = fit::reach(&rows)?;

    let mut positions = BTreeMap::new();
    let mut fit_values = BTreeMap::new();
    for (index, group) in groups.iter().enumerate() {
        if group.plot.is_none() {
            continue;
        }
        let data = matches!(
            scale::domain_source(chart, group.spec.domain.as_ref(), &selections)?,
            scale::DomainSource::Data | scale::DomainSource::Explicit(_)
        ) || selection_domain(&group.spec);
        let values: FitValues = group
            .members
            .iter()
            .filter_map(|member| {
                let values = member_values(chart, unit.unit, member.channel, &drawn)?;
                let along = match (&reach, values.len() == drawn.num_rows() && data) {
                    (Some(reach), true) => {
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

    let systems = CoordSystems::prepare(&SystemInputs {
        units: &units,
        groups: &groups,
    })?;
    let (guides, layout) = guides_and_layout(
        chart,
        &units,
        &groups,
        &positions,
        &scales,
        &systems.guide(0),
        theme,
        engine,
    )?;
    Ok(Prepared {
        groups,
        positions,
        fit_values,
        scales,
        scale_values,
        drawn,
        reach,
        systems,
        guides,
        layout,
    })
}

/// Warns when a mark of a unit whose position scale has the producer's summary as its domain
/// reaches past the edges of its plot at the fitted domain.
fn check_summary_reach(
    rows: &UnitRows,
    prepared: &Prepared,
    path: &str,
    warnings: &mut Vec<EvaluationWarning>,
) -> Result<()> {
    let Some(reach) = &prepared.reach else {
        return Ok(());
    };
    for (index, position) in &prepared.positions {
        if !position.summary {
            continue;
        }
        let group = &prepared.groups[*index];
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
