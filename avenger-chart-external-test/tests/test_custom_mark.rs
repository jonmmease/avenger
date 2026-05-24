use avenger_chart::{cartesian::Cartesian, plot::Plot};
use avenger_chart_external_test::external_mark::HexBin;

#[test]
fn test_external_mark_can_be_created() {
    // Create a custom mark
    let hexbin = HexBin::<Cartesian>::new()
        .x("temperature")
        .y("humidity")
        .fill("category")
        .size(15.0);

    // Verify state access works
    assert_eq!(hexbin.state().zindex, None);

    // Note: mark_type() and supported_channels() are no longer public API methods
    // The mark can still be used in plots and compiled to SceneMarks
}

#[test]
fn test_external_mark_can_be_added_to_plot() {
    // Create a plot
    let plot = Plot::<Cartesian>::new();

    // Create a custom mark
    let hexbin = HexBin::<Cartesian>::new()
        .x("temperature")
        .y("humidity")
        .fill("category");

    // Add the mark to the plot - this tests that the types work correctly
    let _plot_with_mark = plot.mark(hexbin);

    // The plot should accept our custom mark without issue
    // Note: marks() is no longer a public method, but the mark is successfully added
}

#[test]
fn test_external_mark_state_mutation() {
    // Create a custom mark
    let mut hexbin = HexBin::<Cartesian>::new();

    // Verify we can mutate the state
    hexbin.state_mut().zindex = Some(10);
    assert_eq!(hexbin.state().zindex, Some(10));

    // Verify builder methods work
    let hexbin2 = HexBin::<Cartesian>::new().zindex(5);
    assert_eq!(hexbin2.state().zindex, Some(5));
}
