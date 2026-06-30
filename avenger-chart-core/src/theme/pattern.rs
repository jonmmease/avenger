use std::fmt;

use avenger_common::types::SymbolShape;
use avenger_scenegraph::marks::pattern::default_pattern_opacity;
use datafusion_common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternSymbol, ScaleRange, StripeDash,
    StripePatternLayer, SymbolLattice2d, SymbolPaint, SymbolPatternLayer, theme::ThemeValue,
};

#[derive(Debug, Clone)]
pub(crate) struct PatternThemeError {
    path: String,
    message: String,
}

impl PatternThemeError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    fn missing(path: &str) -> Self {
        Self::new(path, "is required")
    }

    fn type_error(path: &str, expected: &str, actual: &ThemeValue) -> Self {
        Self::new(path, format!("expected {}, got {:?}", expected, actual))
    }
}

impl fmt::Display for PatternThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.path, self.message)
    }
}

impl std::error::Error for PatternThemeError {}

pub(crate) fn pattern_range_from_theme_value(
    value: ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
) -> Result<ScaleRange, PatternThemeError> {
    let values = match value {
        ThemeValue::List(values) | ThemeValue::Array(values) => values,
        value => vec![value],
    };

    let mut patterns = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        patterns.push(parse_pattern_entry(
            value,
            params,
            base_font_size,
            &format!("fill-pattern-discrete[{}]", index),
        )?);
    }

    Ok(ScaleRange::new_pattern(patterns))
}

