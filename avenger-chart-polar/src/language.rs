//! Avenger-language registration for polar coordinates and primitive marks.

use avenger_chart_lang_types::{
    CoordinateLanguageDefinition, NativeLoweringError, ResolvedDeclaration,
};
use avenger_chart_marks::language::{
    channel, lower_line, lower_symbol, lower_text, primitive_schema, primitive_text_schema,
};
use avenger_chart_schema::{BodyMode, KindSchema, NativeKindKey, NativeKindNamespace};

use crate::Polar;

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
