//! The style tree of a chart, built once for each chart from its resolved models, as
//! the style proposal's table of nodes gives it, with the inline styles of the format's style
//! fields.

use std::collections::HashMap;

use arrow::datatypes::DataType;
use avenger_chart::{
    layer::{CoordKind, LayerRef, MarkMessage},
    proto::{self, encoding, layer_tool, node::Node, plot::Coord, select, tool},
    resolve::{PlotRef, Resolved, ScaleGroup},
    Chart, FitKind,
};
use avenger_style::{Element, NodeId, StyleTree};

use crate::{
    polar::Sector,
    variants::{Variant, Variants},
    EvalWarning,
};

use super::{
    guide_kind, inline,
    inline::{AxisNodes, ColorbarNodes, FacetNodes, GraticuleNodes, LegendNodes, Styled},
    scale_type, GuideKind,
};

/// The nodes of a guide and its parts.
#[derive(Clone, Debug)]
pub(crate) enum GuideNodes {
    Axis(AxisNodes),
    PolarAxis(AxisNodes),
    Legend(LegendNodes),
    Colorbar(ColorbarNodes),
}

/// The nodes that a plot, a concat, or a wrap has: the `plot` inside its `chart`, and its title and
/// subtitle.
#[derive(Clone, Debug)]
pub(crate) struct NodeNodes {
    pub(crate) plot: NodeId,
    pub(crate) title: Option<NodeId>,
    pub(crate) subtitle: Option<NodeId>,
    /// The graticule of a geo plot that has one.
    #[cfg_attr(not(feature = "geo"), allow(dead_code))]
    pub(crate) graticule: Option<GraticuleNodes>,
}

/// The nodes of the style tree that stand for the parts of a chart.
#[derive(Clone, Debug)]
pub(crate) struct StyleNodes {
    /// The root `chart`.
    pub(crate) chart: NodeId,
    /// The nodes of each plot, concat, and wrap, by its path, such as `root.concat` or
    /// `root.concat.children[1].plot`.
    pub(crate) nodes: HashMap<String, NodeNodes>,
    /// The `attribution` of each plot that has one, in the order of `Tree::plots`.
    pub(crate) attributions: Vec<Option<NodeId>>,
    /// The `mark` of each layer, in the order of `Tree::layers`.
    pub(crate) marks: Vec<NodeId>,
    /// The variants of the `mark` of each layer, in the order of `Tree::layers`: an `other` where
    /// its encodings read a column of lumps, which styles the rows of the Other group, and a
    /// `null` where an encoding of a categorical scale reads a NULL category, which styles its
    /// rows.
    pub(crate) variants: Vec<Variants>,
    /// The `scale` of each scale group, in the order of `Resolved::scales`.
    pub(crate) scales: Vec<NodeId>,
    /// The nodes of each scale group's guide, or `None` for a group without a guide.
    pub(crate) guides: Vec<Option<GuideNodes>>,
    /// The `facet` of each concat or wrap with a facet that shows, by the path of the concat or
    /// the wrap.
    pub(crate) facets: HashMap<String, FacetNodes>,
    /// The `widget` of each widget group, under the `plot` of the node that holds the group, in
    /// the order of `Resolved::widgets`.
    pub(crate) widgets: Vec<NodeId>,
}

impl StyleNodes {
    /// The nodes of the plot at `path`.
    pub(crate) fn plot(&self, path: &str) -> &NodeNodes {
        &self.nodes[path]
    }
}

/// The path of the innermost plot, concat, or wrap of `nodes` at or above `path`.
pub(crate) fn owner_path<'a, T>(nodes: &HashMap<String, T>, path: &'a str) -> &'a str {
    let mut end = path.len();
    loop {
        let candidate = &path[..end];
        if nodes.contains_key(candidate) {
            return candidate;
        }
        end = candidate
            .rfind('.')
            .expect("a path is below the root's node");
    }
}

