//! Total ordering comparison for ScalarValue.
//!
//! `ScalarValue` implements `PartialOrd`, but can return `None` for
//! incomparable values such as NaN or mixed types. This module provides a
//! deterministic ordering for domain values, facet slots, and other chart
//! grouping keys.

use std::cmp::Ordering;

use datafusion::common::ScalarValue;

/// Compare two ScalarValues with total ordering.
///
/// Ordering rules:
/// - Null values come first.
/// - Within float types: -Inf < negative numbers < 0 < positive numbers < +Inf < NaN.
/// - Different non-null types fall back to Equal.
/// - Same non-float types use their natural ordering when available.
pub fn scalar_total_cmp(a: &ScalarValue, b: &ScalarValue) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (false, false) => {}
    }

    match (a, b) {
        (ScalarValue::Float32(Some(fa)), ScalarValue::Float32(Some(fb))) => {
            return fa.total_cmp(fb);
        }
        (ScalarValue::Float64(Some(fa)), ScalarValue::Float64(Some(fb))) => {
            return fa.total_cmp(fb);
        }
        _ => {}
    }

    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_ordering() {
        let null = ScalarValue::Utf8(None);
        let value = ScalarValue::Utf8(Some("hello".to_string()));

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

        assert_eq!(scalar_total_cmp(&nan, &neg_inf), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&nan, &neg), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&nan, &zero), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&nan, &pos), Ordering::Greater);
        assert_eq!(scalar_total_cmp(&nan, &pos_inf), Ordering::Greater);

        assert_eq!(scalar_total_cmp(&neg_inf, &neg), Ordering::Less);
        assert_eq!(scalar_total_cmp(&neg, &zero), Ordering::Less);
        assert_eq!(scalar_total_cmp(&zero, &pos), Ordering::Less);
        assert_eq!(scalar_total_cmp(&pos, &pos_inf), Ordering::Less);
    }

    #[test]
    fn test_f32_nan_ordering() {
        let nan = ScalarValue::Float32(Some(f32::NAN));
        let value = ScalarValue::Float32(Some(1.0));

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

        assert_eq!(scalar_total_cmp(&null, &nan), Ordering::Less);
        assert_eq!(scalar_total_cmp(&null, &value), Ordering::Less);
        assert_eq!(scalar_total_cmp(&value, &nan), Ordering::Less);
    }

    #[test]
    fn test_sorting_with_nan_and_null() {
        let mut values = [
            ScalarValue::Float64(Some(2.0)),
            ScalarValue::Float64(None),
            ScalarValue::Float64(Some(f64::NAN)),
            ScalarValue::Float64(Some(1.0)),
            ScalarValue::Float64(Some(f64::NEG_INFINITY)),
        ];

        values.sort_by(scalar_total_cmp);

        assert!(values[0].is_null());
        assert_eq!(values[1], ScalarValue::Float64(Some(f64::NEG_INFINITY)));
        assert_eq!(values[2], ScalarValue::Float64(Some(1.0)));
        assert_eq!(values[3], ScalarValue::Float64(Some(2.0)));
        assert!(matches!(&values[4], ScalarValue::Float64(Some(f)) if f.is_nan()));
    }
}
