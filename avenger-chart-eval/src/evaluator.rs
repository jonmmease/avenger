//! The preparation of a chart, which reads it once, and its evaluation at a size.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{ArrayRef, RecordBatch},
    datatypes::DataType,
};
use avenger_chart::{
    layer::{LayerRef, MarkMessage},
    proto::{encoding, node::Node, plot::Coord, Channel},
    resolve::{scale_channel, Identity, Layer, PlotRef, Resolved},
    Chart, FitKind,
};
use avenger_color::ColorOrGradient;
use avenger_coords::PlotCoords;
use avenger_geometry::data::SceneData;
use avenger_layout::{Measured, Size};
use avenger_scales::scales::{fitting::Direction, ConfiguredScale};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
        rect::SceneRectMark,
    },
    scene_graph::SceneGraph,
};
use avenger_selection::SelectionState;
use avenger_style::{Cascade, ComputedStyle, ComputedStyles, HostInputs, Property, StyleTree};
use avenger_typst_label::LabelEngine;

use crate::convert::domain::FitDefaults;
use crate::{
    convert::{scale::fit_kind, selection},
    encode::{self, at, LayerRows},
    fit::{self, FitInputs, MemberValues, Position},
    guides::{self, GuideInputs, Guides},
    layout::{self, Lowered, Lowering, Measurer, SlabId},
    marks,
    polar::{self, PolarInputs, Polars},
    reach::{self, Reach},
    style::tree::{self, StyleNodes},
    style::{color as style_color, number as style_number},
    EvalError, EvalWarning, Registry,
};

type Result<T> = std::result::Result<T, EvalError>;

/// A chart evaluated at one size: its scene, the rows behind its marks, its plots, and the
/// warnings of preparing and evaluating it.
pub struct EvaluatedChart {
    pub scene: SceneGraph,
    /// The rows of each mark's table that draw its instances, by the mark's name.
    pub data: SceneData,
    pub plots: Vec<EvaluatedPlot>,
    pub warnings: Vec<EvalWarning>,
}

/// A plot of an evaluation: its name and identity, its rectangle in the scene, its coordinates,
/// and its layers.
#[derive(Clone, Debug)]
pub struct EvaluatedPlot {
    /// The name of its identity, or its node path when its node has no id, which names its group
    /// in the scene.
    pub name: String,
    /// The identity of its node, when it has an id.
    pub identity: Option<Identity>,
    /// Its left, top, width, and height in the scene.
    pub rect: [f32; 4],
    /// The projection between its pixels, from its top left, and the positions of `x` and `y`,
    /// with the scales of its first `x` and its first `y`.
    pub coords: PlotCoords,
    /// Its layers, in the order that they draw.
    pub layers: Vec<EvaluatedLayer>,
}

/// A layer of a plot of an evaluation: its path, the names of its marks, and its coordinates.
#[derive(Clone, Debug)]
pub struct EvaluatedLayer {
    /// Its path in the chart, such as `root.plot.cartesian.layers[0]`.
    pub path: String,
    /// The names of its marks with rows in `EvaluatedChart::data`, in the order that they draw.
    pub marks: Vec<String>,
    /// The coordinates of the plot with the scales of the layer's `x` and `y`, which differ from
    /// the plot's for a layer with an independent scale.
    pub coords: PlotCoords,
}

/// What the evaluator derives from the computed styles, at no size: the scales, the reach of the
/// marks, the guides, and the layout.
struct Prepared {
    /// The position scale of each position group, by its position in `Resolved::scales`.
    positions: Vec<Option<Position>>,
    /// The scale of each other group, by its position in `Resolved::scales`.
    scales: Vec<Option<ConfiguredScale>>,
    /// The reach of each drawn row of each layer.
    reach: Vec<Option<Vec<Reach>>>,
    guides: Guides,
    polars: Polars,
    lowered: Lowered,
    warnings: Vec<EvalWarning>,
}

