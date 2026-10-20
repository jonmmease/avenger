//! A tooltip's marks: a box of rows that describes the data under the pointer, beside the data
//! with a callout whose tip touches the data's drawn edge, or beside the pointer. A group focus
//! also draws a crosshair, with a marker on each of its members.

use arrow_array::Array;
use avenger_color::contrast::contrast_ratio;
use avenger_color::{
    mix_colors, AbsoluteColor, ColorOrGradient, ColorSpace, HueInterpolationMethod,
};
use avenger_common::types::StrokeJoin;
use avenger_common::types::{FontWeight, SymbolShape, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_format::TickSpacing;
use avenger_geometry::data::Datum;
use avenger_geometry::focus::{Focus, Member, Side};
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
    path::ScenePathMark,
    rule::SceneRuleMark,
    symbol::SceneSymbolMark,
    text::SceneTextMark,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{escape_text, LabelEngine, LabelValue, LabelWidth, TextBounds};
use geo::{Coord, Geometry, LineString};
use lyon_path::{geom::point, Path};

use crate::error::AvengerTooltipError;

/// How a tooltip draws.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipStyle {
    pub font: String,
    pub font_size: f32,
    /// The color of the header and the values.
    pub color: [f32; 4],
    /// The color of the rows' labels.
    pub label_color: [f32; 4],
    pub header_weight: FontWeight,
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub stroke_width: f32,
    /// How far a tooltip's tint moves its fill from `fill` toward the tint's color, from 0, the
    /// fill, to 1, the color.
    pub tint_amount: f32,
    pub corner_radius: f32,
    /// Space between the box's edges and its content.
    pub padding: f32,
    /// Space between the header and the rows, and between rows.
    pub row_gap: f32,
    /// Space between a row's swatch, label, and value.
    pub column_gap: f32,
    /// The width and height of a row's swatch.
    pub swatch_size: f32,
    /// The callout's length, from the box to its tip.
    pub callout: f32,
    /// The callout's width where it meets the box.
    pub callout_width: f32,
    /// How far right of and below the pointer a tooltip beside it stands, as Vega's tooltips do,
    /// so the pointer's arrow stays in view.
    pub pointer_offset: [f32; 2],
    /// The widest that the header, a label, or a value draws. Longer text wraps, and text past
    /// `max_lines` lines ends in "…".
    pub max_width: f32,
    pub max_lines: usize,
    /// The sides of the anchor where the box may lie, in order of preference.
    pub sides: Vec<Side>,
    /// The color of a group focus's crosshair.
    pub crosshair: [f32; 4],
    /// The width and height of the marker at each member of a group focus.
    pub marker_size: f32,
}

impl Default for TooltipStyle {
    fn default() -> Self {
        Self {
            font: "sans-serif".to_string(),
            font_size: 12.0,
            color: [0.12, 0.12, 0.12, 1.0],
            label_color: [0.4, 0.4, 0.4, 1.0],
            header_weight: FontWeight::BOLD,
            fill: [1.0, 1.0, 1.0, 1.0],
            stroke: [0.7, 0.7, 0.7, 1.0],
            stroke_width: 1.0,
            tint_amount: 0.25,
            corner_radius: 4.0,
            padding: 7.0,
            row_gap: 3.0,
            column_gap: 8.0,
            swatch_size: 10.0,
            callout: 7.0,
            callout_width: 12.0,
            pointer_offset: [10.0, 10.0],
            max_width: 240.0,
            max_lines: 3,
            sides: vec![Side::Right, Side::Left, Side::Above, Side::Below],
            crosshair: [0.45, 0.45, 0.45, 1.0],
            marker_size: 8.0,
        }
    }
}

/// One row of a tooltip: a swatch, a label, and a value.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipRow {
    pub swatch: Option<Swatch>,
    pub label: String,
    pub value: String,
    /// How the value reads: as plain text or as Typst markup.
    pub value_syntax: TextSyntaxMode,
}

