//! A tooltip's box, with its header, rows, and swatch, placed beside its anchor, and the outline
//! of its instance.

use avenger_color::ColorOrGradient;
use avenger_common::types::{FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_geometry::geo_types::{Geometry, LineString};
use avenger_geometry::query::Extent;
use avenger_geometry::rtree::GeometryInstance;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::{MarkInstance, SceneMark};
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{escape_text, LabelEngine, TextBounds};
use lyon_path::geom::point;
use lyon_path::Path;

use crate::content::Text;
use crate::error::AvengerTooltipError;

/// How a tooltip looks.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipStyle {
    pub font_family: String,
    pub font_size: f32,
    /// The color of the header and the values.
    pub color: [f32; 4],
    /// The color of the rows' titles.
    pub title_color: [f32; 4],
    pub header_font_weight: FontWeight,
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub stroke_width: f32,
    pub corner_radius: f32,
    /// The space between the box's edge and its text.
    pub padding: f32,
    /// The space between the box's lines.
    pub row_gap: f32,
    /// The space between a row's title and its value, and between the swatch and the header.
    pub column_gap: f32,
    /// The side of the swatch beside the header.
    pub swatch_size: f32,
    /// How far a data-anchored box's callout reaches from the box to the instance's edge.
    pub callout: f32,
    /// How wide the callout is where it meets the box.
    pub callout_width: f32,
    /// How far a pointer-anchored box stands from the pointer, across and down.
    pub pointer_offset: [f32; 2],
    /// The color of the outline around the instance.
    pub highlight: [f32; 4],
    pub highlight_width: f32,
}

impl Default for TooltipStyle {
    fn default() -> Self {
        Self {
            font_family: "sans-serif".to_string(),
            font_size: 12.0,
            color: [0.12, 0.12, 0.12, 1.0],
            title_color: [0.4, 0.4, 0.4, 1.0],
            header_font_weight: FontWeight::BOLD,
            fill: [1.0, 1.0, 1.0, 1.0],
            stroke: [0.7, 0.7, 0.7, 1.0],
            stroke_width: 1.0,
            corner_radius: 4.0,
            padding: 7.0,
            row_gap: 3.0,
            column_gap: 8.0,
            swatch_size: 10.0,
            callout: 7.0,
            callout_width: 12.0,
            pointer_offset: [10.0, 10.0],
            highlight: [0.1, 0.1, 0.12, 0.9],
            highlight_width: 1.5,
        }
    }
}

/// What a tooltip shows: its header, beside a swatch of its instance's color, and its rows'
/// titles and values.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Content {
    pub(crate) header: Option<Text>,
    pub(crate) swatch: Option<[f32; 4]>,
    pub(crate) rows: Vec<(String, Text)>,
}

/// Where a tooltip's box stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Placement {
    /// Beside an instance's drawn box, `[left, top, right, bottom]`, with a callout to its edge.
    Beside([f32; 4]),
    /// Below and to the right of the pointer.
    AtPointer([f32; 2]),
}

/// A side of an anchor that a tooltip's box stands on: a box on the anchor's right has its
/// callout on its left edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Right,
    Left,
    Above,
    Below,
}

/// The sides that a data-anchored box tries, in order.
const SIDES: [Side; 4] = [Side::Right, Side::Left, Side::Above, Side::Below];