/// A chart prepared for evaluation at any size: its resolved models, its style tree, its drawn rows
/// and selection states, and what the computed styles of the last host inputs give.
///
/// The computed styles are computed again only when the host's preferences or theme change, or
/// when a resize changes the outcome of a condition. The scales, the guides, and the layout follow
/// the styles, and a resize within a breakpoint only solves the layout and fits the position
/// scales again.
pub struct ChartEvaluator {
    chart: Chart,
    cascade: Arc<Cascade>,
    engine: LabelEngine,
    registry: Registry,
    tree: StyleTree,
    nodes: StyleNodes,
    /// The rows of each layer that draw an instance, by its position in the tree's layers.
    drawn: Vec<RecordBatch>,
    /// The kind of fit of each scale group, by its position in `Resolved::scales`.
    kinds: Vec<FitKind>,
    /// The state of each selection by name, with its initial clauses.
    states: BTreeMap<String, SelectionState>,
    styles: Option<ComputedStyles>,
    prepared: Option<Prepared>,
    measured: Option<Measured<SlabId>>,
    warnings: Vec<EvalWarning>,
}

impl ChartEvaluator {
    /// Prepares `chart` with the cascade of its style table, a label engine, and the custom scales
    /// of `registry`. It fails for a chart with a geo plot, and for an empty chart, which this
    /// version of the evaluator does not draw.
    pub fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        engine: LabelEngine,
        registry: Registry,
    ) -> Result<Self> {
        if matches!(chart.root().node, Some(Node::Empty(_))) {
            return Err(EvalError::Unsupported("an empty chart".into()));
        }
        for plot in &chart.resolve().tree.plots {
            if let Some(Coord::Geo(_)) = plot.plot.coord.as_ref() {
                return Err(EvalError::Unsupported("a geo plot".into()));
            }
        }
        crate::convert::format::check(&chart, &engine)?;
        let states = selection::states(&chart)?;
        let (tree, nodes, drawn, kinds, warnings) = {
            let resolved = chart.resolve();
            let kinds = (resolved.scales.iter())
                .map(|group| fit_kind(&group.spec, &registry))
                .collect::<Result<Vec<_>>>()?;
            let (tree, nodes, mut warnings) = tree::build(&chart, &resolved, &kinds);
            let drawn = (resolved.tree.layers.iter())
                .map(|layer| encode::drawn_rows(layer.mark, layer.table))
                .collect::<Result<Vec<_>>>()?;
            warnings.extend((resolved.widgets.iter()).map(|group| {
                let member = &group.members[0];
                let layer = &resolved.tree.layers[member.layer];
                EvalWarning::NotDrawn {
                    path: format!("{}.widgets[{}]", layer.path, member.index),
                    reason: format!(
                        "the widget `{}` does not draw yet, but its initial clause applies",
                        group.key
                    ),
                }
            }));
            warnings.extend(crate::convert::domain::selection_warnings(
                &resolved.scales,
                &states,
            ));
            (tree, nodes, drawn, kinds, warnings)
        };
        Ok(Self {
            chart,
            cascade,
            engine,
            registry,
            tree,
            nodes,
            drawn,
            kinds,
            states,
            styles: None,
            prepared: None,
            measured: None,
            warnings,
        })
    }

    /// The chart that the evaluator draws.
    pub fn chart(&self) -> &Chart {
        &self.chart
    }

    /// The style tree of the chart, whose nodes stand for its parts.
    pub fn style_tree(&self) -> &StyleTree {
        &self.tree
    }

    /// The warnings of preparing the chart, such as widgets that do not draw yet.
    pub fn warnings(&self) -> &[EvalWarning] {
        &self.warnings
    }

    /// The computed styles of the chart's style tree for `host`, computed again only when the
    /// host's preferences or theme change, or when a resize changes the outcome of a condition.
    pub fn styles(&mut self, host: &HostInputs) -> &ComputedStyles {
        self.restyle(host);
        self.styles.as_ref().expect("the styles are computed")
    }

    /// Computes the styles for `host` when they are stale, and then drops what they gave.
    fn restyle(&mut self, host: &HostInputs) {
        let stale = (self.styles.as_ref()).is_none_or(|styles| styles.needs_update(host));
        if stale {
            self.styles = Some(self.cascade.compute(&self.tree, host));
            self.prepared = None;
            self.measured = None;
        }
    }

    /// Evaluates the chart for `host`: at the host's size, at the chart's `width` and `height` in
    /// a dimension where the host gives none, and at the chart's natural size in a dimension
    /// without either.
    pub fn evaluate(&mut self, host: &HostInputs) -> Result<EvaluatedChart> {
        self.restyle(host);
        let styles = self.styles.as_ref().expect("the styles are computed");
        let chart = &self.chart;
        let resolved = chart.resolve();
        if self.prepared.is_none() {
            self.prepared = Some(prepare(&Inputs {
                chart,
                resolved: &resolved,
                nodes: &self.nodes,
                styles,
                registry: &self.registry,
                engine: &self.engine,
                states: &self.states,
                drawn: &self.drawn,
                kinds: &self.kinds,
            })?);
        }
        let prepared = self.prepared.as_ref().expect("the chart is prepared");

        let message = chart.message();
        let size = [host.width.or(message.width), host.height.or(message.height)];
        let options = layout::solve_options(size, &styles[self.nodes.chart]);
        let mut measurer = Measurer {
            guides: &prepared.guides,
        };
        let layout = &prepared.lowered.layout;
        let measured = match (self.measured.take(), size) {
            (Some(mut measured), [Some(width), Some(height)])
                if measured.options().width.is_some() && measured.options().height.is_some() =>
            {
                measured.remeasure(Size::new(width, height), &mut measurer)?;
                measured
            }
            (Some(previous), _) => layout.measure_reusing(&previous, &options, &mut measurer)?,
            (None, _) => layout.measure(&options, &mut measurer)?,
        };
        let solution = measured.solution().clone();
        self.measured = Some(measured);

        let mut warnings = self.warnings.clone();
        warnings.extend(styles.warnings().iter().cloned().map(EvalWarning::Style));
        warnings.extend(prepared.warnings.iter().cloned());
        if solution.explanation().stopped_at_limit {
            warnings.push(EvalWarning::Fit {
                path: "root".to_string(),
                reason: "the layout stopped at its limit of solves, so its parts may not fit"
                    .to_string(),
            });
        }

        let rects: Vec<avenger_layout::Rect> = (0..resolved.tree.plots.len())
            .map(|plot| {
                solution
                    .region(&SlabId::Plot(plot))
                    .expect("the layout has every plot")
                    .content
            })
            .collect();

        // Each position scale of a Cartesian plot fits at the size of its first plot: the plots of
        // a shared scale stand in matched tracks of one length.
        let mut positions: Vec<Option<ConfiguredScale>> = vec![None; resolved.scales.len()];
        for (index, position) in prepared.positions.iter().enumerate() {
            let Some(position) = position else {
                continue;
            };
            let group = &resolved.scales[index];
            if matches!(group.channel, Channel::Theta | Channel::Radius) {
                continue;
            }
            let plot = resolved.tree.layers[group.members[0].layer].plot;
            let rect = rects[plot];
            let length = match group.channel {
                Channel::Y => rect.height,
                _ => rect.width,
            };
            let at = position.fitting.at((0.0, f64::from(length)))?;
            if !at.marks_fit {
                warnings.push(EvalWarning::Fit {
                    path: resolved.tree.plots[plot].path.clone(),
                    reason: format!(
                        "the plot is too small along `{}` for the reach of its marks",
                        group.channel.field_name()
                    ),
                });
            }
            positions[index] = Some(at.scale);
        }
        // Each polar plot fits its circle at its size, and the scales of `theta` and `radius` take
        // their ranges at its frame, which the plots of a shared scale share.
        let sizes: Vec<[f32; 2]> = rects.iter().map(|rect| [rect.width, rect.height]).collect();
        let polar_fits = prepared.polars.fit(&sizes, &mut warnings)?;
        for fit in polar_fits.values() {
            for (group, scale) in [&fit.theta, &fit.radius].into_iter().flatten() {
                positions[*group].get_or_insert_with(|| scale.clone());
            }
        }

        let canvas = solution.size();
        let mut marks: Vec<SceneMark> = Vec::new();
        marks.extend(backdrop(
            "background",
            &styles[self.nodes.chart],
            [0.0, 0.0, canvas.width, canvas.height],
        ));
        let mut data = SceneData::new();
        let mut plots = Vec::with_capacity(resolved.tree.plots.len());
        for (plot_index, plot_ref) in resolved.tree.plots.iter().enumerate() {
            let rect = rects[plot_index];
            let plot_size = [rect.width, rect.height];
            let in_plot = |group: &avenger_chart::resolve::ScaleGroup| {
                (group.members.iter())
                    .any(|member| resolved.tree.layers[member.layer].plot == plot_index)
            };
            let polar_fit = polar_fits.get(&plot_index);
            let clip = match polar_fit {
                Some(fit) => fit.clip,
                None => {
                    (resolved.scales.iter().zip(&prepared.positions)).any(|(group, position)| {
                        in_plot(group) && position.as_ref().is_some_and(|p| p.clip)
                    })
                }
            };
            let mut plot_marks: Vec<SceneMark> = match polar_fit {
                Some(fit) => prepared.polars.backdrop(fit),
                None => (prepared.guides.plots[plot_index].axes.iter())
                    .map(|axis| &prepared.guides.axes[*axis])
                    .filter(|axis| axis.grid)
                    .map(|axis| axis.axis.grid(plot_size).into())
                    .collect(),
            };
            let mut layers = Vec::with_capacity(plot_ref.layers.len());
            for index in plot_ref.layers.clone() {
                let layer = &resolved.tree.layers[index];
                let mut rows = LayerRows {
                    chart,
                    layer: layer.layer,
                    mark: layer.mark,
                    style: &styles[self.nodes.marks[index]],
                    styles,
                    node: self.nodes.marks[index],
                    table: self.drawn[index].clone(),
                    partition: &layer.partition,
                    scales: layer_scales(&resolved, index, &positions, &prepared.scales),
                    masks: BTreeMap::new(),
                    kinds: layer_kinds(&resolved, index, &self.kinds),
                    engine: &self.engine,
                    size: plot_size,
                    name: format!("{}/{}", plot_ref.name, layer.position),
                };
                rows.masks = encode::masks(
                    &rows,
                    &self.states,
                    &owners(plot_ref, layer),
                    &layer.path,
                    &mut warnings,
                )?;
                // A layer whose `mark` node computes `display: none` draws nothing and has no rows to
                // hit, and its values still fit its scales, so hiding it rescales no other layer.
                let hidden = rows.style.is_hidden();
                if let Some(reason) = marks::image_warning(&rows)?.filter(|_| !hidden) {
                    warnings.push(EvalWarning::NotDrawn {
                        path: layer.path.clone(),
                        reason,
                    });
                }
                if let Some(reason) = rows.size_warning()?.filter(|_| !hidden) {
                    warnings.push(EvalWarning::NotDrawn {
                        path: layer.path.clone(),
                        reason,
                    });
                }
                let mut names = Vec::new();
                let drawn = match (hidden, layer.layer, polar_fit) {
                    (true, _, _) => Vec::new(),
                    (false, LayerRef::Cartesian(cartesian), _) => {
                        let mark = cartesian.mark.as_ref().expect("a valid layer has a mark");
                        marks::marks(&rows, mark)?
                    }
                    (false, LayerRef::Polar(layer), Some(fit)) => {
                        let mark = layer.mark.as_ref().expect("a valid layer has a mark");
                        polar::marks(&rows, mark, fit)?
                    }
                    _ => unreachable!("a plot draws the layers of its coordinate system"),
                };
                for drawn in drawn {
                    let name = drawn.mark.mark_name().to_string();
                    data.insert(name.clone(), Arc::new(drawn.rows));
                    names.push(name);
                    plot_marks.push(drawn.mark);
                }
                let reach = prepared.reach[index].as_deref();
                summary_reach(&rows, &resolved, index, reach, &layer.path, &mut warnings)?;
                let coords = match polar_fit {
                    Some(fit) => fit.coords(),
                    None => {
                        let [x, y] = [Channel::X, Channel::Y].map(|channel| {
                            (resolved.scales.iter().enumerate())
                                .find(|(_, group)| {
                                    (group.members.iter()).any(|member| {
                                        member.layer == index && member.channel == channel
                                    })
                                })
                                .and_then(|(group, _)| positions[group].clone())
                        });
                        PlotCoords::cartesian(plot_size, [x, y])
                    }
                };
                layers.push(EvaluatedLayer {
                    path: layer.path.clone(),
                    marks: names,
                    coords,
                });
            }
            let coords = match polar_fit {
                Some(fit) => {
                    plot_marks.extend(prepared.polars.overlay(fit));
                    fit.coords()
                }
                None => {
                    let [x, y] = [Channel::X, Channel::Y].map(|channel| {
                        (resolved.scales.iter().enumerate())
                            .find(|(_, group)| group.channel == channel && in_plot(group))
                            .and_then(|(group, _)| positions[group].clone())
                    });
                    PlotCoords::cartesian(plot_size, [x, y])
                }
            };
            marks.extend(backdrop(
                &format!("{}/background", plot_ref.name),
                &styles[self.nodes.plot(&plot_ref.path).plot],
                [rect.x, rect.y, rect.width, rect.height],
            ));
            marks.push(
                SceneGroup {
                    name: plot_ref.name.clone(),
                    origin: [rect.x, rect.y],
                    clip: match clip {
                        true => Clip::Rect {
                            x: 0.0,
                            y: 0.0,
                            width: rect.width,
                            height: rect.height,
                        },
                        false => Clip::None,
                    },
                    marks: plot_marks,
                    ..Default::default()
                }
                .into(),
            );
            plots.push(EvaluatedPlot {
                name: plot_ref.name.clone(),
                identity: plot_ref.identity.clone(),
                rect: [rect.x, rect.y, rect.width, rect.height],
                coords,
                layers,
            });
        }
        marks.extend(layout::draw(&prepared.guides, &prepared.lowered, &solution));
        Ok(EvaluatedChart {
            scene: SceneGraph {
                marks,
                width: canvas.width,
                height: canvas.height,
                origin: [0.0, 0.0],
            },
            data,
            plots,
            warnings,
        })
    }
}

