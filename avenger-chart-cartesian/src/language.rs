//! Avenger-language registration for Cartesian coordinates and primitive marks.

use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ObjectLanguageDefinition,
    ResolvedDeclaration, ResolvedValue, resolved_expr,
};
use avenger_chart_marks::language::{
    channel, lower_area, lower_image, lower_line, lower_path, lower_rect, lower_rule, lower_symbol,
    lower_text, lower_trail, lower_uniform_raster, primitive_schema, primitive_text_schema,
    uniform_raster_schema,
};
use avenger_chart_schema::{
    BodyMode, EnumValueSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema,
    ValueShape,
};
use avenger_text::types::TextSyntaxMode;

use crate::{Cartesian, CartesianAxis};

const AREA_CHANNELS: &[&str] = &[
    "x",
    "y",
    "x2",
    "y2",
    "orientation",
    "fill",
    "fill_pattern",
    "stroke",
    "stroke_width",
    "stroke_dash",
    "stroke_cap",
    "stroke_join",
    "opacity",
    "defined",
    "order",
];
const IMAGE_CHANNELS: &[&str] = &[
    "x", "y", "image", "width", "height", "align", "baseline", "aspect", "smooth",
];
const LINE_CHANNELS: &[&str] = &[
    "x",
    "y",
    "stroke",
    "stroke_width",
    "stroke_dash",
    "opacity",
    "stroke_cap",
    "stroke_join",
    "defined",
    "order",
];
const PATH_CHANNELS: &[&str] = &[
    "x",
    "y",
    "path",
    "path_transform",
    "fill",
    "fill_pattern",
    "stroke",
    "stroke_width",
    "stroke_cap",
    "stroke_join",
    "opacity",
];
const RECT_CHANNELS: &[&str] = &[
    "x",
    "x2",
    "y",
    "y2",
    "fill",
    "fill_pattern",
    "stroke",
    "stroke_width",
    "corner_radius",
    "opacity",
];
const RULE_CHANNELS: &[&str] = &[
    "x",
    "y",
    "x2",
    "y2",
    "stroke",
    "stroke_width",
    "stroke_dash",
    "stroke_cap",
    "opacity",
];
const SYMBOL_CHANNELS: &[&str] = &[
    "x",
    "y",
    "size",
    "fill",
    "fill_pattern",
    "stroke",
    "stroke_width",
    "shape",
    "angle",
    "opacity",
];
const TEXT_CHANNELS: &[&str] = &[
    "x",
    "y",
    "text",
    "align",
    "baseline",
    "angle",
    "color",
    "font",
    "font_size",
    "font_weight",
    "font_style",
    "limit",
    "opacity",
    "defined",
    "leader",
    "leader_offset_x",
    "leader_offset_y",
    "leader_stroke",
    "leader_stroke_width",
    "leader_stroke_dash",
    "leader_stroke_cap",
    "leader_stroke_join",
    "leader_label_padding",
    "leader_target_radius",
    "leader_min_length",
    "leader_shape",
    "leader_arrow",
    "leader_arrow_length",
    "leader_arrow_width",
];
const TRAIL_CHANNELS: &[&str] = &["x", "y", "size", "stroke", "opacity", "defined", "order"];

pub fn definition() -> CoordinateLanguageDefinition<Cartesian> {
    CoordinateLanguageDefinition::new("cartesian", coordinate_schema(), lower_cartesian)
        .mark(
            "area",
            mark_schema("area", "A filled Cartesian area mark.", AREA_CHANNELS),
            lower_area::<Cartesian>,
        )
        .mark(
            "image",
            mark_schema(
                "image",
                "An image positioned in Cartesian coordinates.",
                IMAGE_CHANNELS,
            ),
            lower_image::<Cartesian>,
        )
        .mark(
            "line",
            mark_schema("line", "A Cartesian line mark.", LINE_CHANNELS),
            lower_line::<Cartesian>,
        )
        .mark(
            "path",
            mark_schema("path", "An arbitrary Cartesian path mark.", PATH_CHANNELS),
            lower_path::<Cartesian>,
        )
        .mark(
            "rect",
            mark_schema("rect", "A Cartesian rectangle mark.", RECT_CHANNELS),
            lower_rect::<Cartesian>,
        )
        .mark(
            "rule",
            mark_schema("rule", "A Cartesian rule mark.", RULE_CHANNELS),
            lower_rule::<Cartesian>,
        )
        .mark(
            "symbol",
            mark_schema(
                "symbol",
                "A point symbol in Cartesian coordinates.",
                SYMBOL_CHANNELS,
            ),
            lower_symbol::<Cartesian>,
        )
        .mark("text", text_schema(), lower_text::<Cartesian>)
        .mark(
            "trail",
            mark_schema(
                "trail",
                "A variable-width Cartesian trail mark.",
                TRAIL_CHANNELS,
            ),
            lower_trail::<Cartesian>,
        )
        .mark(
            "uniform_raster_2d",
            uniform_raster_schema("cartesian"),
            lower_uniform_raster::<Cartesian>,
        )
}

fn coordinate_schema() -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "cartesian"),
        "A two-dimensional Cartesian coordinate system.",
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "unit_aspect",
        PropertySchema::optional(
            ValueShape::Number,
            "Optional positive ratio between x and y data units.",
        ),
    )
}