fn parse_pattern_entry(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<Option<PatternFill>, PatternThemeError> {
    match value {
        ThemeValue::None => Ok(None),
        ThemeValue::Object(object) => {
            reject_unknown_fields(object, &["anchor", "ink", "layers"], path)?;

            let anchor = match object.get("anchor") {
                Some(value) => parse_anchor(value, &format!("{}.anchor", path))?,
                None => PatternAnchor::default(),
            };

            let ink = match object.get("ink") {
                Some(value) => parse_ink(value, params, base_font_size, &format!("{}.ink", path))?,
                None => PatternInk::default(),
            };

            let layers_value = object
                .get("layers")
                .ok_or_else(|| PatternThemeError::missing(&format!("{}.layers", path)))?;
            let layers = parse_layers(
                layers_value,
                params,
                base_font_size,
                &format!("{}.layers", path),
            )?;

            let pattern = PatternFill {
                anchor,
                ink,
                layers,
            };

            pattern.validate().map_err(|error| {
                PatternThemeError::new(path, format!("failed validation: {:?}", error))
            })?;

            Ok(Some(pattern))
        }
        _ => Err(PatternThemeError::type_error(
            path,
            "none or a pattern object",
            value,
        )),
    }
}

fn parse_anchor(value: &ThemeValue, path: &str) -> Result<PatternAnchor, PatternThemeError> {
    let value = expect_string(value, path)?;
    match value {
        "plot" => Ok(PatternAnchor::Plot),
        "mark" => Ok(PatternAnchor::Mark),
        "chart" => Ok(PatternAnchor::Chart),
        _ => Err(PatternThemeError::new(
            path,
            "must be one of plot, mark, or chart",
        )),
    }
}

fn parse_ink(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<PatternInk, PatternThemeError> {
    let object = expect_object(value, path)?;
    let ink_type = optional_string_field(object, "type", path)?.unwrap_or("auto-contrast");

    match ink_type {
        "auto-contrast" => {
            reject_unknown_fields(object, &["type", "opacity"], path)?;
            Ok(PatternInk::AutoContrast {
                opacity: optional_opacity_field(object, "opacity", params, base_font_size, path)?
                    .unwrap_or_else(default_pattern_opacity),
            })
        }
        "solid" => {
            reject_unknown_fields(object, &["type", "color", "opacity"], path)?;
            let color = required_color_field(object, "color", params, base_font_size, path)?;
            Ok(PatternInk::Solid {
                color,
                opacity: optional_opacity_field(object, "opacity", params, base_font_size, path)?
                    .unwrap_or_else(default_pattern_opacity),
            })
        }
        _ => Err(PatternThemeError::new(
            format!("{}.type", path),
            "must be auto-contrast or solid",
        )),
    }
}

fn parse_layers(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<Vec<PatternLayer>, PatternThemeError> {
    let values = expect_array(value, path)?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            parse_layer(
                value,
                params,
                base_font_size,
                &format!("{}[{}]", path, index),
            )
        })
        .collect()
}

fn parse_layer(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<PatternLayer, PatternThemeError> {
    let object = expect_object(value, path)?;
    let layer_type = required_string_field(object, "type", path)?;

    match layer_type {
        "stripe" => parse_stripe_layer(object, params, base_font_size, path),
        "symbol" => parse_symbol_layer(object, params, base_font_size, path),
        _ => Err(PatternThemeError::new(
            format!("{}.type", path),
            "must be stripe or symbol",
        )),
    }
}

fn parse_stripe_layer(
    object: &IndexMap<String, ThemeValue>,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<PatternLayer, PatternThemeError> {
    reject_unknown_fields(
        object,
        &["type", "angle", "spacing", "stroke-width", "phase", "dash"],
        path,
    )?;

    let mut layer = StripePatternLayer::new(
        required_angle_field(object, "angle", path)?,
        required_length_field(object, "spacing", params, base_font_size, path)?,
        required_length_field(object, "stroke-width", params, base_font_size, path)?,
    );
    layer.phase =
        optional_length_field(object, "phase", params, base_font_size, path)?.unwrap_or_default();
    layer.dash = match object.get("dash") {
        Some(value) => Some(parse_dash(
            value,
            params,
            base_font_size,
            &format!("{}.dash", path),
        )?),
        None => None,
    };

    Ok(PatternLayer::Stripe(layer))
}

fn parse_dash(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<StripeDash, PatternThemeError> {
    let object = expect_object(value, path)?;
    reject_unknown_fields(object, &["length", "gap", "phase"], path)?;

    Ok(StripeDash {
        length: required_length_field(object, "length", params, base_font_size, path)?,
        gap: required_length_field(object, "gap", params, base_font_size, path)?,
        phase: optional_length_field(object, "phase", params, base_font_size, path)?
            .unwrap_or_default(),
    })
}

fn parse_symbol_layer(
    object: &IndexMap<String, ThemeValue>,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<PatternLayer, PatternThemeError> {
    reject_unknown_fields(object, &["type", "lattice", "symbol", "paint"], path)?;

    let lattice = parse_symbol_lattice(
        required_field(object, "lattice", path)?,
        params,
        base_font_size,
        &format!("{}.lattice", path),
    )?;
    let symbol = parse_symbol(
        required_field(object, "symbol", path)?,
        params,
        base_font_size,
        &format!("{}.symbol", path),
    )?;
    let paint = parse_symbol_paint(
        required_field(object, "paint", path)?,
        params,
        base_font_size,
        &format!("{}.paint", path),
    )?;

    Ok(PatternLayer::Symbol(SymbolPatternLayer {
        lattice,
        symbol,
        paint,
    }))
}

fn parse_symbol_lattice(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<SymbolLattice2d, PatternThemeError> {
    let object = expect_object(value, path)?;
    reject_unknown_fields(
        object,
        &[
            "u-spacing",
            "u-angle",
            "v-spacing",
            "v-angle",
            "u-phase",
            "v-phase",
        ],
        path,
    )?;

    Ok(SymbolLattice2d {
        u_spacing: required_length_field(object, "u-spacing", params, base_font_size, path)?,
        u_angle: required_angle_field(object, "u-angle", path)?,
        v_spacing: required_length_field(object, "v-spacing", params, base_font_size, path)?,
        v_angle: required_angle_field(object, "v-angle", path)?,
        u_phase: optional_length_field(object, "u-phase", params, base_font_size, path)?
            .unwrap_or_default(),
        v_phase: optional_length_field(object, "v-phase", params, base_font_size, path)?
            .unwrap_or_default(),
    })
}

fn parse_symbol(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<PatternSymbol, PatternThemeError> {
    let object = expect_object(value, path)?;
    reject_unknown_fields(object, &["shape", "size", "rotation"], path)?;

    let shape = required_string_field(object, "shape", path)?.to_string();
    SymbolShape::from_vega_str(&shape).map_err(|error| {
        PatternThemeError::new(
            format!("{}.shape", path),
            format!("is not a valid symbol shape: {}", error),
        )
    })?;

    Ok(PatternSymbol {
        shape,
        size: required_length_field(object, "size", params, base_font_size, path)?,
        rotation: optional_angle_field(object, "rotation", path)?.unwrap_or_default(),
    })
}

fn parse_symbol_paint(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<SymbolPaint, PatternThemeError> {
    let object = expect_object(value, path)?;
    let paint_type = required_string_field(object, "type", path)?;

    match paint_type {
        "filled" => {
            reject_unknown_fields(object, &["type"], path)?;
            Ok(SymbolPaint::Filled)
        }
        "open" => {
            reject_unknown_fields(object, &["type", "stroke-width"], path)?;
            Ok(SymbolPaint::Open {
                stroke_width: required_length_field(
                    object,
                    "stroke-width",
                    params,
                    base_font_size,
                    path,
                )?,
            })
        }
        _ => Err(PatternThemeError::new(
            format!("{}.type", path),
            "must be filled or open",
        )),
    }
}

fn reject_unknown_fields(
    object: &IndexMap<String, ThemeValue>,
    allowed: &[&str],
    path: &str,
) -> Result<(), PatternThemeError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(PatternThemeError::new(
                format!("{}.{}", path, key),
                "is not a supported pattern field",
            ));
        }
    }

    Ok(())
}

fn required_field<'a>(
    object: &'a IndexMap<String, ThemeValue>,
    field: &str,
    path: &str,
) -> Result<&'a ThemeValue, PatternThemeError> {
    object
        .get(field)
        .ok_or_else(|| PatternThemeError::missing(&format!("{}.{}", path, field)))
}

fn required_string_field<'a>(
    object: &'a IndexMap<String, ThemeValue>,
    field: &str,
    path: &str,
) -> Result<&'a str, PatternThemeError> {
    let field_path = format!("{}.{}", path, field);
    expect_string(required_field(object, field, path)?, &field_path)
}

fn optional_string_field<'a>(
    object: &'a IndexMap<String, ThemeValue>,
    field: &str,
    path: &str,
) -> Result<Option<&'a str>, PatternThemeError> {
    object
        .get(field)
        .map(|value| expect_string(value, &format!("{}.{}", path, field)))
        .transpose()
}