impl TooltipRow {
    /// A row of a label and a plain text value, without a swatch.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            swatch: None,
            label: label.into(),
            value: value.into(),
            value_syntax: TextSyntaxMode::Plain,
        }
    }
}

/// A row's swatch: one instance of a mark, in the style of the instance that the row
/// describes, which the tooltip sizes and places.
#[derive(Debug, Clone, PartialEq)]
pub enum Swatch {
    /// A symbol, such as a scatter point's, at the swatch's center.
    Symbol(SceneSymbolMark),
    /// A rule across the swatch, in a line's stroke.
    Line(SceneRuleMark),
}

impl Swatch {
    /// The swatch of `member`'s instance in `scene`: a symbol's shape and paint, or a line's
    /// stroke. Other marks, and a member whose mark isn't in the scene, have none.
    pub fn of(member: &Member, scene: &SceneGraph) -> Option<Self> {
        let instance = member.mark();
        let index = instance.instance_index.unwrap_or(0);
        match scene.get_mark(&instance.mark_path)? {
            SceneMark::Symbol(mark) => Some(Swatch::Symbol(SceneSymbolMark {
                name: "tooltip_swatch".to_string(),
                len: 1,
                gradients: mark.gradients.clone(),
                shapes: vec![mark.shapes.get(*at(&mark.shape_index, index)?)?.clone()],
                stroke_width: mark.stroke_width,
                fill: at(&mark.fill, index)?.clone().into(),
                stroke: at(&mark.stroke, index)?.clone().into(),
                angle: (*at(&mark.angle, index)?).into(),
                clip: false,
                ..Default::default()
            })),
            SceneMark::Line(mark) => Some(Swatch::Line(SceneRuleMark {
                name: "tooltip_swatch".to_string(),
                len: 1,
                gradients: mark.gradients.clone(),
                stroke: mark.stroke.clone().into(),
                stroke_width: mark.stroke_width.into(),
                stroke_dash: mark.stroke_dash.clone().map(Into::into),
                clip: false,
                ..Default::default()
            })),
            _ => None,
        }
    }

    /// The swatch's paint: a symbol's fill, or a line's stroke.
    pub fn paint(&self) -> Option<&ColorOrGradient> {
        match self {
            Swatch::Symbol(mark) => mark.fill.first(),
            Swatch::Line(mark) => mark.stroke.first(),
        }
    }

    /// The swatch's mark, filling the square of `size` with its center at `center`.
    fn placed(&self, center: [f32; 2], size: f32) -> SceneMark {
        match self.clone() {
            Swatch::Symbol(mark) => SceneSymbolMark {
                x: center[0].into(),
                y: center[1].into(),
                size: (size * size).into(),
                ..mark
            }
            .into(),
            Swatch::Line(mark) => SceneRuleMark {
                x: (center[0] - size / 2.0).into(),
                x2: (center[0] + size / 2.0).into(),
                y: center[1].into(),
                y2: center[1].into(),
                ..mark
            }
            .into(),
        }
    }
}

