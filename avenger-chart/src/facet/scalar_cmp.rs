//! Total ordering comparison for ScalarValue.
//!
//! ScalarValue implements PartialOrd but can return None for incomparable values
//! (e.g., NaN vs number, different types). This module provides a total ordering
//! comparison that defines a consistent order for all ScalarValue variants.

use std::cmp::Ordering;

use datafusion::common::ScalarValue;

/// Compare two ScalarValues with total ordering.
///
/// This function provides a consistent ordering for all ScalarValue variants,
/// handling cases where `partial_cmp` returns None:
///
/// Ordering rules:
/// - Null values come first
/// - Within float types: -Inf < negative numbers < 0 < positive numbers < +Inf < NaN
///   (IEEE 754 total ordering places NaN at the end)
/// - Different types fall back to Equal (same behavior as before)
/// - Same types use their natural ordering when available
///
/// This ensures deterministic sorting of facet domain values regardless of
/// whether they contain nulls, NaN, or mixed types.
pub fn scalar_total_cmp(a: &ScalarValue, b: &ScalarValue) -> Ordering {
    // Handle nulls first - null values sort before non-null
    match (a.is_null(), b.is_null()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (false, false) => {} // Continue to value comparison
    }

    // Handle floats specially using total_cmp to properly order NaN
    // We do this BEFORE partial_cmp because partial_cmp returns None for NaN
    match (a, b) {
        (ScalarValue::Float32(Some(fa)), ScalarValue::Float32(Some(fb))) => {
            return fa.total_cmp(fb);
        }
        (ScalarValue::Float64(Some(fa)), ScalarValue::Float64(Some(fb))) => {
            return fa.total_cmp(fb);
        }
        _ => {}
    }

    // For non-float types, use partial_cmp with Equal fallback
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_ordering() {
        let null = ScalarValue::Utf8(None);
        let value = ScalarValue::Utf8(Some("hello".to_string()));

        // Null comes before non-null
        assert_eq!(scalar_total_cmp(&null, &value), Ordering::Less);
        assert_eq!(scalar_total_cmp(&value, &null), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&null, &null), Ordering::Equal);
    }

    #[test]
    fn test_f64_nan_ordering() {
        let nan = ScalarValue::Float64(Some(f64::NAN));
        let neg_inf = ScalarValue::Float64(Some(f64::NEG_INFINITY));
        let neg = ScalarValue::Float64(Some(-1.0));
        let zero = ScalarValue::Float64(Some(0.0));
        let pos = ScalarValue::Float64(Some(1.0));
        let pos_inf = ScalarValue::Float64(Some(f64::INFINITY));

        // IEEE 754 total ordering: NaN comes AFTER all other values
        assert_eq!(scalar_total_cmp(&nan, &neg_inf), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&nan, &neg), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&nan, &zero), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&nan, &pos), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&nan, &pos_inf), Ordering::Greater);

        // Normal ordering for non-NaN values
        assert_eq!(scalar_total_cmp(&neg_inf, &neg), Ordering::Less);
        assert_eq!(scalar_total_cmp(&neg, &zero), Ordering::Less);
        assert_eq!(scalar_total_cmp(&zero, &pos), Ordering::Less);
        assert_eq!(scalar_total_cmp(&pos, &pos_inf), Ordering::Less);
    }

    #[test]
    fn test_f32_nan_ordering() {
        let nan = ScalarValue::Float32(Some(f32::NAN));
        let value = ScalarValue::Float32(Some(1.0));

        // IEEE 754 total ordering: NaN comes AFTER regular values
        assert_eq!(scalar_total_cmp(&nan, &value), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&value, &nan), Ordering::Less);
    }

    #[test]
    fn test_string_ordering() {
        let a = ScalarValue::Utf8(Some("apple".to_string()));
        let b = ScalarValue::Utf8(Some("banana".to_string()));
        let c = ScalarValue::Utf8(Some("apple".to_string()));

        assert_eq!(scalar_total_cmp(&a, &b), Ordering::Less);
        assert_eq!(scalar_total_cmp(&b, &a), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&a, &c), Ordering::Equal);
    }

    #[test]
    fn test_integer_ordering() {
        let a = ScalarValue::Int64(Some(-10));
        let b = ScalarValue::Int64(Some(0));
        let c = ScalarValue::Int64(Some(10));

        assert_eq!(scalar_total_cmp(&a, &b), Ordering::Less);
        assert_eq!(scalar_total_cmp(&b, &c), Ordering::Less);
        assert_eq!(scalar_total_cmp(&a, &c), Ordering::Less);
    }

    #[test]
    fn test_mixed_null_and_values() {
        let null = ScalarValue::Float64(None);
        let nan = ScalarValue::Float64(Some(f64::NAN));
        let value = ScalarValue::Float64(Some(1.0));

        // Null < value < NaN (IEEE 754 puts NaN at the end)
        assert_eq!(scalar_total_cmp(&null, &nan), Ordering::Less);
        assert_eq!(scalar_total_cmp(&null, &value), Ordering::Less);
        assert_eq!(scalar_total_cmp(&value, &nan), Ordering::Less); // value < NaN
    }

    #[test]
    fn test_sorting_with_nan_and_null() {
        let mut values = vec![
            ScalarValue::Float64(Some(2.0)),
            ScalarValue::Float64(None),
            ScalarValue::Float64(Some(f64::NAN)),
            ScalarValue::Float64(Some(1.0)),
            ScalarValue::Float64(Some(f64::NEG_INFINITY)),
        ];

        values.sort_by(scalar_total_cmp);

        // Expected order: Null, -Inf, 1.0, 2.0, NaN (IEEE 754 puts NaN at end)
        assert!(values[0].is_null());
        assert_eq!(values[1], ScalarValue::Float64(Some(f64::NEG_INFINITY)));
        assert_eq!(values[2], ScalarValue::Float64(Some(1.0)));
        assert_eq!(values[3], ScalarValue::Float64(Some(2.0)));
        assert!(matches!(&values[4], ScalarValue::Float64(Some(f)) if f.is_nan()));
    }
}