/// The style tree of `chart`, whose plots are Cartesian or polar, with its nodes and a warning for each
/// style field whose text is not a value of its property. `kinds` holds the kind of fit of each
/// scale group, by its position in `Resolved::scales`.
pub(crate) fn build(
    chart: &Chart,
    resolved: &Resolved,
    kinds: &[FitKind],
) -> (StyleTree, StyleNodes, Vec<EvalWarning>) {
    let mut tree = Styled {
        tree: StyleTree::new(Element::Chart),
        warnings: Vec::new(),
    };
    let chart_node = tree.root();
    inline::chart(&mut tree, chart_node, chart.message());

    let mut nodes = HashMap::new();
    let mut facets = HashMap::new();
    composition(
        &mut tree,
        chart_node,
        chart.root(),
        "root",
        resolved,
        kinds,
        &mut nodes,
        &mut facets,
    );

    // The scale groups that fit categories with a NULL category, and the layers of their members.
    let nulls: Vec<bool> = (resolved.scales.iter().zip(kinds))
        .map(|(group, kind)| *kind == FitKind::Categories && holds_null(resolved, group))
        .collect();
    let null_layers: Vec<usize> = (resolved.scales.iter().zip(&nulls))
        .filter(|(_, null)| **null)
        .flat_map(|(group, _)| group.members.iter().map(|member| member.layer))
        .collect();
    let mut marks = Vec::with_capacity(resolved.tree.layers.len());
    let mut variants = Vec::with_capacity(resolved.tree.layers.len());
    for (index, layer) in resolved.tree.layers.iter().enumerate() {
        let plot_node = nodes[&resolved.tree.plots[layer.plot].path].plot;
        let node = tree.add(plot_node, Element::Mark);
        tree.set_attribute_forms(node, "type", layer.mark.name(), function_name(layer.mark));
        identify(&mut tree, node, layer.layer.id(), layer.layer.classes());
        match layer.layer {
            LayerRef::Cartesian(cartesian) => {
                let mark = cartesian.mark.as_ref().expect("a valid layer has a mark");
                inline::mark(&mut tree, node, mark);
            }
            LayerRef::Polar(polar) => {
                let mark = polar.mark.as_ref().expect("a valid layer has a mark");
                inline::polar_mark(&mut tree, node, mark);
            }
            LayerRef::Geo(geo) => {
                let mark = geo.mark.as_ref().expect("a valid layer has a mark");
                inline::geo_mark(&mut tree, node, mark);
            }
        }
        marks.push(node);
        let reads_lumps = layer
            .mark
            .encodings()
            .any(|(_, encoding)| match &encoding.kind {
                // A lump is never a direct field, so only scaled fields read lumps.
                Some(encoding::Kind::ScaledField(name)) => (layer.table.column_by_name(name))
                    .is_some_and(|column| avenger_chart::lump::is_lump(column.data_type())),
                _ => false,
            });
        variants.push(Variants {
            other: reads_lumps.then(|| tree.add(node, Element::Other)),
            null: null_layers
                .contains(&index)
                .then(|| tree.add(node, Element::Null)),
        });
    }

    let mut scales = Vec::with_capacity(resolved.scales.len());
    for (group, kind) in resolved.scales.iter().zip(kinds) {
        let owner = nodes[owner_path(&nodes, &group.owner)].plot;
        let node = tree.add(owner, Element::Scale);
        let type_name = scale_type(&group.spec);
        tree.set_attribute(node, "type", type_name);
        tree.set_attribute(node, "channel", group.channel.field_name());
        tree.set_attribute(node, "kind", scale_kind(chart, resolved, group, *kind));
        let [kinds, functions] = mark_names(resolved, group);
        tree.set_attribute_forms(node, "marks", kinds, functions);
        inline::scale(&mut tree, node, &group.spec, chart);
        scales.push(node);
    }

    let mut guides = Vec::with_capacity(resolved.scales.len());
    for ((group, kind), null) in resolved.scales.iter().zip(kinds).zip(&nulls) {
        let owner = nodes[owner_path(&nodes, &group.owner)].plot;
        guides.push(guide_nodes(
            &mut tree, owner, chart, resolved, group, *kind, *null,
        ));
    }

    let mut attributions = Vec::with_capacity(resolved.tree.plots.len());
    for plot_ref in &resolved.tree.plots {
        let plot_node = nodes[&plot_ref.path].plot;
        attributions.push(plot_ref.plot.attribution.as_ref().map(|attribution| {
            let node = tree.add(plot_node, Element::Attribution);
            inline::attribution(&mut tree, node, attribution);
            node
        }));
        store_marks(&mut tree, plot_node, resolved, plot_ref);
    }

    let mut widgets = Vec::with_capacity(resolved.widgets.len());
    for group in &resolved.widgets {
        let owner = nodes[owner_path(&nodes, &group.owner)].plot;
        let node = tree.add(owner, Element::Widget);
        if let Some(kind) = &group.widget.kind {
            tree.set_attribute(node, "type", crate::widgets::kind_name(kind));
        }
        tree.set_attribute(node, "key", group.key.clone());
        identify(&mut tree, node, "", &group.widget.classes);
        widgets.push(node);
    }

    let nodes = StyleNodes {
        chart: chart_node,
        nodes,
        attributions,
        marks,
        variants,
        scales,
        guides,
        facets,
        widgets,
    };
    (tree.tree, nodes, tree.warnings)
}

