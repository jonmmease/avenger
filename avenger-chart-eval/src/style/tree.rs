//! The style tree of a chart of one plot, built once for each chart from its resolved models, as
//! the style proposal's table of nodes gives it, with the inline styles of the format's style
//! fields.

use arrow::datatypes::DataType;
use avenger_chart::{
    proto::{self, encoding, guide, layer_tool, plot::Coord, select, tool},
    resolve::{Resolved, ScaleGroup},
    Chart,
};
use avenger_style::{Element, NodeId, StyleTree};

use crate::EvalWarning;

use super::{
    guide_kind, inline,
    inline::{AxisNodes, ColorbarNodes, LegendNodes, Styled},
    is_discrete, scale_type, GuideKind,
};

/// The nodes of a guide and its parts.
#[derive(Clone, Debug)]
pub(crate) enum GuideNodes {
    Axis(AxisNodes),
    Legend(LegendNodes),
    Colorbar(ColorbarNodes),
}

/// The nodes of the style tree that stand for the parts of a chart of one plot.
#[derive(Clone, Debug)]
pub(crate) struct StyleNodes {
    pub(crate) chart: NodeId,
    pub(crate) title: Option<NodeId>,
    pub(crate) subtitle: Option<NodeId>,
    pub(crate) plot: NodeId,
    pub(crate) attribution: Option<NodeId>,
    /// The `mark` of each layer, in the order of `Tree::layers`.
    pub(crate) marks: Vec<NodeId>,
    /// The `scale` of each scale group, in the order of `Resolved::scales`.
    pub(crate) scales: Vec<NodeId>,
    /// The nodes of each scale group's guide, or `None` for a group without a guide.
    pub(crate) guides: Vec<Option<GuideNodes>>,
}