fn required_length_field(
    object: &IndexMap<String, ThemeValue>,
    field: &str,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<f32, PatternThemeError> {
    let field_path = format!("{}.{}", path, field);
    parse_length(
        required_field(object, field, path)?,
        params,
        base_font_size,
        &field_path,
    )
}

fn optional_length_field(
    object: &IndexMap<String, ThemeValue>,
    field: &str,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<Option<f32>, PatternThemeError> {
    object
        .get(field)
        .map(|value| {
            parse_length(
                value,
                params,
                base_font_size,
                &format!("{}.{}", path, field),
            )
        })
        .transpose()
}

fn required_angle_field(
    object: &IndexMap<String, ThemeValue>,
    field: &str,
    path: &str,
) -> Result<f32, PatternThemeError> {
    let field_path = format!("{}.{}", path, field);
    parse_angle(required_field(object, field, path)?, &field_path)
}

fn optional_angle_field(
    object: &IndexMap<String, ThemeValue>,
    field: &str,
    path: &str,
) -> Result<Option<f32>, PatternThemeError> {
    object
        .get(field)
        .map(|value| parse_angle(value, &format!("{}.{}", path, field)))
        .transpose()
}

fn required_color_field(
    object: &IndexMap<String, ThemeValue>,
    field: &str,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<[f32; 4], PatternThemeError> {
    let field_path = format!("{}.{}", path, field);
    let value = required_field(object, field, path)?;
    value
        .as_color_with_params(params, base_font_size)
        .map(|rgba| rgba.to_array())
        .ok_or_else(|| PatternThemeError::type_error(&field_path, "a color", value))
}

fn optional_opacity_field(
    object: &IndexMap<String, ThemeValue>,
    field: &str,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<Option<f32>, PatternThemeError> {
    object
        .get(field)
        .map(|value| {
            parse_opacity(
                value,
                params,
                base_font_size,
                &format!("{}.{}", path, field),
            )
        })
        .transpose()
}

fn parse_length(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<f32, PatternThemeError> {
    value
        .as_font_size(params, base_font_size)
        .ok_or_else(|| PatternThemeError::type_error(path, "a number or length", value))
}

fn parse_angle(value: &ThemeValue, path: &str) -> Result<f32, PatternThemeError> {
    value
        .as_angle_degrees()
        .map(|value| value as f32)
        .ok_or_else(|| PatternThemeError::type_error(path, "a number or angle", value))
}

fn parse_opacity(
    value: &ThemeValue,
    params: &IndexMap<String, ScalarValue>,
    base_font_size: f32,
    path: &str,
) -> Result<f32, PatternThemeError> {
    let opacity = match value {
        ThemeValue::Percentage(value) => (*value / 100.0) as f32,
        _ => value
            .as_font_size(params, base_font_size)
            .ok_or_else(|| PatternThemeError::type_error(path, "a number or percentage", value))?,
    };

    if (0.0..=1.0).contains(&opacity) {
        Ok(opacity)
    } else {
        Err(PatternThemeError::new(path, "must be between 0 and 1"))
    }
}

fn expect_object<'a>(
    value: &'a ThemeValue,
    path: &str,
) -> Result<&'a IndexMap<String, ThemeValue>, PatternThemeError> {
    match value {
        ThemeValue::Object(object) => Ok(object),
        _ => Err(PatternThemeError::type_error(
            path,
            "a declaration block object",
            value,
        )),
    }
}

fn expect_array<'a>(
    value: &'a ThemeValue,
    path: &str,
) -> Result<&'a [ThemeValue], PatternThemeError> {
    match value {
        ThemeValue::Array(values) => Ok(values),
        _ => Err(PatternThemeError::type_error(
            path,
            "a bracket array",
            value,
        )),
    }
}

fn expect_string<'a>(value: &'a ThemeValue, path: &str) -> Result<&'a str, PatternThemeError> {
    match value {
        ThemeValue::String(value) => Ok(value),
        _ => Err(PatternThemeError::type_error(path, "an identifier", value)),
    }
}