/// The nodes of `node` at `path`, whose `chart` is `chart_node`, and of the nodes below it: a
/// `plot` with the node's type, its title, and the `chart` of each child under the `plot`.
#[allow(clippy::too_many_arguments)]
fn composition(
    tree: &mut Styled,
    chart_node: NodeId,
    node: &proto::Node,
    path: &str,
    resolved: &Resolved,
    kinds: &[FitKind],
    nodes: &mut HashMap<String, NodeNodes>,
    facets: &mut HashMap<String, FacetNodes>,
) {
    let kind = node.node.as_ref().expect("a valid node has a kind");
    let (path, title, children, facet) = match kind {
        Node::Plot(plot) => (format!("{path}.plot"), plot.title.as_ref(), &[][..], None),
        Node::Concat(concat) => (
            format!("{path}.concat"),
            concat.title.as_ref(),
            &concat.children[..],
            concat.facet.as_ref(),
        ),
        Node::Wrap(wrap) => (
            format!("{path}.wrap"),
            wrap.title.as_ref(),
            &wrap.children[..],
            wrap.facet.as_ref(),
        ),
        Node::Empty(_) => return,
    };
    let (title_node, subtitle_node) = match title {
        Some(title) => {
            let title_node = tree.add(chart_node, Element::Title);
            let subtitle_node =
                (!title.subtitle.is_empty()).then(|| tree.add(chart_node, Element::Subtitle));
            inline::title(tree, title_node, subtitle_node, title);
            (Some(title_node), subtitle_node)
        }
        None => (None, None),
    };
    let plot_node = tree.add(chart_node, Element::Plot);
    let mut graticule = None;
    match kind {
        Node::Plot(plot) => {
            let coord = CoordKind::of(plot).expect("a valid plot has a coordinate system");
            tree.set_attribute(plot_node, "type", coord.field());
            identify(tree, plot_node, &plot.id, &plot.classes);
            let plot_index = (resolved.tree.plots.iter())
                .position(|plot_ref| plot_ref.path == path)
                .expect("the tree has every plot");
            let discrete: Vec<&str> = (resolved.scales.iter().zip(kinds))
                .filter(|(group, kind)| {
                    group.is_position()
                        && **kind == FitKind::Categories
                        && (group.members.iter())
                            .any(|member| resolved.tree.layers[member.layer].plot == plot_index)
                })
                .map(|(group, _)| group.channel.field_name())
                .collect();
            if !discrete.is_empty() {
                tree.set_attribute(plot_node, "discrete", discrete.join(" "));
            }
            inline::plot(tree, plot_node, plot);
            if let Some(Coord::Geo(geo)) = &plot.coord {
                graticule = geo_plot(tree, plot_node, geo);
            }
        }
        Node::Concat(concat) => {
            let row = !matches!(
                proto::Direction::try_from(concat.direction),
                Ok(proto::Direction::Column)
            );
            let name = match (row, concat.facet.is_some()) {
                (true, false) => "hconcat",
                (false, false) => "vconcat",
                (true, true) => "facet_col",
                (false, true) => "facet_row",
            };
            tree.set_attribute(plot_node, "type", name);
            identify(tree, plot_node, &concat.id, &concat.classes);
            inline::concat(tree, plot_node, concat);
        }
        Node::Wrap(wrap) => {
            let name = match wrap.facet.is_some() {
                true => "facet_wrap",
                false => "wrap",
            };
            tree.set_attribute(plot_node, "type", name);
            identify(tree, plot_node, &wrap.id, &wrap.classes);
            inline::wrap(tree, plot_node, wrap);
        }
        Node::Empty(_) => unreachable!("an empty node has no nodes"),
    }
    if let Some(facet) = facet.filter(|facet| facet.show != Some(false)) {
        let facet_node = tree.add(plot_node, Element::Facet);
        let facet_type = match kind {
            Node::Wrap(_) => "by",
            Node::Concat(concat)
                if matches!(
                    proto::Direction::try_from(concat.direction),
                    Ok(proto::Direction::Column)
                ) =>
            {
                "row"
            }
            _ => "col",
        };
        tree.set_attribute(facet_node, "type", facet_type);
        identify(tree, facet_node, &facet.id, &facet.classes);
        let facet_nodes = FacetNodes {
            facet: facet_node,
            title: tree.add(facet_node, Element::Title),
            label: tree.add(facet_node, Element::Label),
            tick: tree.add(facet_node, Element::Tick),
            rule: tree.add(facet_node, Element::Rule),
        };
        inline::facet(tree, &facet_nodes, facet);
        facets.insert(path.clone(), facet_nodes);
    }
    nodes.insert(
        path.clone(),
        NodeNodes {
            plot: plot_node,
            title: title_node,
            subtitle: subtitle_node,
            graticule,
        },
    );
    for (index, child) in children.iter().enumerate() {
        let child_chart = match child.node {
            Some(Node::Empty(_)) => continue,
            _ => tree.add(plot_node, Element::Chart),
        };
        composition(
            tree,
            child_chart,
            child,
            &format!("{path}.children[{index}]"),
            resolved,
            kinds,
            nodes,
            facets,
        );
    }
}

