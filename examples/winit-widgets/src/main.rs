#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
    use avenger_widgets::{FocusBoundary, WidgetTarget};
    use avenger_winit_wgpu::{WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions};
    use winit::{dpi::LogicalSize, window::WindowAttributes};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let engine = avenger_text::default_text_engine();
    let mut state = winit_widgets::state::State::new(engine.clone());
    state.widgets = state
        .widgets
        .with_focus_boundary(FocusBoundary::Cycle)
        .with_text_shortcuts(if cfg!(target_os = "macos") {
            avenger_widgets::TextShortcuts::Mac
        } else {
            avenger_widgets::TextShortcuts::Control
        });
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|v| v == "--screenshot") {
        winit_widgets::scene::build(&mut state)?;
        state.widgets.request_focus(
            Some(WidgetTarget::new("source")),
            avenger_common::time::Instant::now(),
        )?;
        let scene = winit_widgets::scene::build(&mut state)?.scene_graph;
        let mut canvas = runtime.block_on(PngCanvas::new(
            avenger_common::canvas::CanvasDimensions {
                size: state.size,
                scale: 1.0,
            },
            CanvasConfig {
                text_engine: Some(engine),
                ..Default::default()
            },
        ))?;
        canvas.set_scene(&scene)?;
        runtime
            .block_on(canvas.render())?
            .save(args.get(2).map_or("widget-studio.png", String::as_str))?;
        return Ok(());
    }
    let app = runtime.block_on(winit_widgets::make_app(state))?;
    let options =
        WinitWgpuAvengerAppOptions::new(if cfg!(target_os = "macos") { 2.0 } else { 1.0 })
            .window_attributes(
                WindowAttributes::default()
                    .with_title("Plot Style Studio")
                    .with_resizable(true)
                    .with_min_inner_size(LogicalSize::new(960.0, 820.0)),
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
