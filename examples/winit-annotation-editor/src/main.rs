use std::{error::Error, sync::Arc};

use avenger_wgpu::canvas::CanvasConfig;
use avenger_winit_wgpu::{WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions};
use winit::{dpi::LogicalSize, window::WindowAttributes};
use winit_annotation_editor::{
    make_app,
    reload::ReloadCoordinator,
    state::{Sample, State},
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let slow = args.iter().any(|arg| arg == "--slow-loads");
    let mut scale = if cfg!(target_os = "macos") { 2.0 } else { 1.0 };
    if let Some(i) = args.iter().position(|arg| arg == "--scale") {
        scale = args
            .get(i + 1)
            .ok_or("--scale needs a number")?
            .parse::<f32>()?;
        if !scale.is_finite() || scale <= 0.0 {
            return Err("--scale must be positive".into());
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let reload = ReloadCoordinator::new(runtime.handle().clone(), slow);
    let engine = avenger_text::default_text_engine();
    let mut state = State::new(Sample::A, 0, engine.clone());
    state.reload = Arc::downgrade(&reload);
    let app = runtime.block_on(make_app(state))?;
    let options = WinitWgpuAvengerAppOptions::new(scale)
        .window_attributes(
            WindowAttributes::default()
                .with_title("Annotation editor — sample A")
                .with_resizable(true)
                .with_min_inner_size(LogicalSize::new(820.0, 560.0)),
        )
        .canvas_config(CanvasConfig {
            text_engine: Some(engine),
            ..Default::default()
        });
    let (mut host, event_loop) =
        WinitWgpuAvengerApp::try_new_and_event_loop_with_options(app, options, runtime)?;
    reload.attach(host.host_update_sender());
    let result = event_loop.run_app(&mut host);
    reload.close();
    result?;
    if let Some(error) = host.take_fatal_error() {
        return Err(error.into());
    }
    Ok(())
}
