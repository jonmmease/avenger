//! The style fields of the format as inline styles of their nodes, as Appendix B of the style
//! proposal maps them: a present field is an inline declaration of its node, and an absent field
//! declares nothing. Each message has one function, and [`STYLE_FIELDS`] lists the fields that
//! they map, which a test checks against the descriptors of the format.

use avenger_chart::proto::{
    self, cartesian_layer, geo_layer, polar_layer, Side, TextAlign, TextBaseline, TextWidthMode,
    TitleAlign,
};
use avenger_style::{NodeId, Property, StyleTree};

use crate::EvalWarning;

/// The style fields that this module maps, by message: every field of the format that is neither
/// content nor deferred to a later evaluator, which `tests/style_fields.rs` checks.
#[doc(hidden)]
pub const STYLE_FIELDS: &[(&str, &[&str])] = &[
    (
        "Chart",
        &[
            "padding_top",
            "padding_right",
            "padding_bottom",
            "padding_left",
        ],
    ),
    ("Plot", &["width", "height"]),
    ("Concat", &["gap"]),
    ("Wrap", &["min_column_width", "max_columns", "gap"]),
    (
        "Facet",
        &[
            "title",
            "orient",
            "labels",
            "label_angle",
            "label_padding",
            "tick_size",
            "rule_color",
            "rule_width",
        ],
    ),
    ("Cartesian", &["aspect"]),
    ("SphereStyle", &["fill", "stroke", "stroke_width"]),
    (
        "Graticule",
        &[
            "label_font_family",
            "label_font_size",
            "label_font_weight",
            "label_color",
            "label_padding",
            "stroke",
            "stroke_width",
        ],
    ),
    (
        "Title",
        &[
            "show",
            "orient",
            "align",
            "font_family",
            "font_size",
            "font_weight",
            "color",
            "subtitle_font_family",
            "subtitle_font_size",
            "subtitle_font_weight",
            "subtitle_color",
        ],
    ),
    ("SymbolMark", &["filled"]),
    ("BarMark", &["corner_radius"]),
    ("RectMark", &["corner_radius", "inset"]),
    (
        "TextMark",
        &[
            "align",
            "baseline",
            "font_family",
            "max_lines",
            "wrap",
            "ellipsis",
            "width_mode",
        ],
    ),
    ("ImageMark", &["align", "baseline", "aspect", "smooth"]),
    ("PolarSymbolMark", &["filled"]),
    ("PolarBarMark", &["corner_radius"]),
    ("PolarRectMark", &["corner_radius", "inset"]),
    (
        "PolarTextMark",
        &[
            "font_family",
            "max_lines",
            "wrap",
            "ellipsis",
            "width_mode",
            "radius_side",
            "theta_side",
            "orient",
        ],
    ),
    ("GeoSymbolMark", &["filled"]),
    (
        "GeoTextMark",
        &[
            "align",
            "baseline",
            "font_family",
            "max_lines",
            "wrap",
            "ellipsis",
            "width_mode",
        ],
    ),
    ("GeoImageMark", &["align", "baseline", "aspect", "smooth"]),
    (
        "Axis",
        &[
            "title",
            "orient",
            "labels",
            "label_angle",
            "label_align",
            "label_baseline",
            "tick_count",
            "ticks",
            "tick_size",
            "tick_color",
            "rule",
            "rule_color",
            "grid",
            "grid_color",
            "grid_width",
            "levels",
            "label_orient",
            "angle",
        ],
    ),
    ("NestedAxisLevel", &["labels", "label_angle"]),
    (
        "Labels",
        &["show", "font_family", "font_size", "font_weight", "color"],
    ),
    (
        "Legend",
        &[
            "title",
            "labels",
            "label_angle",
            "orient",
            "background",
            "border_color",
            "border_radius",
            "padding",
            "wrap",
            "columns",
            "label_padding",
            "column_gap",
            "row_gap",
            "tick_count",
            "swatches",
            "thickness",
            "axis_gap",
            "max_length",
            "label_align",
            "label_baseline",
            "ticks",
            "tick_size",
            "tick_color",
            "rule",
            "rule_color",
        ],
    ),
    (
        "Attribution",
        &[
            "orient",
            "align",
            "separator",
            "separator_space",
            "font_family",
            "font_size",
            "font_weight",
            "color",
            "line_gap",
            "background",
            "border_color",
            "border_radius",
            "padding",
        ],
    ),
    ("Scale", &["values", "palette", "reverse"]),
    ("BandScale", &["padding_inner", "padding_outer"]),
    ("PointScale", &["padding"]),
    ("NestedBandScale", &["padding_inner", "padding_outer"]),
];

