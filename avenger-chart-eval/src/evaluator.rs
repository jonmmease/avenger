//! The preparation of a chart, which reads it once, and its evaluation at a size.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{ArrayRef, RecordBatch},
    datatypes::DataType,
};
use avenger_chart::{
    layer::{CoordKind, LayerRef, MarkMessage},
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
use avenger_selection::{Condition, SelectionState};
use avenger_style::{Cascade, ComputedStyle, ComputedStyles, HostInputs, Property, StyleTree};
use avenger_typst_label::LabelEngine;

use crate::{
    convert::{scale::fit_kind, selection},
    encode::{self, at, LayerRows},
    fit::{self, FitInputs, MemberValues, Position},
    geo::{self, MapInputs, Maps, ProjectionNavigation, ProjectionView},
    guides::{self, GuideInputs, Guides},
    layout::{self, Lowered, Lowering, Measurer, SlabId},
    marks,
    polar::{self, PolarInputs, Polars},
    reach::{self, Reach},
    state::{
        self, Carried, Carry, Fit, ProjectionIdentity, ScaleIdentity, ScaleNavigation, ScaleView,
        UpdateOptions,
    },
    style::tree::{self, StyleNodes},
    style::{color as style_color, number as style_number},
    widgets::{self, EvaluatedWidget, PreparedWidget, WidgetInputs},
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
    /// The widgets that draw, with the rectangles of their drawings.
    pub widgets: Vec<EvaluatedWidget>,
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
    /// The kind of its coordinate system, whose positions its coordinates take.
    pub kind: CoordKind,
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
    /// The identity of the layer, when it has an id.
    pub identity: Option<Identity>,
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
    maps: Maps,
    widgets: Vec<PreparedWidget>,
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
    /// The identity of each scale group that has one, by its position in `Resolved::scales`.
    identities: Vec<Option<ScaleIdentity>>,
    /// The identity of each projection group that has one, by its position in
    /// `Resolved::projections`.
    projection_identities: Vec<Option<ProjectionIdentity>>,
    /// The first scale group of each channel of each plot, by plot.
    plot_groups: Vec<BTreeMap<Channel, usize>>,
    /// The view of each position group that has one, by its position in `Resolved::scales`.
    views: BTreeMap<usize, ScaleView>,
    /// The fitted domains that an update carried from the old chart, by scale group.
    carried: BTreeMap<usize, Carried>,
    /// The views and the carried fits of the projections of maps.
    projections: geo::ProjectionState,
    /// The last revision of an axis, which each new view of an axis takes the next of.
    revision: u64,
    /// The rectangle of each plot at the last evaluation.
    rects: Vec<avenger_layout::Rect>,
    /// Whether the scene holds the drawings of the widgets.
    draws_widgets: bool,
    /// The image resolver of the host, which loads the tiles of maps.
    #[cfg(feature = "geo")]
    resolver: Option<Arc<dyn avenger_image::ImageResolver>>,
}

/// A chart prepared to install in a [`ChartEvaluator`]: its resolved models, its style tree, the
/// rows of its layers that draw an instance, and the initial clauses of its selections. They depend
/// only on the chart, the label engine, and the registry, so a chart prepares on any thread, such
/// as for an update off the render loop, and [`ChartEvaluator::install`] only installs it.
pub struct PreparedChart {
    chart: Chart,
    tree: StyleTree,
    nodes: StyleNodes,
    drawn: Vec<RecordBatch>,
    kinds: Vec<FitKind>,
    states: BTreeMap<String, SelectionState>,
    warnings: Vec<EvalWarning>,
    identities: Vec<Option<ScaleIdentity>>,
    projection_identities: Vec<Option<ProjectionIdentity>>,
    plot_groups: Vec<BTreeMap<Channel, usize>>,
}

impl PreparedChart {
    /// Prepares `chart` with `engine` and the custom scales of `registry`, the label engine and the
    /// registry of the evaluator that installs it. It fails for an empty chart, which this version
    /// of the evaluator does not draw, and for a chart with a map plot without the `geo` feature.
    pub fn new(chart: Chart, engine: &LabelEngine, registry: &Registry) -> Result<Self> {
        if matches!(chart.root().node, Some(Node::Empty(_))) {
            return Err(EvalError::Unsupported("an empty chart".into()));
        }
        for plot in &chart.resolve().tree.plots {
            if let (Some(Coord::Geo(_)), false) = (plot.plot.coord.as_ref(), geo::SUPPORTED) {
                return Err(EvalError::Unsupported(
                    "a map plot without the `geo` feature".into(),
                ));
            }
        }
        crate::convert::format::check(&chart, engine)?;
        let states = selection::states(&chart)?;
        let (tree, nodes, drawn, kinds, warnings, identities, projection_identities, plot_groups) = {
            let resolved = chart.resolve();
            let kinds = (resolved.scales.iter())
                .map(|group| fit_kind(&group.spec, registry))
                .collect::<Result<Vec<_>>>()?;
            let (tree, nodes, warnings) = tree::build(&chart, &resolved, &kinds);
            let drawn = (resolved.tree.layers.iter())
                .map(|layer| encode::drawn_rows(layer.mark, layer.table))
                .collect::<Result<Vec<_>>>()?;
            (
                tree,
                nodes,
                drawn,
                kinds,
                warnings,
                state::scale_identities(&resolved),
                state::projection_identities(&resolved),
                state::plot_groups(&resolved),
            )
        };
        Ok(Self {
            chart,
            tree,
            nodes,
            drawn,
            kinds,
            states,
            warnings,
            identities,
            projection_identities,
            plot_groups,
        })
    }

    /// The chart that the preparation prepared.
    pub fn chart(&self) -> &Chart {
        &self.chart
    }
}

impl ChartEvaluator {
    /// Prepares `chart` with the cascade of its style table, a label engine, and the custom scales
    /// of `registry`, as [`PreparedChart::new`] does, with the initial clauses of its selections.
    pub fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        engine: LabelEngine,
        registry: Registry,
    ) -> Result<Self> {
        let prepared = PreparedChart::new(chart, &engine, &registry)?;
        let PreparedChart {
            chart,
            tree,
            nodes,
            drawn,
            kinds,
            states,
            warnings,
            identities,
            projection_identities,
            plot_groups,
        } = prepared;
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
            identities,
            projection_identities,
            plot_groups,
            views: BTreeMap::new(),
            carried: BTreeMap::new(),
            projections: geo::ProjectionState::default(),
            revision: 0,
            rects: Vec::new(),
            draws_widgets: true,
            #[cfg(feature = "geo")]
            resolver: None,
        })
    }

    /// Loads the tiles of the chart's maps through `resolver`, the image resolver of the host.
    #[cfg(feature = "geo")]
    pub fn set_image_resolver(&mut self, resolver: Arc<dyn avenger_image::ImageResolver>) {
        self.resolver = Some(resolver);
        self.prepared = None;
    }

    /// The chart that the evaluator draws.
    pub fn chart(&self) -> &Chart {
        &self.chart
    }

    /// Replaces the cascade of the style table, such as after an edit of a stylesheet. The styles
    /// compute again at the next evaluation unless `cascade` is the cascade that the evaluator
    /// holds.
    pub fn set_cascade(&mut self, cascade: Arc<Cascade>) {
        if !Arc::ptr_eq(&self.cascade, &cascade) {
            self.cascade = cascade;
            self.styles = None;
            self.prepared = None;
            self.measured = None;
        }
    }

    /// Sets whether the scene holds the drawings of the widgets, as it does by default. An app that
    /// draws the widgets in a scene of its own leaves them out and places its own at the
    /// rectangles of [`EvaluatedChart::widgets`].
    pub fn set_draws_widgets(&mut self, draws: bool) {
        self.draws_widgets = draws;
    }

    /// The style tree of the chart, whose nodes stand for its parts.
    pub fn style_tree(&self) -> &StyleTree {
        &self.tree
    }

    /// The warnings of preparing the chart, such as style fields whose text is not a value of
    /// their property.
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
        self.views.retain(|_, view| view.moved());
        self.restyle(host);
        let styles = self.styles.as_ref().expect("the styles are computed");
        let chart = &self.chart;
        let resolved = chart.resolve();
        if self.prepared.is_none() {
            #[allow(unused_mut)]
            let mut prepared = prepare(&Inputs {
                chart,
                resolved: &resolved,
                nodes: &self.nodes,
                styles,
                registry: &self.registry,
                engine: &self.engine,
                states: &self.states,
                drawn: &self.drawn,
                kinds: &self.kinds,
                carried: &self.carried,
                projections: &self.projections,
            })?;
            #[cfg(feature = "geo")]
            if let Some(resolver) = &self.resolver {
                prepared.maps.set_image_resolver(resolver.clone())?;
            }
            self.prepared = Some(prepared);
        }
        // The axes of the scales whose views changed show their new views, and the layout lowers
        // again, so their slabs measure again.
        let prepared = self.prepared.as_mut().expect("the chart is prepared");
        let mut changed = false;
        for axis in &mut prepared.guides.axes {
            let view = self.views.get(&axis.group).map(|view| &view.scale);
            let shown = axis.view.as_ref().map(|domain| domain.as_ref());
            if shown == view.map(|view| view.domain().as_ref()) {
                continue;
            }
            let Some(position) = &prepared.positions[axis.group] else {
                continue;
            };
            self.revision += 1;
            axis.set_view(view, &position.fitting, self.revision, &self.engine)?;
            changed = true;
        }
        if changed {
            prepared.lowered = layout::lower(&Lowering {
                chart,
                resolved: &resolved,
                guides: &prepared.guides,
                maps: &prepared.maps,
                widgets: &prepared.widgets,
                nodes: &self.nodes,
                styles,
            });
        }
        let prepared = self.prepared.as_ref().expect("the chart is prepared");

        let message = chart.message();
        let size = [host.width.or(message.width), host.height.or(message.height)];
        let options = layout::solve_options(size, &styles[self.nodes.chart]);
        let mut measurer = Measurer {
            guides: &prepared.guides,
            maps: &prepared.maps,
            widgets: &prepared.widgets,
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
            // A view keeps its domain at every length, in the direction of the fit.
            if let Some(view) = self.views.get(&index) {
                let range = match position.fitting.direction() {
                    Direction::Forward => (0.0, f64::from(length)),
                    Direction::Reverse => (f64::from(length), 0.0),
                };
                positions[index] = Some(view.scale.clone().with_range_interval(range));
                continue;
            }
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
        let map_fits = prepared
            .maps
            .fit(&sizes, &mut self.projections, &mut warnings)?;
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
            let map_fit = map_fits.get(&plot_index);
            let clip = match (polar_fit, map_fit) {
                (Some(fit), _) => fit.clip,
                (_, Some(fit)) => fit.clip(),
                _ => (resolved.scales.iter().zip(&prepared.positions).enumerate()).any(
                    |(index, (group, position))| {
                        in_plot(group)
                            && (position.as_ref().is_some_and(|p| p.clip)
                                || self.views.contains_key(&index))
                    },
                ),
            };
            let mut plot_marks: Vec<SceneMark> = match (polar_fit, map_fit) {
                (Some(fit), _) => prepared.polars.backdrop(fit),
                (_, Some(fit)) => prepared.maps.backdrop(plot_index, fit, &mut warnings)?,
                _ => (prepared.guides.plots[plot_index].axes.iter())
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
                // A mark at places on a map draws the rows whose places the projection keeps.
                let mut points = None;
                let located = map_fit.map(|fit| fit.locate(&rows)).transpose()?;
                if let Some((kept, located)) = located.flatten() {
                    if kept.len() != rows.len() {
                        rows.table = rows.rows(&kept)?;
                    }
                    points = Some(located);
                }
                rows.masks = encode::masks(
                    &rows,
                    &self.states,
                    &owners(plot_ref, layer),
                    &layer.path,
                    &mut warnings,
                )?;
                if let Some(reason) = marks::image_warning(&rows)? {
                    warnings.push(EvalWarning::NotDrawn {
                        path: layer.path.clone(),
                        reason,
                    });
                }
                if let Some(reason) = rows.size_warning()? {
                    warnings.push(EvalWarning::NotDrawn {
                        path: layer.path.clone(),
                        reason,
                    });
                }
                let mut names = Vec::new();
                let drawn = match (layer.layer, polar_fit) {
                    (LayerRef::Cartesian(cartesian), _) => {
                        let mark = cartesian.mark.as_ref().expect("a valid layer has a mark");
                        marks::marks(&rows, mark)?
                    }
                    (LayerRef::Polar(layer), Some(fit)) => {
                        let mark = layer.mark.as_ref().expect("a valid layer has a mark");
                        polar::marks(&rows, mark, fit)?
                    }
                    (LayerRef::Geo(layer), _) => {
                        let mark = layer.mark.as_ref().expect("a valid layer has a mark");
                        let fit = map_fit.expect("a map plot fits its projection");
                        geo::marks(&rows, mark, fit, points)?
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
                let coords = match (polar_fit, map_fit) {
                    (Some(fit), _) => fit.coords(),
                    (_, Some(fit)) => fit.coords(),
                    _ => {
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
                    identity: layer.identity.clone(),
                    marks: names,
                    coords,
                });
            }
            let coords = match (polar_fit, map_fit) {
                (Some(fit), _) => {
                    plot_marks.extend(prepared.polars.overlay(fit));
                    fit.coords()
                }
                (_, Some(fit)) => fit.coords(),
                _ => {
                    let [x, y] = [Channel::X, Channel::Y].map(|channel| {
                        (resolved.scales.iter().enumerate())
                            .find(|(_, group)| group.channel == channel && in_plot(group))
                            .and_then(|(group, _)| positions[group].clone())
                    });
                    PlotCoords::cartesian(plot_size, [x, y])
                }
            };
            // The background and the border of a map plot draw its sphere.
            if map_fit.is_none() {
                marks.extend(backdrop(
                    &format!("{}/background", plot_ref.name),
                    &styles[self.nodes.plot(&plot_ref.path).plot],
                    [rect.x, rect.y, rect.width, rect.height],
                ));
            }
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
                kind: plot_ref.kind,
                rect: [rect.x, rect.y, rect.width, rect.height],
                coords,
                layers,
            });
        }
        marks.extend(layout::draw(&prepared.guides, &prepared.lowered, &solution));
        // The labels of each graticule, in their slabs.
        for id in &prepared.lowered.slabs {
            let SlabId::Graticule { plot, side } = *id else {
                continue;
            };
            let (Some(region), Some(fit), Some(guide)) = (
                solution.region(id),
                map_fits.get(&plot),
                prepared.maps.guide(plot),
            ) else {
                continue;
            };
            let rect = region.content;
            let slot = [rect.x, rect.y, rect.width, rect.height];
            let labels = prepared.maps.labels(plot, fit, guide.labels[side].0, slot);
            marks.extend(labels.map(SceneMark::from));
        }
        // Each drawing of each widget, in its slab, at the value of its clause.
        let mut widgets: Vec<EvaluatedWidget> = (prepared.widgets.iter())
            .map(|widget| widget.evaluated.clone())
            .collect();
        for (id, slot) in layout::widget_slots(&prepared.widgets, &prepared.lowered, &solution) {
            let (SlabId::Widget { widget, .. } | SlabId::HeldWidget(widget)) = id else {
                continue;
            };
            let prepared_widget = &prepared.widgets[widget];
            let evaluated = &prepared_widget.evaluated;
            let value = evaluated.value(self.states.get(&evaluated.selection));
            let (drawn, rect) = prepared_widget.draw(&self.engine, &value, slot)?;
            if self.draws_widgets {
                marks.push(
                    SceneGroup {
                        name: format!("widget {}", evaluated.key),
                        marks: drawn,
                        ..Default::default()
                    }
                    .into(),
                );
            }
            widgets[widget].rects.push(rect);
        }
        self.rects = rects;
        Ok(EvaluatedChart {
            scene: SceneGraph {
                marks,
                width: canvas.width,
                height: canvas.height,
                origin: [0.0, 0.0],
            },
            data,
            plots,
            widgets,
            warnings,
        })
    }
}