/// What preparing a chart's scales, guides, and layout reads.
struct Inputs<'a> {
    chart: &'a Chart,
    resolved: &'a Resolved<'a>,
    nodes: &'a StyleNodes,
    styles: &'a ComputedStyles,
    registry: &'a Registry,
    engine: &'a LabelEngine,
    states: &'a BTreeMap<String, SelectionState>,
    drawn: &'a [RecordBatch],
    kinds: &'a [FitKind],
}

/// The scales, the reach of the marks, the guides, and the layout of a chart at its computed
/// styles. The non-position scales fit first, without a size, so that the reach of each mark takes
/// them, and the position scales fit their domains to the values with the reach of their marks.
fn prepare(inputs: &Inputs) -> Result<Prepared> {
    let Inputs {
        chart,
        resolved,
        nodes,
        styles,
        registry,
        engine,
        states,
        drawn,
        kinds,
    } = *inputs;
    let groups = &resolved.scales;
    let layers = &resolved.tree.layers;
    let fit_inputs = FitInputs {
        chart,
        registry,
        states,
    };
    let mut warnings = Vec::new();
    let values_of = |member: &avenger_chart::resolve::Member| {
        member_values(
            chart,
            layers[member.layer].mark,
            member.channel,
            &drawn[member.layer],
        )
    };

    let mut scales: Vec<Option<ConfiguredScale>> = vec![None; groups.len()];
    for (index, group) in groups.iter().enumerate() {
        if group.is_position() {
            continue;
        }
        let values = (group.members.iter())
            .map(values_of)
            .collect::<Result<Vec<Option<ArrayRef>>>>()?;
        scales[index] = Some(fit::non_position(
            &fit_inputs,
            group,
            fit_defaults(resolved, index, kinds),
            &styles[nodes.scales[index]],
            styles,
            &values,
            &mut warnings,
        )?);
    }

    // The rows of each layer at no size, with the non-position scales.
    let no_positions = vec![None; groups.len()];
    let unsized_rows = |index: usize| {
        let layer = &layers[index];
        LayerRows {
            chart,
            layer: layer.layer,
            mark: layer.mark,
            style: &styles[nodes.marks[index]],
            styles,
            node: nodes.marks[index],
            table: drawn[index].clone(),
            partition: &layer.partition,
            scales: layer_scales(resolved, index, &no_positions, &scales),
            masks: BTreeMap::new(),
            kinds: layer_kinds(resolved, index, kinds),
            engine,
            size: [0.0, 0.0],
            name: String::new(),
        }
    };
    let reach = (0..layers.len())
        .map(|index| reach::reach(&unsized_rows(index)))
        .collect::<Result<Vec<_>>>()?;

    let mut positions: Vec<Option<Position>> = vec![None; groups.len()];
    for (index, group) in groups.iter().enumerate() {
        if !group.is_position() {
            continue;
        }
        // The domain of `x` or `y` fits the reach of the marks of its own rows, toward its lower
        // values and its higher ones, which depend on the direction of the scale along the plot.
        // The reach of the marks of a polar plot is in pixels across the plot, which its outer
        // radius takes instead.
        let along_plot = matches!(group.channel, Channel::X | Channel::Y);
        let reversed = match group.channel {
            Channel::Y => fit::direction(group, kinds[index]) == Direction::Forward,
            _ => fit::direction(group, kinds[index]) == Direction::Reverse,
        };
        let mut values: Vec<Option<MemberValues>> = Vec::with_capacity(group.members.len());
        for member in &group.members {
            let Some(member_values) = values_of(member)? else {
                values.push(None);
                continue;
            };
            if outer_level(layers[member.layer].mark, member.channel, &member_values) {
                values.push(None);
                continue;
            }
            // The values hold each source of the encoding for every row, one source after
            // another, so each source's values take the reach of their rows.
            let along = match &reach[member.layer] {
                Some(reach)
                    if along_plot
                        && !reach.is_empty()
                        && member_values.len() % reach.len() == 0
                        && member.summary.is_none() =>
                {
                    let sources = member_values.len() / reach.len();
                    let (low, high) = reach::along(reach, group.channel, reversed);
                    Some((low.repeat(sources), high.repeat(sources)))
                }
                _ => None,
            };
            values.push(Some((member_values, along)));
        }
        positions[index] = Some(fit::position(
            &fit_inputs,
            group,
            fit_defaults(resolved, index, kinds),
            &styles[nodes.scales[index]],
            &values,
            polar::wraps(resolved, group),
        )?);
    }
    let polars = Polars::prepare(
        &PolarInputs {
            chart,
            resolved,
            nodes,
            styles,
            engine,
            positions: &positions,
        },
        unsized_rows,
        &reach,
    )?;

    let fittings: Vec<_> = (positions.iter())
        .map(|position| position.as_ref().map(|position| position.fitting.clone()))
        .collect();
    let guides = guides::guides(&GuideInputs {
        chart,
        resolved,
        nodes,
        styles,
        engine,
        positions: &fittings,
        scales: &scales,
        kinds,
    })?;
    // A discrete position scale with a `step` gives each plot of its layers a natural length along
    // its channel.
    let mut natural: Vec<[Option<f32>; 2]> = vec![[None, None]; resolved.tree.plots.len()];
    for (index, group) in groups.iter().enumerate() {
        let Some(position) = &positions[index] else {
            continue;
        };
        let slot = match group.channel {
            Channel::X => 0,
            Channel::Y => 1,
            _ => continue,
        };
        let Some(step) = styles[nodes.scales[index]].number(Property::Step) else {
            continue;
        };
        let Ok(at) = position.fitting.at((0.0, 1.0)) else {
            continue;
        };
        let Some(length) = layout::natural_length(&at.scale, step) else {
            continue;
        };
        for member in &group.members {
            let plot = &mut natural[resolved.tree.layers[member.layer].plot][slot];
            *plot = Some(plot.map_or(length, |known: f32| known.max(length)));
        }
    }
    let lowered = layout::lower(&Lowering {
        chart,
        resolved,
        guides: &guides,
        nodes,
        styles,
        natural: &natural,
    });
    Ok(Prepared {
        positions,
        scales,
        reach,
        guides,
        polars,
        lowered,
        warnings,
    })
}