/// The values of a range as CSS text: strings as they are, such as colors and shape names,
/// numbers as numbers, and lists, such as dashes, as their numbers apart by spaces.
fn css_items(values: &arrow::array::ArrayRef) -> Vec<String> {
    use arrow::{
        array::{Array, AsArray},
        datatypes::DataType,
    };
    let text = |values: &arrow::array::ArrayRef| -> Vec<String> {
        let values = arrow::compute::cast(values, &DataType::Utf8).expect("a range casts to text");
        let strings = values.as_string::<i32>();
        (0..strings.len())
            .map(|index| strings.value(index).to_string())
            .collect()
    };
    let list = |index: usize| -> arrow::array::ArrayRef {
        match values.data_type() {
            DataType::List(_) => values.as_list::<i32>().value(index),
            DataType::LargeList(_) => values.as_list::<i64>().value(index),
            _ => values.as_fixed_size_list().value(index),
        }
    };
    match values.data_type() {
        DataType::List(_) | DataType::LargeList(_) | DataType::FixedSizeList(..) => (0..values
            .len())
            .map(|index| text(&list(index)).join(" "))
            .collect(),
        _ => text(values),
    }
}

/// A style tree under construction, with a warning for each style field whose text is not a value
/// of its property.
pub(crate) struct Styled {
    pub(crate) tree: StyleTree,
    pub(crate) warnings: Vec<EvalWarning>,
}

impl std::ops::Deref for Styled {
    type Target = StyleTree;

    fn deref(&self) -> &StyleTree {
        &self.tree
    }
}

impl std::ops::DerefMut for Styled {
    fn deref_mut(&mut self) -> &mut StyleTree {
        &mut self.tree
    }
}

/// Sets the inline style of `property` on `node` from CSS text. The reader does not check the CSS
/// text of the format's style fields, such as a color, so text that is not a value of the
/// property sets nothing, the styles give the property, and a warning names the text.
fn set(tree: &mut Styled, node: NodeId, property: Property, text: &str) {
    match property.parse_value(text) {
        Some(value) => tree.set_inline(node, property, value),
        None => {
            let element = tree.node(node).element;
            tree.warnings.push(EvalWarning::NotDrawn {
                path: format!("a style field of `{element}`"),
                reason: format!(
                    "`{text}` is not a value of `{}`, so the styles give it",
                    property.name()
                ),
            });
        }
    }
}

fn px(tree: &mut Styled, node: NodeId, property: Property, value: Option<f32>) {
    if let Some(value) = value {
        set(tree, node, property, &format!("{value}px"));
    }
}

fn deg(tree: &mut Styled, node: NodeId, property: Property, value: Option<f32>) {
    if let Some(value) = value {
        set(tree, node, property, &format!("{value}deg"));
    }
}

/// A CSS color, or nothing for an empty string.
fn color(tree: &mut Styled, node: NodeId, property: Property, value: &str) {
    if !value.is_empty() {
        set(tree, node, property, value);
    }
}

fn keyword(tree: &mut Styled, node: NodeId, property: Property, value: Option<&str>) {
    if let Some(value) = value {
        set(tree, node, property, value);
    }
}

/// Whether a part shows, as `display`: `none` to hide it, and `block` to show it whatever the
/// stylesheets say.
fn shown(tree: &mut Styled, node: NodeId, show: Option<bool>) {
    let display = show.map(|show| match show {
        true => "block",
        false => "none",
    });
    keyword(tree, node, Property::Display, display);
}

fn side(side: i32) -> Option<&'static str> {
    match Side::try_from(side).ok()? {
        Side::Unspecified => None,
        Side::Top => Some("top"),
        Side::Bottom => Some("bottom"),
        Side::Left => Some("left"),
        Side::Right => Some("right"),
    }
}

/// The `text-align` of labels and titles: the start, the center, or the end of their line.
fn text_align(align: i32) -> Option<&'static str> {
    match TextAlign::try_from(align).ok()? {
        TextAlign::Unspecified => None,
        TextAlign::Left => Some("start"),
        TextAlign::Center => Some("center"),
        TextAlign::Right => Some("end"),
    }
}

fn vertical_align(baseline: i32) -> Option<&'static str> {
    match TextBaseline::try_from(baseline).ok()? {
        TextBaseline::Unspecified => None,
        TextBaseline::Top => Some("top"),
        TextBaseline::Middle => Some("middle"),
        TextBaseline::Alphabetic => Some("alphabetic"),
        TextBaseline::Bottom => Some("bottom"),
    }
}

