//! The preparation of a chart, which reads it once, and its evaluation at a size.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{ArrayRef, RecordBatch};
use avenger_chart::{
    layer::{LayerRef, MarkMessage},
    proto::{cartesian_layer, encoding, node::Node, plot::Coord, Channel},
    resolve::{Identity, Layer, PlotRef, Resolved},
    Chart,
};
use avenger_color::ColorOrGradient;
use avenger_coords::PlotCoords;
use avenger_geometry::data::SceneData;
use avenger_layout::{Layout, Measured, Size};
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

use crate::{
    convert::selection,
    encode::{self, at, LayerRows},
    fit::{self, FitInputs, MemberValues, Position},
    guides::{self, GuideInputs, Guides},
    layout::{self, Measurer, SlabId},
    marks,
    reach::{self, Reach},
    style::tree::{self, StyleNodes},
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
    layout: Layout<SlabId>,
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
    /// The state of each selection by name, with its initial clauses.
    states: BTreeMap<String, SelectionState>,
    styles: Option<ComputedStyles>,
    prepared: Option<Prepared>,
    measured: Option<Measured<SlabId>>,
    warnings: Vec<EvalWarning>,
}

/// The mark of a layer of a Cartesian plot.
fn cartesian_mark<'a>(layer: &Layer<'a>) -> &'a cartesian_layer::Mark {
    match layer.layer {
        LayerRef::Cartesian(layer) => layer.mark.as_ref().expect("a valid layer has a mark"),
        _ => unreachable!("a Cartesian plot draws Cartesian layers"),
    }
}

