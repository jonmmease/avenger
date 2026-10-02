use std::sync::Arc;

use avenger_wgpu::canvas::CanvasConfig;
use avenger_winit_wgpu::{WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions, WinitWgpuEvent};
use wasm_bindgen::prelude::*;
use winit::{
    application::ApplicationHandler, event::WindowEvent, event_loop::ActiveEventLoop,
    platform::web::EventLoopExtWebSys, window::WindowId,
};

use crate::{
    make_app,
    reload::ReloadCoordinator,
    state::{Sample, State},
};

/// Start the shared annotation editor in the page's `wasm-example` element.
#[wasm_bindgen]
pub async fn run(slow_loads: bool) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Warn);
    let reload = ReloadCoordinator::new(slow_loads);
    let engine = avenger_text::default_text_engine();
    let mut state = State::new(Sample::A, 0, engine.clone());
    state.reload = Arc::downgrade(&reload);
    let app = make_app(state)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let scale = web_sys::window().map_or(1.0, |window| window.device_pixel_ratio()) as f32;
    let options = WinitWgpuAvengerAppOptions::new(scale).canvas_config(CanvasConfig {
        text_engine: Some(engine),
        ..Default::default()
    });
    let (host, event_loop) = WinitWgpuAvengerApp::try_new_and_event_loop_with_options(app, options)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    reload.attach(host.host_update_sender());
    event_loop.spawn_app(WebApp { host, reload });
    Ok(())
}

// The browser event loop owns the coordinator for exactly as long as the host.
struct WebApp {
    host: WinitWgpuAvengerApp<State>,
    reload: Arc<ReloadCoordinator>,
}

impl ApplicationHandler<WinitWgpuEvent> for WebApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.host.resumed(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: WinitWgpuEvent) {
        self.host.user_event(event_loop, event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window: WindowId, event: WindowEvent) {
        self.host.window_event(event_loop, window, event);
    }
}

impl Drop for WebApp {
    fn drop(&mut self) {
        self.reload.close();
    }
}