/// The fields of a font and its color, as `font-family`, `font-size`, `font-weight`, and `color`
/// of their node.
struct Font<'a> {
    family: &'a str,
    size: Option<f32>,
    weight: Option<f32>,
    color: &'a str,
}

impl<'a> Font<'a> {
    fn of_title(title: &'a proto::Title) -> Self {
        Font {
            family: &title.font_family,
            size: title.font_size,
            weight: title.font_weight,
            color: &title.color,
        }
    }

    fn of_subtitle(title: &'a proto::Title) -> Self {
        Font {
            family: &title.subtitle_font_family,
            size: title.subtitle_font_size,
            weight: title.subtitle_font_weight,
            color: &title.subtitle_color,
        }
    }

    fn of_labels(labels: &'a proto::Labels) -> Self {
        Font {
            family: &labels.font_family,
            size: labels.font_size,
            weight: labels.font_weight,
            color: &labels.color,
        }
    }
}

fn font(tree: &mut Styled, node: NodeId, font: Font) {
    if !font.family.is_empty() {
        set(tree, node, Property::FontFamily, font.family);
    }
    px(tree, node, Property::FontSize, font.size);
    if let Some(weight) = font.weight {
        set(tree, node, Property::FontWeight, &weight.to_string());
    }
    color(tree, node, Property::Color, font.color);
}

/// A guide's title on its `title` node: whether it shows, and its font.
fn guide_title(tree: &mut Styled, node: NodeId, title: Option<&proto::Title>) {
    let Some(title) = title else { return };
    shown(tree, node, title.show);
    font(tree, node, Font::of_title(title));
}

/// A guide's labels on its `label` node: whether they show, and their font.
fn guide_labels(tree: &mut Styled, node: NodeId, labels: Option<&proto::Labels>) {
    let Some(labels) = labels else { return };
    shown(tree, node, labels.show);
    font(tree, node, Font::of_labels(labels));
}

/// The box behind a legend or an attribution, as `background`, `border-color` with a solid
/// `border-style`, `border-radius`, and `padding` of its node.
fn boxed(
    tree: &mut Styled,
    node: NodeId,
    background: &str,
    border_color: &str,
    border_radius: Option<f32>,
    padding: Option<f32>,
) {
    color(tree, node, Property::Background, background);
    // A border color draws a solid border, whose width the styles give.
    if !border_color.is_empty() {
        color(tree, node, Property::BorderColor, border_color);
        set(tree, node, Property::BorderStyle, "solid");
    }
    px(tree, node, Property::BorderRadius, border_radius);
    for property in [
        Property::PaddingTop,
        Property::PaddingRight,
        Property::PaddingBottom,
        Property::PaddingLeft,
    ] {
        px(tree, node, property, padding);
    }
}

/// `Chart.padding_top`, `padding_right`, `padding_bottom`, and `padding_left`, as the paddings of
/// the root `chart`.
pub(crate) fn chart(tree: &mut Styled, node: NodeId, chart: &proto::Chart) {
    px(tree, node, Property::PaddingTop, chart.padding_top);
    px(tree, node, Property::PaddingRight, chart.padding_right);
    px(tree, node, Property::PaddingBottom, chart.padding_bottom);
    px(tree, node, Property::PaddingLeft, chart.padding_left);
}

/// `Plot.width` and `height`, and the options of a Cartesian plot, on its `plot`.
pub(crate) fn plot(tree: &mut Styled, node: NodeId, plot: &proto::Plot) {
    px(tree, node, Property::Width, plot.width);
    px(tree, node, Property::Height, plot.height);
    if let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord {
        if let Some(ratio) = cartesian
            .options
            .as_ref()
            .and_then(|options| options.aspect)
        {
            set(tree, node, Property::AspectRatio, &format!("{ratio} / 1"));
        }
    }
}

/// `Concat.gap` on the `plot` of a concat.
pub(crate) fn concat(tree: &mut Styled, node: NodeId, concat: &proto::Concat) {
    px(tree, node, Property::RowGap, concat.gap);
    px(tree, node, Property::ColumnGap, concat.gap);
}

/// `Wrap.gap`, `min_column_width`, and `max_columns` on the `plot` of a wrap.
pub(crate) fn wrap(tree: &mut Styled, node: NodeId, wrap: &proto::Wrap) {
    px(tree, node, Property::RowGap, wrap.gap);
    px(tree, node, Property::ColumnGap, wrap.gap);
    px(tree, node, Property::MinColumnWidth, wrap.min_column_width);
    if let Some(columns) = wrap.max_columns {
        set(tree, node, Property::MaxColumns, &columns.to_string());
    }
}

