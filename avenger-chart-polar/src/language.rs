//! Avenger-language registration for polar coordinates and primitive marks.

use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ObjectLanguageDefinition,
    ResolvedDeclaration, resolved_expr,
};
use avenger_chart_marks::language::{
    channel, lower_line, lower_symbol, lower_text, primitive_schema, primitive_text_schema,
};
use avenger_chart_schema::{
    BodyMode, ChildRule, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema, ValueShape,
};

use crate::{Polar, PolarAxis};

const LINE_CHANNELS: &[&str] = &[
    "r",
    "theta",
    "stroke",
    "stroke_width",
    "stroke_dash",
    "opacity",
    "stroke_cap",
    "stroke_join",
    "defined",
    "order",
];
const SYMBOL_CHANNELS: &[&str] = &[
    "r",
    "theta",
    "size",
    "fill",
    "stroke",
    "stroke_width",
    "shape",
    "angle",
];
const TEXT_CHANNELS: &[&str] = &[
    "r",
    "theta",
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

pub fn definition() -> CoordinateLanguageDefinition<Polar> {
    CoordinateLanguageDefinition::new("polar", coordinate_schema(), lower_polar)
        .mark(
            "line",
            primitive_schema(
                "polar",
                "line",
                "A line in radial and angular coordinates.",
                LINE_CHANNELS.iter().copied().map(channel),
            ),
            lower_line::<Polar>,
        )
        .mark(
            "symbol",
            primitive_schema(
                "polar",
                "symbol",
                "A point symbol in radial and angular coordinates.",
                SYMBOL_CHANNELS.iter().copied().map(channel),
            ),
            lower_symbol::<Polar>,
        )
        .mark(
            "text",
            primitive_text_schema(
                "polar",
                "Text positioned in radial and angular coordinates.",
                TEXT_CHANNELS.iter().copied().map(channel),
            ),
            lower_text::<Polar>,
        )
}

pub fn subplot_schema() -> KindSchema {
    primitive_schema(
        "polar",
        "subplot",
        "A data-driven child plot positioned in polar coordinates.",
        [channel("r"), channel("theta"), channel("key")],
    )
    .body_mode(BodyMode::Mixed)
    .property(
        "width",
        PropertySchema::optional(ValueShape::SqlExpression, "Child plot-area width."),
    )
    .property(
        "height",
        PropertySchema::optional(ValueShape::SqlExpression, "Child plot-area height."),
    )
    .child_rule(ChildRule {
        role: "plot".to_string(),
        min: 1,
        max: Some(1),
        docs: "The embedded mixed-coordinate child plot.".to_string(),
    })
}

fn coordinate_schema() -> KindSchema {
    KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Coordinate, "polar"),
        "A radial and angular two-dimensional coordinate system.",
    )
    .body_mode(BodyMode::Mixed)
}

fn lower_polar(_declaration: &ResolvedDeclaration) -> Result<Polar, NativeLoweringError> {
    Ok(Polar::new())
}

/// Owner-provided polar axis authoring surface.
pub fn axis_definition() -> ObjectLanguageDefinition {
    let mut schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Axis, "polar"),
        "A radial or angular polar-axis configuration.",
    );
    for (name, docs) in [
        ("visible", "Whether the axis is visible."),
        ("axis_type", "Whether this is a radial or angular axis."),
        ("title", "Axis title text."),
        ("grid", "Whether to draw grid lines."),
        ("tick_count", "Requested number of ticks."),
        ("format", "Number-format pattern."),
        (
            "grid_levels",
            "Radial or angular values at which grid levels are drawn.",
        ),
        ("start_angle", "Angular-axis starting angle."),
        ("direction", "Angular-axis direction."),
    ] {
        schema = schema.property(
            name,
            PropertySchema::optional(ValueShape::SqlExpression, docs),
        );
    }
    ObjectLanguageDefinition {
        schema,
        lowerer: lower_axis,
    }
}

fn lower_axis(
    declaration: &ResolvedDeclaration,
) -> Result<Box<dyn std::any::Any + Send + Sync>, NativeLoweringError> {
    let mut axis = PolarAxis::new();
    for (name, value) in &declaration.properties {
        let expr =
            resolved_expr(value).ok_or_else(|| NativeLoweringError::InvalidPropertyType {
                property: name.clone(),
                expected: "scalar SQL expression".to_string(),
            })?;
        axis = match name.as_str() {
            "visible" => axis.visible(expr),
            "axis_type" => axis.axis_type(expr),
            "title" => axis.title(expr),
            "grid" => axis.grid(expr),
            "tick_count" => axis.tick_count(expr),
            "format" => axis.format(expr),
            "grid_levels" => axis.grid_levels(expr),
            "start_angle" => axis.start_angle(expr),
            "direction" => axis.direction(expr),
            _ => {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: name.clone(),
                    expected: "a registered polar axis property".to_string(),
                });
            }
        };
    }
    let erased: Box<dyn avenger_chart_core::Axis> = Box::new(axis);
    Ok(Box::new(erased))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polar_definition_registers_runtime_compatible_marks() {
        let definition = definition();
        assert_eq!(
            definition
                .marks
                .iter()
                .map(|mark| mark.kind)
                .collect::<Vec<_>>(),
            ["line", "symbol", "text"]
        );
    }
}
