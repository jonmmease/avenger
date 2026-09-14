use arrow::array::{Array, ArrayRef, Float32Array, StringArray};
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
    let scale = PointScale::configured(domain.clone(), (0.0, 100.0))
        .with_option("padding", 0.1)
        .with_option("align", 0.5);

    // For categorical scales, normalized domain should be the same as original
    let normalized_domain = scale.normalized_domain()?;
    assert_eq!(normalized_domain.len(), domain.len());

    // Test that scale operations work correctly
    let values = scale.scale(&(domain.clone() as ArrayRef))?;
    assert_eq!(values.len(), domain.len());

    Ok(())
}

#[test]
fn test_band_scale_normalize_no_unsupported_options() -> Result<(), AvengerScaleError> {
    // Create a band scale
    let domain = Arc::new(StringArray::from(vec!["x", "y", "z"]));
    let scale = BandScale::configured(domain.clone(), (0.0, 200.0))
        .with_option("padding_inner", 0.1)
        .with_option("round", true);

    // For categorical scales, normalized domain should be the same as original
    let normalized_domain = scale.normalized_domain()?;
    assert_eq!(normalized_domain.len(), domain.len());

    // Test that scale operations work correctly
    let values = scale.scale(&(domain.clone() as ArrayRef))?;
    assert_eq!(values.len(), domain.len());

    Ok(())
}

#[test]
fn test_ordinal_scale_normalize_no_unsupported_options() -> Result<(), AvengerScaleError> {
    // Create an ordinal scale
    let domain = Arc::new(StringArray::from(vec!["red", "green", "blue"]));
    let scale = OrdinalScale::configured(domain.clone())
        .with_range(Arc::new(Float32Array::from(vec![0.0, 0.5, 1.0])));

    // For categorical scales, normalized domain should be the same as original
    let normalized_domain = scale.normalized_domain()?;
    assert_eq!(normalized_domain.len(), domain.len());

    // Test that scale operations work correctly
    let values = scale.scale(&(domain.clone() as ArrayRef))?;
    assert_eq!(values.len(), domain.len());

    Ok(())
}

#[test]
fn test_linear_scale_normalized_domain() -> Result<(), AvengerScaleError> {
    // Create a linear scale with nice and zero options
    let scale = LinearScale::configured((3.0, 10.0), (0.0, 100.0))
        .with_option("zero", true)
        .with_option("nice", true);

    // Get original and normalized domains
    let original_domain = scale.domain();
    let normalized_domain = scale.normalized_domain()?;

    // Original domain should be unchanged
    assert_eq!(original_domain.len(), 2);
    let original_f32 = original_domain
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    assert_eq!(original_f32.value(0), 3.0);
    assert_eq!(original_f32.value(1), 10.0);

    // Normalized domain should include zero and be nice
    assert_eq!(normalized_domain.len(), 2);
    let normalized_f32 = normalized_domain
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    assert_eq!(normalized_f32.value(0), 0.0); // Includes zero
    assert!(normalized_f32.value(1) >= 10.0); // Nice value >= 10

    Ok(())
}

#[test]
fn test_linear_scale_normalized_domain_with_clip_padding() -> Result<(), AvengerScaleError> {
    // Create a linear scale with clip padding
    let scale = LinearScale::configured((0.0, 100.0), (0.0, 500.0))
        .with_option("clip_padding_lower", 10.0)
        .with_option("clip_padding_upper", 10.0);

    // Get original and normalized domains
    let original_domain = scale.domain();
    let normalized_domain = scale.normalized_domain()?;

    // Original domain should be unchanged
    let original_f32 = original_domain
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    assert_eq!(original_f32.value(0), 0.0);
    assert_eq!(original_f32.value(1), 100.0);

    // Normalized domain should be expanded for padding
    // With 10px padding on 500px range, that's 2% on each side
    // So domain should expand from 0-100 to -2 to 102
    let normalized_f32 = normalized_domain
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    assert!(normalized_f32.value(0) < 0.0);
    assert!(normalized_f32.value(1) > 100.0);

    Ok(())
}

#[test]
fn test_scale_operations_use_normalized_domain() -> Result<(), AvengerScaleError> {
    // Create a linear scale with zero option
    let scale = LinearScale::configured((5.0, 10.0), (0.0, 100.0)).with_option("zero", true);

    // Scale a value that would be different with/without normalization
    let values = Arc::new(Float32Array::from(vec![0.0, 5.0, 10.0])) as ArrayRef;
    let scaled = scale.scale(&values)?;
    let scaled_f32 = scaled.as_any().downcast_ref::<Float32Array>().unwrap();

    // With zero=true, domain is normalized to [0, 10]
    // So 0 maps to 0, 5 maps to 50, 10 maps to 100
    assert_eq!(scaled_f32.value(0), 0.0);
    assert_eq!(scaled_f32.value(1), 50.0);
    assert_eq!(scaled_f32.value(2), 100.0);

    Ok(())
}