/// The nodes of a facet and its parts.
#[derive(Clone, Debug)]
pub(crate) struct FacetNodes {
    pub(crate) facet: NodeId,
    pub(crate) title: NodeId,
    pub(crate) label: NodeId,
    pub(crate) tick: NodeId,
    pub(crate) rule: NodeId,
}

/// A `Facet` on its nodes.
pub(crate) fn facet(tree: &mut Styled, nodes: &FacetNodes, facet: &proto::Facet) {
    keyword(tree, nodes.facet, Property::Orient, side(facet.orient));
    deg(tree, nodes.facet, Property::LabelAngle, facet.label_angle);
    px(
        tree,
        nodes.facet,
        Property::LabelPadding,
        facet.label_padding,
    );
    guide_title(tree, nodes.title, facet.title.as_ref());
    guide_labels(tree, nodes.label, facet.labels.as_ref());
    px(tree, nodes.tick, Property::TickSize, facet.tick_size);
    for node in [nodes.tick, nodes.rule] {
        color(tree, node, Property::Stroke, &facet.rule_color);
        px(tree, node, Property::StrokeWidth, facet.rule_width);
    }
}

/// A `Title` on its `title` node, and its subtitle's style on its `subtitle` node.
pub(crate) fn title(
    tree: &mut Styled,
    title_node: NodeId,
    subtitle_node: Option<NodeId>,
    title: &proto::Title,
) {
    keyword(tree, title_node, Property::Orient, side(title.orient));
    let align = match TitleAlign::try_from(title.align).unwrap_or_default() {
        TitleAlign::Unspecified => None,
        TitleAlign::Start => Some("start"),
        TitleAlign::Center => Some("center"),
        TitleAlign::End => Some("end"),
    };
    keyword(tree, title_node, Property::TextAlign, align);
    font(tree, title_node, Font::of_title(title));
    if let Some(subtitle) = subtitle_node {
        font(tree, subtitle, Font::of_subtitle(title));
    }
}

/// The style fields of a Cartesian mark on its `mark` node.
pub(crate) fn mark(tree: &mut Styled, node: NodeId, mark: &cartesian_layer::Mark) {
    use cartesian_layer::Mark;
    match mark {
        Mark::Symbol(symbol) => {
            if symbol.filled == Some(false) {
                set(tree, node, Property::Fill, "none");
                set(tree, node, Property::Stroke, "currentColor");
            }
        }
        Mark::Bar(bar) => px(tree, node, Property::CornerRadius, bar.corner_radius),
        Mark::Rect(rect) => {
            px(tree, node, Property::CornerRadius, rect.corner_radius);
            px(tree, node, Property::Inset, rect.inset);
        }
        Mark::Text(text) => {
            text_fields(
                tree,
                node,
                TextFields {
                    font_family: &text.font_family,
                    max_lines: text.max_lines,
                    wrap: text.wrap,
                    ellipsis: text.ellipsis,
                    width_mode: text.width_mode,
                },
            );
            text_anchor(tree, node, text.align, text.baseline);
        }
        Mark::Image(image) => image_fields(
            tree,
            node,
            image.align,
            image.baseline,
            image.aspect,
            image.smooth,
        ),
        Mark::Line(_) | Mark::Trail(_) | Mark::Area(_) | Mark::Rule(_) | Mark::Path(_) => {}
    }
}

/// The `align` and the `baseline` of a text mark on its `mark` node.
fn text_anchor(tree: &mut Styled, node: NodeId, align: i32, baseline: i32) {
    let align = match TextAlign::try_from(align).unwrap_or_default() {
        TextAlign::Unspecified => None,
        TextAlign::Left => Some("left"),
        TextAlign::Center => Some("center"),
        TextAlign::Right => Some("right"),
    };
    keyword(tree, node, Property::Align, align);
    keyword(tree, node, Property::Baseline, vertical_align(baseline));
}

/// The fields of an image mark on its `mark` node.
fn image_fields(
    tree: &mut Styled,
    node: NodeId,
    align: i32,
    baseline: i32,
    aspect: Option<bool>,
    smooth: Option<bool>,
) {
    let align = match proto::ImageAlign::try_from(align).unwrap_or_default() {
        proto::ImageAlign::Unspecified => None,
        proto::ImageAlign::Left => Some("left"),
        proto::ImageAlign::Center => Some("center"),
        proto::ImageAlign::Right => Some("right"),
    };
    keyword(tree, node, Property::Align, align);
    let baseline = match proto::ImageBaseline::try_from(baseline).unwrap_or_default() {
        proto::ImageBaseline::Unspecified => None,
        proto::ImageBaseline::Top => Some("top"),
        proto::ImageBaseline::Middle => Some("middle"),
        proto::ImageBaseline::Bottom => Some("bottom"),
    };
    keyword(tree, node, Property::Baseline, baseline);
    if let Some(aspect) = aspect {
        let fit = if aspect { "contain" } else { "fill" };
        set(tree, node, Property::ObjectFit, fit);
    }
    if let Some(smooth) = smooth {
        let rendering = if smooth { "smooth" } else { "pixelated" };
        set(tree, node, Property::ImageRendering, rendering);
    }
}

