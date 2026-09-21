//! Decode a portable chart definition with the destination's transform registry.
use avenger_chart::{Chart, ChartOptions, RenderOptions};
use avenger_chart_definition::ChartDefinition;
use avenger_datafusion_dataflow::{
    datafusion::{common::ScalarValue, execution::context::SessionContext},
    Runtime, RuntimeConfig,
};
use avenger_vegalite_compiler::{compile_vegalite, spec::UnitSpec};
use std::{path::Path, sync::Arc};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let spec = UnitSpec::from_json(include_str!("histogram.json"))?;
    let definition = compile_vegalite(&spec, &Default::default(), Path::new(".")).await?;
    let codec = Arc::new(avenger_transform::TransformExtensionCodec::default());
    let bytes = definition.to_bytes_with_codec(codec.clone())?;
    drop((spec, definition));
    let runtime = Runtime::with_session_state_and_codec(
        SessionContext::new().state(),
        RuntimeConfig {
            function_versions: avenger_transform::function_versions(),
            ..Default::default()
        },
        codec,
    )?;
    let restored = ChartDefinition::from_bytes(&bytes, &runtime)?;
    let chart = Chart::prepare(
        restored,
        ChartOptions {
            dataflow: Some(runtime),
            text_engine: Some(d3_text_engine()),
        },
    )
    .await?;
    let initial = chart.render(Default::default()).await?;
    let frame = chart
        .render(RenderOptions::default().parameter("minimum", ScalarValue::Float64(Some(10.))))
        .await?;
    println!(
        "Initial nodes: {:?}\nUpdated nodes: {:?}",
        initial.report().executed_nodes,
        frame.report().executed_nodes
    );
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "restored-bars.svg".into());
    std::fs::write(&path, frame.to_svg()?)?;
    println!("Decoded {} bytes and wrote {path}", bytes.len());
    Ok(())
}

fn d3_text_engine() -> avenger_text::TextEngine {
    let mut registry = avenger_text::NumberFormatRegistry::default();
    registry.register(
        "d3",
        std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider),
    );
    avenger_text::default_text_engine().with_number_formatting(
        avenger_text::NumberFormatConfig::new("d3"),
        std::sync::Arc::new(registry),
    )
}