fn lower_cartesian(declaration: &ResolvedDeclaration) -> Result<Cartesian, NativeLoweringError> {
    let mut coordinate = Cartesian::new();
    if let Some(ResolvedValue::Number(ratio)) = declaration.properties.get("unit_aspect") {
        coordinate = coordinate.unit_aspect(*ratio);
    }
    Ok(coordinate)
}

fn mark_schema(kind: &str, docs: &str, channels: &[&'static str]) -> KindSchema {
    primitive_schema(
        "cartesian",
        kind,
        docs,
        channels.iter().copied().map(channel),
    )
}

fn text_schema() -> KindSchema {
    primitive_text_schema(
        "cartesian",
        "Text positioned in Cartesian coordinates.",
        TEXT_CHANNELS.iter().copied().map(channel),
    )
}

/// Owner-provided Cartesian axis authoring surface.
pub fn axis_definition() -> ObjectLanguageDefinition {
    let mut schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Axis, "cartesian"),
        "A Cartesian position-axis configuration.",
    );
    for (name, docs) in [
        ("visible", "Whether the axis is visible."),
        ("position", "Axis side or crossing position."),
        ("title", "Axis title text."),
        ("grid", "Whether to draw grid lines."),
        ("tick_count", "Requested number of ticks."),
        (
            "tick_spacing",
            "Structured start/step tick-spacing expression.",
        ),
        ("label_angle", "Tick-label rotation angle in degrees."),
        ("format", "Number-format pattern."),
        ("datetime_format", "Date/time-format pattern."),
        ("number_locale", "Number-format locale identifier."),
        ("tick_label", "Expression producing tick-label text."),
        ("title_font_family", "Axis-title font family."),
        ("label_font_family", "Tick-label font family."),
        (
            "show_title",
            "Whether to render the title while retaining its configuration.",
        ),
    ] {
        schema = schema.property(
            name,
            PropertySchema::optional(ValueShape::SqlExpression, docs),
        );
    }
    schema = schema.property(
        "title_syntax",
        PropertySchema::optional(
            ValueShape::Atom {
                values: [
                    ("plain", "Render the title literally."),
                    ("typst", "Render the title as Typst markup."),
                ]
                .into_iter()
                .map(|(value, docs)| EnumValueSchema {
                    value: value.to_string(),
                    docs: docs.to_string(),
                })
                .collect(),
            },
            "Axis-title text syntax.",
        ),
    );
    ObjectLanguageDefinition {
        schema,
        lowerer: lower_axis,
    }
}

fn lower_axis(
    declaration: &ResolvedDeclaration,
) -> Result<Box<dyn std::any::Any + Send + Sync>, NativeLoweringError> {
    let mut axis = CartesianAxis::new();
    for (name, value) in &declaration.properties {
        axis = match name.as_str() {
            "visible" => axis.visible(axis_expr(name, value)?),
            "position" => axis.position(axis_expr(name, value)?),
            "title" => axis.title(axis_expr(name, value)?),
            "grid" => axis.grid(axis_expr(name, value)?),
            "tick_count" => axis.tick_count(axis_expr(name, value)?),
            "tick_spacing" => axis.tick_spacing(axis_expr(name, value)?),
            "label_angle" => axis.label_angle(axis_expr(name, value)?),
            "format" => axis.format(axis_expr(name, value)?),
            "datetime_format" => axis.datetime_format(axis_expr(name, value)?),
            "number_locale" => axis.number_locale(axis_expr(name, value)?),
            "tick_label" => axis.tick_label(axis_expr(name, value)?),
            "title_font_family" => axis.title_font_family(axis_expr(name, value)?),
            "label_font_family" => axis.label_font_family(axis_expr(name, value)?),
            "show_title" => axis.show_title(axis_expr(name, value)?),
            "title_syntax" => match value {
                ResolvedValue::String(value) if value == "plain" => {
                    axis.syntax_mode(TextSyntaxMode::Plain)
                }
                ResolvedValue::String(value) if value == "typst" => {
                    axis.syntax_mode(TextSyntaxMode::TypstMarkup)
                }
                _ => return Err(axis_invalid(name, "plain or typst")),
            },
            _ => return Err(axis_invalid(name, "a registered Cartesian axis property")),
        };
    }
    let erased: Box<dyn avenger_chart_core::Axis> = Box::new(axis);
    Ok(Box::new(erased))
}

fn axis_expr(
    property: &str,
    value: &ResolvedValue,
) -> Result<datafusion::logical_expr::Expr, NativeLoweringError> {
    resolved_expr(value).ok_or_else(|| axis_invalid(property, "scalar SQL expression"))
}

fn axis_invalid(property: &str, expected: &str) -> NativeLoweringError {
    NativeLoweringError::InvalidPropertyType {
        property: property.to_string(),
        expected: expected.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cartesian_definition_registers_the_ordinary_primitive_inventory() {
        let definition = definition();
        assert_eq!(
            definition
                .marks
                .iter()
                .map(|mark| mark.kind)
                .collect::<Vec<_>>(),
            [
                "area",
                "image",
                "line",
                "path",
                "rect",
                "rule",
                "symbol",
                "text",
                "trail",
                "uniform_raster_2d"
            ]
        );
        for mark in definition.marks {
            assert!(
                !mark.schema.channels.is_empty(),
                "{} has no channels",
                mark.kind
            );
            assert!(
                mark.schema.docs.len() > 10,
                "{} has incomplete docs",
                mark.kind
            );
        }
    }
}
