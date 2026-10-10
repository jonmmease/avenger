#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use avenger_winit_wgpu::{WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions};
    use winit::{dpi::LogicalSize, window::WindowAttributes};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let app = runtime.block_on(winit_panels::make_app(winit_panels::state::State::new(
        avenger_typst_label::bundled_label_engine(),
        d3_formatting(),
    )))?;
    let options =
        WinitWgpuAvengerAppOptions::new(if cfg!(target_os = "macos") { 2.0 } else { 1.0 })
            .window_attributes(
                WindowAttributes::default()
                    .with_title("Panel explorer")
                    .with_resizable(true)
                    .with_min_inner_size(LogicalSize::new(720.0, 780.0)),
            );
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

#[cfg(not(target_arch = "wasm32"))]
fn d3_formatting() -> std::sync::Arc<dyn avenger_format::NumberFormatProvider> {
    std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider::new())
}
