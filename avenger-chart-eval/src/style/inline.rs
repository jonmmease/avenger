//! The style fields of the format as inline styles of their nodes, as Appendix B of the style
//! proposal maps them: a present field is an inline declaration of its node, and an absent field
//! declares nothing. Each message has one function, and [`STYLE_FIELDS`] lists the fields that
//! they map, which a test checks against the descriptors of the format.

use avenger_chart::proto::{
    self, cartesian_layer, geo_layer, polar_layer, Side, TextAlign, TextBaseline, TextWidthMode,
    TitleAlign, TitleSide,
};
use avenger_style::{NodeId, Property, StyleTree};

use crate::EvalWarning;

/// The style fields that this module maps, by message: every field of the format that is neither
/// content nor deferred to a later evaluator, which `tests/style_fields.rs` checks.
#[doc(hidden)]
pub const STYLE_FIELDS: &[(&str, &[&str])] = &[
    ("Chart", &["padding"]),
    ("Padding", &["top", "right", "bottom", "left"]),
    ("Plot", &["width", "height"]),
    ("ConcatLayout", &["gap"]),
    ("WrapLayout", &["min_column_width", "max_columns", "gap"]),
    (
        "FacetHeader",
        &[
            "title",
            "side",
            "labels",
            "label_angle",
            "label_padding",
            "tick_length",
            "rule_color",
            "rule_width",
        ],
    ),
    ("Cartesian", &["aspect_ratio"]),
    ("SphereStyle", &["fill", "stroke", "stroke_width"]),
    (
        "Graticule",
        &["label_style", "label_padding", "color", "width"],
    ),
    ("Title", &["side", "align", "style", "subtitle_style"]),
    (
        "TextStyle",
        &["font_family", "font_size", "font_weight", "color"],
    ),
    (
        "Background",
        &["fill", "stroke", "corner_radius", "padding"],
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
        "PolarAxis",
        &[
            "labels",
            "show_labels",
            "label_angle",
            "label_orient",
            "ticks",
            "domain_color",
            "grid",
            "angle",
        ],
    ),
    (
        "Axis",
        &[
            "title",
            "side",
            "labels",
            "show_labels",
            "label_angle",
            "label_align",
            "label_baseline",
            "ticks",
            "domain_color",
            "grid",
            "levels",
        ],
    ),
    ("NestedAxisLevel", &["show_labels", "label_angle"]),
    ("GuideTitle", &["style"]),
    ("GuideLabels", &["style"]),
    ("GuideTicks", &["count", "length", "color"]),
    ("GuideGrid", &["show", "color", "width"]),
    (
        "Legend",
        &[
            "title",
            "labels",
            "side",
            "wrap",
            "label_padding",
            "column_gap",
            "row_gap",
            "background",
            "tick_count",
        ],
    ),
    ("LegendGrid", &["columns"]),
    (
        "Colorbar",
        &["axis", "thickness", "axis_gap", "background", "max_length"],
    ),
    (
        "ColorbarAxis",
        &[
            "title",
            "side",
            "labels",
            "show_labels",
            "label_angle",
            "label_align",
            "label_baseline",
            "ticks",
            "domain_color",
        ],
    ),
    (
        "Attribution",
        &[
            "side",
            "align",
            "separator",
            "separator_space",
            "text_style",
            "line_gap",
            "background",
        ],
    ),
    ("Range", &["values", "palette", "reverse"]),
    ("BandScale", &["padding_inner", "padding_outer"]),
    ("PointScale", &["padding"]),
];

