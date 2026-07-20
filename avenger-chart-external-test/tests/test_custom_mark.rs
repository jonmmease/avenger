use avenger_chart::plot::{Chart, Plot};
use avenger_chart_cartesian::Cartesian;
use avenger_chart_core::{CoordinateSystemCore, Mark};
use avenger_chart_external_test::external_mark::HexBin;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;

#[derive(Clone)]
struct CoreOnlyCoord;

impl CoordinateSystemCore for CoreOnlyCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

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

#[test]
fn test_external_mark_impl_only_needs_core_coordinate_trait() {
    fn assert_mark_impl<M: Mark<CoreOnlyCoord>>() {}

    assert_mark_impl::<HexBin<CoreOnlyCoord>>();
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
async fn external_mark_renders_minimal_visible_geometry() {
    let ctx = SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .mark(HexBin::<Cartesian>::new().x(1.0).y(1.0))
        .compile(&ctx)
        .await
        .expect("compile external mark");
    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate external mark");
    assert_eq!(count_symbols(&evaluated.scene_graph.marks), 1);
}