/// The state of an evaluation: the clauses of the selections, the views of scales and
/// projections, and the installation of a new chart, which carries them.
impl ChartEvaluator {
    /// Installs `prepared` in place of the chart, and carries the state of the old chart by
    /// `options`: the fitted domains of its scales and projections, the views of pans and zooms,
    /// and the clauses of its selections, as they stand at the installation. A scale carries over
    /// when the new chart has a scale with its identity and its kind of fit, and a projection when
    /// the new chart has a projection with its identity. The new chart keeps the computed styles
    /// of the old one when their style trees are equal.
    pub fn install(&mut self, prepared: PreparedChart, options: &UpdateOptions) -> Result<()> {
        let PreparedChart {
            chart,
            tree,
            nodes,
            drawn,
            kinds,
            states,
            warnings,
            identities,
            projection_identities,
            plot_groups,
        } = prepared;
        let new_resolved = chart.resolve();
        let mut views = BTreeMap::new();
        let mut carried = BTreeMap::new();
        for (index, identity) in identities.iter().enumerate() {
            let Some(identity) = identity else {
                continue;
            };
            let Some(old_index) =
                (self.identities.iter()).position(|own| own.as_ref() == Some(identity))
            else {
                continue;
            };
            if self.kinds[old_index] != kinds[index] {
                continue;
            }
            let group = &new_resolved.scales[index];
            let viewable = group.is_navigated()
                && kinds[index] == FitKind::Interval
                && !state::selection_domain(&group.spec);
            if let (true, true, Some(view)) =
                (options.keeps_views(), viewable, self.views.get(&old_index))
            {
                if view.moved() {
                    views.insert(index, view.clone());
                }
            }
            let fit = options.scale_fit(identity);
            if fit == Fit::Refit
                || state::selection_domain(&group.spec)
                || state::explicit_domain(&group.spec)
            {
                continue;
            }
            // The fit carries the scale of the old chart as it was last prepared, or the scale
            // that the old chart carried in turn.
            let old =
                self.prepared
                    .as_ref()
                    .and_then(|prepared| match group.is_position() {
                        true => (prepared.positions[old_index].clone())
                            .map(|position| Carried::Position(fit, position)),
                        false => (prepared.scales[old_index].clone())
                            .map(|scale| Carried::Scale(fit, scale)),
                    });
            if let Some(old) = old.or_else(|| self.carried.get(&old_index).cloned()) {
                carried.insert(index, old);
            }
        }
        let projections = self.carry_projections(&projection_identities, options);
        let states = carry_states(&self.states, states, &new_resolved, options);
        drop(new_resolved);
        // An equal style tree keeps the computed styles, which the same cascade computed.
        if tree != self.tree {
            self.styles = None;
        }
        self.chart = chart;
        self.tree = tree;
        self.nodes = nodes;
        self.drawn = drawn;
        self.kinds = kinds;
        self.states = states;
        self.warnings = warnings;
        self.identities = identities;
        self.projection_identities = projection_identities;
        self.plot_groups = plot_groups;
        self.views = views;
        self.carried = carried;
        self.projections = projections;
        self.prepared = None;
        self.measured = None;
        self.rects.clear();
        Ok(())
    }