/// The style fields of a geo mark on its `mark` node.
pub(crate) fn geo_mark(tree: &mut Styled, node: NodeId, mark: &geo_layer::Mark) {
    use geo_layer::Mark;
    match mark {
        Mark::Symbol(symbol) => {
            if symbol.filled == Some(false) {
                set(tree, node, Property::Fill, "none");
                set(tree, node, Property::Stroke, "currentColor");
            }
        }
        Mark::Text(text) => {
            text_fields(
                tree,
                node,
                TextFields {
                    font_family: &text.font_family,
                    max_lines: text.max_lines,
                    wrap: text.wrap,
                    ellipsis: text.ellipsis,
                    width_mode: text.width_mode,
                },
            );
            text_anchor(tree, node, text.align, text.baseline);
        }
        Mark::Image(image) => image_fields(
            tree,
            node,
            image.align,
            image.baseline,
            image.aspect,
            image.smooth,
        ),
        Mark::Line(_) | Mark::Trail(_) | Mark::Rule(_) | Mark::Path(_) | Mark::Shape(_) => {}
    }
}

/// A `SphereStyle` on the `plot[type=geo]` of its geo plot: its fill as the `background`, and its
/// stroke and width as a solid border.
pub(crate) fn sphere(tree: &mut Styled, node: NodeId, sphere: &proto::SphereStyle) {
    color(tree, node, Property::Background, &sphere.fill);
    if !sphere.stroke.is_empty() {
        color(tree, node, Property::BorderColor, &sphere.stroke);
        set(tree, node, Property::BorderStyle, "solid");
    }
    px(tree, node, Property::BorderWidth, sphere.stroke_width);
}

/// The nodes of a graticule and its parts.
#[derive(Clone, Debug)]
pub(crate) struct GraticuleNodes {
    pub(crate) graticule: NodeId,
    pub(crate) grid: NodeId,
    pub(crate) label: NodeId,
}

/// A `Graticule` on its nodes.
pub(crate) fn graticule(tree: &mut Styled, nodes: &GraticuleNodes, graticule: &proto::Graticule) {
    color(tree, nodes.grid, Property::Stroke, &graticule.stroke);
    px(
        tree,
        nodes.grid,
        Property::StrokeWidth,
        graticule.stroke_width,
    );
    font(
        tree,
        nodes.label,
        Font {
            family: &graticule.label_font_family,
            size: graticule.label_font_size,
            weight: graticule.label_font_weight,
            color: &graticule.label_color,
        },
    );
    px(
        tree,
        nodes.graticule,
        Property::LabelPadding,
        graticule.label_padding,
    );
}

/// The fields that the text marks of every coordinate system share.
struct TextFields<'a> {
    font_family: &'a str,
    max_lines: Option<u32>,
    wrap: Option<bool>,
    ellipsis: Option<bool>,
    width_mode: i32,
}

/// The shared fields of a text mark on its `mark` node.
fn text_fields(tree: &mut Styled, node: NodeId, text: TextFields) {
    if !text.font_family.is_empty() {
        set(tree, node, Property::FontFamily, text.font_family);
    }
    if let Some(lines) = text.max_lines {
        set(tree, node, Property::MaxLines, &lines.to_string());
    }
    if let Some(wrap) = text.wrap {
        let white_space = if wrap { "normal" } else { "nowrap" };
        set(tree, node, Property::WhiteSpace, white_space);
    }
    if let Some(ellipsis) = text.ellipsis {
        let overflow = if ellipsis { "ellipsis" } else { "clip" };
        set(tree, node, Property::TextOverflow, overflow);
    }
    let mode = match TextWidthMode::try_from(text.width_mode).unwrap_or_default() {
        TextWidthMode::Unspecified => None,
        TextWidthMode::Max => Some("max"),
        TextWidthMode::Fixed => Some("fixed"),
    };
    keyword(tree, node, Property::WidthMode, mode);
}

/// The keyword of a `PolarTextOrient`, or `None` when it is unspecified.
fn polar_orient(orient: i32) -> Option<&'static str> {
    match proto::PolarTextOrient::try_from(orient).ok()? {
        proto::PolarTextOrient::Unspecified => None,
        proto::PolarTextOrient::Radial => Some("radial"),
        proto::PolarTextOrient::Tangential => Some("tangential"),
    }
}

