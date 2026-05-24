//! CSS Media Query support
//!
//! Implements CSS Media Queries Level 4 for responsive theming.
//! Initially supports width and height features.

use datafusion_common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A dimension value with unit information
///
/// Stores the numeric value and its unit type for later conversion
/// to pixels using the appropriate base font size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DimensionValue {
    /// Absolute pixels
    Pixels(f32),
    /// Relative to root font size
    Rem(f32),
}

impl DimensionValue {
    /// Convert to pixels using the given base font size
    pub fn to_pixels(&self, base_font_size: f32) -> f32 {
        match self {
            Self::Pixels(px) => *px,
            Self::Rem(rem) => rem * base_font_size,
        }
    }
}

/// CSS media query condition
///
/// Represents the logical structure of a media query condition that can be
/// evaluated against runtime parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaCondition {
    /// A feature test like (width >= 400px)
    Feature(MediaFeature),
    /// Logical NOT
    Not(Box<MediaCondition>),
    /// Logical AND - all conditions must match
    And(Vec<MediaCondition>),
    /// Logical OR - any condition must match
    Or(Vec<MediaCondition>),
}

impl MediaCondition {
    /// Evaluate condition against params with base font size
    ///
    /// Returns true if the condition matches, false otherwise.
    /// If a required param is not available, returns false (graceful degradation).
    pub fn evaluate(&self, params: &IndexMap<String, ScalarValue>, base_font_size: f32) -> bool {
        match self {
            Self::Feature(f) => f.evaluate(params, base_font_size),
            Self::Not(cond) => !cond.evaluate(params, base_font_size),
            Self::And(conds) => conds.iter().all(|c| c.evaluate(params, base_font_size)),
            Self::Or(conds) => conds.iter().any(|c| c.evaluate(params, base_font_size)),
        }
    }
}

/// A media feature expression
///
/// Represents a test against a media feature like width or height.
/// Supports both single comparisons and multi-range syntax.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaFeature {
    /// Single comparison: width >= 600px
    Single {
        name: String,
        op: MediaOperator,
        value: DimensionValue,
    },
    /// Multi-range comparison: 600px <= width < 1200px
    Range {
        name: String,
        left_value: DimensionValue,
        left_op: MediaOperator,
        right_op: MediaOperator,
        right_value: DimensionValue,
    },
}

impl MediaFeature {
    /// Create a new single-comparison media feature
    #[allow(dead_code)] // Will be used by parser
    pub fn new(name: impl Into<String>, op: MediaOperator, value: DimensionValue) -> Self {
        Self::Single {
            name: name.into(),
            op,
            value,
        }
    }

    /// Create a new range media feature
    #[allow(dead_code)] // Will be used by parser
    pub fn new_range(
        name: impl Into<String>,
        left_value: DimensionValue,
        left_op: MediaOperator,
        right_op: MediaOperator,
        right_value: DimensionValue,
    ) -> Self {
        Self::Range {
            name: name.into(),
            left_value,
            left_op,
            right_op,
            right_value,
        }
    }

    /// Evaluate feature against params with base font size
    ///
    /// Extracts the feature value from params and compares using the operator.
    /// Returns false if the feature is not available (graceful degradation).
    pub fn evaluate(&self, params: &IndexMap<String, ScalarValue>, base_font_size: f32) -> bool {
        match self {
            Self::Single { name, op, value } => {
                // Extract width or height from params
                let actual_value = match params.get(name) {
                    Some(ScalarValue::Float32(Some(v))) => *v,
                    Some(ScalarValue::Float64(Some(v))) => *v as f32,
                    Some(ScalarValue::Int32(Some(v))) => *v as f32,
                    Some(ScalarValue::Int64(Some(v))) => *v as f32,
                    Some(ScalarValue::UInt32(Some(v))) => *v as f32,
                    Some(ScalarValue::UInt64(Some(v))) => *v as f32,
                    _ => return false, // Feature not available → condition is false
                };

                // Convert comparison value to pixels using base font size
                let comparison_px = value.to_pixels(base_font_size);

                // Compare using operator
                op.compare(actual_value, comparison_px)
            }
            Self::Range {
                name,
                left_value,
                left_op,
                right_op,
                right_value,
            } => {
                // Extract width or height from params
                let actual_value = match params.get(name) {
                    Some(ScalarValue::Float32(Some(v))) => *v,
                    Some(ScalarValue::Float64(Some(v))) => *v as f32,
                    Some(ScalarValue::Int32(Some(v))) => *v as f32,
                    Some(ScalarValue::Int64(Some(v))) => *v as f32,
                    Some(ScalarValue::UInt32(Some(v))) => *v as f32,
                    Some(ScalarValue::UInt64(Some(v))) => *v as f32,
                    _ => return false, // Feature not available → condition is false
                };

                // Convert comparison values to pixels using base font size
                let left_px = left_value.to_pixels(base_font_size);
                let right_px = right_value.to_pixels(base_font_size);

                // Evaluate left comparison: left_value left_op actual
                let left_ok = left_op.compare(left_px, actual_value);
                // Evaluate right comparison: actual right_op right_value
                let right_ok = right_op.compare(actual_value, right_px);
                left_ok && right_ok
            }
        }
    }
}