    /// The views and the carried fits of the projections of a new chart with
    /// `projection_identities`, by `options`.
    #[cfg(feature = "geo")]
    fn carry_projections(
        &self,
        projection_identities: &[Option<ProjectionIdentity>],
        options: &UpdateOptions,
    ) -> geo::ProjectionState {
        let mut state = geo::ProjectionState::default();
        for (index, identity) in projection_identities.iter().enumerate() {
            let Some(identity) = identity else {
                continue;
            };
            let Some(old_index) =
                (self.projection_identities.iter()).position(|own| own.as_ref() == Some(identity))
            else {
                continue;
            };
            if let (true, Some(view)) = (
                options.keeps_views(),
                self.projections.views.get(&old_index),
            ) {
                state.views.insert(index, view.carried());
            }
            let fit = options.projection_fit(identity);
            let summary = (self.prepared.as_ref())
                .and_then(|prepared| prepared.maps.data_summary(old_index))
                .or_else(|| {
                    (self.projections.carried.get(&old_index)).map(|(_, summary)| *summary)
                });
            if let (Fit::Keep | Fit::Extend, Some(summary)) = (fit, summary) {
                state.carried.insert(index, (fit, summary));
            }
        }
        state
    }

    #[cfg(not(feature = "geo"))]
    fn carry_projections(
        &self,
        _projection_identities: &[Option<ProjectionIdentity>],
        _options: &UpdateOptions,
    ) -> geo::ProjectionState {
        geo::ProjectionState::default()
    }

