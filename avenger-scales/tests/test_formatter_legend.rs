use std::sync::Arc;

use arrow::array::StringArray;
use avenger_scales::scales::{
    quantile::QuantileScale, quantize::QuantizeScale, threshold::ThresholdScale,
};

#[test]
fn test_threshold_legend_with_formatter() {
    let scale = ThresholdScale::configured(
        vec![10.0, 20.0],
        Arc::new(StringArray::from(vec!["small", "medium", "large"])),
    );

    let entries = scale.scale_impl.legend_entries(&scale.config).unwrap();

    // Check that entries are formatted consistently
    println!("Threshold legend entries:");
    for entry in &entries {
        println!("  {}: {:?}", entry.label, entry.representative_value);
    }

    assert_eq!(entries.len(), 3);
    // First entry should be "< 10" (formatted)
    assert!(entries[0].label.starts_with("<"));
    // Middle entry should have a dash
    assert!(entries[1].label.contains(" - "));
    // Last entry should be "≥ 20" (formatted)
    assert!(entries[2].label.starts_with("≥"));
}

#[test]
fn test_quantize_legend_with_formatter() {
    let scale = QuantizeScale::configured(
        (0.0, 99.0),
        Arc::new(StringArray::from(vec!["low", "medium", "high"])),
    );

    let entries = scale.scale_impl.legend_entries(&scale.config).unwrap();

    println!("Quantize legend entries:");
    for entry in &entries {
        println!("  {}: {:?}", entry.label, entry.representative_value);
    }

    assert_eq!(entries.len(), 3);
    // All entries should have " - " separator
    for entry in &entries {
        assert!(entry.label.contains(" - "));
    }
}

#[test]
fn test_quantile_legend_with_formatter() {
    let scale = QuantileScale::configured(
        vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        Arc::new(StringArray::from(vec!["low", "medium", "high"])),
    );

    let entries = scale.scale_impl.legend_entries(&scale.config).unwrap();

    println!("Quantile legend entries:");
    for entry in &entries {
        println!("  {}: {:?}", entry.label, entry.representative_value);
    }

    assert_eq!(entries.len(), 3);
    // All entries should have " - " separator
    for entry in &entries {
        assert!(entry.label.contains(" - "));
    }
}
