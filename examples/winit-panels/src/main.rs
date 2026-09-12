#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use avenger_wgpu::canvas::CanvasConfig;
    use avenger_winit_wgpu::{WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions};
    use winit::{dpi::LogicalSize, window::WindowAttributes};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let engine = avenger_text::default_text_engine();
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