    /// The states of the evaluator's selections, by name.
    pub fn selections(&self) -> &BTreeMap<String, SelectionState> {
        &self.states
    }

    /// Replaces the states of the evaluator's selections. The conditional values of encodings take
    /// them at the next evaluation, and the scales whose domains read a selection fit again.
    pub fn set_selections(&mut self, states: BTreeMap<String, SelectionState>) {
        self.states = states;
        let resolved = self.chart.resolve();
        if (resolved.scales.iter()).any(|group| state::selection_domain(&group.spec)) {
            self.prepared = None;
        }
    }

    /// The first scale group of `channel` of the plot at index `plot`.
    fn plot_group(&self, plot: usize, channel: Channel) -> Option<usize> {
        self.plot_groups.get(plot)?.get(&channel).copied()
    }

    /// The width and the height of the plot at index `plot` at the last evaluation.
    fn plot_size(&self, plot: usize) -> Option<[f32; 2]> {
        let rect = self.rects.get(plot)?;
        Some([rect.width, rect.height])
    }

    /// The fitted scale of `channel` of the plot at index `plot` at its size at the last
    /// evaluation, without its view, which a navigation returns to.
    pub fn home_scale(&self, plot: usize, channel: Channel) -> Option<ConfiguredScale> {
        let group = self.plot_group(plot, channel)?;
        let position = self.prepared.as_ref()?.positions[group].as_ref()?;
        let [width, height] = self.plot_size(plot)?;
        let length = match channel {
            Channel::Y => height,
            _ => width,
        };
        Some(position.fitting.at((0.0, f64::from(length))).ok()?.scale)
    }

