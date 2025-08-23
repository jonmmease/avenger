use avenger_chart::coords::CoordinateSystem;
use avenger_chart::marks::Mark;
use avenger_chart::plot::Plot;
use avenger_chart_external_test::external_coord_system::{Cube, Isometric};
use datafusion::logical_expr::col;
use std::collections::HashMap;

#[test]
fn test_external_coord_system_can_be_created() {
    // Create a custom coordinate system
    let iso = Isometric::new();

    // Verify required channels
    assert_eq!(iso.required_channels(), &["iso_x", "iso_y", "iso_z"]);

    // Test default ranges
    assert_eq!(iso.default_range("iso_x", 100.0, 100.0), Some((0.0, 80.0)));
    assert_eq!(iso.default_range("iso_y", 100.0, 100.0), Some((0.0, 80.0)));
    assert_eq!(iso.default_range("iso_z", 100.0, 100.0), Some((0.0, 40.0)));
}

#[test]
fn test_external_coord_system_transform() {
    // Create a custom coordinate system
    let iso = Isometric::new();

    // Create channel expressions
    let mut channels = HashMap::new();
    channels.insert("iso_x".to_string(), col("x"));
    channels.insert("iso_y".to_string(), col("y"));
    channels.insert("iso_z".to_string(), col("z"));

    // Transform should succeed
    let result = iso.transform_expressions(channels);
    assert!(result.is_ok());

    let transform = result.unwrap();
    assert!(transform.depth.is_some()); // Should have depth for 3D
}

#[test]
fn test_external_mark_with_external_coord() {
    // Create a plot with custom coordinate system
    let _plot = Plot::new(Isometric::new());

    // Create a custom mark for the custom coordinate system
    let cube = Cube::<Isometric>::new()
        .iso_x("x_pos")
        .iso_y("y_pos")
        .iso_z("z_pos")
        .fill("category")
        .size(15.0);

    // Verify state access works
    assert_eq!(cube.mark_type(), "cube");
    assert_eq!(cube.state().zindex, None);

    // Verify channels are properly defined
    let channels = cube.supported_channels();
    let channel_names: Vec<_> = channels.iter().map(|c| c.name).collect();
    assert!(channel_names.contains(&"iso_x"));
    assert!(channel_names.contains(&"iso_y"));
    assert!(channel_names.contains(&"iso_z"));
    assert!(channel_names.contains(&"fill"));
    assert!(channel_names.contains(&"size"));
}

#[test]
fn test_external_coord_in_plot() {
    // Create a plot with custom coordinate system
    let plot = Plot::new(Isometric::new());

    // Create a custom mark
    let cube = Cube::<Isometric>::new().iso_x("x").iso_y("y").iso_z("z");

    // Add the mark to the plot - this tests that the types work correctly
    let plot_with_mark = plot.mark(cube);

    // The plot should accept our custom mark without issue
    assert!(!plot_with_mark.marks().is_empty());
}

#[test]
fn test_external_coord_axes() {
    let iso = Isometric::new();
    let scales = HashMap::new();
    let marks: Vec<Box<dyn Mark<Isometric>>> = vec![];

    // Create default axes
    let axes = iso.create_default_axes(&scales, &marks);

    // Should be empty since we have no scales
    assert_eq!(axes.len(), 0);

    // Add a scale and try again
    let mut scales = HashMap::new();
    scales.insert(
        "iso_x".to_string(),
        avenger_scales::scales::linear::LinearScale::configured((0.0, 100.0), (0.0, 500.0)),
    );

    let axes = iso.create_default_axes(&scales, &marks);
    assert_eq!(axes.len(), 1);
    assert!(axes.contains_key("iso_x"));
}