/// The `projection` of the `plot[type=geo]` of a geo plot, its sphere's inline styles, and the nodes of
/// its graticule, when it has one.
fn geo_plot(tree: &mut Styled, plot_node: NodeId, plot: &proto::GeoPlot) -> Option<GraticuleNodes> {
    let spherical = match plot.options.as_ref()?.surface.as_ref()? {
        proto::geo::Surface::Planar(_) => {
            tree.set_attribute(plot_node, "projection", "planar");
            return None;
        }
        proto::geo::Surface::Spherical(spherical) => spherical,
    };
    if let Some(name) = projection_name(spherical) {
        tree.set_attribute(plot_node, "projection", name);
    }
    if let Some(sphere) = &spherical.sphere {
        inline::sphere(tree, plot_node, sphere);
    }
    let graticule = spherical.graticule.as_ref()?;
    let graticule_node = tree.add(plot_node, Element::Graticule);
    let nodes = GraticuleNodes {
        graticule: graticule_node,
        grid: tree.add(graticule_node, Element::Grid),
        label: tree.add(graticule_node, Element::Label),
    };
    inline::graticule(tree, &nodes, graticule);
    Some(nodes)
}

/// The name of a spherical projection: the name of its member of the `projection` oneof, such as
/// `equal_earth`, or the name of a custom projection.
pub(crate) fn projection_name(spherical: &proto::Spherical) -> Option<String> {
    use proto::spherical::Projection;
    Some(
        match spherical.projection.as_ref()? {
            Projection::EqualEarth(_) => "equal_earth",
            Projection::NaturalEarth1(_) => "natural_earth1",
            Projection::Robinson(_) => "robinson",
            Projection::Mollweide(_) => "mollweide",
            Projection::Eckert4(_) => "eckert4",
            Projection::WinkelTripel(_) => "winkel_tripel",
            Projection::Aitoff(_) => "aitoff",
            Projection::Sinusoidal(_) => "sinusoidal",
            Projection::Equirectangular(_) => "equirectangular",
            Projection::Mercator(_) => "mercator",
            Projection::TransverseMercator(_) => "transverse_mercator",
            Projection::Miller(_) => "miller",
            Projection::Orthographic(_) => "orthographic",
            Projection::Stereographic(_) => "stereographic",
            Projection::Gnomonic(_) => "gnomonic",
            Projection::AzimuthalEqualArea(_) => "azimuthal_equal_area",
            Projection::AzimuthalEquidistant(_) => "azimuthal_equidistant",
            Projection::ConicConformal(_) => "conic_conformal",
            Projection::ConicEqualArea(_) => "conic_equal_area",
            Projection::ConicEquidistant(_) => "conic_equidistant",
            Projection::CylindricalEqualArea(_) => "cylindrical_equal_area",
            Projection::Hammer(_) => "hammer",
            Projection::Custom(custom) => return Some(custom.name.clone()),
        }
        .to_string(),
    )
}

