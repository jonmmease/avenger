//! Avenger-language schema and erased lowerer for the standard legend.

use avenger_chart_lang_types::{
    NativeLoweringError, ObjectLanguageDefinition, ResolvedDeclaration, ResolvedValue,
    resolved_expr,
};
use avenger_chart_schema::{
    EnumValueSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema, ValueShape,
};
use avenger_text::types::TextSyntaxMode;

use crate::Legend;

pub fn definition() -> ObjectLanguageDefinition {
    let mut schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Legend, "standard"),
        "A standard discrete or continuous chart legend configuration.",
    );
    for (name, docs) in [
        ("visible", "Whether the legend is visible."),
        ("title", "Legend title text."),
        ("position", "Legend chrome-slot position."),
        ("orientation", "Legend item-flow orientation."),
        ("symbol_size", "Discrete legend symbol area."),
        ("gradient_thickness", "Continuous colorbar thickness."),
        ("columns", "Number of discrete legend columns."),
        ("label_limit", "Maximum item-label width in pixels."),
        ("format_number", "Numeric label format pattern."),
        ("background_fill", "Legend background fill color."),
        ("background_stroke", "Legend background stroke color."),
        ("background_stroke_width", "Legend background stroke width."),
        (
            "background_corner_radius",
            "Legend background corner radius.",
        ),
        (
            "background_padding",
            "Padding between the background and content.",
        ),
        ("order", "Expression controlling discrete item order."),
        ("title_color", "Legend-title color."),
        ("label_color", "Discrete item-label color."),
        ("tick_color", "Continuous colorbar tick-label color."),
        ("title_font_family", "Legend-title font family."),
        ("title_font_size", "Legend-title font size."),
        ("title_font_weight", "Legend-title font weight."),
        ("label_font_family", "Discrete item-label font family."),
        ("label_font_size", "Discrete item-label font size."),
        ("label_font_weight", "Discrete item-label font weight."),
        ("tick_font_family", "Continuous tick-label font family."),
        ("tick_font_size", "Continuous tick-label font size."),
        ("tick_font_weight", "Continuous tick-label font weight."),
    ] {
        schema = schema.property(
            name,
            PropertySchema::optional(ValueShape::SqlExpression, docs),
        );
    }
    for (name, docs) in [
        ("title_syntax", "Legend-title text syntax."),
        ("label_syntax", "Legend-label text syntax."),
    ] {
        schema = schema.property(
            name,
            PropertySchema::optional(
                ValueShape::Atom {
                    values: [
                        ("plain", "Render text literally."),
                        ("typst", "Render text as Typst markup."),
                    ]
                    .into_iter()
                    .map(|(value, docs)| EnumValueSchema {
                        value: value.to_string(),
                        docs: docs.to_string(),
                    })
                    .collect(),
                },
                docs,
            ),
        );
    }
    ObjectLanguageDefinition {
        schema,
        lowerer: lower_legend,
    }
}

fn lower_legend(
    declaration: &ResolvedDeclaration,
) -> Result<Box<dyn std::any::Any + Send + Sync>, NativeLoweringError> {
    let mut legend = Legend::new();
    for (name, value) in &declaration.properties {
        legend = match name.as_str() {
            "title_syntax" => legend.title_syntax_mode(syntax(name, value)?),
            "label_syntax" => legend.label_syntax_mode(syntax(name, value)?),
            "visible" => legend.visible(expr(name, value)?),
            "title" => legend.title(expr(name, value)?),
            "position" => legend.position(expr(name, value)?),
            "orientation" => legend.orientation(expr(name, value)?),
            "symbol_size" => legend.symbol_size(expr(name, value)?),
            "gradient_thickness" => legend.gradient_thickness(expr(name, value)?),
            "columns" => legend.columns(expr(name, value)?),
            "label_limit" => legend.label_limit(expr(name, value)?),
            "format_number" => legend.format_number(expr(name, value)?),
            "background_fill" => legend.background_fill(expr(name, value)?),
            "background_stroke" => legend.background_stroke(expr(name, value)?),
            "background_stroke_width" => legend.background_stroke_width(expr(name, value)?),
            "background_corner_radius" => legend.background_corner_radius(expr(name, value)?),
            "background_padding" => legend.background_padding(expr(name, value)?),
            "order" => legend.order(expr(name, value)?),
            "title_color" => legend.title_color(expr(name, value)?),
            "label_color" => legend.label_color(expr(name, value)?),
            "tick_color" => legend.tick_color(expr(name, value)?),
            "title_font_family" => legend.title_font_family(expr(name, value)?),
            "title_font_size" => legend.title_font_size(expr(name, value)?),
            "title_font_weight" => legend.title_font_weight(expr(name, value)?),
            "label_font_family" => legend.label_font_family(expr(name, value)?),
            "label_font_size" => legend.label_font_size(expr(name, value)?),
            "label_font_weight" => legend.label_font_weight(expr(name, value)?),
            "tick_font_family" => legend.tick_font_family(expr(name, value)?),
            "tick_font_size" => legend.tick_font_size(expr(name, value)?),
            "tick_font_weight" => legend.tick_font_weight(expr(name, value)?),
            _ => return Err(invalid(name, "a registered standard legend property")),
        };
    }
    Ok(Box::new(legend))
}

fn expr(
    property: &str,
    value: &ResolvedValue,
) -> Result<datafusion::logical_expr::Expr, NativeLoweringError> {
    resolved_expr(value).ok_or_else(|| invalid(property, "scalar SQL expression"))
}

fn syntax(property: &str, value: &ResolvedValue) -> Result<TextSyntaxMode, NativeLoweringError> {
    match value {
        ResolvedValue::String(value) if value == "plain" => Ok(TextSyntaxMode::Plain),
        ResolvedValue::String(value) if value == "typst" => Ok(TextSyntaxMode::TypstMarkup),
        _ => Err(invalid(property, "plain or typst")),
    }
}

fn invalid(property: &str, expected: &str) -> NativeLoweringError {
    NativeLoweringError::InvalidPropertyType {
        property: property.to_string(),
        expected: expected.to_string(),
    }
}