/// The color of `member`'s instance in `scene`: its fill, or its stroke when it has no fill, as
/// for an open symbol, or a line's, a rule's, or a trail's stroke. Alpha is the paint's. `None`
/// for a gradient, a mark without paint, and a member whose mark isn't in the scene.
pub fn instance_color(member: &Member, scene: &SceneGraph) -> Option<[f32; 4]> {
    let instance = member.mark();
    let index = instance.instance_index.unwrap_or(0);
    let color = |paint: Option<&ColorOrGradient>| match paint? {
        ColorOrGradient::Color(color) if color[3] > 0.0 => Some(*color),
        _ => None,
    };
    let paint = |fill, stroke| color(fill).or_else(|| color(stroke));
    match scene.get_mark(&instance.mark_path)? {
        SceneMark::Symbol(mark) => paint(at(&mark.fill, index), at(&mark.stroke, index)),
        SceneMark::Rect(mark) => paint(at(&mark.fill, index), at(&mark.stroke, index)),
        SceneMark::Path(mark) => paint(at(&mark.fill, index), at(&mark.stroke, index)),
        SceneMark::Arc(mark) => paint(at(&mark.fill, index), at(&mark.stroke, index)),
        SceneMark::Area(mark) => paint(Some(&mark.fill), Some(&mark.stroke)),
        SceneMark::Line(mark) => color(Some(&mark.stroke)),
        SceneMark::Trail(mark) => color(Some(&mark.stroke)),
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

/// Where a tooltip's box lies beside its anchor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The side of the anchor that the box lies on.
    pub side: Side,
    /// The box's top left corner.
    pub origin: [f32; 2],
    /// The callout's tip: the anchor on the box's side.
    pub tip: [f32; 2],
}

/// Place a box of `size` `gap` from an anchor, on the first of `sides` where it fits inside
/// `bounds`, `[x, y, width, height]`, or on the first side when none fits. `anchor` gives the
/// anchor on each side. The box centers on the anchor along its side, and moves along the side
/// to stay inside the bounds, while the tip stays on the anchor.
pub fn place(
    anchor: impl Fn(Side) -> [f32; 2],
    size: [f32; 2],
    bounds: [f32; 4],
    sides: &[Side],
    gap: f32,
) -> Placement {
    let [left, top, width, height] = bounds;
    let (right, bottom) = (left + width, top + height);
    let at_side = |side: Side| {
        let tip = anchor(side);
        let [x, y] = match side {
            Side::Right => [tip[0] + gap, tip[1] - size[1] / 2.0],
            Side::Left => [tip[0] - gap - size[0], tip[1] - size[1] / 2.0],
            Side::Above => [tip[0] - size[0] / 2.0, tip[1] - gap - size[1]],
            Side::Below => [tip[0] - size[0] / 2.0, tip[1] + gap],
        };
        let fits = match side {
            Side::Right => x + size[0] <= right,
            Side::Left => x >= left,
            Side::Above => y >= top,
            Side::Below => y + size[1] <= bottom,
        };
        // Along the side, the box moves to stay inside the bounds, and from the anchor it
        // stays inside them only as far as the side leaves room.
        let clamp = |value: f32, low: f32, high: f32| value.min(high).max(low);
        let origin = [
            clamp(x, left, right - size[0]),
            clamp(y, top, bottom - size[1]),
        ];
        (
            fits,
            Placement {
                side,
                origin: [origin[0].round(), origin[1].round()],
                tip,
            },
        )
    };
    let first = sides.first().copied().unwrap_or(Side::Right);
    sides
        .iter()
        .map(|side| at_side(*side))
        .find(|(fits, _)| *fits)
        .unwrap_or_else(|| at_side(first))
        .1
}

/// A column of a focus's rows that a tooltip shows: its label, and the pattern that the text
/// engine formats its values with.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipField {
    pub column: String,
    /// The field's label, or `None` for the column's name.
    pub label: Option<String>,
    /// The pattern in the text engine's provider's syntax, such as `".2f"` for numbers, or ""
    /// for the provider's default.
    pub format: String,
}

impl TooltipField {
    pub fn new(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            label: None,
            format: String::new(),
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = format.into();
        self
    }

    fn label(&self) -> String {
        self.label.clone().unwrap_or_else(|| self.column.clone())
    }
}

impl From<&str> for TooltipField {
    fn from(column: &str) -> Self {
        Self::new(column)
    }
}

/// What a tooltip shows of its focus's rows: a header field, and the fields of its rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TooltipContent {
    /// The field whose value heads the tooltip, after its label when it has one, or `None` for
    /// no header.
    pub header: Option<TooltipField>,
    /// The fields of the rows, in order, or `None` for every column but the header's, labeled
    /// by name, in the provider's default format.
    pub fields: Option<Vec<TooltipField>>,
}