/// The nodes of the guide of `group` under `owner`, the `plot` of the node that owns it.
#[allow(clippy::too_many_arguments)]
fn guide_nodes(
    tree: &mut Styled,
    owner: NodeId,
    chart: &Chart,
    resolved: &Resolved,
    group: &ScaleGroup,
    kind: FitKind,
    null: bool,
) -> Option<GuideNodes> {
    let attributes = |tree: &mut StyleTree, node: NodeId| {
        tree.set_attribute(node, "channel", group.channel.field_name());
        tree.set_attribute(node, "scale", scale_type(&group.spec));
        tree.set_attribute(node, "kind", scale_kind(chart, resolved, group, kind));
    };
    let lumps =
        (values_type(chart, resolved, group).as_ref()).is_some_and(avenger_chart::lump::is_lump);
    // The label of the Other group of a lump is the `other` variant of the guide's `label`, and
    // the label of the NULL category its `null` variant.
    let other = |tree: &mut Styled, label: NodeId| Variants {
        other: lumps.then(|| tree.add(label, Element::Other)),
        null: null.then(|| tree.add(label, Element::Null)),
    };
    match guide_kind(group, kind)? {
        GuideKind::PolarAxis => {
            let axis = tree.add(owner, Element::Axis);
            attributes(tree, axis);
            let title = tree.add(axis, Element::Title);
            let label = tree.add(axis, Element::Label);
            let nodes = AxisNodes {
                axis,
                title,
                label,
                variants: other(tree, label),
                tick: tree.add(axis, Element::Tick),
                rule: tree.add(axis, Element::Rule),
                grid: tree.add(axis, Element::Grid),
                levels: Vec::new(),
            };
            if let Some(message) = &group.spec.axis {
                identify(tree, axis, &message.id, &message.classes);
                let plot = resolved.tree.layers[group.members[0].layer].plot;
                let start = (Sector::of_plot(resolved.tree.plots[plot].plot))
                    .map_or(0.0, |sector| sector.start());
                inline::polar_axis(tree, &nodes, message, start);
            }
            Some(GuideNodes::PolarAxis(nodes))
        }
        GuideKind::Axis => {
            let axis = tree.add(owner, Element::Axis);
            attributes(tree, axis);
            let title = tree.add(axis, Element::Title);
            let label = tree.add(axis, Element::Label);
            let nodes = AxisNodes {
                axis,
                title,
                label,
                variants: other(tree, label),
                tick: tree.add(axis, Element::Tick),
                rule: tree.add(axis, Element::Rule),
                grid: tree.add(axis, Element::Grid),
                levels: (0..nested_levels(chart, resolved, group))
                    .map(|_| {
                        let level = tree.add(axis, Element::Level);
                        (level, tree.add(level, Element::Label))
                    })
                    .collect(),
            };
            if let Some(message) = &group.spec.axis {
                identify(tree, axis, &message.id, &message.classes);
                inline::axis(tree, &nodes, message);
            }
            Some(GuideNodes::Axis(nodes))
        }
        GuideKind::Legend => {
            let legend = tree.add(owner, Element::Legend);
            tree.set_attribute(legend, "type", "symbol");
            attributes(tree, legend);
            let title = tree.add(legend, Element::Title);
            let swatch = tree.add(legend, Element::Swatch);
            let label = tree.add(legend, Element::Label);
            let nodes = LegendNodes {
                legend,
                title,
                swatch,
                label,
                variants: other(tree, label),
            };
            if let Some(message) = &group.spec.legend {
                identify(tree, legend, &message.id, &message.classes);
                inline::legend(tree, &nodes, message);
            }
            Some(GuideNodes::Legend(nodes))
        }
        GuideKind::Colorbar => {
            let legend = tree.add(owner, Element::Legend);
            tree.set_attribute(legend, "type", "colorbar");
            attributes(tree, legend);
            let nodes = ColorbarNodes {
                legend,
                title: tree.add(legend, Element::Title),
                ramp: tree.add(legend, Element::Ramp),
                label: tree.add(legend, Element::Label),
                tick: tree.add(legend, Element::Tick),
                rule: tree.add(legend, Element::Rule),
            };
            if let Some(message) = &group.spec.legend {
                identify(tree, legend, &message.id, &message.classes);
                inline::colorbar(tree, &nodes, message);
            }
            Some(GuideNodes::Colorbar(nodes))
        }
    }
}