/// The style fields of a polar mark on its `mark` node.
pub(crate) fn polar_mark(tree: &mut Styled, node: NodeId, mark: &polar_layer::Mark) {
    use polar_layer::Mark;
    match mark {
        Mark::Symbol(symbol) => {
            if symbol.filled == Some(false) {
                set(tree, node, Property::Fill, "none");
                set(tree, node, Property::Stroke, "currentColor");
            }
        }
        Mark::Bar(bar) => px(tree, node, Property::CornerRadius, bar.corner_radius),
        Mark::Rect(rect) => {
            px(tree, node, Property::CornerRadius, rect.corner_radius);
            px(tree, node, Property::Inset, rect.inset);
        }
        Mark::Text(text) => {
            text_fields(
                tree,
                node,
                TextFields {
                    font_family: &text.font_family,
                    max_lines: text.max_lines,
                    wrap: text.wrap,
                    ellipsis: text.ellipsis,
                    width_mode: text.width_mode,
                },
            );
            let radius_side = match proto::RadiusSide::try_from(text.radius_side).ok() {
                Some(proto::RadiusSide::Outward) => Some("outward"),
                Some(proto::RadiusSide::Middle) => Some("middle"),
                Some(proto::RadiusSide::Inward) => Some("inward"),
                _ => None,
            };
            keyword(tree, node, Property::RadiusSide, radius_side);
            let theta_side = match proto::ThetaSide::try_from(text.theta_side).ok() {
                Some(proto::ThetaSide::Clockwise) => Some("clockwise"),
                Some(proto::ThetaSide::Middle) => Some("middle"),
                Some(proto::ThetaSide::Counterclockwise) => Some("counterclockwise"),
                _ => None,
            };
            keyword(tree, node, Property::ThetaSide, theta_side);
            keyword(tree, node, Property::TextOrient, polar_orient(text.orient));
        }
        Mark::Line(_) | Mark::Trail(_) | Mark::Area(_) | Mark::Rule(_) => {}
    }
}

/// The nodes of an axis and its parts.
#[derive(Clone, Debug)]
pub(crate) struct AxisNodes {
    pub(crate) axis: NodeId,
    pub(crate) title: NodeId,
    pub(crate) label: NodeId,
    /// The variants of the `label` of an axis: an `other` over a column of lumps, which styles the
    /// label of the Other group, and a `null` where the scale has a NULL category, which styles
    /// its label.
    pub(crate) variants: crate::variants::Variants,
    pub(crate) tick: NodeId,
    pub(crate) rule: NodeId,
    pub(crate) grid: NodeId,
    /// The `level` of each level of a nested band axis, with its `label`.
    pub(crate) levels: Vec<(NodeId, NodeId)>,
}

/// The fields that an `Axis` and the axis of a colorbar share, on the axis's node and its parts.
#[allow(clippy::too_many_arguments)]
fn axis_fields(
    tree: &mut Styled,
    axis: NodeId,
    title: NodeId,
    label: NodeId,
    tick: NodeId,
    rule: NodeId,
    fields: AxisFields,
) {
    keyword(tree, axis, Property::Orient, side(fields.orient));
    deg(tree, axis, Property::LabelAngle, fields.label_angle);
    guide_title(tree, title, fields.title);
    guide_labels(tree, label, fields.labels);
    shown(tree, tick, fields.ticks);
    shown(tree, rule, fields.rule);
    keyword(
        tree,
        label,
        Property::TextAlign,
        text_align(fields.label_align),
    );
    keyword(
        tree,
        label,
        Property::VerticalAlign,
        vertical_align(fields.label_baseline),
    );
    if let Some(count) = fields.tick_count {
        set(tree, axis, Property::TickCount, &count.to_string());
    }
    px(tree, tick, Property::TickSize, fields.tick_size);
    color(tree, tick, Property::Stroke, fields.tick_color);
    color(tree, rule, Property::Stroke, fields.rule_color);
}

/// The fields of an axis that an `Axis` and the axis of a colorbar share.
struct AxisFields<'a> {
    orient: i32,
    title: Option<&'a proto::Title>,
    labels: Option<&'a proto::Labels>,
    label_angle: Option<f32>,
    label_align: i32,
    label_baseline: i32,
    tick_count: Option<f64>,
    ticks: Option<bool>,
    tick_size: Option<f32>,
    tick_color: &'a str,
    rule: Option<bool>,
    rule_color: &'a str,
}

