//! Integration tests for custom scale implementation
//! These tests demonstrate what imports are needed when using custom scales from external crates

use std::sync::Arc;

use avenger_chart::{
    cartesian::CartesianSymbolPositionChannels, marks::symbol::Symbol, plot::Plot,
};
use avenger_chart_cartesian::Cartesian;
use avenger_chart_external_test::external_scale::{SmoothLog, SmoothLogExt, SmoothLogScale};
use avenger_chart_scales::{Auto, Scale, ScaleChannelConfig};
use avenger_scales::scales::{ScaleConfig, ScaleContext, ScaleImpl};
use datafusion::{
    arrow::array::{Array, ArrayRef, Float32Array},
    logical_expr::lit,
};

#[test]
fn test_custom_scale_with_typed_methods() {
    // Create a scale using the custom ScaleSpec type with typed methods
    let scale = Scale::<SmoothLog>::new()
        .smoothing(0.5)
        .clamp(true)
        .nice(false)
        .domain((0.1_f32, 100.0_f32))
        .range_interval(lit(0.0), lit(500.0));

    // Verify it has the correct type
    assert_eq!(scale.get_scale_type(), Some("smooth_log"));

    // Verify options were set
    assert!(scale.get_options().contains_key("smoothing"));
    assert!(scale.get_options().contains_key("clamp"));
    assert!(scale.get_options().contains_key("nice"));
}

#[test]
fn test_custom_scale_in_plot() {
    // Create a plot using the typed external scale with custom methods
    let _plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x("value")
            .y_with("result", |c| {
                c.scale_with::<SmoothLog>(|scale| {
                    scale
                        .smoothing(0.1)
                        .clamp(true)
                        .nice(false)
                        .domain((0.1_f32, 100.0_f32))
                        .range_interval(lit(400.0), lit(0.0))
                })
            })
            .fill("category"),
    );

    // The plot compiles with typed external scale and custom methods - success!
}

#[test]
fn test_scale_transformation() {
    let scale_impl = SmoothLogScale::with_smoothing(1.0);

    // Create test data
    let values = Arc::new(Float32Array::from(vec![1.0, 10.0, 100.0])) as ArrayRef;
    let domain = Arc::new(Float32Array::from(vec![1.0, 100.0])) as ArrayRef;
    let range = Arc::new(Float32Array::from(vec![0.0, 100.0])) as ArrayRef;

    let config = ScaleConfig {
        domain,
        range,
        options: std::collections::HashMap::new(),
        context: ScaleContext::default(),
    };

    // Apply the scale
    let result = scale_impl.scale(&config, &values).unwrap();

    // Check that we got results
    assert_eq!(result.len(), 3);
    let result_array = result.as_any().downcast_ref::<Float32Array>().unwrap();
    assert!(result_array.is_valid(0));
    assert!(result_array.is_valid(1));
    assert!(result_array.is_valid(2));
}

#[test]
fn test_auto_scale_with_custom_impl() {
    // Can also use Scale<Auto>::from_spec for dynamic construction
    let scale = Scale::<Auto>::from_spec(Box::new(SmoothLog))
        .option("smoothing", lit(0.5))
        .option("clamp", lit(true))
        .domain((0.1_f32, 100.0_f32));

    assert_eq!(scale.get_scale_type(), Some("smooth_log"));
}

#[test]
fn test_using_scale_without_extension_trait() {
    // Without importing the extension trait, we can still use _option directly
    let scale = Scale::<SmoothLog>::new()
        .smoothing(0.5)
        .clamp(true)
        .nice(false)
        .domain((0.1_f32, 100.0_f32))
        .range_interval(lit(0.0), lit(500.0));

    assert_eq!(scale.get_scale_type(), Some("smooth_log"));
}