    /// The view of the scale of `channel`, `x` or `y`, of the plot at index `plot` of the
    /// evaluation, to move, as a pan or a zoom does. A scale without a view takes its fitted scale
    /// at the plot's size at the last evaluation as its view, and the next evaluation drops a view
    /// that still has the domain that it started from, so reading a view, as a press does, leaves
    /// the scale fitting. A plot's first scale of the channel has a view when it is continuous and
    /// its domain does not read a selection, and the plot has been evaluated, and otherwise this
    /// is `None`.
    pub fn scale_view_mut(
        &mut self,
        plot: usize,
        channel: Channel,
    ) -> Option<&mut ConfiguredScale> {
        let group = self.plot_group(plot, channel)?;
        let resolved = self.chart.resolve();
        let spec_group = &resolved.scales[group];
        let viewable = spec_group.is_navigated()
            && self.kinds[group] == FitKind::Interval
            && !state::selection_domain(&spec_group.spec);
        drop(resolved);
        if !viewable {
            return None;
        }
        if !self.views.contains_key(&group) {
            let scale = self.home_scale(plot, channel)?;
            let start = scale.domain().clone();
            self.views.insert(group, ScaleView { scale, start });
        }
        Some(&mut self.views.get_mut(&group)?.scale)
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
        Some(&self.views.get(&self.plot_group(plot, channel)?)?.scale)
    }