/// The tooltip's box with `content`, placed by `placement` inside `bounds`, `[x, y, width,
/// height]`, as a group at the box's top left corner.
pub(crate) fn tooltip(
    content: &Content,
    placement: Placement,
    bounds: [f32; 4],
    style: &TooltipStyle,
    engine: &LabelEngine,
) -> Result<SceneGroup, AvengerTooltipError> {
    let (size, text) = lay_out(content, style, engine)?;
    let (origin, callout) = match placement {
        Placement::Beside(envelope) => {
            let (side, origin, tip) = beside(envelope, size, bounds, style.callout);
            (
                origin,
                Some((side, [tip[0] - origin[0], tip[1] - origin[1]])),
            )
        }
        Placement::AtPointer(pointer) => (
            at_pointer(pointer, size, bounds, style.pointer_offset),
            None,
        ),
    };
    let outline = ScenePathMark {
        name: "tooltip_box".to_string(),
        path: outline(size, style, callout).into(),
        fill: ColorOrGradient::Color(style.fill).into(),
        stroke: ColorOrGradient::Color(style.stroke).into(),
        stroke_width: Some(style.stroke_width),
        ..Default::default()
    };
    let mut marks = vec![outline.into()];
    marks.extend(text);
    Ok(SceneGroup {
        name: "tooltip".to_string(),
        origin,
        marks,
        ..Default::default()
    })
}

/// The box's size, and its swatch and text with the box's top left corner at 0: the header, then
/// each row's title on the left and its value on the right.
fn lay_out(
    content: &Content,
    style: &TooltipStyle,
    engine: &LabelEngine,
) -> Result<([f32; 2], Vec<SceneMark>), AvengerTooltipError> {
    let pad = style.padding;
    let text =
        |name: &str, lines: Vec<String>, syntax, color: [f32; 4], weight, align| SceneTextMark {
            name: name.to_string(),
            len: lines.len() as u32,
            text: lines.into(),
            text_syntax: syntax,
            align: ScalarOrArray::new_scalar(align),
            baseline: TextBaseline::Top.into(),
            color: color.into(),
            font: style.font_family.clone().into(),
            font_size: style.font_size.into(),
            font_weight: ScalarOrArray::new_scalar(weight),
            ..Default::default()
        };
    let header = (content.header.as_ref()).map(|header| {
        let lines = vec![header.text.clone()];
        let weight = style.header_font_weight;
        text(
            "tooltip_header",
            lines,
            header.syntax,
            style.color,
            weight,
            TextAlign::Left,
        )
    });
    let header_bounds = match &header {
        Some(header) => Some(bounds(header, engine)?[0]),
        None => None,
    };
    let titles: Vec<String> = content
        .rows
        .iter()
        .map(|(title, _)| title.clone())
        .collect();
    let (values, values_syntax) = column(content.rows.iter().map(|(_, value)| value));
    let normal = FontWeight::NORMAL;
    let plain = TextSyntaxMode::Plain;
    let mut titles = text(
        "tooltip_titles",
        titles,
        plain,
        style.title_color,
        normal,
        TextAlign::Left,
    );
    let mut values = text(
        "tooltip_values",
        values,
        values_syntax,
        style.color,
        normal,
        TextAlign::Right,
    );
    let (title_bounds, value_bounds) = (bounds(&titles, engine)?, bounds(&values, engine)?);

    let widest = |bounds: &[TextBounds]| bounds.iter().map(|b| b.width).fold(0.0, f32::max);
    let swatch = content.swatch.filter(|_| header.is_some());
    let header_x = pad + swatch.map_or(0.0, |_| style.swatch_size + style.column_gap);
    let header_width = header_bounds.map_or(0.0, |bounds| header_x - pad + bounds.width);
    let rows_width = if content.rows.is_empty() {
        0.0
    } else {
        widest(&title_bounds) + style.column_gap + widest(&value_bounds)
    };
    let width = 2.0 * pad + header_width.max(rows_width);

    let mut marks: Vec<SceneMark> = Vec::new();
    let mut y = pad;
    let mut lines = 0;
    if let (Some(mut header), Some(bounds)) = (header, header_bounds) {
        if let Some(color) = swatch {
            let swatch = SceneRectMark {
                name: "tooltip_swatch".to_string(),
                x: pad.into(),
                y: (y + (bounds.height - style.swatch_size) / 2.0).into(),
                width: Some(style.swatch_size.into()),
                height: Some(style.swatch_size.into()),
                fill: ColorOrGradient::Color(color).into(),
                corner_radius: 2.0.into(),
                ..Default::default()
            };
            marks.push(swatch.into());
        }
        header.x = header_x.into();
        header.y = y.into();
        marks.push(header.into());
        y += bounds.height + style.row_gap;
        lines += 1;
    }
    let mut row_y = Vec::with_capacity(content.rows.len());
    for (title, value) in title_bounds.iter().zip(&value_bounds) {
        row_y.push(y);
        y += title.height.max(value.height) + style.row_gap;
        lines += 1;
    }
    if !row_y.is_empty() {
        titles.x = pad.into();
        titles.y = row_y.clone().into();
        values.x = (width - pad).into();
        values.y = row_y.into();
        marks.push(titles.into());
        marks.push(values.into());
    }
    let height = if lines == 0 {
        2.0 * pad
    } else {
        y - style.row_gap + pad
    };
    Ok(([width, height], marks))
}