impl TooltipContent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_header(mut self, field: impl Into<TooltipField>) -> Self {
        self.header = Some(field.into());
        self
    }

    pub fn with_fields(
        mut self,
        fields: impl IntoIterator<Item = impl Into<TooltipField>>,
    ) -> Self {
        self.fields = Some(fields.into_iter().map(Into::into).collect());
        self
    }

    /// The fields of `datum`'s rows: the content's, or every column but the header's.
    fn fields_of(&self, datum: &Datum) -> Vec<TooltipField> {
        if let Some(fields) = &self.fields {
            return fields.clone();
        }
        let header = self.header.as_ref().map(|field| field.column.as_str());
        datum
            .rows
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .filter(|name| Some(*name) != header)
            .map(TooltipField::new)
            .collect()
    }

    /// The header of `datum`'s row, and how it reads.
    fn header_of(
        &self,
        datum: &Datum,
        engine: &LabelEngine,
    ) -> Result<(Option<String>, TextSyntaxMode), AvengerTooltipError> {
        let Some(field) = &self.header else {
            return Ok((None, TextSyntaxMode::Plain));
        };
        let (value, syntax) = field_value(datum, field, engine)?;
        let header = match &field.label {
            Some(label) if syntax == TextSyntaxMode::TypstMarkup => {
                format!("{} {value}", escape_text(label))
            }
            Some(label) => format!("{label} {value}"),
            None => value,
        };
        Ok((Some(header), syntax))
    }
}

/// The value of `datum`'s row in `field`'s column, as the text engine formats it with the
/// field's pattern, and how it reads. A null is empty text.
fn field_value(
    datum: &Datum,
    field: &TooltipField,
    engine: &LabelEngine,
) -> Result<(String, TextSyntaxMode), AvengerTooltipError> {
    let column = datum
        .rows
        .column_by_name(&field.column)
        .ok_or_else(|| AvengerTooltipError::Column(field.column.clone()))?;
    let value = column.slice(datum.row, 1);
    if value.is_null(0) {
        return Ok((String::new(), TextSyntaxMode::Plain));
    }
    let formatted = engine.format_array(&value, &field.format, TickSpacing::Varying)?;
    Ok(match formatted.into_iter().next() {
        Some(LabelValue::Markup(markup)) => (markup, TextSyntaxMode::TypstMarkup),
        Some(LabelValue::Str(text)) => (text, TextSyntaxMode::Plain),
        _ => (String::new(), TextSyntaxMode::Plain),
    })
}

/// The top left corner of a box of `size` that stands `offset` right of and below `pointer`, as
/// Vega's tooltips stand, so the pointer's arrow stays in view. Along each axis, a box that would
/// leave `bounds`, `[x, y, width, height]`, stands on the pointer's other side, and a box that
/// fits on neither side moves to stay inside the bounds.
pub fn place_beside_pointer(
    pointer: [f32; 2],
    size: [f32; 2],
    bounds: [f32; 4],
    offset: [f32; 2],
) -> [f32; 2] {
    let axis = |axis: usize| {
        let (low, high) = (bounds[axis], bounds[axis] + bounds[axis + 2]);
        let after = pointer[axis] + offset[axis];
        let start = if after + size[axis] <= high {
            after
        } else {
            pointer[axis] - offset[axis] - size[axis]
        };
        start.min(high - size[axis]).max(low).round()
    };
    [axis(0), axis(1)]
}

/// A tooltip: a header, and rows of a swatch, a label, and a value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tooltip {
    pub header: Option<String>,
    /// How the header reads: as plain text or as Typst markup.
    pub header_syntax: TextSyntaxMode,
    pub rows: Vec<TooltipRow>,
    pub style: TooltipStyle,
    /// A color that tints the box, such as its data's [`instance_color`], or `None` for the
    /// style's colors. A tinted box's fill mixes from the style's fill toward the color in
    /// Oklab by the style's `tint_amount`, and its outline is the color. Its text takes the
    /// style's color or white, whichever contrasts more with the fill, and its labels take the
    /// text's color a third of the way to the fill.
    pub tint: Option<[f32; 4]>,
}

