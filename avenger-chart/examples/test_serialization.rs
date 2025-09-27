//! Test serialization of CompiledPlot

use avenger_chart::plot::{Plot, CompiledPlot};
use avenger_chart::marks::rect::Rect;
use datafusion::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple DataFrame
    let ctx = SessionContext::new();
    let df = ctx
        .sql("SELECT * FROM (VALUES ('A', 10), ('B', 20), ('C', 15)) AS t(category, value)")
        .await?;

    // Create a simple plot with rect mark
    let plot = Plot::new()
        .data(df)
        .mark(
            Rect::new()
                .x_with(col("category"), |c| c)
                .y_with(col("value"), |c| c)
        );

    // Compile the plot
    let compiled = plot.compile().await?;

    // Serialize to JSON
    let json = serde_json::to_string_pretty(&compiled)?;
    println!("Serialized CompiledPlot:");
    println!("{}", &json[..json.len().min(500)]); // Print first 500 chars
    println!("...");

    // Deserialize back
    let deserialized: CompiledPlot = serde_json::from_str(&json)?;
    println!("\n✅ Successfully deserialized!");

    // Try to render the deserialized plot
    let ctx2 = SessionContext::new();
    let render_result = deserialized.render(&ctx2).await?;
    println!("\n✅ Successfully rendered deserialized plot!");
    println!("Scene graph size: {} marks", render_result.scene_graph.marks.len());

    Ok(())
}