impl ChartEvaluator {
    /// Prepares `chart` with the cascade of its style table, a label engine, and the custom scales
    /// of `registry`. It fails for a chart whose root is not one Cartesian plot, which this version
    /// of the evaluator does not draw.
    pub fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        engine: LabelEngine,
        registry: Registry,
    ) -> Result<Self> {
        match chart
            .root()
            .node
            .as_ref()
            .expect("a valid chart has a root")
        {
            Node::Plot(plot) => match plot
                .coord
                .as_ref()
                .expect("a valid plot has a coordinate system")
            {
                Coord::Cartesian(_) => {}
                Coord::Polar(_) => return Err(EvalError::Unsupported("a polar plot".into())),
                Coord::Geo(_) => return Err(EvalError::Unsupported("a map plot".into())),
            },
            Node::Concat(_) => return Err(EvalError::Unsupported("a concat".into())),
            Node::Wrap(_) => return Err(EvalError::Unsupported("a wrap".into())),
            Node::Empty(_) => return Err(EvalError::Unsupported("an empty chart".into())),
        }
        crate::convert::format::check(&chart, &engine)?;
        let states = selection::states(&chart)?;
        let (tree, nodes, drawn, warnings) = {
            let resolved = chart.resolve();
            debug_assert!(tree::is_cartesian(resolved.tree.plots[0].plot));
            let (tree, nodes) = tree::build(&chart, &resolved);
            let drawn = (resolved.tree.layers.iter())
                .map(|layer| encode::drawn_rows(cartesian_mark(layer), layer.table))
                .collect::<Result<Vec<_>>>()?;
            let warnings: Vec<EvalWarning> = (resolved.widgets.iter())
                .map(|group| {
                    let member = &group.members[0];
                    let layer = &resolved.tree.layers[member.layer];
                    EvalWarning::NotDrawn {
                        path: format!("{}.widgets[{}]", layer.path, member.index),
                        reason: format!(
                            "the widget `{}` does not draw yet, but its initial clause applies",
                            group.key
                        ),
                    }
                })
                .collect();
            (tree, nodes, drawn, warnings)
        };
        Ok(Self {
            chart,
            cascade,
            engine,
            registry,
            tree,
            nodes,
            drawn,
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
            })?);
        }
        let prepared = self.prepared.as_ref().expect("the chart is prepared");

        let message = chart.message();
        let size = [host.width.or(message.width), host.height.or(message.height)];
        let options = layout::solve_options(size, &styles[self.nodes.chart]);
        let mut measurer = Measurer {
            guides: &prepared.guides,
        };
        let measured = match (self.measured.take(), size) {
            (Some(mut measured), [Some(width), Some(height)])
                if measured.options().width.is_some() && measured.options().height.is_some() =>
            {
                measured.remeasure(Size::new(width, height), &mut measurer)?;
                measured
            }
            (Some(previous), _) => {
                (prepared.layout).measure_reusing(&previous, &options, &mut measurer)?
            }
            (None, _) => prepared.layout.measure(&options, &mut measurer)?,
        };
        let solution = measured.solution().clone();
        self.measured = Some(measured);

        let mut warnings = self.warnings.clone();
        warnings.extend(styles.warnings().iter().cloned().map(EvalWarning::Style));
        warnings.extend(prepared.warnings.iter().cloned());

        let plot_ref = &resolved.tree.plots[0];
        let rect = solution
            .region(&SlabId::Plot)
            .expect("the layout has the plot")
            .content;
        let plot_size = [rect.width, rect.height];

        // The position scales fit at the plot's size.
        let mut positions: Vec<Option<ConfiguredScale>> = vec![None; resolved.scales.len()];
        for (index, position) in prepared.positions.iter().enumerate() {
            let Some(position) = position else {
                continue;
            };
            let channel = resolved.scales[index].channel;
            let length = match channel {
                Channel::Y => rect.height,
                _ => rect.width,
            };
            let at = position.at((0.0, f64::from(length)))?;
            if !at.marks_fit {
                warnings.push(EvalWarning::Fit {
                    path: plot_ref.path.clone(),
                    reason: format!(
                        "the plot is too small along `{}` for the reach of its marks",
                        channel.field_name()
                    ),
                });
            }
            positions[index] = Some(at.scale);
        }
        let clip = (prepared.positions.iter().flatten()).any(|position| position.clip);

        let mut data = SceneData::new();
        let mut plot_marks: Vec<SceneMark> = (prepared.guides.axes.iter())
            .filter(|axis| axis.grid)
            .map(|axis| axis.axis.grid(plot_size).into())
            .collect();
        let mut layers = Vec::with_capacity(resolved.tree.layers.len());
        for (index, layer) in resolved.tree.layers.iter().enumerate() {
            let mark = cartesian_mark(layer);
            let mut rows = LayerRows {
                chart,
                mark,
                style: &styles[self.nodes.marks[index]],
                table: self.drawn[index].clone(),
                partition: &layer.partition,
                scales: layer_scales(&resolved, index, &positions, &prepared.scales),
                masks: BTreeMap::new(),
                engine: &self.engine,
                size: plot_size,
                name: format!("{}/{}", plot_ref.name, layer.position),
            };
            rows.masks = encode::masks(
                mark,
                &rows,
                &self.states,
                &owners(plot_ref, layer),
                &layer.path,
                &mut warnings,
            )?;
            let mut names = Vec::new();
            for drawn in marks::marks(&rows)? {
                let name = drawn.mark.mark_name().to_string();
                data.insert(name.clone(), Arc::new(drawn.rows));
                names.push(name);
                plot_marks.push(drawn.mark);
            }
            let reach = prepared.reach[index].as_deref();
            summary_reach(&rows, &resolved, index, reach, &layer.path, &mut warnings)?;
            let [x, y] = [Channel::X, Channel::Y].map(|channel| {
                (resolved.scales.iter().enumerate())
                    .find(|(_, group)| {
                        (group.members.iter())
                            .any(|member| member.layer == index && member.channel == channel)
                    })
                    .and_then(|(group, _)| positions[group].clone())
            });
            layers.push(EvaluatedLayer {
                path: layer.path.clone(),
                marks: names,
                coords: PlotCoords::cartesian(plot_size, [x, y]),
            });
        }
        let [x, y] = [Channel::X, Channel::Y].map(|channel| {
            (resolved.scales.iter().enumerate())
                .find(|(_, group)| group.channel == channel)
                .and_then(|(group, _)| positions[group].clone())
        });

        let canvas = solution.size();
        let mut marks: Vec<SceneMark> = Vec::new();
        marks.extend(backdrop(
            "background",
            &styles[self.nodes.chart],
            [0.0, 0.0, canvas.width, canvas.height],
        ));
        marks.extend(backdrop(
            &format!("{}/background", plot_ref.name),
            &styles[self.nodes.plot],
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
        marks.extend(layout::draw(&prepared.guides, &solution));
        Ok(EvaluatedChart {
            scene: SceneGraph {
                marks,
                width: canvas.width,
                height: canvas.height,
                origin: [0.0, 0.0],
            },
            data,
            plots: vec![EvaluatedPlot {
                name: plot_ref.name.clone(),
                identity: plot_ref.identity.clone(),
                rect: [rect.x, rect.y, rect.width, rect.height],
                coords: PlotCoords::cartesian(plot_size, [x, y]),
                layers,
            }],
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
        let values: Vec<Option<ArrayRef>> = group.members.iter().map(values_of).collect();
        scales[index] = Some(fit::non_position(
            &fit_inputs,
            group,
            &styles[nodes.scales[index]],
            styles,
            &values,
            &mut warnings,
        )?);
    }

    let no_positions = vec![None; groups.len()];
    let reach = (layers.iter().enumerate())
        .map(|(index, layer)| {
            reach::reach(&LayerRows {
                chart,
                mark: cartesian_mark(layer),
                style: &styles[nodes.marks[index]],
                table: drawn[index].clone(),
                partition: &layer.partition,
                scales: layer_scales(resolved, index, &no_positions, &scales),
                masks: BTreeMap::new(),
                engine,
                size: [0.0, 0.0],
                name: String::new(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut positions: Vec<Option<Position>> = vec![None; groups.len()];
    for (index, group) in groups.iter().enumerate() {
        if !group.is_position() {
            continue;
        }
        // The domain of `x` or `y` fits the reach of the marks of its own rows, toward its lower
        // values and its higher ones, which depend on the direction of the scale along the plot.
        let reversed = match group.channel {
            Channel::Y => fit::direction(group) == Direction::Forward,
            _ => fit::direction(group) == Direction::Reverse,
        };
        let values: Vec<Option<MemberValues>> = (group.members.iter())
            .map(|member| {
                let values = values_of(member)?;
                let along = match &reach[member.layer] {
                    Some(reach) if values.len() == reach.len() && member.summary.is_none() => {
                        Some(reach::along(reach, group.channel, reversed))
                    }
                    _ => None,
                };
                Some((values, along))
            })
            .collect();
        positions[index] = Some(fit::position(
            &fit_inputs,
            group,
            &styles[nodes.scales[index]],
            &values,
        )?);
    }

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
    })?;
    let layout = layout::lower(&guides, &styles[nodes.plot]);
    Ok(Prepared {
        positions,
        scales,
        reach,
        guides,
        layout,
        warnings,
    })
}

/// The values of the encoding of `channel` of `mark`: its column in `table`, or its scaled
/// constant.
fn member_values(
    chart: &Chart,
    mark: &dyn MarkMessage,
    channel: Channel,
    table: &RecordBatch,
) -> Option<ArrayRef> {
    match mark.encoding(channel)?.value.as_ref()? {
        encoding::Value::Column(name) => table.column_by_name(name).cloned(),
        encoding::Value::ScaledConstant(value) => Some(chart.value(value).clone()),
        encoding::Value::Constant(_) => None,
    }
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
    let fill = style.color(Property::Background).unwrap_or([0.0; 4]);
    let width = style.number(Property::BorderWidth).unwrap_or(0.0);
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
            corner_radius: style.number(Property::BorderRadius).unwrap_or(0.0).into(),
            zindex: Some(BACKGROUND_ZINDEX),
            ..Default::default()
        }
        .into(),
    )
}