/// A tooltip's colors: its box's fill and outline, its text's, and its labels'.
struct Paint {
    fill: [f32; 4],
    stroke: [f32; 4],
    text: [f32; 4],
    label: [f32; 4],
}

impl Tooltip {
    /// The tooltip's colors, tinted when it has a tint.
    fn paint(&self) -> Paint {
        let style = &self.style;
        let Some([r, g, b, _]) = self.tint else {
            return Paint {
                fill: style.fill,
                stroke: style.stroke,
                text: style.color,
                label: style.label_color,
            };
        };
        let mix = |from: [f32; 4], to: [f32; 4], amount: f32| {
            let [r, g, b, _] = mix_colors(
                ColorSpace::Oklab,
                &AbsoluteColor::from_rgba(from),
                1.0 - amount,
                &AbsoluteColor::from_rgba(to),
                amount,
                HueInterpolationMethod::Shorter,
            )
            .to_rgba();
            [r, g, b, from[3]]
        };
        let tint = [r, g, b, 1.0];
        let fill = mix(style.fill, tint, style.tint_amount.clamp(0.0, 1.0));
        let contrast = |color: [f32; 4]| {
            contrast_ratio(
                &AbsoluteColor::from_rgba(color),
                &AbsoluteColor::from_rgba(fill),
            )
        };
        let white = [1.0, 1.0, 1.0, 1.0];
        let text = if contrast(style.color) >= contrast(white) {
            style.color
        } else {
            white
        };
        Paint {
            fill,
            stroke: tint,
            text,
            label: mix(text, fill, 1.0 / 3.0),
        }
    }

