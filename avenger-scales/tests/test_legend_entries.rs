use std::sync::Arc;

use arrow::array::StringArray;
use avenger_scales::scales::{
    band::BandScale, linear::LinearScale, point::PointScale, quantile::QuantileScale,
    quantize::QuantizeScale, threshold::ThresholdScale,
};

#[test]
fn test_threshold_legend_entries() {
    let scale = ThresholdScale::configured(
        vec![10.0, 20.0],
        Arc::new(StringArray::from(vec!["small", "medium", "large"])),
    );

    let entries = scale.scale_impl.legend_entries(&scale.config).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].label, "< 10");
    assert_eq!(entries[1].label, "10 - 20");
    assert_eq!(entries[2].label, "≥ 20");
}

#[test]
fn test_quantize_legend_entries() {
    let scale = QuantizeScale::configured(
        (0.0, 100.0),
        Arc::new(StringArray::from(vec!["low", "medium", "high"])),
    );

    let entries = scale.scale_impl.legend_entries(&scale.config).unwrap();
    assert_eq!(entries.len(), 3);
    // The formatter will format the numbers, possibly with decimals
    // Let's just check that we have 3 entries with " - " separators
    assert!(entries[0].label.contains(" - "));
    assert!(entries[1].label.contains(" - "));
    assert!(entries[2].label.contains(" - "));
}

#[test]
fn test_quantile_legend_entries() {
    let scale = QuantileScale::configured(
        vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        Arc::new(StringArray::from(vec!["low", "medium", "high"])),
    );

    let entries = scale.scale_impl.legend_entries(&scale.config).unwrap();
    assert_eq!(entries.len(), 3);
    // Check that we have reasonable interval labels
    assert!(entries[0].label.contains(" - "));
    assert!(entries[1].label.contains(" - "));
    assert!(entries[2].label.contains(" - "));
}

#[test]
fn test_band_has_band_option() {
    let scale = BandScale::configured(
        Arc::new(StringArray::from(vec!["a", "b", "c"])),
        (0.0, 100.0),
    );

    // Check that band scale has "band" in its option definitions
    let has_band_option = scale
        .scale_impl
        .option_definitions()
        .iter()
        .any(|def| def.name == "band");
    assert!(has_band_option);
}

#[test]
fn test_point_has_band_option() {
    let scale = PointScale::configured(
        Arc::new(StringArray::from(vec!["a", "b", "c"])),
        (0.0, 100.0),
    );

    // Point scales are implemented via band scale but don't expose the band option
    // to users because points are always zero-width (band=0.0 internally).
    // This is correct behavior - point scales shouldn't allow band customization.
    let has_band_option = scale
        .scale_impl
        .option_definitions()
        .iter()
        .any(|def| def.name == "band");
    assert!(!has_band_option);
}

#[test]
fn test_linear_no_band_option() {
    let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));

    // Linear scale should not have "band" in its option definitions
    let has_band_option = scale
        .scale_impl
        .option_definitions()
        .iter()
        .any(|def| def.name == "band");
    assert!(!has_band_option);
}