/// A `mark[store]` under `plot_node` for each `Select` tool of the plot and its layers with a box,
/// which a select without a form also draws, or a lasso, for the mark of its selection's store,
/// which the app draws.
fn store_marks(tree: &mut Styled, plot_node: NodeId, resolved: &Resolved, plot_ref: &PlotRef) {
    let layer_selects = (resolved.tree.layers[plot_ref.layers.clone()].iter())
        .flat_map(|layer| layer.layer.tools())
        .filter_map(|tool| match &tool.kind {
            Some(layer_tool::Kind::Select(select)) => Some(select),
            _ => None,
        });
    let selects = (plot_ref.plot.tools.iter())
        .filter_map(|tool| match &tool.kind {
            Some(tool::Kind::Select(select)) => Some(select),
            _ => None,
        })
        .chain(layer_selects);
    for select in selects {
        let mark = match select.form {
            // A select without a form is an interval on the plot's positional channels.
            Some(select::Form::Interval(_)) | None => "rect",
            Some(select::Form::Lasso(_)) => "path",
            _ => continue,
        };
        let node = tree.add(plot_node, Element::Mark);
        tree.set_attribute_forms(node, "type", mark, mark);
        tree.set_attribute(node, "store", select.selection.clone());
    }
}

/// Sets the id and the classes of a node from the format.
fn identify(tree: &mut StyleTree, node: NodeId, id: &str, classes: &[String]) {
    if !id.is_empty() {
        tree.set_id(node, id);
    }
    for class in classes {
        tree.add_class(node, class.clone());
    }
}

/// The function name of a mark, as avenger-sql names it: its kind with the prefix of its
/// coordinate system, such as `polar_bar`, and a Cartesian mark's kind as it is.
fn function_name(mark: &dyn MarkMessage) -> String {
    match mark.kind() {
        CoordKind::Cartesian => mark.name().to_string(),
        kind => format!("{}_{}", kind.field(), mark.name()),
    }
}

/// The kinds and the function names of the marks of the layers of a scale group, each once, in
/// tree order, separated by spaces.
fn mark_names(resolved: &Resolved, group: &ScaleGroup) -> [String; 2] {
    let mut kinds: Vec<&str> = Vec::new();
    let mut functions: Vec<String> = Vec::new();
    for member in &group.members {
        let mark = resolved.tree.layers[member.layer].mark;
        if !kinds.contains(&mark.name()) {
            kinds.push(mark.name());
        }
        let function = function_name(mark);
        if !functions.contains(&function) {
            functions.push(function);
        }
    }
    [kinds.join(" "), functions.join(" ")]
}

/// The Arrow type of the values that a scale group maps: of its first member's scaled column or
/// scaled constant, or of its explicit domain.
pub(crate) fn values_type(
    chart: &Chart,
    resolved: &Resolved,
    group: &ScaleGroup,
) -> Option<DataType> {
    let member = &group.members[0];
    let layer = &resolved.tree.layers[member.layer];
    let from_encoding = match layer.mark.encoding(member.channel)?.kind.as_ref()? {
        encoding::Kind::ScaledField(name) => Some(avenger_chart::span::values_type(
            layer.table.column_by_name(name)?.data_type(),
        )),
        encoding::Kind::ScaledValue(value) => Some(chart.value(value).data_type().clone()),
        encoding::Kind::DirectField(_) | encoding::Kind::DirectValue(_) => None,
    };
    from_encoding.or_else(|| match group.spec.domain.as_ref()?.domain.as_ref()? {
        proto::domain::Domain::Explicit(values) => {
            Some(chart.list_elements(values).data_type().clone())
        }
        _ => None,
    })
}

