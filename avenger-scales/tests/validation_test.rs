use arrow::array::{ArrayRef, Float32Array};
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::log::LogScale;
use std::sync::Arc;

#[test]
fn test_linear_scale_valid_options() {
    let scale = LinearScale::configured((0.0, 100.0), (0.0, 1.0))
        .with_option("clamp", true)
        .with_option("round", true)
        .with_option("range_offset", 10.0)
        .with_option("nice", true)
        .with_option("zero", false);

    let values = Arc::new(Float32Array::from(vec![0.0, 50.0, 100.0])) as ArrayRef;

    // This should succeed
    let result = scale.scale(&values);
    assert!(result.is_ok());
}

#[test]
fn test_linear_scale_invalid_option_type() {
    let scale =
        LinearScale::configured((0.0, 100.0), (0.0, 1.0)).with_option("clamp", "not_a_boolean");

    let values = Arc::new(Float32Array::from(vec![0.0, 50.0, 100.0])) as ArrayRef;

    // This should fail due to invalid boolean type
    let result = scale.scale(&values);
    assert!(result.is_err());

    if let Err(AvengerScaleError::InvalidScalePropertyValue(msg)) = result {
        assert!(msg.contains("clamp"));
        assert!(msg.contains("boolean"));
    } else {
        panic!("Expected InvalidScalePropertyValue error");
    }
}

#[test]
fn test_linear_scale_unknown_option() {
    let scale = LinearScale::configured((0.0, 100.0), (0.0, 1.0)).with_option("unknown_option", 42);

    let values = Arc::new(Float32Array::from(vec![0.0, 50.0, 100.0])) as ArrayRef;

    // This should fail due to unknown option
    let result = scale.scale(&values);
    assert!(result.is_err());

    if let Err(AvengerScaleError::InvalidScalePropertyValue(msg)) = result {
        assert!(msg.contains("unknown_option"));
        assert!(msg.contains("Unknown option"));
    } else {
        panic!("Expected InvalidScalePropertyValue error");
    }
}

#[test]
fn test_log_scale_valid_base() {
    let scale = LogScale::configured((1.0, 100.0), (0.0, 1.0)).with_option("base", 10.0);

    let values = Arc::new(Float32Array::from(vec![1.0, 10.0, 100.0])) as ArrayRef;

    // This should succeed
    let result = scale.scale(&values);
    assert!(result.is_ok());
}

#[test]
fn test_log_scale_invalid_base() {
    let scale = LogScale::configured((1.0, 100.0), (0.0, 1.0)).with_option("base", 1.0); // base cannot be 1

    let values = Arc::new(Float32Array::from(vec![1.0, 10.0, 100.0])) as ArrayRef;

    // This should fail due to invalid base
    let result = scale.scale(&values);
    assert!(result.is_err());

    if let Err(AvengerScaleError::InvalidScalePropertyValue(msg)) = result {
        assert!(msg.contains("base"));
        assert!(msg.contains("not equal to 1"));
    } else {
        panic!("Expected InvalidScalePropertyValue error");
    }
}

#[test]
fn test_log_scale_negative_base() {
    let scale = LogScale::configured((1.0, 100.0), (0.0, 1.0)).with_option("base", -2.0); // base must be positive

    let values = Arc::new(Float32Array::from(vec![1.0, 10.0, 100.0])) as ArrayRef;

    // This should fail due to negative base
    let result = scale.scale(&values);
    assert!(result.is_err());

    if let Err(AvengerScaleError::InvalidScalePropertyValue(msg)) = result {
        assert!(msg.contains("base"));
        assert!(msg.contains("positive"));
    } else {
        panic!("Expected InvalidScalePropertyValue error");
    }
}

#[test]
fn test_nice_option_boolean() {
    let scale = LinearScale::configured((0.0, 100.0), (0.0, 1.0)).with_option("nice", true);

    let values = Arc::new(Float32Array::from(vec![0.0, 50.0, 100.0])) as ArrayRef;

    // This should succeed - nice can be boolean
    let result = scale.scale(&values);
    assert!(result.is_ok());
}

#[test]
fn test_nice_option_numeric() {
    let scale = LinearScale::configured((0.0, 100.0), (0.0, 1.0)).with_option("nice", 5.0);

    let values = Arc::new(Float32Array::from(vec![0.0, 50.0, 100.0])) as ArrayRef;

    // This should succeed - nice can be numeric
    let result = scale.scale(&values);
    assert!(result.is_ok());
}

#[test]
fn test_nice_option_invalid() {
    let scale = LinearScale::configured((0.0, 100.0), (0.0, 1.0)).with_option("nice", "invalid");

    let values = Arc::new(Float32Array::from(vec![0.0, 50.0, 100.0])) as ArrayRef;

    // This should fail - nice must be boolean or numeric
    let result = scale.scale(&values);
    assert!(result.is_err());

    if let Err(AvengerScaleError::InvalidScalePropertyValue(msg)) = result {
        assert!(msg.contains("nice"));
        assert!(msg.contains("boolean or numeric"));
    } else {
        panic!("Expected InvalidScalePropertyValue error");
    }
}