/// The style tree of `chart`, whose root is one Cartesian plot, with its nodes and a warning for
/// each style field whose text is not a value of its property.
pub(crate) fn build(
    chart: &Chart,
    resolved: &Resolved,
) -> (StyleTree, StyleNodes, Vec<EvalWarning>) {
    let plot_ref = &resolved.tree.plots[0];
    let plot = plot_ref.plot;
    let mut tree = Styled {
        tree: StyleTree::new(Element::Chart),
        warnings: Vec::new(),
    };
    let chart_node = tree.root();
    inline::chart(&mut tree, chart_node, chart.message());

    let (title, subtitle) = match &plot.title {
        Some(title) => {
            let title_node = tree.add(chart_node, Element::Title);
            let subtitle_node =
                (!title.subtitle.is_empty()).then(|| tree.add(chart_node, Element::Subtitle));
            inline::title(&mut tree, title_node, subtitle_node, title);
            (Some(title_node), subtitle_node)
        }
        None => (None, None),
    };

    let plot_node = tree.add(chart_node, Element::Plot);
    tree.set_attribute(plot_node, "type", "cartesian");
    identify(&mut tree, plot_node, &plot.id, &plot.classes);
    let discrete: Vec<&str> = (resolved.scales.iter())
        .filter(|group| group.is_position() && is_discrete(&group.spec))
        .map(|group| group.channel.field_name())
        .collect();
    if !discrete.is_empty() {
        tree.set_attribute(plot_node, "discrete", discrete.join(" "));
    }
    inline::plot(&mut tree, plot_node, plot);

    let mut marks = Vec::with_capacity(resolved.tree.layers.len());
    for layer in &resolved.tree.layers {
        let node = tree.add(plot_node, Element::Mark);
        // A Cartesian mark has the same name in both forms.
        let name = layer.mark.name();
        tree.set_attribute_forms(node, "type", name, name);
        identify(&mut tree, node, layer.layer.id(), layer.layer.classes());
        if let avenger_chart::layer::LayerRef::Cartesian(cartesian) = layer.layer {
            let mark = cartesian.mark.as_ref().expect("a valid layer has a mark");
            inline::mark(&mut tree, node, mark);
        }
        marks.push(node);
    }

    let mut scales = Vec::with_capacity(resolved.scales.len());
    for group in &resolved.scales {
        let node = tree.add(plot_node, Element::Scale);
        let type_name = scale_type(&group.spec);
        tree.set_attribute(node, "type", type_name);
        tree.set_attribute(node, "channel", group.channel.field_name());
        tree.set_attribute(node, "kind", scale_kind(chart, resolved, group));
        let names = mark_names(resolved, group);
        tree.set_attribute_forms(node, "marks", names.clone(), names);
        inline::scale(&mut tree, node, &group.spec, chart);
        scales.push(node);
    }

    let mut guides = Vec::with_capacity(resolved.scales.len());
    for group in &resolved.scales {
        let spec_guide = group
            .spec
            .guide
            .as_ref()
            .and_then(|guide| guide.guide.as_ref());
        let attributes = |tree: &mut StyleTree, node: NodeId| {
            tree.set_attribute(node, "channel", group.channel.field_name());
            tree.set_attribute(node, "scale", scale_type(&group.spec));
            tree.set_attribute(node, "kind", scale_kind(chart, resolved, group));
        };
        let nodes = match guide_kind(group) {
            None => None,
            Some(GuideKind::Axis) => {
                let axis = tree.add(plot_node, Element::Axis);
                attributes(&mut tree, axis);
                let nodes = AxisNodes {
                    axis,
                    title: tree.add(axis, Element::Title),
                    label: tree.add(axis, Element::Label),
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
                if let Some(guide::Guide::Axis(message)) = spec_guide {
                    identify(&mut tree, axis, &message.id, &message.classes);
                    inline::axis(&mut tree, &nodes, message);
                }
                Some(GuideNodes::Axis(nodes))
            }
            Some(GuideKind::Legend) => {
                let legend = tree.add(plot_node, Element::Legend);
                tree.set_attribute(legend, "type", "symbol");
                attributes(&mut tree, legend);
                let nodes = LegendNodes {
                    legend,
                    title: tree.add(legend, Element::Title),
                    swatch: tree.add(legend, Element::Swatch),
                    label: tree.add(legend, Element::Label),
                };
                if let Some(guide::Guide::Legend(message)) = spec_guide {
                    identify(&mut tree, legend, &message.id, &message.classes);
                    inline::legend(&mut tree, &nodes, message);
                }
                Some(GuideNodes::Legend(nodes))
            }
            Some(GuideKind::Colorbar) => {
                let legend = tree.add(plot_node, Element::Legend);
                tree.set_attribute(legend, "type", "colorbar");
                attributes(&mut tree, legend);
                let nodes = ColorbarNodes {
                    legend,
                    title: tree.add(legend, Element::Title),
                    ramp: tree.add(legend, Element::Ramp),
                    label: tree.add(legend, Element::Label),
                    tick: tree.add(legend, Element::Tick),
                    rule: tree.add(legend, Element::Rule),
                };
                if let Some(guide::Guide::Colorbar(message)) = spec_guide {
                    identify(&mut tree, legend, &message.id, &message.classes);
                    inline::colorbar(&mut tree, &nodes, message);
                }
                Some(GuideNodes::Colorbar(nodes))
            }
        };
        guides.push(nodes);
    }

    let attribution = plot.attribution.as_ref().map(|attribution| {
        let node = tree.add(plot_node, Element::Attribution);
        inline::attribution(&mut tree, node, attribution);
        node
    });

    // Each `Select` tool with a box or a lasso has a `mark[store]` for the mark of its selection's
    // store, which the app draws.
    let layer_selects = (resolved.tree.layers.iter())
        .flat_map(|layer| layer.layer.tools())
        .filter_map(|tool| match &tool.kind {
            Some(layer_tool::Kind::Select(select)) => Some(select),
            _ => None,
        });
    let selects = (plot.tools.iter())
        .filter_map(|tool| match &tool.kind {
            Some(tool::Kind::Select(select)) => Some(select),
            _ => None,
        })
        .chain(layer_selects);
    for select in selects {
        let mark = match select.form {
            Some(select::Form::Interval(_)) => "rect",
            Some(select::Form::Lasso(_)) => "path",
            _ => continue,
        };
        let node = tree.add(plot_node, Element::Mark);
        tree.set_attribute_forms(node, "type", mark, mark);
        tree.set_attribute(node, "store", select.selection.clone());
    }

    let nodes = StyleNodes {
        chart: chart_node,
        title,
        subtitle,
        plot: plot_node,
        attribution,
        marks,
        scales,
        guides,
    };
    (tree.tree, nodes, tree.warnings)
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

/// The names of the marks of the layers of a scale group, each once, in tree order, separated by
/// spaces.
fn mark_names(resolved: &Resolved, group: &ScaleGroup) -> String {
    let mut names: Vec<&str> = Vec::new();
    for member in &group.members {
        let name = resolved.tree.layers[member.layer].mark.name();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.join(" ")
}

/// The Arrow type of the values that a scale group maps: of its first member's column or scaled
/// constant, or of its explicit domain.
fn values_type(chart: &Chart, resolved: &Resolved, group: &ScaleGroup) -> Option<DataType> {
    let member = &group.members[0];
    let layer = &resolved.tree.layers[member.layer];
    let from_encoding = match layer.mark.encoding(member.channel)?.value.as_ref()? {
        encoding::Value::Column(name) => {
            Some(layer.table.column_by_name(name)?.data_type().clone())
        }
        encoding::Value::ScaledConstant(value) => Some(chart.value(value).data_type().clone()),
        encoding::Value::Constant(_) => None,
    };
    from_encoding.or_else(|| match group.spec.domain.as_ref()?.domain.as_ref()? {
        proto::domain::Domain::Explicit(values) => {
            Some(chart.list_elements(values).data_type().clone())
        }
        _ => None,
    })
}

/// The `kind` of a scale: `sequential` for a continuous scale, and for a discrete scale, its own
/// kind, or `categorical` over strings, booleans, and dictionaries of strings and `ordinal` over
/// other values.
fn scale_kind(chart: &Chart, resolved: &Resolved, group: &ScaleGroup) -> &'static str {
    use proto::scale_spec::Type;
    if !is_discrete(&group.spec) {
        return "sequential";
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
    let values = data_type.as_ref().map(plain_type);
    match values {
        Some(DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Boolean)
        | None => "categorical",
        Some(_) => "ordinal",
    }
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
    if !matches!(
        group.spec.r#type,
        Some(proto::scale_spec::Type::NestedBand(_))
    ) {
        return 0;
    }
    match values_type(chart, resolved, group) {
        Some(DataType::Struct(fields)) => fields.len(),
        _ => 0,
    }
}

/// Whether a plot is the Cartesian root that this evaluator draws.
pub(crate) fn is_cartesian(plot: &proto::Plot) -> bool {
    matches!(plot.coord, Some(Coord::Cartesian(_)))
}
