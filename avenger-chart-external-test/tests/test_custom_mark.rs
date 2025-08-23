use avenger_chart::cartesian::Cartesian;
use avenger_chart::marks::Mark;
use avenger_chart::plot::Plot;
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
    assert_eq!(hexbin.mark_type(), "hexbin");
    assert_eq!(hexbin.state().zindex, None);

    // Verify channels are properly defined
    let channels = hexbin.supported_channels();
    let channel_names: Vec<_> = channels.iter().map(|c| c.name).collect();
    assert!(channel_names.contains(&"x"));
    assert!(channel_names.contains(&"y"));
    assert!(channel_names.contains(&"fill"));
    assert!(channel_names.contains(&"size"));
}

#[test]
fn test_external_mark_can_be_added_to_plot() {
    // Create a plot
    let plot = Plot::new(Cartesian);

    // Create a custom mark
    let hexbin = HexBin::<Cartesian>::new()
        .x("temperature")
        .y("humidity")
        .fill("category");

    // Add the mark to the plot - this tests that the types work correctly
    let plot_with_mark = plot.mark(hexbin);

    // The plot should accept our custom mark without issue
    assert!(!plot_with_mark.marks().is_empty());
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