/// Whether a member of `group` reads a NULL category: a NULL value of a column that is not a
/// struct, or the NULL category of a column of lumps.
fn holds_null(resolved: &Resolved, group: &ScaleGroup) -> bool {
    (group.members.iter()).any(|member| {
        let layer = &resolved.tree.layers[member.layer];
        let Some(encoding::Kind::ScaledField(name)) = layer
            .mark
            .encoding(member.channel)
            .and_then(|e| e.kind.as_ref())
        else {
            return false;
        };
        (layer.table.column_by_name(name)).is_some_and(|column| {
            let keys = avenger_chart::lump::keys(column).unwrap_or_else(|| column.clone());
            Variant::Null.holds(&keys)
        })
    })
}

/// The `kind` of a scale: `sequential` for a continuous scale, and for a discrete scale, its own
/// kind, or `categorical` over strings, booleans, dictionaries of strings, and lumps, and
/// `ordinal` over other values.
fn scale_kind(
    chart: &Chart,
    resolved: &Resolved,
    group: &ScaleGroup,
    kind: FitKind,
) -> &'static str {
    use proto::scale::Type;
    if kind != FitKind::Categories {
        return match pins_middle(&group.spec) {
            true => "diverging",
            false => "sequential",
        };
    }
    let kind = match &group.spec.r#type {
        Some(Type::Band(band)) => band.kind,
        Some(Type::Point(point)) => point.kind,
        Some(Type::NestedBand(nested)) => nested.kind,
        Some(Type::Ordinal(ordinal)) => ordinal.kind,
        _ => 0,
    };
    match proto::ScaleKind::try_from(kind).unwrap_or_default() {
        proto::ScaleKind::Categorical => return "categorical",
        proto::ScaleKind::Ordinal => return "ordinal",
        proto::ScaleKind::Unspecified => {}
    }
    let data_type = values_type(chart, resolved, group);
    // The kept categories of a lump follow their weights, not their values, so they are
    // categories whatever their type.
    if (data_type.as_ref()).is_some_and(avenger_chart::lump::is_lump) {
        return "categorical";
    }
    let values = data_type.as_ref().map(plain_type);
    match values {
        Some(DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean)
        | None => "categorical",
        Some(_) => "ordinal",
    }
}

/// Whether the interval fit of `spec`'s domain, or of its selection domain's fallback, pins its
/// middle, as a diverging scale's fit does.
fn pins_middle(spec: &proto::Scale) -> bool {
    use proto::{domain, domain_fit, selection_domain};
    let fit = match (spec.domain.as_ref()).and_then(|domain| domain.domain.as_ref()) {
        Some(domain::Domain::Summary(summary)) => summary.fit.as_ref(),
        Some(domain::Domain::Data(data)) => data.fit.as_ref(),
        Some(domain::Domain::Selection(selection)) => match &selection.fallback {
            Some(selection_domain::Fallback::Summary(summary)) => summary.fit.as_ref(),
            Some(selection_domain::Fallback::Data(data)) => data.fit.as_ref(),
            _ => None,
        },
        _ => None,
    };
    matches!(
        fit.and_then(|fit| fit.fit.as_ref()),
        Some(domain_fit::Fit::Interval(proto::IntervalFit {
            pin: Some(pin),
            ..
        })) if pin.position() == proto::PinPosition::Mid
    )
}

/// The type of the values of `data_type`, without a dictionary.
fn plain_type(data_type: &DataType) -> DataType {
    match data_type {
        DataType::Dictionary(_, values) => values.as_ref().clone(),
        other => other.clone(),
    }
}

/// The number of the levels of a nested band scale's axis: the fields of the structs that its
/// values are, or 0 for another scale.
fn nested_levels(chart: &Chart, resolved: &Resolved, group: &ScaleGroup) -> usize {
    if !matches!(group.spec.r#type, Some(proto::scale::Type::NestedBand(_))) {
        return 0;
    }
    match values_type(chart, resolved, group) {
        Some(DataType::Struct(fields)) => fields.len(),
        _ => 0,
    }
}