/// Each line's text, and how the column reads: as markup when any line is markup, with the plain
/// lines escaped, and otherwise as plain text.
fn column<'a>(lines: impl Iterator<Item = &'a Text> + Clone) -> (Vec<String>, TextSyntaxMode) {
    let markup = lines
        .clone()
        .any(|line| line.syntax == TextSyntaxMode::TypstMarkup);
    let text = lines
        .map(|line| match line.syntax {
            TextSyntaxMode::Plain if markup => escape_text(&line.text),
            _ => line.text.clone(),
        })
        .collect();
    let syntax = if markup {
        TextSyntaxMode::TypstMarkup
    } else {
        TextSyntaxMode::Plain
    };
    (text, syntax)
}

/// The box that each of `mark`'s labels takes.
fn bounds(
    mark: &SceneTextMark,
    engine: &LabelEngine,
) -> Result<Vec<TextBounds>, AvengerTooltipError> {
    Ok(mark
        .labels()
        .map(|label| engine.bounds(&label.label))
        .collect::<Result<_, _>>()?)
}

/// The side of `envelope`, `[left, top, right, bottom]`, that a box of `size` stands on, `callout`
/// away: the first of [`SIDES`] where it fits inside `bounds`, or else the side where the least of
/// it leaves them. Returns the side, the box's top left corner, moved inside the bounds, and the
/// callout's tip, at the middle of the envelope's edge on that side.
fn beside(
    envelope: [f32; 4],
    size: [f32; 2],
    bounds: [f32; 4],
    callout: f32,
) -> (Side, [f32; 2], [f32; 2]) {
    let [left, top, right, bottom] = envelope;
    let [cx, cy] = [(left + right) / 2.0, (top + bottom) / 2.0];
    let [width, height] = size;
    let tip = |side| match side {
        Side::Right => [right, cy],
        Side::Left => [left, cy],
        Side::Above => [cx, top],
        Side::Below => [cx, bottom],
    };
    let origin = |side| {
        let [x, y] = tip(side);
        match side {
            Side::Right => [x + callout, y - height / 2.0],
            Side::Left => [x - callout - width, y - height / 2.0],
            Side::Above => [x - width / 2.0, y - callout - height],
            Side::Below => [x - width / 2.0, y + callout],
        }
    };
    let overflow = |side| overflow(origin(side), size, bounds);
    let side = (SIDES.into_iter())
        .min_by(|a, b| overflow(*a).total_cmp(&overflow(*b)))
        .unwrap_or(Side::Right);
    (side, inside(origin(side), size, bounds), tip(side))
}

/// The top left corner of a box of `size` below and right of `pointer`, `offset` away, or on the
/// pointer's other side across or down where the box would leave `bounds`, moved inside them.
fn at_pointer(pointer: [f32; 2], size: [f32; 2], bounds: [f32; 4], offset: [f32; 2]) -> [f32; 2] {
    let [x, y] = pointer;
    let [width, height] = size;
    let [left, top, bounds_width, bounds_height] = bounds;
    let x = if x + offset[0] + width <= left + bounds_width {
        x + offset[0]
    } else {
        x - offset[0] - width
    };
    let y = if y + offset[1] + height <= top + bounds_height {
        y + offset[1]
    } else {
        y - offset[1] - height
    };
    inside([x, y], size, bounds)
}

