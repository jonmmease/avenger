//! Update a compiled parameter through the existing chart app and event stream.
use avenger_chart::{Chart, ChartAppState, RenderOptions};
use avenger_datafusion_dataflow::datafusion::common::ScalarValue;
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
    window::{Key, NamedKey},
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_vegalite_compiler::{spec::UnitSpec, FromVegaLite, VegaLiteOptions};
use std::sync::Arc;

struct Keyboard;
#[async_trait::async_trait]
impl EventStreamHandler<ChartAppState> for Keyboard {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let SceneGraphEvent::KeyPress(e) = event else {
            return UpdateStatus::default();
        };
        let current = match state.parameter("minimum") {
            Ok(ScalarValue::Float64(Some(v))) => *v,
            _ => 0.,
        };
        let next = match e.key {
            Key::Named(NamedKey::ArrowRight) => (current + 5.).min(30.),
            Key::Named(NamedKey::ArrowLeft) => (current - 5.).max(0.),
            Key::Named(NamedKey::Escape) => 0.,
            _ => return UpdateStatus::default(),
        };
        if let Err(e) = state.set_parameter("minimum", ScalarValue::Float64(Some(next))) {
            eprintln!("{e}");
        } else {
            println!("Minimum amount: {next}");
        }
        UpdateStatus::default()
    }
}
fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let spec = UnitSpec::from_json(include_str!("bars.json"))?;
    let chart = runtime.block_on(Chart::from_vegalite(
        &spec,
        &Default::default(),
        VegaLiteOptions::default(),
    ))?;
    let mut app = runtime.block_on(chart.into_app(RenderOptions::default()))?;
    app.register_handler(
        EventStreamConfig {
            types: vec![SceneGraphEventType::KeyPress],
            ..Default::default()
        },
        Arc::new(Keyboard),
    );
    let options = avenger_winit_wgpu::WinitWgpuAvengerAppOptions::new(2.).window_attributes(
        winit::window::WindowAttributes::default()
            .with_title("Vega-Lite · parameter bars")
            .with_resizable(false),
    );
    let (mut host, event_loop) =
        avenger_winit_wgpu::WinitWgpuAvengerApp::try_new_and_event_loop_with_options(
            app, options, runtime,
        )?;
    event_loop.run_app(&mut host)?;
    if let Some(e) = host.take_fatal_error() {
        anyhow::bail!(e);
    }
    Ok(())
}
