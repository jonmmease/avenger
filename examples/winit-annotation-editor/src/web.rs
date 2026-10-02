use crate::{make_app, state::State};
use avenger_wgpu::canvas::CanvasConfig;
use avenger_winit_wgpu::{WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions};
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use winit::platform::web::EventLoopExtWebSys;

/// Start the annotation editor in the page's `wasm-example` element.
#[wasm_bindgen]
pub async fn run() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Warn);
    let engine = avenger_text::default_text_engine();
    let state = State::new(engine.clone());
    let clipboard = state.clipboard_text.clone();
    let app = make_app(state)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let scale = web_sys::window().map_or(1.0, |window| window.device_pixel_ratio()) as f32;
    let options = WinitWgpuAvengerAppOptions::new(scale)
        .canvas_config(CanvasConfig {
            text_engine: Some(engine),
            ..Default::default()
        })
        .clipboard_payload_provider(Arc::new(move || Some(clipboard.lock().unwrap().clone())));
    let (host, event_loop) = WinitWgpuAvengerApp::try_new_and_event_loop_with_options(app, options)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    event_loop.spawn_app(host);
    Ok(())
}