/// The values of a range as CSS text: strings as they are, such as colors and shape names, and
/// numbers as numbers.
fn css_items(values: &arrow::array::ArrayRef) -> Vec<String> {
    use arrow::{
        array::{Array, AsArray},
        datatypes::DataType,
    };
    let values = arrow::compute::cast(values, &DataType::Utf8).expect("a range casts to text");
    let strings = values.as_string::<i32>();
    (0..strings.len())
        .map(|index| strings.value(index).to_string())
        .collect()
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

/// A `TextStyle`, as `font-family`, `font-size`, `font-weight`, and `color` of its node.
pub(crate) fn text_style(tree: &mut Styled, node: NodeId, style: Option<&proto::TextStyle>) {
    let Some(style) = style else { return };
    if !style.font_family.is_empty() {
        set(tree, node, Property::FontFamily, &style.font_family);
    }
    px(tree, node, Property::FontSize, style.font_size);
    if let Some(weight) = style.font_weight {
        set(tree, node, Property::FontWeight, &weight.to_string());
    }
    color(tree, node, Property::Color, &style.color);
}

/// A `Background`, as `background`, `border-color` with a solid `border-style`, `border-radius`,
/// and `padding` of its node.
pub(crate) fn background(tree: &mut Styled, node: NodeId, background: Option<&proto::Background>) {
    let Some(background) = background else { return };
    color(tree, node, Property::Background, &background.fill);
    // A stroke draws a solid border, whose width the styles give.
    if !background.stroke.is_empty() {
        color(tree, node, Property::BorderColor, &background.stroke);
        set(tree, node, Property::BorderStyle, "solid");
    }
    px(tree, node, Property::BorderRadius, background.corner_radius);
    for property in [
        Property::PaddingTop,
        Property::PaddingRight,
        Property::PaddingBottom,
        Property::PaddingLeft,
    ] {
        px(tree, node, property, background.padding);
    }
}

/// `Chart.padding`, as the paddings of the root `chart`.
pub(crate) fn chart(tree: &mut Styled, node: NodeId, chart: &proto::Chart) {
    let Some(padding) = &chart.padding else {
        return;
    };
    px(tree, node, Property::PaddingTop, padding.top);
    px(tree, node, Property::PaddingRight, padding.right);
    px(tree, node, Property::PaddingBottom, padding.bottom);
    px(tree, node, Property::PaddingLeft, padding.left);
}

/// `Plot.width` and `height`, and the options of a Cartesian plot, on its `plot`.
pub(crate) fn plot(tree: &mut Styled, node: NodeId, plot: &proto::Plot) {
    px(tree, node, Property::Width, plot.width);
    px(tree, node, Property::Height, plot.height);
    if let Some(proto::plot::Coord::Cartesian(cartesian)) = &plot.coord {
        if let Some(ratio) = cartesian
            .options
            .as_ref()
            .and_then(|options| options.aspect_ratio)
        {
            set(tree, node, Property::AspectRatio, &format!("{ratio} / 1"));
        }
    }
}

/// `ConcatLayout.gap` on the `plot` of a concat.
pub(crate) fn concat(tree: &mut Styled, node: NodeId, concat: &proto::Concat) {
    if let Some(layout) = &concat.layout {
        px(tree, node, Property::RowGap, layout.gap);
        px(tree, node, Property::ColumnGap, layout.gap);
    }
}

/// `WrapLayout.gap`, `min_column_width`, and `max_columns` on the `plot` of a wrap.
pub(crate) fn wrap(tree: &mut Styled, node: NodeId, wrap: &proto::Wrap) {
    let Some(layout) = &wrap.layout else {
        return;
    };
    px(tree, node, Property::RowGap, layout.gap);
    px(tree, node, Property::ColumnGap, layout.gap);
    px(
        tree,
        node,
        Property::MinColumnWidth,
        layout.min_column_width,
    );
    if let Some(columns) = layout.max_columns {
        set(tree, node, Property::MaxColumns, &columns.to_string());
    }
}

/// The nodes of a facet header and its parts.
#[derive(Clone, Debug)]
pub(crate) struct HeaderNodes {
    pub(crate) header: NodeId,
    pub(crate) title: NodeId,
    pub(crate) label: NodeId,
    pub(crate) tick: NodeId,
    pub(crate) rule: NodeId,
}

/// A `FacetHeader` on its nodes.
pub(crate) fn header(tree: &mut Styled, nodes: &HeaderNodes, header: &proto::FacetHeader) {
    keyword(tree, nodes.header, Property::Orient, side(header.side));
    deg(tree, nodes.header, Property::LabelAngle, header.label_angle);
    px(
        tree,
        nodes.header,
        Property::LabelPadding,
        header.label_padding,
    );
    text_style(
        tree,
        nodes.title,
        header.title.as_ref().and_then(|title| title.style.as_ref()),
    );
    text_style(
        tree,
        nodes.label,
        header
            .labels
            .as_ref()
            .and_then(|labels| labels.style.as_ref()),
    );
    px(tree, nodes.tick, Property::TickSize, header.tick_length);
    for node in [nodes.tick, nodes.rule] {
        color(tree, node, Property::Stroke, &header.rule_color);
        px(tree, node, Property::StrokeWidth, header.rule_width);
    }
}

/// A `Title` on its `title` node, and its subtitle's style on its `subtitle` node.
pub(crate) fn title(
    tree: &mut Styled,
    title_node: NodeId,
    subtitle_node: Option<NodeId>,
    title: &proto::Title,
) {
    let side = match TitleSide::try_from(title.side).unwrap_or_default() {
        TitleSide::Unspecified => None,
        TitleSide::Top => Some("top"),
        TitleSide::Left => Some("left"),
        TitleSide::Right => Some("right"),
        TitleSide::Bottom => Some("bottom"),
    };
    keyword(tree, title_node, Property::Orient, side);
    let align = match TitleAlign::try_from(title.align).unwrap_or_default() {
        TitleAlign::Unspecified => None,
        TitleAlign::Start => Some("start"),
        TitleAlign::Center => Some("center"),
        TitleAlign::End => Some("end"),
    };
    keyword(tree, title_node, Property::TextAlign, align);
    text_style(tree, title_node, title.style.as_ref());
    if let Some(subtitle) = subtitle_node {
        text_style(tree, subtitle, title.subtitle_style.as_ref());
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

/// The style fields of a map mark on its `mark` node.
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

/// A `SphereStyle` on the `plot[type=geo]` of its map: its fill as the `background`, and its
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
    color(tree, nodes.grid, Property::Stroke, &graticule.color);
    px(tree, nodes.grid, Property::StrokeWidth, graticule.width);
    text_style(tree, nodes.label, graticule.label_style.as_ref());
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
            if let Some(proto::polar_text_mark::Turn::Orient(orient)) = text.turn {
                keyword(tree, node, Property::TextOrient, polar_orient(orient));
            }
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
    pub(crate) tick: NodeId,
    pub(crate) rule: NodeId,
    pub(crate) grid: NodeId,
    /// The `level` of each level of a nested band axis, with its `label`.
    pub(crate) levels: Vec<(NodeId, NodeId)>,
}

/// The fields that an `Axis` and a `ColorbarAxis` share, on the axis's node and its parts.
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
    keyword(tree, axis, Property::Orient, side(fields.side));
    deg(tree, axis, Property::LabelAngle, fields.label_angle);
    text_style(
        tree,
        title,
        fields.title.and_then(|title| title.style.as_ref()),
    );
    text_style(
        tree,
        label,
        fields.labels.and_then(|labels| labels.style.as_ref()),
    );
    shown(tree, label, fields.show_labels);
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
    if let Some(ticks) = fields.ticks {
        if let Some(proto::guide_ticks::At::Count(count)) = ticks.at {
            set(tree, axis, Property::TickCount, &count.to_string());
        }
        px(tree, tick, Property::TickSize, ticks.length);
        color(tree, tick, Property::Stroke, &ticks.color);
    }
    color(tree, rule, Property::Stroke, fields.domain_color);
}

/// The fields of an axis that the two kinds of axes share.
struct AxisFields<'a> {
    side: i32,
    title: Option<&'a proto::GuideTitle>,
    labels: Option<&'a proto::GuideLabels>,
    show_labels: Option<bool>,
    label_angle: Option<f32>,
    label_align: i32,
    label_baseline: i32,
    ticks: Option<&'a proto::GuideTicks>,
    domain_color: &'a str,
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
            side: axis.side,
            title: axis.title.as_ref(),
            labels: axis.labels.as_ref(),
            show_labels: axis.show_labels,
            label_angle: axis.label_angle,
            label_align: axis.label_align,
            label_baseline: axis.label_baseline,
            ticks: axis.ticks.as_ref(),
            domain_color: &axis.domain_color,
        },
    );
    if let Some(grid) = &axis.grid {
        shown(tree, nodes.grid, grid.show);
        color(tree, nodes.grid, Property::Stroke, &grid.color);
        px(tree, nodes.grid, Property::StrokeWidth, grid.width);
    }
    for (level, (level_node, label_node)) in axis.levels.iter().zip(&nodes.levels) {
        shown(tree, *label_node, level.show_labels);
        deg(tree, *level_node, Property::LabelAngle, level.label_angle);
    }
}