    /// The identity of the scale of `channel` of the plot at index `plot`, when it has one.
    pub fn scale_identity(&self, plot: usize, channel: Channel) -> Option<&ScaleIdentity> {
        self.identities
            .get(self.plot_group(plot, channel)?)?
            .as_ref()
    }

    /// The selection whose clauses give the domain of the scale of `channel` of the plot at index
    /// `plot`, when its domain reads a selection. A navigation of such a scale moves the
    /// selection, because the scale has no view.
    pub fn domain_selection(&self, plot: usize, channel: Channel) -> Option<String> {
        let group = self.plot_group(plot, channel)?;
        let resolved = self.chart.resolve();
        match resolved.scales[group]
            .spec
            .domain
            .as_ref()?
            .domain
            .as_ref()?
        {
            avenger_chart::proto::domain::Domain::Selection(selection) => {
                Some(selection.selection.clone())
            }
            _ => None,
        }
    }

    /// The column that a selection through `channel` of the plot at index `plot` records: the
    /// selection field of the plot's first encoding of the channel, or its encoded column.
    pub fn selection_column(&self, plot: usize, channel: Channel) -> Option<String> {
        let resolved = self.chart.resolve();
        let plot_ref = resolved.tree.plots.get(plot)?;
        resolved.tree.layers[plot_ref.layers.clone()]
            .iter()
            .find_map(|layer| {
                let encoding = layer.mark.encoding(channel)?;
                if !encoding.selection_field.is_empty() {
                    return Some(encoding.selection_field.clone());
                }
                match encoding.value.as_ref()? {
                    encoding::Value::Column(column) | encoding::Value::ScaledColumn(column) => {
                        Some(column.clone())
                    }
                    _ => None,
                }
            })
    }