    /// The tooltip of one member: `content`'s header and a row for each of its fields, from
    /// the member's row, with the default style. A member without a row has an empty tooltip.
    pub fn of_member(
        member: &Member,
        content: &TooltipContent,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerTooltipError> {
        let Some(datum) = &member.datum else {
            return Ok(Self::default());
        };
        let (header, header_syntax) = content.header_of(datum, engine)?;
        let rows = content
            .fields_of(datum)
            .iter()
            .map(|field| {
                let (value, value_syntax) = field_value(datum, field, engine)?;
                Ok(TooltipRow {
                    swatch: None,
                    label: field.label(),
                    value,
                    value_syntax,
                })
            })
            .collect::<Result<_, AvengerTooltipError>>()?;
        Ok(Self {
            header,
            header_syntax,
            rows,
            ..Default::default()
        })
    }

    /// The tooltip of a group focus, such as series at one x: `content`'s header from the
    /// first member's row, and a row for each member, labeled by its mark's name, with its
    /// swatch and its row's value in `content`'s first field, or in its first column but the
    /// header's.
    pub fn of_group(
        focus: &Focus,
        content: &TooltipContent,
        scene: &SceneGraph,
        engine: &LabelEngine,
    ) -> Result<Self, AvengerTooltipError> {
        let mut tooltip = Self::default();
        for member in &focus.members {
            let Some(datum) = &member.datum else {
                continue;
            };
            if tooltip.header.is_none() {
                (tooltip.header, tooltip.header_syntax) = content.header_of(datum, engine)?;
            }
            let Some(field) = content.fields_of(datum).into_iter().next() else {
                continue;
            };
            let (value, value_syntax) = field_value(datum, &field, engine)?;
            tooltip.rows.push(TooltipRow {
                swatch: Swatch::of(member, scene),
                label: member.mark().name.clone(),
                value,
                value_syntax,
            });
        }
        Ok(tooltip)
    }

    /// The tooltip's marks: its box placed by [`place`] beside `anchor` inside `bounds`, `[x, y,
    /// width, height]`, with its callout's tip on the anchor.
    pub fn marks(
        &self,
        anchor: impl Fn(Side) -> [f32; 2],
        bounds: [f32; 4],
        engine: &LabelEngine,
    ) -> Result<SceneGroup, AvengerTooltipError> {
        let style = &self.style;
        self.boxed(engine, |size| {
            let placement = place(anchor, size, bounds, &style.sides, style.callout);
            (placement.origin, Some((placement.side, placement.tip)))
        })
    }

    /// The tooltip's marks: its box placed by [`place_beside_pointer`] below and right of
    /// `pointer` inside `bounds`, at the style's pointer offset, without a callout.
    pub fn marks_beside(
        &self,
        pointer: [f32; 2],
        bounds: [f32; 4],
        engine: &LabelEngine,
    ) -> Result<SceneGroup, AvengerTooltipError> {
        let offset = self.style.pointer_offset;
        self.boxed(engine, |size| {
            (place_beside_pointer(pointer, size, bounds, offset), None)
        })
    }

    /// The tooltip's box and content, at the top left corner that `place` gives for the box's
    /// size, with the callout to the tip that it gives on the side that it gives, if any.
    fn boxed(
        &self,
        engine: &LabelEngine,
        place: impl FnOnce([f32; 2]) -> ([f32; 2], Option<(Side, [f32; 2])>),
    ) -> Result<SceneGroup, AvengerTooltipError> {
        let style = &self.style;
        let paint = self.paint();
        let content = self.content(&paint, engine)?;
        let (origin, callout) = place(content.size);
        let callout = callout.map(|(side, tip)| (side, [tip[0] - origin[0], tip[1] - origin[1]]));
        let outline = ScenePathMark {
            name: "tooltip_box".to_string(),
            clip: false,
            path: outline(content.size, style, callout).into(),
            fill: ColorOrGradient::Color(paint.fill).into(),
            stroke: ColorOrGradient::Color(paint.stroke).into(),
            stroke_width: Some(style.stroke_width),
            ..Default::default()
        };
        let mut marks = vec![outline.into()];
        marks.extend(content.marks);
        Ok(SceneGroup {
            name: "tooltip".to_string(),
            origin,
            marks,
            ..Default::default()
        })
    }

    /// The header, the rows, and their swatches, with the box's top left corner at 0, and the
    /// box's size.
    fn content(&self, paint: &Paint, engine: &LabelEngine) -> Result<Content, AvengerTooltipError> {
        let style = &self.style;
        let pad = style.padding;
        let text = |text: Vec<String>, syntax, color: [f32; 4], weight: FontWeight| SceneTextMark {
            name: "tooltip_text".to_string(),
            clip: false,
            len: text.len() as u32,
            text: text.into(),
            text_syntax: syntax,
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Top.into(),
            color: color.into(),
            font: style.font.clone().into(),
            font_size: style.font_size.into(),
            font_weight: weight.into(),
            width: LabelWidth::Max(style.max_width).into(),
            wrap: true,
            max_lines: std::num::NonZeroUsize::new(style.max_lines),
            ellipsis: true,
            ..Default::default()
        };
        let bounds = |mark: &SceneTextMark| -> Result<Vec<TextBounds>, AvengerTooltipError> {
            Ok(mark
                .labels()
                .map(|label| engine.bounds(&label.label))
                .collect::<Result<_, _>>()?)
        };

        let mut header = self.header.as_ref().map(|header| {
            text(
                vec![header.clone()],
                self.header_syntax,
                paint.text,
                style.header_weight,
            )
        });
        let header_bounds = match &header {
            Some(mark) => bounds(mark)?.first().copied(),
            None => None,
        };
        let mut labels = text(
            self.rows.iter().map(|row| row.label.clone()).collect(),
            TextSyntaxMode::Plain,
            paint.label,
            FontWeight::NORMAL,
        );
        let (values, values_syntax) = row_values(&self.rows);
        let mut values = text(values, values_syntax, paint.text, FontWeight::NORMAL);
        let label_bounds = bounds(&labels)?;
        let value_bounds = bounds(&values)?;

        // Columns: the swatches, the labels, and the values.
        let widest = |bounds: &[TextBounds]| bounds.iter().map(|b| b.width).fold(0.0, f32::max);
        let swatches = self.rows.iter().any(|row| row.swatch.is_some());
        let swatch_column = if swatches {
            style.swatch_size + style.column_gap
        } else {
            0.0
        };
        let label_column = widest(&label_bounds);
        let label_x = pad + swatch_column;
        let value_x = label_x
            + if label_column > 0.0 {
                label_column + style.column_gap
            } else {
                0.0
            };
        let rows_width = value_x - pad + widest(&value_bounds);
        let header_width = header_bounds.map_or(0.0, |b| b.width);

        // Rows, below the header.
        let mut y = pad;
        if let Some(bounds) = header_bounds {
            y += bounds.height + style.row_gap;
        }
        let mut marks: Vec<SceneMark> = Vec::new();
        let (mut row_y, mut swatch_marks) = (Vec::new(), Vec::new());
        for (index, row) in self.rows.iter().enumerate() {
            let (label, value) = (label_bounds[index], value_bounds[index]);
            let line = label.height.max(style.swatch_size);
            if let Some(swatch) = &row.swatch {
                let center = [pad + style.swatch_size / 2.0, y + label.height / 2.0];
                swatch_marks.push(swatch.placed(center, style.swatch_size));
            }
            row_y.push(y);
            y += line.max(value.height) + style.row_gap;
        }
        if header_bounds.is_some() || !self.rows.is_empty() {
            y -= style.row_gap;
        }
        let size = [
            (2.0 * pad + rows_width.max(header_width)).ceil(),
            (y + pad).ceil(),
        ];

        if let Some(header) = &mut header {
            header.x = pad.into();
            header.y = pad.into();
        }
        labels.x = label_x.into();
        labels.y = row_y.clone().into();
        values.x = value_x.into();
        values.y = row_y.into();
        marks.extend(header.map(SceneMark::from));
        marks.extend(swatch_marks);
        marks.push(labels.into());
        marks.push(values.into());
        Ok(Content { marks, size })
    }
}

/// The rows' values, and how they read: as Typst markup when any value is markup, with plain
/// values escaped, and as plain text otherwise.
fn row_values(rows: &[TooltipRow]) -> (Vec<String>, TextSyntaxMode) {
    let markup = rows
        .iter()
        .any(|row| row.value_syntax == TextSyntaxMode::TypstMarkup);
    let values = rows
        .iter()
        .map(|row| match row.value_syntax {
            TextSyntaxMode::TypstMarkup => row.value.clone(),
            _ if markup => escape_text(&row.value),
            _ => row.value.clone(),
        })
        .collect();
    let syntax = if markup {
        TextSyntaxMode::TypstMarkup
    } else {
        TextSyntaxMode::Plain
    };
    (values, syntax)
}

/// A tooltip's content and its box's size.
struct Content {
    marks: Vec<SceneMark>,
    size: [f32; 2],
}

/// The outline of a box of `size` with its top left corner at 0 and rounded corners, with a
/// callout, when there's one, from the edge that faces the anchor on the box's side to the tip.
/// The callout meets the box at the tip's position along that edge, kept clear of the corners.
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
    // The callout faces the anchor: the box's left edge for a box on the anchor's right.
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

/// Where a group focus's crosshair stands: the x of its member nearest the pointer along x.
pub fn crosshair_x(focus: &Focus) -> f32 {
    let distance = |x: f32| (x - focus.pointer[0]).abs();
    focus
        .members
        .iter()
        .map(|member| member.position[0])
        .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
        .unwrap_or(focus.pointer[0])
}

/// A group focus's crosshair and markers: a rule at `x` down `plot`, `[x, y, width, height]`,
/// and at each member's position a circle in its swatch's paint, ringed in white.
pub fn crosshair(
    focus: &Focus,
    x: f32,
    plot: [f32; 4],
    scene: &SceneGraph,
    style: &TooltipStyle,
) -> Vec<SceneMark> {
    let rule = SceneRuleMark {
        name: "tooltip_crosshair".to_string(),
        clip: false,
        x: x.into(),
        x2: x.into(),
        y: plot[1].into(),
        y2: (plot[1] + plot[3]).into(),
        stroke: ColorOrGradient::Color(style.crosshair).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    };
    let mut marks: Vec<SceneMark> = vec![rule.into()];
    marks.extend(focus.members.iter().filter_map(|member| {
        let paint = Swatch::of(member, scene)?.paint()?.clone();
        let marker = SceneSymbolMark {
            name: "tooltip_marker".to_string(),
            clip: false,
            shapes: vec![SymbolShape::Circle],
            x: member.position[0].into(),
            y: member.position[1].into(),
            size: (style.marker_size * style.marker_size).into(),
            fill: paint.into(),
            stroke: ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0]).into(),
            stroke_width: Some(1.5),
            ..Default::default()
        };
        Some(marker.into())
    }));
    marks
}

