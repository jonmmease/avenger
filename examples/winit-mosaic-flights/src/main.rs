use anyhow::Result;
mod config;
mod controller;
mod dataflow;
mod layout;
mod scene;
mod selection;

use config::Config;
use controller::{State, make_app};
fn main() -> Result<()> {
    let config = Config::parse()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (state, tasks) = runtime.block_on(State::load(
        config,
        avenger_typst_label::bundled_label_engine(),
        d3_formatting(),
    ))?;
    let app = runtime
        .block_on(make_app(state))?
        .with_background_tasks(tasks);
    let options =
        avenger_winit_wgpu::WinitWgpuAvengerAppOptions::new(if cfg!(target_os = "macos") {
            2.
        } else {
            1.
        })
        .window_attributes(
            winit::window::WindowAttributes::default()
                .with_title("Mosaic Flights · 10M")
                .with_resizable(false),
        );
    let (mut host, event_loop) =
        avenger_winit_wgpu::WinitWgpuAvengerApp::try_new_and_event_loop_with_options(
            app, options, runtime,
        )?;
    event_loop.run_app(&mut host)?;
    if let Some(error) = host.take_fatal_error() {
        anyhow::bail!(error);
    }
    Ok(())
}

fn d3_formatting() -> std::sync::Arc<dyn avenger_format::NumberFormatProvider> {
    std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider::new())
}
