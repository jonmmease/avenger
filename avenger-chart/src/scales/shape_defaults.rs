//! Default shape palettes for scales

use datafusion_common::ScalarValue;

/// The default shape names used for ordinal shape scales
/// These are used both when setting default ranges for shape scales
/// and when creating legends for shape channels
pub const DEFAULT_SHAPES: &[&str] = &[
    "circle",
    "square",
    "triangle-up",
    "cross",
    "diamond",
    "triangle-down",
    "triangle-left",
    "triangle-right",
];

/// Get the default shape names as scalar values, limited to the specified count
pub fn get_default_shape_scalars(count: Option<usize>) -> Vec<ScalarValue> {
    let shapes = match count {
        Some(n) if n <= DEFAULT_SHAPES.len() => &DEFAULT_SHAPES[..n],
        _ => DEFAULT_SHAPES,
    };

    shapes
        .iter()
        .map(|&s| ScalarValue::Utf8(Some(s.to_string())))
        .collect()
}
