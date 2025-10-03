//! Default scale creation using theme and trait-based type detection

use crate::error::AvengerChartError;
use crate::render::RenderContext;
use crate::scales::spec::ScaleSpec;
use crate::scales::{Auto, Scale, ScaleRange};
use crate::serialization::SerializableScalar;
use avenger_scales::scales::RangeKind;
use datafusion::prelude::lit;
use palette::Srgba;

/// Returns the default discrete color range using the Okabe-Ito palette
///
/// # Arguments
/// * `domain_cardinality` - Optional number of colors to return. If `None`, returns all colors.
pub fn default_color_range_discrete(domain_cardinality: Option<usize>) -> ScaleRange {
    use crate::theme::DEFAULT_CATEGORICAL_COLORS;

    let colors: Vec<String> = DEFAULT_CATEGORICAL_COLORS
        .iter()
        .map(|s| s.to_string())
        .collect();

    let scalars: Vec<SerializableScalar> = colors
        .iter()
        .take(domain_cardinality.unwrap_or(colors.len()))
        .map(|c| SerializableScalar::new(datafusion_common::ScalarValue::Utf8(Some(c.clone()))))
        .collect();
    ScaleRange::Discrete(scalars)
}

/// Returns the default continuous color range (viridis-like gradient)
pub fn default_color_range_continuous() -> ScaleRange {
    let colors = vec![
        Srgba::new(0.267, 0.004, 0.329, 1.0), // Dark purple
        Srgba::new(0.193, 0.408, 0.556, 1.0), // Blue
        Srgba::new(0.208, 0.718, 0.473, 1.0), // Green
        Srgba::new(0.993, 0.906, 0.144, 1.0), // Yellow
    ];
    ScaleRange::new_color(colors)
}

/// Returns the default discrete size range
///
/// # Arguments
/// * `domain_cardinality` - Optional number of sizes to return. If `None`, returns all sizes.
pub fn default_size_range_discrete(domain_cardinality: Option<usize>) -> ScaleRange {
    let sizes = vec![20.0, 40.0, 60.0, 80.0, 100.0];
    let scalars: Vec<SerializableScalar> = sizes
        .iter()
        .take(domain_cardinality.unwrap_or(sizes.len()))
        .map(|s| SerializableScalar::new(datafusion_common::ScalarValue::Float32(Some(*s as f32))))
        .collect();
    ScaleRange::Discrete(scalars)
}

/// Returns the default continuous size range (10.0 to 200.0)
pub fn default_size_range_continuous() -> ScaleRange {
    ScaleRange::new_interval(lit(10.0), lit(200.0))
}

/// Returns the default discrete opacity range
///
/// # Arguments
/// * `domain_cardinality` - Optional number of opacities to return. If `None`, returns all opacities.
pub fn default_opacity_range_discrete(domain_cardinality: Option<usize>) -> ScaleRange {
    let opacities = vec![0.3, 0.5, 0.7, 0.9, 1.0];
    let scalars: Vec<SerializableScalar> = opacities
        .iter()
        .take(domain_cardinality.unwrap_or(opacities.len()))
        .map(|o| SerializableScalar::new(datafusion_common::ScalarValue::Float32(Some(*o as f32))))
        .collect();
    ScaleRange::Discrete(scalars)
}

/// Returns the default continuous opacity range (0.2 to 1.0)
pub fn default_opacity_range_continuous() -> ScaleRange {
    ScaleRange::new_interval(lit(0.2), lit(1.0))
}

/// Returns the default discrete stroke width range
///
/// # Arguments
/// * `domain_cardinality` - Optional number of widths to return. If `None`, returns all widths.
pub fn default_stroke_width_range_discrete(domain_cardinality: Option<usize>) -> ScaleRange {
    let widths = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let scalars: Vec<SerializableScalar> = widths
        .iter()
        .take(domain_cardinality.unwrap_or(widths.len()))
        .map(|w| SerializableScalar::new(datafusion_common::ScalarValue::Float32(Some(*w as f32))))
        .collect();
    ScaleRange::Discrete(scalars)
}

/// Returns the default continuous stroke width range (0.5 to 5.0)
pub fn default_stroke_width_range_continuous() -> ScaleRange {
    ScaleRange::new_interval(lit(0.5), lit(5.0))
}

/// Returns the default discrete shape range
///
/// # Arguments
/// * `domain_cardinality` - Optional number of shapes to return. If `None`, returns all shapes.
pub fn default_shape_range_discrete(domain_cardinality: Option<usize>) -> ScaleRange {
    let shapes = vec!["circle", "square", "triangle", "diamond", "cross"];
    let scalars: Vec<SerializableScalar> = shapes
        .iter()
        .take(domain_cardinality.unwrap_or(shapes.len()))
        .map(|s| SerializableScalar::new(datafusion_common::ScalarValue::Utf8(Some(s.to_string()))))
        .collect();
    ScaleRange::Discrete(scalars)
}

/// Returns a generic default discrete range with a single value (1.0)
pub fn default_generic_range_discrete() -> ScaleRange {
    ScaleRange::Discrete(vec![SerializableScalar::new(
        datafusion_common::ScalarValue::Float32(Some(1.0)),
    )])
}

/// Returns a generic default continuous range (0.0 to 1.0)
pub fn default_generic_range_continuous() -> ScaleRange {
    ScaleRange::new_interval(lit(0.0), lit(1.0))
}

/// Returns default range for a channel based on channel name and range kind
pub fn default_range_for_channel(channel: &str, range_kind: RangeKind) -> ScaleRange {
    match (channel, range_kind) {
        // Color channels
        ("fill" | "stroke" | "color", RangeKind::Discrete) => default_color_range_discrete(None),
        ("fill" | "stroke" | "color", RangeKind::Continuous) => default_color_range_continuous(),

        // Size channels
        ("size", RangeKind::Discrete) => default_size_range_discrete(None),
        ("size", RangeKind::Continuous) => default_size_range_continuous(),

        // Opacity channels
        ("opacity", RangeKind::Discrete) => default_opacity_range_discrete(None),
        ("opacity", RangeKind::Continuous) => default_opacity_range_continuous(),

        // Stroke width channels
        ("stroke_width", RangeKind::Discrete) => default_stroke_width_range_discrete(None),
        ("stroke_width", RangeKind::Continuous) => default_stroke_width_range_continuous(),

        // Shape channel (always discrete)
        ("shape", _) => default_shape_range_discrete(None),

        // Angle channel (always continuous, in degrees)
        ("angle", _) => ScaleRange::new_interval(lit(0.0), lit(360.0)),

        // Generic fallback
        _ => match range_kind {
            RangeKind::Discrete => default_generic_range_discrete(),
            RangeKind::Continuous => default_generic_range_continuous(),
        },
    }
}