/// Whether the encoding of `channel` of `mark` places its marks at an outer level of a nested
/// band scale, whose `values` name only their groups, so they add no values to the fit of the
/// scale's domain.
fn outer_level(mark: &dyn MarkMessage, channel: Channel, values: &ArrayRef) -> bool {
    let level = (mark.encoding(scale_channel(channel))).and_then(|encoding| encoding.band_level);
    match (level, values.data_type()) {
        (Some(level), DataType::Struct(fields)) => (level as usize) + 1 < fields.len(),
        _ => false,
    }
}

/// The values of the encoding of `channel` of `mark` that go through its scale: the scaled column in
/// `table` or the scaled constant of its value and of each of its conditional values, in order, as
/// the type of the first. A conditional value can be the only source of the scale, as a gray
/// constant whose selected rows take the scale of a column.
fn member_values(
    chart: &Chart,
    mark: &dyn MarkMessage,
    channel: Channel,
    table: &RecordBatch,
) -> Result<Option<ArrayRef>> {
    use avenger_chart::proto::conditional_value;
    let Some(encoding) = mark.encoding(channel) else {
        return Ok(None);
    };
    let base = match encoding.kind.as_ref() {
        Some(encoding::Kind::ScaledField(name)) => (table.column_by_name(name))
            .map(|column| avenger_chart::span::channel_values(column, channel)),
        Some(encoding::Kind::ScaledValue(value)) => Some(chart.value(value).clone()),
        Some(encoding::Kind::DirectField(_) | encoding::Kind::DirectValue(_)) | None => None,
    };
    let conditionals = (encoding.conditional.iter()).filter_map(|conditional| {
        match conditional.kind.as_ref()? {
            conditional_value::Kind::ScaledField(name) => table.column_by_name(name).cloned(),
            conditional_value::Kind::ScaledValue(value) => Some(chart.value(value).clone()),
            conditional_value::Kind::DirectField(_) | conditional_value::Kind::DirectValue(_) => {
                None
            }
        }
    });
    let sources: Vec<ArrayRef> = base.into_iter().chain(conditionals).collect();
    let Some(first) = sources.first() else {
        return Ok(None);
    };
    if sources.len() == 1 {
        return Ok(Some(first.clone()));
    }
    let data_type = first.data_type().clone();
    let sources = (sources.iter())
        .map(|source| arrow::compute::cast(source, &data_type))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let sources: Vec<&dyn arrow::array::Array> =
        sources.iter().map(|source| source.as_ref()).collect();
    Ok(Some(arrow::compute::concat(&sources)?))
}