/// Comparison operators for media features
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaOperator {
    /// Equality: width: 800px
    Equal,
    /// Greater than: width > 600px
    GreaterThan,
    /// Greater than or equal: width >= 600px
    GreaterEqual,
    /// Less than: width < 1200px
    LessThan,
    /// Less than or equal: width <= 1200px
    LessEqual,
}

impl MediaOperator {
    /// Compare two values using this operator
    pub fn compare(&self, left: f32, right: f32) -> bool {
        match self {
            Self::Equal => (left - right).abs() < 0.5,
            Self::GreaterThan => left > right,
            Self::GreaterEqual => left >= right,
            Self::LessThan => left < right,
            Self::LessEqual => left <= right,
        }
    }

    /// Check if this operator can be used with another in a multi-range
    ///
    /// Per CSS spec, both operators must be in the same direction.
    /// Valid: `<`/`<=` with `<`/`<=`, or `>`/`>=` with `>`/`>=`
    /// Invalid: mixing `<` with `>`, or using `=` in ranges
    pub fn is_compatible_with(&self, other: Self) -> bool {
        match self {
            Self::Equal => false, // = cannot be in multi-range
            Self::GreaterThan | Self::GreaterEqual => {
                matches!(other, Self::GreaterThan | Self::GreaterEqual)
            }
            Self::LessThan | Self::LessEqual => {
                matches!(other, Self::LessThan | Self::LessEqual)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params_with_dimensions(width: f32, height: f32) -> IndexMap<String, ScalarValue> {
        let mut params = IndexMap::new();
        params.insert("width".to_string(), ScalarValue::Float32(Some(width)));
        params.insert("height".to_string(), ScalarValue::Float32(Some(height)));
        params
    }

    #[test]
    fn test_feature_equal() {
        let feature =
            MediaFeature::new("width", MediaOperator::Equal, DimensionValue::Pixels(800.0));
        let params = params_with_dimensions(800.0, 600.0);
        assert!(feature.evaluate(&params, 16.0));

        let params2 = params_with_dimensions(799.0, 600.0);
        assert!(!feature.evaluate(&params2, 16.0));
    }

    #[test]
    fn test_feature_greater_than() {
        let feature = MediaFeature::new(
            "width",
            MediaOperator::GreaterThan,
            DimensionValue::Pixels(600.0),
        );

        let params = params_with_dimensions(800.0, 600.0);
        assert!(feature.evaluate(&params, 16.0));

        let params2 = params_with_dimensions(600.0, 600.0);
        assert!(!feature.evaluate(&params2, 16.0));

        let params3 = params_with_dimensions(400.0, 600.0);
        assert!(!feature.evaluate(&params3, 16.0));
    }

    #[test]
    fn test_feature_greater_equal() {
        let feature = MediaFeature::new(
            "width",
            MediaOperator::GreaterEqual,
            DimensionValue::Pixels(600.0),
        );

        let params = params_with_dimensions(800.0, 600.0);
        assert!(feature.evaluate(&params, 16.0));

        let params2 = params_with_dimensions(600.0, 600.0);
        assert!(feature.evaluate(&params2, 16.0));

        let params3 = params_with_dimensions(400.0, 600.0);
        assert!(!feature.evaluate(&params3, 16.0));
    }

    #[test]
    fn test_feature_less_than() {
        let feature = MediaFeature::new(
            "width",
            MediaOperator::LessThan,
            DimensionValue::Pixels(600.0),
        );

        let params = params_with_dimensions(400.0, 600.0);
        assert!(feature.evaluate(&params, 16.0));

        let params2 = params_with_dimensions(600.0, 600.0);
        assert!(!feature.evaluate(&params2, 16.0));

        let params3 = params_with_dimensions(800.0, 600.0);
        assert!(!feature.evaluate(&params3, 16.0));
    }

    #[test]
    fn test_feature_less_equal() {
        let feature = MediaFeature::new(
            "width",
            MediaOperator::LessEqual,
            DimensionValue::Pixels(600.0),
        );

        let params = params_with_dimensions(400.0, 600.0);
        assert!(feature.evaluate(&params, 16.0));

        let params2 = params_with_dimensions(600.0, 600.0);
        assert!(feature.evaluate(&params2, 16.0));

        let params3 = params_with_dimensions(800.0, 600.0);
        assert!(!feature.evaluate(&params3, 16.0));
    }

    #[test]
    fn test_feature_missing_param() {
        let feature = MediaFeature::new(
            "width",
            MediaOperator::GreaterEqual,
            DimensionValue::Pixels(600.0),
        );

        // No params at all
        let params = IndexMap::new();
        assert!(!feature.evaluate(&params, 16.0));

        // Only height, no width
        let mut params2 = IndexMap::new();
        params2.insert("height".to_string(), ScalarValue::Float32(Some(600.0)));
        assert!(!feature.evaluate(&params2, 16.0));
    }

    #[test]
    fn test_condition_and() {
        let cond = MediaCondition::And(vec![
            MediaCondition::Feature(MediaFeature::new(
                "width",
                MediaOperator::GreaterEqual,
                DimensionValue::Pixels(600.0),
            )),
            MediaCondition::Feature(MediaFeature::new(
                "height",
                MediaOperator::GreaterEqual,
                DimensionValue::Pixels(400.0),
            )),
        ]);

        // Both true
        let params = params_with_dimensions(800.0, 600.0);
        assert!(cond.evaluate(&params, 16.0));

        // First true, second false
        let params2 = params_with_dimensions(800.0, 300.0);
        assert!(!cond.evaluate(&params2, 16.0));

        // First false, second true
        let params3 = params_with_dimensions(400.0, 600.0);
        assert!(!cond.evaluate(&params3, 16.0));

        // Both false
        let params4 = params_with_dimensions(400.0, 300.0);
        assert!(!cond.evaluate(&params4, 16.0));
    }

    #[test]
    fn test_condition_or() {
        let cond = MediaCondition::Or(vec![
            MediaCondition::Feature(MediaFeature::new(
                "width",
                MediaOperator::LessThan,
                DimensionValue::Pixels(400.0),
            )),
            MediaCondition::Feature(MediaFeature::new(
                "height",
                MediaOperator::LessThan,
                DimensionValue::Pixels(300.0),
            )),
        ]);

        // Both true
        let params = params_with_dimensions(300.0, 200.0);
        assert!(cond.evaluate(&params, 16.0));

        // First true, second false
        let params2 = params_with_dimensions(300.0, 500.0);
        assert!(cond.evaluate(&params2, 16.0));

        // First false, second true
        let params3 = params_with_dimensions(500.0, 200.0);
        assert!(cond.evaluate(&params3, 16.0));

        // Both false
        let params4 = params_with_dimensions(500.0, 500.0);
        assert!(!cond.evaluate(&params4, 16.0));
    }

    #[test]
    fn test_condition_not() {
        let cond = MediaCondition::Not(Box::new(MediaCondition::Feature(MediaFeature::new(
            "width",
            MediaOperator::LessThan,
            DimensionValue::Pixels(600.0),
        ))));

        // Inner condition false → not returns true
        let params = params_with_dimensions(800.0, 600.0);
        assert!(cond.evaluate(&params, 16.0));

        // Inner condition true → not returns false
        let params2 = params_with_dimensions(400.0, 600.0);
        assert!(!cond.evaluate(&params2, 16.0));
    }

    #[test]
    fn test_condition_complex() {
        // (width >= 600px) and ((height >= 400px) or (height <= 200px))
        let cond = MediaCondition::And(vec![
            MediaCondition::Feature(MediaFeature::new(
                "width",
                MediaOperator::GreaterEqual,
                DimensionValue::Pixels(600.0),
            )),
            MediaCondition::Or(vec![
                MediaCondition::Feature(MediaFeature::new(
                    "height",
                    MediaOperator::GreaterEqual,
                    DimensionValue::Pixels(400.0),
                )),
                MediaCondition::Feature(MediaFeature::new(
                    "height",
                    MediaOperator::LessEqual,
                    DimensionValue::Pixels(200.0),
                )),
            ]),
        ]);

        // width >= 600 and height >= 400
        let params = params_with_dimensions(800.0, 500.0);
        assert!(cond.evaluate(&params, 16.0));

        // width >= 600 and height <= 200
        let params2 = params_with_dimensions(800.0, 150.0);
        assert!(cond.evaluate(&params2, 16.0));

        // width >= 600 but height in middle range
        let params3 = params_with_dimensions(800.0, 300.0);
        assert!(!cond.evaluate(&params3, 16.0));

        // width < 600
        let params4 = params_with_dimensions(400.0, 500.0);
        assert!(!cond.evaluate(&params4, 16.0));
    }

    #[test]
    fn test_different_scalar_types() {
        let feature = MediaFeature::new(
            "width",
            MediaOperator::GreaterEqual,
            DimensionValue::Pixels(600.0),
        );

        // Float32
        let mut params = IndexMap::new();
        params.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));
        assert!(feature.evaluate(&params, 16.0));

        // Float64
        let mut params = IndexMap::new();
        params.insert("width".to_string(), ScalarValue::Float64(Some(800.0)));
        assert!(feature.evaluate(&params, 16.0));

        // Int32
        let mut params = IndexMap::new();
        params.insert("width".to_string(), ScalarValue::Int32(Some(800)));
        assert!(feature.evaluate(&params, 16.0));

        // Int64
        let mut params = IndexMap::new();
        params.insert("width".to_string(), ScalarValue::Int64(Some(800)));
        assert!(feature.evaluate(&params, 16.0));

        // UInt32
        let mut params = IndexMap::new();
        params.insert("width".to_string(), ScalarValue::UInt32(Some(800)));
        assert!(feature.evaluate(&params, 16.0));

        // UInt64
        let mut params = IndexMap::new();
        params.insert("width".to_string(), ScalarValue::UInt64(Some(800)));
        assert!(feature.evaluate(&params, 16.0));
    }

    #[test]
    fn test_range_feature_in_range() {
        let feature = MediaFeature::Range {
            name: "width".to_string(),
            left_value: DimensionValue::Pixels(600.0),
            left_op: MediaOperator::LessEqual,
            right_op: MediaOperator::LessThan,
            right_value: DimensionValue::Pixels(1200.0),
        };

        // 800 is in range [600, 1200)
        let params = params_with_dimensions(800.0, 400.0);
        assert!(feature.evaluate(&params, 16.0));

        // 600 is in range [600, 1200)
        let params2 = params_with_dimensions(600.0, 400.0);
        assert!(feature.evaluate(&params2, 16.0));

        // 1199 is in range [600, 1200)
        let params3 = params_with_dimensions(1199.0, 400.0);
        assert!(feature.evaluate(&params3, 16.0));
    }

    #[test]
    fn test_range_feature_below_range() {
        let feature = MediaFeature::Range {
            name: "width".to_string(),
            left_value: DimensionValue::Pixels(600.0),
            left_op: MediaOperator::LessEqual,
            right_op: MediaOperator::LessThan,
            right_value: DimensionValue::Pixels(1200.0),
        };

        // 400 is below range [600, 1200)
        let params = params_with_dimensions(400.0, 400.0);
        assert!(!feature.evaluate(&params, 16.0));

        // 599 is below range [600, 1200)
        let params2 = params_with_dimensions(599.0, 400.0);
        assert!(!feature.evaluate(&params2, 16.0));
    }

    #[test]
    fn test_range_feature_above_range() {
        let feature = MediaFeature::Range {
            name: "width".to_string(),
            left_value: DimensionValue::Pixels(600.0),
            left_op: MediaOperator::LessEqual,
            right_op: MediaOperator::LessThan,
            right_value: DimensionValue::Pixels(1200.0),
        };

        // 1200 is not in range [600, 1200)
        let params = params_with_dimensions(1200.0, 400.0);
        assert!(!feature.evaluate(&params, 16.0));

        // 1500 is above range [600, 1200)
        let params2 = params_with_dimensions(1500.0, 400.0);
        assert!(!feature.evaluate(&params2, 16.0));
    }

    #[test]
    fn test_range_feature_exclusive_boundaries() {
        let feature = MediaFeature::Range {
            name: "width".to_string(),
            left_value: DimensionValue::Pixels(600.0),
            left_op: MediaOperator::LessThan,
            right_op: MediaOperator::LessThan,
            right_value: DimensionValue::Pixels(1200.0),
        };

        // 600 is not in range (600, 1200)
        let params = params_with_dimensions(600.0, 400.0);
        assert!(!feature.evaluate(&params, 16.0));

        // 601 is in range (600, 1200)
        let params2 = params_with_dimensions(601.0, 400.0);
        assert!(feature.evaluate(&params2, 16.0));
    }

    #[test]
    fn test_operator_compatibility() {
        assert!(MediaOperator::LessThan.is_compatible_with(MediaOperator::LessEqual));
        assert!(MediaOperator::LessEqual.is_compatible_with(MediaOperator::LessThan));
        assert!(MediaOperator::LessEqual.is_compatible_with(MediaOperator::LessEqual));

        assert!(MediaOperator::GreaterThan.is_compatible_with(MediaOperator::GreaterEqual));
        assert!(MediaOperator::GreaterEqual.is_compatible_with(MediaOperator::GreaterThan));
        assert!(MediaOperator::GreaterEqual.is_compatible_with(MediaOperator::GreaterEqual));

        // Incompatible: mixing < and >
        assert!(!MediaOperator::LessThan.is_compatible_with(MediaOperator::GreaterThan));
        assert!(!MediaOperator::LessEqual.is_compatible_with(MediaOperator::GreaterEqual));

        // Equal cannot be in multi-range
        assert!(!MediaOperator::Equal.is_compatible_with(MediaOperator::Equal));
        assert!(!MediaOperator::Equal.is_compatible_with(MediaOperator::LessThan));
        assert!(!MediaOperator::Equal.is_compatible_with(MediaOperator::GreaterThan));
    }

    #[test]
    fn test_dimension_value_rem_conversion() {
        let dim = DimensionValue::Rem(2.0);
        assert_eq!(dim.to_pixels(16.0), 32.0);
        assert_eq!(dim.to_pixels(20.0), 40.0);
    }

    #[test]
    fn test_media_feature_with_rem_value() {
        let feature = MediaFeature::Single {
            name: "width".to_string(),
            op: MediaOperator::GreaterEqual,
            value: DimensionValue::Rem(30.0), // 30rem
        };

        let params = params_with_dimensions(600.0, 400.0);

        // With 16px base: 30rem = 480px, 600px >= 480px → true
        assert!(feature.evaluate(&params, 16.0));

        // With 20px base: 30rem = 600px, 600px >= 600px → true
        assert!(feature.evaluate(&params, 20.0));

        // With 25px base: 30rem = 750px, 600px >= 750px → false
        assert!(!feature.evaluate(&params, 25.0));
    }

    #[test]
    fn test_range_with_rem_values() {
        let feature = MediaFeature::Range {
            name: "width".to_string(),
            left_value: DimensionValue::Rem(30.0), // 30rem
            left_op: MediaOperator::LessEqual,
            right_op: MediaOperator::LessThan,
            right_value: DimensionValue::Rem(50.0), // 50rem
        };

        let params = params_with_dimensions(600.0, 400.0);

        // With 16px base: 30rem = 480px, 50rem = 800px, 480px <= 600px < 800px → true
        assert!(feature.evaluate(&params, 16.0));

        // With 20px base: 30rem = 600px, 50rem = 1000px, 600px <= 600px < 1000px → true
        assert!(feature.evaluate(&params, 20.0));

        // With 25px base: 30rem = 750px, 50rem = 1250px, 750px <= 600px < 1250px → false
        assert!(!feature.evaluate(&params, 25.0));
    }
}