    /// Whether every row that the layers of the plot at index `plot` draw matches `condition`,
    /// with the partition values of the layers. A plot shows a clause of a selection when it draws
    /// some of its conditions and every other condition holds there. A layer without a column of
    /// the condition matches none of it.
    pub fn satisfies(&self, plot: usize, condition: &Condition) -> Result<bool> {
        let resolved = self.chart.resolve();
        let Some(plot_ref) = resolved.tree.plots.get(plot) else {
            return Ok(false);
        };
        for index in plot_ref.layers.clone() {
            let layer = &resolved.tree.layers[index];
            let table = &self.drawn[index];
            let columns: Option<Vec<ArrayRef>> = (condition.columns().into_iter())
                .map(|name| match table.column_by_name(name) {
                    Some(column) => Some(column.clone()),
                    None => {
                        let value = layer.partition.get(name)?;
                        let rows = arrow::array::UInt32Array::from(vec![0; table.num_rows()]);
                        arrow::compute::take(value, &rows, None).ok()
                    }
                })
                .collect();
            let Some(columns) = columns else {
                return Ok(false);
            };
            if condition.matches(&columns)?.false_count() > 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The navigation of the scale of `channel` of the plot at index `plot`, from the scale's
    /// navigation limits, at the plot's size at the last evaluation: the bounds that its domain
    /// stays inside, with the fitted domain in place of data bounds, and its smallest width, in the
    /// numeric values of the scale. A scale whose domain reads a selection has no data bounds,
    /// because its fitted domain is the selection. This is `None` for a plot without a scale of the
    /// channel.
    pub fn scale_navigation(&self, plot: usize, channel: Channel) -> Option<ScaleNavigation> {
        use avenger_chart::proto::navigation_limits::Bounds;
        let group = self.plot_group(plot, channel)?;
        let resolved = self.chart.resolve();
        let spec = &resolved.scales[group].spec;
        let Some(limits) = &spec.navigation_limits else {
            return Some(ScaleNavigation::default());
        };
        let bounds = match &limits.bounds {
            Some(Bounds::Data(_)) if !state::selection_domain(spec) => self
                .home_scale(plot, channel)
                .and_then(|home| home.numeric_interval_domain().ok())
                .map(|(start, end)| [start.min(end), start.max(end)]),
            Some(Bounds::Interval(interval)) => Some([interval.low, interval.high]),
            Some(Bounds::Data(_)) | None => None,
        };
        Some(ScaleNavigation {
            bounds,
            min_span: limits.min_span,
        })
    }

    /// The view of the projection of the plot at index `plot` of the evaluation, to move, as a pan
    /// or a zoom does. A projection without a view takes its fitted projection at the plot's size
    /// at the last evaluation as its view. The plots of the projection show the view, scaled and
    /// moved with their fitted view as they resize, and clip their marks. This is `None` for a
    /// plot without a projection, such as a Cartesian plot, or that has not been evaluated.
    pub fn projection_view_mut(&mut self, plot: usize) -> Option<&mut ProjectionView> {
        let size = self.plot_size(plot)?;
        let prepared = self.prepared.as_ref()?;
        prepared.maps.view_mut(&mut self.projections, plot, size)
    }

    /// Removes the view of the projection of the plot at index `plot`, so that it fits again.
    pub fn clear_projection_view(&mut self, plot: usize) {
        if let Some(prepared) = &self.prepared {
            prepared.maps.clear_view(&mut self.projections, plot);
        }
    }

    /// The view of the projection of the plot at index `plot`, when it has one.
    pub fn projection_view(&self, plot: usize) -> Option<&ProjectionView> {
        self.prepared.as_ref()?.maps.view(&self.projections, plot)
    }

    /// The fitted projection of the plot at index `plot` at its size at the last evaluation,
    /// without its view, which a navigation returns to.
    pub fn home_projection(&self, plot: usize) -> Option<ProjectionView> {
        self.prepared
            .as_ref()?
            .maps
            .home(plot, self.plot_size(plot)?)
    }

    /// The identity of the projection of the plot at index `plot`, when it has one.
    pub fn projection_identity(&self, plot: usize) -> Option<&ProjectionIdentity> {
        let group = self.prepared.as_ref()?.maps.group_of(plot)?;
        self.projection_identities.get(group)?.as_ref()
    }

    /// The navigation of the projection of the plot at index `plot`, from the specs of its
    /// coordinate system, at the plot's size at the last evaluation. This is `None` for a plot
    /// without a projection.
    pub fn projection_navigation(&self, plot: usize) -> Option<ProjectionNavigation> {
        self.prepared
            .as_ref()?
            .maps
            .navigation(plot, self.plot_size(plot)?)
    }
}

/// The states of the selections of a new chart, whose models are `resolved`, by `options`, from
/// the states `old` of the old chart and the initial states `initial` of the new chart. A clause of
/// a selection that is not global carries when its owner is a plot or a layer of the new chart,
/// which can draw and clear it.
fn carry_states(
    old: &BTreeMap<String, SelectionState>,
    initial: BTreeMap<String, SelectionState>,
    resolved: &Resolved,
    options: &UpdateOptions,
) -> BTreeMap<String, SelectionState> {
    let owners: std::collections::BTreeSet<String> = (resolved.tree.identities())
        .map(|(identity, _)| identity.id.clone())
        .collect();
    let names: std::collections::BTreeSet<String> =
        old.keys().chain(initial.keys()).cloned().collect();
    let mut states = BTreeMap::new();
    for name in names {
        let carried = match (options.carry_for(&name), old.get(&name)) {
            (Carry::Keep, Some(kept)) => {
                let global = kept.resolution == avenger_selection::Resolution::Global;
                let mut kept = kept.clone();
                if !global {
                    kept.clauses.retain(|clause| owners.contains(&clause.owner));
                }
                Some(kept)
            }
            (Carry::Keep, None) | (Carry::Reset, _) => initial.get(&name).cloned(),
        };
        if let Some(carried) = carried {
            states.insert(name, carried);
        }
    }
    states
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
    /// The fitted domains that an update carried, by scale group.
    carried: &'a BTreeMap<usize, Carried>,
    projections: &'a geo::ProjectionState,
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
        carried,
        projections,
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
        let scale = fit::non_position(
            &fit_inputs,
            group,
            &styles[nodes.scales[index]],
            styles,
            &values,
            &mut warnings,
        )?;
        scales[index] = Some(match carried.get(&index) {
            Some(Carried::Scale(fit, old)) => state::carry_scale(kinds[index], *fit, old, &scale)?,
            _ => scale,
        });
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
        let values: Vec<Option<MemberValues>> = (group.members.iter())
            .map(|member| {
                let values = values_of(member)?;
                if outer_level(layers[member.layer].mark, member.channel, &values) {
                    return None;
                }
                let along = match &reach[member.layer] {
                    Some(reach)
                        if along_plot
                            && values.len() == reach.len()
                            && member.summary.is_none() =>
                    {
                        Some(reach::along(reach, group.channel, reversed))
                    }
                    _ => None,
                };
                Some((values, along))
            })
            .collect();
        let position = fit::position(
            &fit_inputs,
            group,
            &styles[nodes.scales[index]],
            &values,
            polar::wraps(resolved, group),
        )?;
        positions[index] = Some(match carried.get(&index) {
            Some(Carried::Position(fit, old)) => {
                state::carry_position(chart, group, kinds[index], *fit, old, &position)?
            }
            _ => position,
        });
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
    let maps = Maps::prepare(
        &MapInputs {
            resolved,
            nodes,
            styles,
            engine,
        },
        unsized_rows,
        &reach,
        projections,
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
    })?;
    let widgets = widgets::widgets(
        &WidgetInputs {
            chart,
            resolved,
            drawn,
            styles: (nodes.widgets.iter()).map(|node| &styles[*node]).collect(),
            engine,
        },
        &mut warnings,
    );
    let lowered = layout::lower(&Lowering {
        chart,
        resolved,
        guides: &guides,
        maps: &maps,
        widgets: &widgets,
        nodes,
        styles,
    });
    Ok(Prepared {
        positions,
        scales,
        reach,
        guides,
        polars,
        maps,
        widgets,
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

/// The values of the encoding of `channel` of `mark` that go through its scale: its scaled column in
/// `table`, or its scaled constant.
fn member_values(
    chart: &Chart,
    mark: &dyn MarkMessage,
    channel: Channel,
    table: &RecordBatch,
) -> Option<ArrayRef> {
    match mark.encoding(channel)?.value.as_ref()? {
        encoding::Value::ScaledColumn(name) => table.column_by_name(name).cloned(),
        encoding::Value::ScaledConstant(value) => Some(chart.value(value).clone()),
        encoding::Value::Column(_) | encoding::Value::Constant(_) => None,
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

#[cfg(test)]
mod tests;