/// An `Axis` on its nodes.
pub(crate) fn axis(tree: &mut Styled, nodes: &AxisNodes, axis: &proto::Axis) {
    axis_fields(
        tree,
        nodes.axis,
        nodes.title,
        nodes.label,
        nodes.tick,
        nodes.rule,
        AxisFields {
            orient: axis.orient,
            title: axis.title.as_ref(),
            labels: axis.labels.as_ref(),
            label_angle: match axis.label_rotation {
                Some(proto::axis::LabelRotation::LabelAngle(angle)) => Some(angle),
                _ => None,
            },
            label_align: axis.label_align,
            label_baseline: axis.label_baseline,
            tick_count: match axis.tick_placement {
                Some(proto::axis::TickPlacement::TickCount(count)) => Some(count),
                _ => None,
            },
            ticks: axis.ticks,
            tick_size: axis.tick_size,
            tick_color: &axis.tick_color,
            rule: axis.rule,
            rule_color: &axis.rule_color,
        },
    );
    shown(tree, nodes.grid, axis.grid);
    color(tree, nodes.grid, Property::Stroke, &axis.grid_color);
    px(tree, nodes.grid, Property::StrokeWidth, axis.grid_width);
    for (level, (level_node, label_node)) in axis.levels.iter().zip(&nodes.levels) {
        shown(
            tree,
            *label_node,
            level.labels.as_ref().and_then(|labels| labels.show),
        );
        deg(tree, *level_node, Property::LabelAngle, level.label_angle);
    }
}

/// The `Axis` of `theta` or `radius` on its nodes, in a plot whose angles start at `start_angle`:
/// the fields of every axis, the orient of its labels, and its `angle`, which runs from the start
/// angle, so its initial 0 is the start angle, as an absent `angle` is.
pub(crate) fn polar_axis(
    tree: &mut Styled,
    nodes: &AxisNodes,
    axis_message: &proto::Axis,
    start_angle: f32,
) {
    axis(tree, nodes, axis_message);
    if let Some(proto::axis::LabelRotation::LabelOrient(orient)) = axis_message.label_rotation {
        keyword(
            tree,
            nodes.axis,
            Property::LabelOrient,
            polar_orient(orient),
        );
    }
    let angle = axis_message.angle.map(|angle| angle - start_angle);
    deg(tree, nodes.axis, Property::Angle, angle);
}

/// The nodes of a symbol legend and its parts.
#[derive(Clone, Debug)]
pub(crate) struct LegendNodes {
    pub(crate) legend: NodeId,
    pub(crate) title: NodeId,
    pub(crate) swatch: NodeId,
    pub(crate) label: NodeId,
    /// The variants of the `label` of a legend: an `other` over a column of lumps, which styles
    /// the label of the Other group, and a `null` where the scale has a NULL category, which
    /// styles its label.
    pub(crate) variants: crate::variants::Variants,
}

/// A `Legend` on its nodes.
pub(crate) fn legend(tree: &mut Styled, nodes: &LegendNodes, legend: &proto::Legend) {
    let node = nodes.legend;
    keyword(tree, node, Property::Orient, side(legend.orient));
    let wrap = match proto::LegendWrap::try_from(legend.wrap).unwrap_or_default() {
        proto::LegendWrap::Flow => Some("flow"),
        proto::LegendWrap::Grid => Some("grid"),
        // A count of columns lays out a grid.
        proto::LegendWrap::Unspecified => legend.columns.map(|_| "grid"),
    };
    keyword(tree, node, Property::Wrap, wrap);
    if let Some(columns) = legend.columns {
        set(tree, node, Property::Columns, &columns.to_string());
    }
    px(tree, node, Property::LabelPadding, legend.label_padding);
    px(tree, node, Property::ColumnGap, legend.column_gap);
    px(tree, node, Property::RowGap, legend.row_gap);
    legend_box(tree, node, legend);
    if let Some(proto::legend::TickPlacement::TickCount(count)) = legend.tick_placement {
        set(tree, node, Property::TickCount, &count.to_string());
    }
    deg(tree, node, Property::LabelAngle, legend.label_angle);
    shown(tree, nodes.swatch, legend.swatches);
    guide_title(tree, nodes.title, legend.title.as_ref());
    guide_labels(tree, nodes.label, legend.labels.as_ref());
}

/// The box behind a legend on its `legend` node.
fn legend_box(tree: &mut Styled, node: NodeId, legend: &proto::Legend) {
    boxed(
        tree,
        node,
        &legend.background,
        &legend.border_color,
        legend.border_radius,
        legend.padding,
    );
}

