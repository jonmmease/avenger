use anyhow::Result;
use winit_dataflow_flights::{
    config::Config,
    controller::{State, make_app},
};
fn main() -> Result<()> {
    let config = Config::parse()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    if config.headless {
        return runtime.block_on(winit_dataflow_flights::headless::run(config));
    }
    let text = avenger_text::default_text_engine();
    let (state, worker) = runtime.block_on(State::load(config, text.clone()))?;
    let app = runtime.block_on(make_app(state))?;
    let options =
        avenger_winit_wgpu::WinitWgpuAvengerAppOptions::new(if cfg!(target_os = "macos") {
            2.
        } else {
            1.
        })
        .window_attributes(
            winit::window::WindowAttributes::default()
                .with_title("Flight Delay Explorer")
                .with_resizable(true)
                .with_min_inner_size(winit::dpi::LogicalSize::new(720., 780.)),
        )
        .canvas_config(avenger_wgpu::canvas::CanvasConfig {
            text_engine: Some(text),
            ..Default::default()
        });
    let (mut host, event_loop) =
        avenger_winit_wgpu::WinitWgpuAvengerApp::try_new_and_event_loop_with_options(
            app, options, runtime,
        )?;
    *worker.proxy.lock().unwrap() = Some(event_loop.create_proxy());
    event_loop.run_app(&mut host)?;
    worker.shutdown();
    if let Some(error) = host.take_fatal_error() {
        anyhow::bail!(error);
    }
    Ok(())
}