/// An outline of `member`'s drawn shape, for the overlay to highlight it: its geometry's edges
/// in `color`, `width` wide, clipped as its mark is. A point's outline is a ring around its
/// reach, such as a circle symbol's edge.
pub fn highlight(member: &Member, color: [f32; 4], width: f32) -> SceneGroup {
    let stroke = ColorOrGradient::Color(color);
    let transparent = ColorOrGradient::Color([0.0; 4]);
    let mark: SceneMark = match &member.entry.geometry {
        Geometry::Point(_) => {
            let diameter = 2.0 * (member.entry.reach + width);
            SceneSymbolMark {
                name: "tooltip_highlight".to_string(),
                clip: false,
                shapes: vec![SymbolShape::Circle],
                x: member.position[0].into(),
                y: member.position[1].into(),
                size: (diameter * diameter).into(),
                fill: transparent.into(),
                stroke: stroke.into(),
                stroke_width: Some(width),
                ..Default::default()
            }
            .into()
        }
        geometry => {
            let mut path = Path::builder();
            edges(geometry, &mut path);
            ScenePathMark {
                name: "tooltip_highlight".to_string(),
                clip: false,
                path: path.build().into(),
                fill: transparent.into(),
                stroke: stroke.into(),
                stroke_width: Some(width),
                stroke_join: StrokeJoin::Round,
                ..Default::default()
            }
            .into()
        }
    };
    SceneGroup {
        name: "tooltip_highlight".to_string(),
        clip: member.clip.clone().unwrap_or(Clip::None),
        marks: vec![mark],
        ..Default::default()
    }
}

