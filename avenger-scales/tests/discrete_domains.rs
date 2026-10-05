//! Ordinal, band, and point scales with 64-bit numeric and temporal domains.

use arrow::array::{
    ArrayRef, Date32Array, Date64Array, Float32Array, Float64Array, Int64Array,
    TimestampMillisecondArray, TimestampNanosecondArray,
};
use avenger_scales::scales::{
    band::BandScale, ordinal::OrdinalScale, point::PointScale, ConfiguredScale,
};
use std::sync::Arc;

/// Three-value domains of each type, with values of the same type: the third domain value, the
/// first, and a value outside the domain.
fn domains() -> Vec<(ArrayRef, ArrayRef)> {
    // Midnight UTC on Jan 1, 2024, plus whole days.
    let days = |offsets: [i64; 3]| offsets.map(|day| 1_704_067_200_000 + day * 86_400_000);
    let nanos = |offsets| days(offsets).map(|millis| millis * 1_000_000);
    vec![
        (
            Arc::new(Int64Array::from(vec![10, 20, 30])),
            Arc::new(Int64Array::from(vec![30, 10, 40])),
        ),
        (
            Arc::new(Float64Array::from(vec![0.5, 1.5, 2.5])),
            Arc::new(Float64Array::from(vec![2.5, 0.5, 3.5])),
        ),
        (
            Arc::new(Date32Array::from(vec![19723, 19724, 19725])),
            Arc::new(Date32Array::from(vec![19725, 19723, 19726])),
        ),
        (
            Arc::new(Date64Array::from(days([0, 1, 2]).to_vec())),
            Arc::new(Date64Array::from(days([2, 0, 3]).to_vec())),
        ),
        (
            Arc::new(TimestampMillisecondArray::from(days([0, 1, 2]).to_vec())),
            Arc::new(TimestampMillisecondArray::from(days([2, 0, 3]).to_vec())),
        ),
        (
            Arc::new(
                TimestampNanosecondArray::from(nanos([0, 1, 2]).to_vec())
                    .with_timezone("America/New_York"),
            ),
            Arc::new(
                TimestampNanosecondArray::from(nanos([2, 0, 3]).to_vec())
                    .with_timezone("America/New_York"),
            ),
        ),
    ]
}

fn assert_positions(scale: ConfiguredScale, values: &ArrayRef, expected: [f32; 2]) {
    let positions = scale
        .scale_to_numeric(values)
        .unwrap()
        .as_vec(values.len(), None);
    assert_eq!(positions[..2], expected, "{}", values.data_type());
    assert!(positions[2].is_nan(), "{}", values.data_type());
}

#[test]
fn ordinal_scales_map_64_bit_and_temporal_domains() {
    for (domain, values) in domains() {
        let range = Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef;
        let scale = OrdinalScale::configured(domain).with_range(range);
        assert_positions(scale, &values, [3.0, 1.0]);
    }
}

#[test]
fn band_scales_map_64_bit_and_temporal_domains() {
    for (domain, values) in domains() {
        let scale = BandScale::configured(domain, (0.0, 300.0));
        assert_positions(scale, &values, [200.0, 0.0]);
    }
}

#[test]
fn point_scales_map_64_bit_and_temporal_domains() {
    for (domain, values) in domains() {
        let scale = PointScale::configured(domain, (0.0, 200.0));
        assert_positions(scale, &values, [200.0, 0.0]);
    }
}