#[cfg(test)]
mod tests {
    use avenger_scenegraph::marks::pattern::PatternInk;

    use super::*;

    #[test]
    fn lowers_stripe_and_symbol_pattern_entries() {
        let value = ThemeValue::List(vec![
            ThemeValue::Object(IndexMap::from([
                ("anchor".to_string(), ThemeValue::String("plot".to_string())),
                (
                    "ink".to_string(),
                    ThemeValue::Object(IndexMap::from([
                        (
                            "type".to_string(),
                            ThemeValue::String("auto-contrast".to_string()),
                        ),
                        ("opacity".to_string(), ThemeValue::Number(0.13)),
                    ])),
                ),
                (
                    "layers".to_string(),
                    ThemeValue::Array(vec![ThemeValue::Object(IndexMap::from([
                        ("type".to_string(), ThemeValue::String("stripe".to_string())),
                        (
                            "angle".to_string(),
                            ThemeValue::Angle(45.0, crate::theme::AngleUnit::Deg),
                        ),
                        (
                            "spacing".to_string(),
                            ThemeValue::Length(16.0, crate::theme::LengthUnit::Px),
                        ),
                        (
                            "stroke-width".to_string(),
                            ThemeValue::Length(1.25, crate::theme::LengthUnit::Px),
                        ),
                    ]))]),
                ),
            ])),
            ThemeValue::None,
            ThemeValue::Object(IndexMap::from([(
                "layers".to_string(),
                ThemeValue::Array(vec![ThemeValue::Object(IndexMap::from([
                    ("type".to_string(), ThemeValue::String("symbol".to_string())),
                    (
                        "lattice".to_string(),
                        ThemeValue::Object(IndexMap::from([
                            (
                                "u-spacing".to_string(),
                                ThemeValue::Length(12.0, crate::theme::LengthUnit::Px),
                            ),
                            (
                                "u-angle".to_string(),
                                ThemeValue::Angle(0.0, crate::theme::AngleUnit::Deg),
                            ),
                            (
                                "v-spacing".to_string(),
                                ThemeValue::Length(12.0, crate::theme::LengthUnit::Px),
                            ),
                            (
                                "v-angle".to_string(),
                                ThemeValue::Angle(90.0, crate::theme::AngleUnit::Deg),
                            ),
                        ])),
                    ),
                    (
                        "symbol".to_string(),
                        ThemeValue::Object(IndexMap::from([
                            (
                                "shape".to_string(),
                                ThemeValue::String("circle".to_string()),
                            ),
                            (
                                "size".to_string(),
                                ThemeValue::Length(5.0, crate::theme::LengthUnit::Px),
                            ),
                        ])),
                    ),
                    (
                        "paint".to_string(),
                        ThemeValue::Object(IndexMap::from([(
                            "type".to_string(),
                            ThemeValue::String("filled".to_string()),
                        )])),
                    ),
                ]))]),
            )])),
        ]);

        let range =
            pattern_range_from_theme_value(value, &IndexMap::new(), 12.0).expect("valid range");

        let ScaleRange::Pattern(patterns) = range else {
            panic!("expected pattern range");
        };
        assert_eq!(patterns.len(), 3);
        assert!(matches!(
            &patterns[0].as_ref().unwrap().ink,
            PatternInk::AutoContrast { opacity } if (opacity - 0.13).abs() < f32::EPSILON
        ));
        assert!(patterns[1].is_none());
        assert!(matches!(
            &patterns[2].as_ref().unwrap().layers[0],
            PatternLayer::Symbol(_)
        ));
    }

    #[test]
    fn lowering_error_reports_nested_value_path() {
        let value = ThemeValue::Array(vec![ThemeValue::Object(IndexMap::from([(
            "layers".to_string(),
            ThemeValue::Array(vec![ThemeValue::Object(IndexMap::from([
                ("type".to_string(), ThemeValue::String("stripe".to_string())),
                (
                    "angle".to_string(),
                    ThemeValue::Angle(45.0, crate::theme::AngleUnit::Deg),
                ),
                (
                    "spacing".to_string(),
                    ThemeValue::Length(16.0, crate::theme::LengthUnit::Px),
                ),
                (
                    "stroke-width".to_string(),
                    ThemeValue::String("wide".to_string()),
                ),
            ]))]),
        )]))]);

        let err = pattern_range_from_theme_value(value, &IndexMap::new(), 12.0)
            .expect_err("invalid stripe stroke width should report a path");

        assert!(
            err.to_string()
                .contains("fill-pattern-discrete[0].layers[0].stroke-width"),
            "unexpected error: {err}"
        );
    }
}