/// Add `geometry`'s edges to `path`: its polygons' rings, closed, and its lines.
fn edges(geometry: &Geometry<f32>, path: &mut lyon_path::path::Builder) {
    let mut string = |string: &LineString<f32>, close: bool| {
        let mut coords = string.coords();
        let Some(Coord { x, y }) = coords.next() else {
            return;
        };
        path.begin(point(*x, *y));
        for Coord { x, y } in coords {
            path.line_to(point(*x, *y));
        }
        path.end(close);
    };
    match geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) => {}
        Geometry::Line(line) => string(&LineString::from(vec![line.start, line.end]), false),
        Geometry::LineString(line) => string(line, false),
        Geometry::MultiLineString(lines) => lines.iter().for_each(|line| string(line, false)),
        Geometry::Polygon(polygon) => {
            std::iter::once(polygon.exterior())
                .chain(polygon.interiors())
                .for_each(|ring| string(ring, true));
        }
        Geometry::MultiPolygon(polygons) => polygons.iter().for_each(|polygon| {
            std::iter::once(polygon.exterior())
                .chain(polygon.interiors())
                .for_each(|ring| string(ring, true));
        }),
        Geometry::Rect(rect) => edges(&Geometry::Polygon(rect.to_polygon()), path),
        Geometry::Triangle(triangle) => edges(&Geometry::Polygon(triangle.to_polygon()), path),
        Geometry::GeometryCollection(collection) => {
            collection.iter().for_each(|geometry| edges(geometry, path))
        }
    }
}