/// The scales of the channels of the layer `layer`: the position scales of `positions` and the
/// other scales of `scales`, by the groups of its encodings.
fn layer_scales<'a>(
    resolved: &Resolved,
    layer: usize,
    positions: &'a [Option<ConfiguredScale>],
    scales: &'a [Option<ConfiguredScale>],
) -> BTreeMap<Channel, &'a ConfiguredScale> {
    let mut channels = BTreeMap::new();
    for (index, group) in resolved.scales.iter().enumerate() {
        let Some(scale) = positions[index].as_ref().or(scales[index].as_ref()) else {
            continue;
        };
        for member in group.members.iter().filter(|member| member.layer == layer) {
            channels.insert(member.channel, scale);
        }
    }
    channels
}

/// The `zero` and the `nice` that the scale group `index` takes where its spec leaves them absent.
/// A quantitative `x`, `y`, or `size` scale contains zero when one of its members asks for it:
/// every member except the key of a bar, an area, or a line, along which the marks stand at their
/// values. A log or a time scale, or a selection domain, contains no zero by default. A
/// quantitative `x` or `y` scale has nice ends at a count of 10. The key of bars of bins takes
/// neither.
pub(crate) fn fit_defaults(resolved: &Resolved, index: usize, kinds: &[FitKind]) -> FitDefaults {
    use avenger_chart::proto::{domain, scale::Type};
    let group = &resolved.scales[index];
    let scale_type = group.spec.r#type.as_ref();
    let linear = matches!(
        scale_type,
        Some(Type::Linear(_) | Type::Pow(_) | Type::Symlog(_))
    );
    let quantitative = linear || matches!(scale_type, Some(Type::Log(_)));
    let selection = matches!(
        (group.spec.domain.as_ref()).and_then(|domain| domain.domain.as_ref()),
        Some(domain::Domain::Selection(_))
    );
    let asks = |member: &avenger_chart::resolve::Member| {
        let layer = &resolved.tree.layers[member.layer];
        let layer_kinds = layer_kinds(resolved, member.layer, kinds);
        let continuous = |channel| layer_kinds.get(&channel) == Some(&FitKind::Interval);
        crate::bins::key_channel(layer, continuous) != Some(scale_channel(member.channel))
    };
    // The key of bars of bins spans from the least start to the greatest end, without zero or
    // nice ends, so the outer edges meet the ends of the plot.
    if crate::bins::edges(resolved, index, kinds).is_some() {
        return FitDefaults::default();
    }
    let zero = linear
        && !selection
        && matches!(group.channel, Channel::X | Channel::Y | Channel::Size)
        && group.members.iter().any(asks);
    let nice = (quantitative && matches!(group.channel, Channel::X | Channel::Y))
        .then_some(FitDefaults::NICE);
    FitDefaults { zero, nice }
}

