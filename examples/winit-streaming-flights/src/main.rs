use anyhow::Result;
mod baseline;
mod config;
mod controller;
mod dataflow;
mod layout;
mod replay;
mod scene;
mod selection;

use config::Config;
use controller::{State, make_app};
fn main() -> Result<()> {
    let config = Config::parse()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
    if config.headless {
        return runtime.block_on(baseline::run(config));
    }
    let text = d3_text_engine();
    let (state, tasks) = runtime.block_on(State::load(config, text.clone()))?;
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
                .with_title("Streaming Mosaic Flights · Full Recompute Baseline")
                .with_resizable(false),
        )
        .canvas_config(avenger_wgpu::canvas::CanvasConfig {
            text_engine: Some(text),
            ..Default::default()
        });
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

#[cfg(test)]
mod tests;

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