/// A `PolarAxis` on its nodes, in a plot whose angles start at `start_angle`. The `angle` of the
/// axis runs from the start angle, so its initial 0 is the start angle, as an absent `angle` is.
pub(crate) fn polar_axis(
    tree: &mut Styled,
    nodes: &AxisNodes,
    axis: &proto::PolarAxis,
    start_angle: f32,
) {
    use proto::polar_axis::LabelTurn;
    text_style(
        tree,
        nodes.label,
        axis.labels
            .as_ref()
            .and_then(|labels| labels.style.as_ref()),
    );
    shown(tree, nodes.label, axis.show_labels);
    match axis.label_turn {
        Some(LabelTurn::LabelAngle(angle)) => {
            deg(tree, nodes.axis, Property::LabelAngle, Some(angle));
        }
        Some(LabelTurn::LabelOrient(orient)) => {
            keyword(
                tree,
                nodes.axis,
                Property::LabelOrient,
                polar_orient(orient),
            );
        }
        None => {}
    }
    if let Some(ticks) = &axis.ticks {
        if let Some(proto::guide_ticks::At::Count(count)) = ticks.at {
            set(tree, nodes.axis, Property::TickCount, &count.to_string());
        }
        px(tree, nodes.tick, Property::TickSize, ticks.length);
        color(tree, nodes.tick, Property::Stroke, &ticks.color);
    }
    color(tree, nodes.rule, Property::Stroke, &axis.domain_color);
    if let Some(grid) = &axis.grid {
        shown(tree, nodes.grid, grid.show);
        color(tree, nodes.grid, Property::Stroke, &grid.color);
        px(tree, nodes.grid, Property::StrokeWidth, grid.width);
    }
    let angle = axis.angle.map(|angle| angle - start_angle);
    deg(tree, nodes.axis, Property::Angle, angle);
}