/// The kind of fit of the scale of each channel of the layer `layer`, by the groups of its
/// encodings.
fn layer_kinds(resolved: &Resolved, layer: usize, kinds: &[FitKind]) -> BTreeMap<Channel, FitKind> {
    let mut channels = BTreeMap::new();
    for (group, kind) in resolved.scales.iter().zip(kinds) {
        for member in group.members.iter().filter(|member| member.layer == layer) {
            channels.insert(member.channel, *kind);
        }
    }
    channels
}

/// The owners of the rows of a layer: the ids of the layer and of its plot.
fn owners(plot: &PlotRef, layer: &Layer) -> Vec<String> {
    [layer.identity.as_ref(), plot.identity.as_ref()]
        .into_iter()
        .flatten()
        .map(|identity| identity.id.clone())
        .collect()
}

/// Warns when a mark of the layer `layer`, whose position scale takes the producer's summary for
/// the layer, reaches past the edges of its plot at the fitted domain: the summary did not hold the
/// reach of the drawn marks.
fn summary_reach(
    rows: &LayerRows,
    resolved: &Resolved,
    layer: usize,
    reach: Option<&[Reach]>,
    path: &str,
    warnings: &mut Vec<EvalWarning>,
) -> Result<()> {
    let Some(reach) = reach else {
        return Ok(());
    };
    for group in &resolved.scales {
        let summarized = (group.members.iter()).any(|member| {
            member.layer == layer && member.channel == group.channel && member.summary.is_some()
        });
        let (length, low, high) = match group.channel {
            Channel::X => (rows.size[0], 0, 1),
            Channel::Y => (rows.size[1], 2, 3),
            _ => continue,
        };
        if !summarized || !rows.has(group.channel) {
            continue;
        }
        let values = rows.position(group.channel)?;
        let outside = (0..rows.len()).any(|row| {
            let position = at(&values, row);
            position - reach[row][low] < -0.5 || position + reach[row][high] > length + 0.5
        });
        if outside {
            warnings.push(EvalWarning::Fit {
                path: path.to_string(),
                reason: format!(
                    "a mark along `{}` reaches past the edge of its plot at the domain of the \
                     producer's summary",
                    group.channel.field_name()
                ),
            });
        }
    }
    Ok(())
}