/// The nodes of a colorbar and its parts.
#[derive(Clone, Debug)]
pub(crate) struct ColorbarNodes {
    pub(crate) legend: NodeId,
    pub(crate) title: NodeId,
    pub(crate) ramp: NodeId,
    pub(crate) label: NodeId,
    pub(crate) tick: NodeId,
    pub(crate) rule: NodeId,
}

/// A `Legend` in the form of a colorbar on its nodes.
pub(crate) fn colorbar(tree: &mut Styled, nodes: &ColorbarNodes, legend: &proto::Legend) {
    px(tree, nodes.ramp, Property::Thickness, legend.thickness);
    px(tree, nodes.ramp, Property::MaxLength, legend.max_length);
    px(tree, nodes.legend, Property::AxisGap, legend.axis_gap);
    legend_box(tree, nodes.legend, legend);
    axis_fields(
        tree,
        nodes.legend,
        nodes.title,
        nodes.label,
        nodes.tick,
        nodes.rule,
        AxisFields {
            orient: legend.orient,
            title: legend.title.as_ref(),
            labels: legend.labels.as_ref(),
            label_angle: legend.label_angle,
            label_align: legend.label_align,
            label_baseline: legend.label_baseline,
            tick_count: match legend.tick_placement {
                Some(proto::legend::TickPlacement::TickCount(count)) => Some(count),
                _ => None,
            },
            ticks: legend.ticks,
            tick_size: legend.tick_size,
            tick_color: &legend.tick_color,
            rule: legend.rule,
            rule_color: &legend.rule_color,
        },
    );
}

/// An `Attribution` on its `attribution` node.
pub(crate) fn attribution(tree: &mut Styled, node: NodeId, attribution: &proto::Attribution) {
    keyword(tree, node, Property::Orient, side(attribution.orient));
    let align = match proto::AttributionAlign::try_from(attribution.align).unwrap_or_default() {
        proto::AttributionAlign::Unspecified => None,
        proto::AttributionAlign::Start => Some("start"),
        proto::AttributionAlign::End => Some("end"),
    };
    keyword(tree, node, Property::TextAlign, align);
    if !attribution.separator.is_empty() {
        let separator = &attribution.separator;
        let quoted = format!(
            "\"{}\"",
            separator.replace('\\', "\\\\").replace('"', "\\\"")
        );
        set(tree, node, Property::Separator, &quoted);
    }
    px(
        tree,
        node,
        Property::SeparatorSpace,
        attribution.separator_space,
    );
    font(
        tree,
        node,
        Font {
            family: &attribution.font_family,
            size: attribution.font_size,
            weight: attribution.font_weight,
            color: &attribution.color,
        },
    );
    px(tree, node, Property::LineGap, attribution.line_gap);
    boxed(
        tree,
        node,
        &attribution.background,
        &attribution.border_color,
        attribution.border_radius,
        attribution.padding,
    );
}

/// The range and the paddings of a scale's spec on its `scale` node: a palette as `scheme`, and a
/// list of values as `range`, each in reverse with `reverse`. The paddings of a nested band scale,
/// which can differ for each level, are content.
pub(crate) fn scale(
    tree: &mut Styled,
    node: NodeId,
    spec: &proto::Scale,
    chart: &avenger_chart::Chart,
) {
    use proto::scale::Type;
    match &spec.range {
        Some(proto::scale::Range::Palette(palette)) => {
            let text = match spec.reverse {
                true => format!("{palette} reverse"),
                false => palette.clone(),
            };
            set(tree, node, Property::Scheme, &text);
        }
        Some(proto::scale::Range::Values(values)) => {
            let mut items = css_items(&chart.list_elements(values));
            if spec.reverse {
                items.reverse();
            }
            set(tree, node, Property::Range, &items.join(", "));
        }
        None => {}
    }
    let number = |tree: &mut Styled, property: Property, value: Option<f64>| {
        if let Some(value) = value {
            set(tree, node, property, &value.to_string());
        }
    };
    match &spec.r#type {
        Some(Type::Band(band)) => {
            number(tree, Property::PaddingInner, band.padding_inner);
            number(tree, Property::PaddingOuter, band.padding_outer);
        }
        Some(Type::Point(point)) => number(tree, Property::PaddingOuter, point.padding),
        Some(Type::NestedBand(nested)) => {
            let list = |tree: &mut Styled, property: Property, values: &[f64]| {
                if !values.is_empty() {
                    let values: Vec<String> = values.iter().map(f64::to_string).collect();
                    set(tree, node, property, &values.join(" "));
                }
            };
            list(tree, Property::PaddingInner, &nested.padding_inner);
            list(tree, Property::PaddingOuter, &nested.padding_outer);
        }
        _ => {}
    }
}
