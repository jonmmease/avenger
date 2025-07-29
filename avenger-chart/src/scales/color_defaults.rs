//! Default color palettes for scales

use crate::scales::ScaleRange;
use datafusion_common::ScalarValue;
use palette::Srgba;

/// Get default color range based on scale type and domain cardinality
pub fn get_default_color_range(scale_type: &str, domain_cardinality: Option<usize>) -> ScaleRange {
    match scale_type {
        "ordinal" => {
            // Category10 palette from D3 for categorical data
            let colors = vec![
                ScalarValue::Utf8(Some("#1f77b4".to_string())), // Blue
                ScalarValue::Utf8(Some("#ff7f0e".to_string())), // Orange
                ScalarValue::Utf8(Some("#2ca02c".to_string())), // Green
                ScalarValue::Utf8(Some("#d62728".to_string())), // Red
                ScalarValue::Utf8(Some("#9467bd".to_string())), // Purple
                ScalarValue::Utf8(Some("#8c564b".to_string())), // Brown
                ScalarValue::Utf8(Some("#e377c2".to_string())), // Pink
                ScalarValue::Utf8(Some("#7f7f7f".to_string())), // Gray
                ScalarValue::Utf8(Some("#bcbd22".to_string())), // Olive
                ScalarValue::Utf8(Some("#17becf".to_string())), // Cyan
            ];

            // If we know the domain cardinality, only return that many colors
            // Otherwise return all 10
            match domain_cardinality {
                Some(n) if n <= colors.len() => {
                    ScaleRange::Enum(colors.into_iter().take(n).collect())
                }
                _ => ScaleRange::Enum(colors),
            }
        }

        "linear" | "log" | "pow" | "sqrt" => {
            // Viridis-inspired gradient for continuous scales
            ScaleRange::Color(vec![
                Srgba::new(0.267, 0.004, 0.329, 1.0), // Dark purple (#440154)
                Srgba::new(0.193, 0.408, 0.556, 1.0), // Blue (#31688e)
                Srgba::new(0.208, 0.718, 0.473, 1.0), // Green (#35b779)
                Srgba::new(0.993, 0.906, 0.144, 1.0), // Yellow (#fde725)
            ])
        }

        "quantize" | "quantile" => {
            // Color brewer RdYlBu for quantized scales
            ScaleRange::Enum(vec![
                ScalarValue::Utf8(Some("#a50026".to_string())), // Dark red
                ScalarValue::Utf8(Some("#d73027".to_string())), // Red
                ScalarValue::Utf8(Some("#f46d43".to_string())), // Orange red
                ScalarValue::Utf8(Some("#fdae61".to_string())), // Light orange
                ScalarValue::Utf8(Some("#fee090".to_string())), // Pale yellow
                ScalarValue::Utf8(Some("#e0f3f8".to_string())), // Pale blue
                ScalarValue::Utf8(Some("#abd9e9".to_string())), // Light blue
                ScalarValue::Utf8(Some("#74add1".to_string())), // Blue
                ScalarValue::Utf8(Some("#4575b4".to_string())), // Dark blue
                ScalarValue::Utf8(Some("#313695".to_string())), // Very dark blue
            ])
        }

        _ => {
            // Default single color
            ScaleRange::Enum(vec![ScalarValue::Utf8(Some("#4682b4".to_string()))]) // Steel blue
        }
    }
}

/// Get default color range for a specific encoding channel
pub fn get_default_color_range_for_channel(
    channel: &str,
    scale_type: &str,
    domain_cardinality: Option<usize>,
) -> Option<ScaleRange> {
    match channel {
        "fill" | "stroke" | "color" => {
            Some(get_default_color_range(scale_type, domain_cardinality))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ordinal_color_range() {
        let range = get_default_color_range("ordinal", None);
        match range {
            ScaleRange::Enum(colors) => {
                assert_eq!(colors.len(), 10);
            }
            _ => panic!("Expected enum range for ordinal scale"),
        }
    }

    #[test]
    fn test_ordinal_with_cardinality() {
        let range = get_default_color_range("ordinal", Some(3));
        match range {
            ScaleRange::Enum(colors) => {
                assert_eq!(colors.len(), 3);
            }
            _ => panic!("Expected enum range for ordinal scale"),
        }
    }

    #[test]
    fn test_linear_color_range() {
        let range = get_default_color_range("linear", None);
        match range {
            ScaleRange::Color(colors) => {
                assert_eq!(colors.len(), 4);
            }
            _ => panic!("Expected color range for linear scale"),
        }
    }

    #[test]
    fn test_channel_color_range() {
        assert!(get_default_color_range_for_channel("fill", "ordinal", None).is_some());
        assert!(get_default_color_range_for_channel("stroke", "linear", None).is_some());
        assert!(get_default_color_range_for_channel("color", "threshold", None).is_some());
        assert!(get_default_color_range_for_channel("x", "linear", None).is_none());
    }
}
