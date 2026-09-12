#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use avenger_wgpu::canvas::CanvasConfig;
    use avenger_winit_wgpu::{WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions};
    use winit::{dpi::LogicalSize, window::WindowAttributes};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let engine = d3_text_engine();
    let app = runtime.block_on(winit_panels::make_app(winit_panels::state::State::new(
        engine.clone(),
    )))?;
    let options =
        WinitWgpuAvengerAppOptions::new(if cfg!(target_os = "macos") { 2.0 } else { 1.0 })
            .window_attributes(
                WindowAttributes::default()
                    .with_title("Panel explorer")
                    .with_resizable(true)
                    .with_min_inner_size(LogicalSize::new(720.0, 780.0)),
            )
            .canvas_config(CanvasConfig {
                text_engine: Some(engine),
                ..Default::default()
            });
    let (mut host, event_loop) =
        WinitWgpuAvengerApp::try_new_and_event_loop_with_options(app, options, runtime)?;
    event_loop.run_app(&mut host)?;
    if let Some(error) = host.take_fatal_error() {
        return Err(error.into());
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}

fn d3_text_engine() -> avenger_text::TextEngine {
    let mut registry = avenger_text::NumberFormatRegistry::default();
    registry.register(
        "d3",
        std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider),
    );
    avenger_text::default_text_engine()
        .with_number_formatting(
            avenger_text::NumberFormatConfig::new("d3"),
            std::sync::Arc::new(registry),
        )
        .with_datetime_formatting(
            avenger_text::DateTimeFormatConfig::new("d3"),
            std::sync::Arc::new({
                let mut registry = avenger_text::DateTimeFormatRegistry::default();
                registry.register(
                    "d3",
                    std::sync::Arc::new(avenger_format_datetime_d3::D3DateTimeFormatProvider),
                );
                registry
            }),
        )
}
