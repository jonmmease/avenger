//! Test for SerializablePlotRenderer serialization

use avenger_chart::plot::{Plot, SerializablePlotRenderer};
use avenger_chart::marks::symbol::Symbol;
use datafusion::prelude::col;

#[test]
fn test_serializable_plot_renderer() {
    // Create a simple plot
    let plot = Plot::new()
        .canvas_size(400.0, 300.0)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
        );

    // Build the serializable renderer
    let renderer = plot.build();

    // Serialize to JSON
    let json = serde_json::to_string_pretty(&renderer).unwrap();

    // Deserialize back
    let _deserialized: SerializablePlotRenderer = serde_json::from_str(&json).unwrap();

    // Just verify it round-trips successfully
    assert!(json.contains("coord_transform"));
    assert!(json.contains("marks"));
}