/// How far a box of `size` at `origin` reaches outside `bounds`, summed over its edges.
fn overflow(origin: [f32; 2], size: [f32; 2], bounds: [f32; 4]) -> f32 {
    let [left, top, width, height] = bounds;
    let past = |low: f32, high: f32, start: f32, length: f32| {
        (start - low).min(0.0).abs() + (start + length - high).max(0.0)
    };
    past(left, left + width, origin[0], size[0]) + past(top, top + height, origin[1], size[1])
}

/// `origin`, moved so that a box of `size` there lies inside `bounds`, or at their top left when
/// the box is larger than they are.
fn inside(origin: [f32; 2], size: [f32; 2], bounds: [f32; 4]) -> [f32; 2] {
    let [left, top, width, height] = bounds;
    let within =
        |start: f32, low: f32, room: f32, length: f32| start.min(low + room - length).max(low);
    [
        within(origin[0], left, width, size[0]),
        within(origin[1], top, height, size[1]),
    ]
}

/// The box's rounded outline, with a callout from the edge facing `callout`'s side to its tip, in
/// the box's frame.
fn outline(size: [f32; 2], style: &TooltipStyle, callout: Option<(Side, [f32; 2])>) -> Path {
    let side = callout.map(|(side, _)| side);
    let tip = callout.map_or([0.0; 2], |(_, tip)| tip);
    let [width, height] = size;
    let radius = style.corner_radius.min(width / 2.0).min(height / 2.0);
    let half = (style.callout_width / 2.0)
        .min((width - 2.0 * radius) / 2.0)
        .min((height - 2.0 * radius) / 2.0)
        .max(0.0);
    // The callout's center along an edge of `length`, at the tip, clear of the corners.
    let center = |at: f32, length: f32| at.min(length - radius - half).max(radius + half);
    // Each corner is a cubic Bézier curve, whose control points lie this far along its tangents.
    let k = radius * 0.552_284_8;
    let mut path = Path::builder();
    path.begin(point(radius, 0.0));
    // The callout faces the anchor: the box's top edge for a box below the anchor.
    if side == Some(Side::Below) {
        let c = center(tip[0], width);
        path.line_to(point(c - half, 0.0));
        path.line_to(point(tip[0], tip[1]));
        path.line_to(point(c + half, 0.0));
    }
    path.line_to(point(width - radius, 0.0));
    path.cubic_bezier_to(
        point(width - radius + k, 0.0),
        point(width, radius - k),
        point(width, radius),
    );
    if side == Some(Side::Left) {
        let c = center(tip[1], height);
        path.line_to(point(width, c - half));
        path.line_to(point(tip[0], tip[1]));
        path.line_to(point(width, c + half));
    }
    path.line_to(point(width, height - radius));
    path.cubic_bezier_to(
        point(width, height - radius + k),
        point(width - radius + k, height),
        point(width - radius, height),
    );
    if side == Some(Side::Above) {
        let c = center(tip[0], width);
        path.line_to(point(c + half, height));
        path.line_to(point(tip[0], tip[1]));
        path.line_to(point(c - half, height));
    }
    path.line_to(point(radius, height));
    path.cubic_bezier_to(
        point(radius - k, height),
        point(0.0, height - radius + k),
        point(0.0, height - radius),
    );
    if side == Some(Side::Right) {
        let c = center(tip[1], height);
        path.line_to(point(0.0, c + half));
        path.line_to(point(tip[0], tip[1]));
        path.line_to(point(0.0, c - half));
    }
    path.line_to(point(0.0, radius));
    path.cubic_bezier_to(
        point(0.0, radius - k),
        point(radius - k, 0.0),
        point(radius, 0.0),
    );
    path.end(true);
    path.build()
}