/// The nodes of a symbol legend and its parts.
#[derive(Clone, Debug)]
pub(crate) struct LegendNodes {
    pub(crate) legend: NodeId,
    pub(crate) title: NodeId,
    pub(crate) swatch: NodeId,
    pub(crate) label: NodeId,
}

/// A `Legend` on its nodes.
pub(crate) fn legend(tree: &mut Styled, nodes: &LegendNodes, legend: &proto::Legend) {
    let node = nodes.legend;
    keyword(tree, node, Property::Orient, side(legend.side));
    match legend.wrap.as_ref().and_then(|wrap| wrap.wrap.as_ref()) {
        Some(proto::legend_wrap::Wrap::Flow(_)) => set(tree, node, Property::Wrap, "flow"),
        Some(proto::legend_wrap::Wrap::Grid(grid)) => {
            set(tree, node, Property::Wrap, "grid");
            if let Some(columns) = grid.columns {
                set(tree, node, Property::Columns, &columns.to_string());
            }
        }
        None => {}
    }
    px(tree, node, Property::LabelPadding, legend.label_padding);
    px(tree, node, Property::ColumnGap, legend.column_gap);
    px(tree, node, Property::RowGap, legend.row_gap);
    background(tree, node, legend.background.as_ref());
    if let Some(proto::legend::Entries::TickCount(count)) = legend.entries {
        set(tree, node, Property::TickCount, &count.to_string());
    }
    text_style(
        tree,
        nodes.title,
        legend.title.as_ref().and_then(|title| title.style.as_ref()),
    );
    text_style(
        tree,
        nodes.label,
        legend
            .labels
            .as_ref()
            .and_then(|labels| labels.style.as_ref()),
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

/// A `Colorbar` on its nodes.
pub(crate) fn colorbar(tree: &mut Styled, nodes: &ColorbarNodes, colorbar: &proto::Colorbar) {
    px(tree, nodes.ramp, Property::Thickness, colorbar.thickness);
    px(tree, nodes.ramp, Property::MaxLength, colorbar.max_length);
    px(tree, nodes.legend, Property::AxisGap, colorbar.axis_gap);
    background(tree, nodes.legend, colorbar.background.as_ref());
    if let Some(axis) = &colorbar.axis {
        axis_fields(
            tree,
            nodes.legend,
            nodes.title,
            nodes.label,
            nodes.tick,
            nodes.rule,
            AxisFields {
                side: axis.side,
                title: axis.title.as_ref(),
                labels: axis.labels.as_ref(),
                show_labels: axis.show_labels,
                label_angle: axis.label_angle,
                label_align: axis.label_align,
                label_baseline: axis.label_baseline,
                ticks: axis.ticks.as_ref(),
                domain_color: &axis.domain_color,
            },
        );
    }
}

/// An `Attribution` on its `attribution` node.
pub(crate) fn attribution(tree: &mut Styled, node: NodeId, attribution: &proto::Attribution) {
    keyword(tree, node, Property::Orient, side(attribution.side));
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
    text_style(tree, node, attribution.text_style.as_ref());
    px(tree, node, Property::LineGap, attribution.line_gap);
    background(tree, node, attribution.background.as_ref());
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
    if let Some(range) = &spec.range {
        match &range.range {
            Some(proto::range::Range::Palette(palette)) => {
                let text = match range.reverse {
                    true => format!("{palette} reverse"),
                    false => palette.clone(),
                };
                set(tree, node, Property::Scheme, &text);
            }
            Some(proto::range::Range::Values(values)) => {
                let mut items = css_items(&chart.list_elements(values));
                if range.reverse {
                    items.reverse();
                }
                set(tree, node, Property::Range, &items.join(", "));
            }
            None => {}
        }
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
        _ => {}
    }
}
