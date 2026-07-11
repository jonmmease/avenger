// Test that demonstrates the serialization path for visual tests

use crate::visual_tests::datasets::simple_categories;
use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::cartesian::CartesianSymbolPositionChannels;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Chart;
use datafusion::prelude::col;

#[tokio::test]
async fn test_serialization_rendering_path() {
    use datafusion::prelude::SessionContext;
    let ctx = SessionContext::new();

    // Create a function to build the plot so we can create it twice
    let build_plot = || {
        Chart::new()
            .canvas_size(400.0, 300.0)
            .data(simple_categories())
            .mark(Symbol::new().x(col("category")).y(col("value")))
    };

    // Compile to get CompiledPlot
    let compiled = build_plot().compile(&ctx).await.unwrap();

    // Serialize to JSON
    let json = serde_json::to_string_pretty(&compiled).unwrap();

    // Deserialize back to verify serialization works
    let _deserialized: avenger_chart::plot::CompiledPlot = serde_json::from_str(&json).unwrap();

    assert_visual_match_default(&compiled, &ctx, None, "serialization", "simple_scatter").await;
}