/// The outline of `instance`'s fill, in a group with its mark's `clip`, or `None` for an instance
/// without a fill, such as a line, or one that its clip shows none of.
pub(crate) fn highlight(
    instance: &GeometryInstance,
    clip: &Clip,
    style: &TooltipStyle,
) -> Option<SceneGroup> {
    let shape = instance.shape(Extent::Fill, clip)?;
    let mut rings = Vec::new();
    rings_of(&shape, &mut rings);
    if rings.is_empty() {
        return None;
    }
    let mut path = Path::builder();
    for ring in rings {
        let mut points = ring.coords().map(|c| point(c.x, c.y));
        let Some(first) = points.next() else {
            continue;
        };
        path.begin(first);
        for next in points {
            path.line_to(next);
        }
        path.end(true);
    }
    let outline = ScenePathMark {
        name: "tooltip_highlight".to_string(),
        path: path.build().into(),
        fill: ColorOrGradient::Color([0.0; 4]).into(),
        stroke: ColorOrGradient::Color(style.highlight).into(),
        stroke_width: Some(style.highlight_width),
        ..Default::default()
    };
    Some(SceneGroup {
        name: "tooltip_highlight".to_string(),
        clip: clip.clone(),
        marks: vec![outline.into()],
        ..Default::default()
    })
}

/// Push the rings of `geometry`'s areas onto `rings`: each polygon's exterior and holes.
fn rings_of(geometry: &Geometry<f32>, rings: &mut Vec<LineString<f32>>) {
    match geometry {
        Geometry::Polygon(polygon) => {
            rings.push(polygon.exterior().clone());
            rings.extend(polygon.interiors().iter().cloned());
        }
        Geometry::MultiPolygon(polygons) => {
            for polygon in polygons {
                rings_of(&Geometry::Polygon(polygon.clone()), rings);
            }
        }
        Geometry::Rect(rect) => rings_of(&Geometry::Polygon(rect.to_polygon()), rings),
        Geometry::Triangle(triangle) => rings_of(&Geometry::Polygon(triangle.to_polygon()), rings),
        Geometry::GeometryCollection(collection) => {
            for geometry in collection {
                rings_of(geometry, rings);
            }
        }
        _ => {}
    }
}

/// The color of `mark`'s instance in `scene`: its fill, or its stroke where the fill is clear, or
/// `None` for a gradient, or a mark without either.
pub(crate) fn instance_color(mark: &MarkInstance, scene: &SceneGraph) -> Option<[f32; 4]> {
    let index = mark.instance_index.unwrap_or(0);
    let color = |paint: Option<&ColorOrGradient>| match paint? {
        ColorOrGradient::Color(color) if color[3] > 0.0 => Some(*color),
        _ => None,
    };
    let paint = |fill, stroke| color(fill).or_else(|| color(stroke));
    match scene.get_mark(&mark.mark_path)? {
        SceneMark::Symbol(mark) => paint(at(&mark.fill, index), at(&mark.stroke, index)),
        SceneMark::Rect(mark) => paint(at(&mark.fill, index), at(&mark.stroke, index)),
        SceneMark::Path(mark) => paint(at(&mark.fill, index), at(&mark.stroke, index)),
        SceneMark::Arc(mark) => paint(at(&mark.fill, index), at(&mark.stroke, index)),
        SceneMark::Rule(mark) => color(at(&mark.stroke, index)),
        _ => None,
    }
}

/// The value at `index` of a mark's per-instance values, or `None` past their end.
fn at<T: Sync + Clone>(values: &ScalarOrArray<T>, index: usize) -> Option<&T> {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => Some(value),
        ScalarOrArrayValue::Array(values) => values.get(index),
    }
}
