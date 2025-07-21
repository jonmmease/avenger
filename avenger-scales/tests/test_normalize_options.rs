use arrow::array::{Float32Array, StringArray};
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scales::scales::point::PointScale;
use std::sync::Arc;

#[test]
fn test_point_scale_normalize_no_unsupported_options() -> Result<(), AvengerScaleError> {
    // Create a point scale
    let domain = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    let scale = PointScale::configured(domain, (0.0, 100.0))
        .with_option("padding", 0.1)
        .with_option("align", 0.5);

    // Normalize the scale
    let normalized = scale.normalize()?;

    // Check that only supported options remain
    let options = normalized.config.options;

    // Point scale supports: align, padding, round, range_offset
    // It should NOT have zero or nice options added
    assert!(
        !options.contains_key("zero"),
        "Point scale should not have 'zero' option after normalization"
    );
    assert!(
        !options.contains_key("nice"),
        "Point scale should not have 'nice' option after normalization"
    );

    // It should still have the supported options
    assert!(options.contains_key("padding"));
    assert!(options.contains_key("align"));

    Ok(())
}

#[test]
fn test_band_scale_normalize_no_unsupported_options() -> Result<(), AvengerScaleError> {
    // Create a band scale
    let domain = Arc::new(StringArray::from(vec!["x", "y", "z"]));
    let scale = BandScale::configured(domain, (0.0, 200.0))
        .with_option("padding_inner", 0.1)
        .with_option("round", true);

    // Normalize the scale
    let normalized = scale.normalize()?;

    // Check that only supported options remain
    let options = normalized.config.options;

    // Band scale supports: align, band, padding, padding_inner, padding_outer, round, range_offset
    // It should NOT have zero or nice options added
    assert!(
        !options.contains_key("zero"),
        "Band scale should not have 'zero' option after normalization"
    );
    assert!(
        !options.contains_key("nice"),
        "Band scale should not have 'nice' option after normalization"
    );

    Ok(())
}

#[test]
fn test_ordinal_scale_normalize_no_unsupported_options() -> Result<(), AvengerScaleError> {
    // Create an ordinal scale
    let domain = Arc::new(StringArray::from(vec!["red", "green", "blue"]));
    let scale = OrdinalScale::configured(domain)
        .with_range(Arc::new(Float32Array::from(vec![0.0, 0.5, 1.0])));

    // Normalize the scale
    let normalized = scale.normalize()?;

    // Check that only supported options remain
    let options = normalized.config.options;

    // Ordinal scale only supports: default
    // It should NOT have zero, nice, or padding options added
    assert!(
        !options.contains_key("zero"),
        "Ordinal scale should not have 'zero' option after normalization"
    );
    assert!(
        !options.contains_key("nice"),
        "Ordinal scale should not have 'nice' option after normalization"
    );
    assert!(
        !options.contains_key("padding"),
        "Ordinal scale should not have 'padding' option after normalization"
    );

    Ok(())
}

#[test]
fn test_linear_scale_normalize_has_supported_options() -> Result<(), AvengerScaleError> {
    // Create a linear scale
    let scale = LinearScale::configured((1.0, 10.0), (0.0, 100.0))
        .with_option("zero", true)
        .with_option("nice", true);

    // Normalize the scale
    let normalized = scale.normalize()?;

    // Check that normalization options are properly set
    let options = normalized.config.options;

    // Linear scale supports zero, nice, and padding
    // After normalization, these should be set to their disabled values
    assert_eq!(
        options.get("zero").and_then(|v| v.as_boolean().ok()),
        Some(false)
    );
    assert_eq!(
        options.get("nice").and_then(|v| v.as_boolean().ok()),
        Some(false)
    );
    assert_eq!(
        options.get("padding").and_then(|v| v.as_f32().ok()),
        Some(0.0)
    );

    Ok(())
}

#[test]
fn test_normalized_scale_validates_successfully() -> Result<(), AvengerScaleError> {
    // This is the key test - ensure normalized scales pass validation

    // Test point scale
    let point_domain = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    let point_scale = PointScale::configured(point_domain, (0.0, 100.0));
    let normalized_point = point_scale.normalize()?;
    // This should not error with "Unknown option 'zero'"
    normalized_point
        .scale_impl
        .validate_options(&normalized_point.config)?;

    // Test band scale
    let band_domain = Arc::new(StringArray::from(vec!["x", "y", "z"]));
    let band_scale = BandScale::configured(band_domain, (0.0, 200.0));
    let normalized_band = band_scale.normalize()?;
    // This should not error with "Unknown option 'zero'"
    normalized_band
        .scale_impl
        .validate_options(&normalized_band.config)?;

    // Test ordinal scale
    let ordinal_domain = Arc::new(StringArray::from(vec!["red", "green", "blue"]));
    let ordinal_scale = OrdinalScale::configured(ordinal_domain)
        .with_range(Arc::new(Float32Array::from(vec![0.0, 0.5, 1.0])));
    let normalized_ordinal = ordinal_scale.normalize()?;
    // This should not error with "Unknown option 'zero'"
    normalized_ordinal
        .scale_impl
        .validate_options(&normalized_ordinal.config)?;

    Ok(())
}
