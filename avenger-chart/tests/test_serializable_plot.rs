//! Test for CompiledPlot serialization

use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::{CompiledPlot, Plot};
use datafusion::prelude::col;

#[tokio::test]
async fn test_compiled_plot() {
    // Create a simple plot
    let plot = Plot::new()
        .canvas_size(400.0, 300.0)
        .mark(Symbol::new().x(col("x")).y(col("y")));

    // Compile the plot to get the renderer
    let renderer = plot.compile().await.unwrap();

    // Serialize to JSON
    let json = serde_json::to_string_pretty(&renderer).unwrap();

    // Deserialize back
    let _deserialized: CompiledPlot = serde_json::from_str(&json).unwrap();

    // Just verify it round-trips successfully
    assert!(json.contains("coord_transform"));
    assert!(json.contains("marks"));
}
