use avenger_wgpu::canvas::CanvasConfig;
use avenger_winit_wgpu::{WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions};
use wasm_bindgen::prelude::*;
use winit::platform::web::EventLoopExtWebSys;

/// Start the same panel explorer in the page's wasm-example element.
#[wasm_bindgen]
pub async fn run() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Warn);
    let engine = avenger_text::default_text_engine();
    let mut state = crate::state::State::new(engine.clone());
    if let Some(window) = web_sys::window() {
        state.size = [
            window.inner_width()?.as_f64().unwrap_or(1280.0).max(720.0) as f32,
            window.inner_height()?.as_f64().unwrap_or(900.0).max(780.0) as f32,
        ];
    }
    let app = crate::make_app(state)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let scale = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio()) as f32;
    let options = WinitWgpuAvengerAppOptions::new(scale).canvas_config(CanvasConfig {
        text_engine: Some(engine),
        ..Default::default()
    });
    let (host, event_loop) = WinitWgpuAvengerApp::try_new_and_event_loop_with_options(app, options)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    event_loop.spawn_app(host);
    Ok(())
}