/// The z-index of the backgrounds of the chart and the plot: renderers order a scene's marks by
/// z-index across the whole scene, and the grid of an axis stands at -1, behind the data.
const BACKGROUND_ZINDEX: i32 = -2;

/// The rectangle `rect` with the computed background and border of `style`, behind the grid, or
/// `None` when it paints neither.
fn backdrop(name: &str, style: &ComputedStyle, rect: [f32; 4]) -> Option<SceneMark> {
    let fill = style_color(style, Property::Background);
    let width = style_number(style, Property::BorderWidth);
    let border = (style.keyword(Property::BorderStyle))
        .is_some_and(|line| line != "none")
        .then(|| style.color(Property::BorderColor))
        .flatten()
        .filter(|color| width > 0.0 && color[3] > 0.0);
    if fill[3] == 0.0 && border.is_none() {
        return None;
    }
    let [x, y, width_, height] = rect;
    Some(
        SceneRectMark {
            name: name.to_string(),
            interactive: Some(false),
            len: 1,
            x: x.into(),
            y: y.into(),
            width: Some(width_.into()),
            height: Some(height.into()),
            fill: ColorOrGradient::Color(fill).into(),
            stroke: ColorOrGradient::Color(border.unwrap_or([0.0; 4])).into(),
            stroke_width: match border {
                Some(_) => width.into(),
                None => 0.0.into(),
            },
            corner_radius: style_number(style, Property::BorderRadius).into(),
            zindex: Some(BACKGROUND_ZINDEX),
            ..Default::default()
        }
        .into(),
    )
}
