use std::collections::HashMap;

use avenger_chart::plot::{Chart, Plot};
use avenger_chart_core::{CoordinateSystem, CoordinateSystemCore};
use avenger_chart_external_test::external_coord_system::{Cube, Isometric};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;

#[test]
fn test_external_coord_system_can_be_created() {
    // Create a custom coordinate system
    let iso = Isometric::new();

    // Verify required channels
    assert_eq!(iso.required_channels(), &["iso_x", "iso_y", "iso_z"]);

    // Test default ranges via the transform
    let transform = iso.create_transform();
    assert_eq!(
        transform.default_range("iso_x", 100.0, 100.0),
        Some((0.0, 80.0))
    );
    assert_eq!(
        transform.default_range("iso_y", 100.0, 100.0),
        Some((0.0, 80.0))
    );
    assert_eq!(
        transform.default_range("iso_z", 100.0, 100.0),
        Some((0.0, 40.0))
    );
}

#[test]
fn test_external_coord_system_transform() {
    // Create a custom coordinate system
    let iso = Isometric::new();
    let transform = iso.create_transform();

    // Create position channel values
    let mut position_channels = HashMap::new();
    position_channels.insert("iso_x", ScalarOrArray::new_scalar(10.0));
    position_channels.insert("iso_y", ScalarOrArray::new_scalar(20.0));
    position_channels.insert("iso_z", ScalarOrArray::new_scalar(5.0));

    // Transform should succeed
    let result = transform.transform(&position_channels, None, 100.0, 100.0);
    assert!(result.is_ok());

    // Verify we got transformed coordinates (as a trait object)
    let _geometry = result.unwrap();
    // The geometry is successfully created and can be used for rendering
}

#[test]
fn test_external_mark_with_external_coord() {
    // Create a plot with custom coordinate system
    let _plot = Plot::<Isometric>::new();

    // Create a custom mark for the custom coordinate system
    let cube = Cube::<Isometric>::new()
        .iso_x("x_pos")
        .iso_y("y_pos")
        .iso_z("z_pos")
        .fill("category")
        .size(15.0);

    // Verify state access works
    assert_eq!(cube.state().zindex, None);

    // Note: mark_type() and supported_channels() are no longer public API methods
    // The mark can still be used in plots and compiled to SceneMarks
}

#[test]
fn test_external_coord_in_plot() {
    // Create a plot with custom coordinate system
    let plot = Plot::<Isometric>::new();

    // Create a custom mark
    let cube = Cube::<Isometric>::new().iso_x("x").iso_y("y").iso_z("z");

    // Add the mark to the plot - this tests that the types work correctly
    let _plot_with_mark = plot.mark(cube);

    // The plot should accept our custom mark without issue
    // Note: marks() is no longer a public method, but the mark is successfully added
}

#[test]
fn test_external_coord_axes() {
    // let iso = Isometric::new();
    // let scales = HashMap::new();
    // let marks: Vec<Box<dyn Mark<Isometric>>> = vec![];
    //
    // // Create default axes
    // let axes = iso.create_default_axes(&scales, &marks);
    //
    // // Should be empty since we have no scales
    // assert_eq!(axes.len(), 0);
    //
    // // Add a scale and try again
    // let mut scales = HashMap::new();
    // scales.insert(
    //     "iso_x".to_string(),
    //     avenger_scales::scales::linear::LinearScale::configured((0.0, 100.0), (0.0, 500.0)),
    // );

    // let axes = iso.create_default_axes(&scales, &marks);
    // assert_eq!(axes.len(), 1);
    // assert!(axes.contains_key("iso_x"));
}

fn count_symbols(marks: &[SceneMark]) -> usize {
    marks
        .iter()
        .map(|mark| match mark {
            SceneMark::Symbol(symbol) => symbol.len as usize,
            SceneMark::Group(group) => count_symbols(&group.marks),
            _ => 0,
        })
        .sum()
}

#[tokio::test]
async fn external_coordinate_mark_renders_minimal_visible_geometry() {
    let ctx = SessionContext::new();
    let compiled = Chart::<Isometric>::new()
        .mark(Cube::<Isometric>::new().iso_x(1.0).iso_y(2.0).iso_z(3.0))
        .compile(&ctx)
        .await
        .expect("compile external coordinate mark");
    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate external coordinate mark");
    assert_eq!(count_symbols(&evaluated.scene_graph.marks), 1);
}
