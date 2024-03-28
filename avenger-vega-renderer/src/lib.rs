mod builder;

use avenger_vega::scene_graph::VegaSceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasDimensions};
use avenger_wgpu::html_canvas::HtmlCanvasCanvas;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use avenger::marks::group::SceneGroup;
use avenger_vega::marks::group::VegaGroupItem;
use avenger_vega::marks::mark::VegaMarkContainer;
use crate::builder::SceneGraph;

pub fn set_panic_hook() {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    console_error_panic_hook::set_once();
}

/// Make console.log available as the log Rust function
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[wasm_bindgen]
pub struct AvengerCanvas {
    canvas: HtmlCanvasCanvas<'static>,
    width: f32,
    height: f32,
    origin: [f32; 2],
}

#[wasm_bindgen]
impl AvengerCanvas {
    #[wasm_bindgen(constructor)]
    pub async fn new(canvas: HtmlCanvasElement, width: f32, height: f32, origin_x: f32, origin_y: f32) -> Result<AvengerCanvas, JsError> {
        set_panic_hook();
        let dimensions = CanvasDimensions {
            size: [width, height],
            scale: 1.0,
        };
        let Ok(canvas) = HtmlCanvasCanvas::new(canvas, dimensions).await else {
            return Err(JsError::new("Failed to construct Avenger Canvas"))
        };
        Ok(AvengerCanvas { canvas, width, height, origin: [origin_x, origin_y] })
    }

    pub fn set_scene(&mut self, scene_graph: SceneGraph) -> Result<(), JsError> {
        let window = web_sys::window().expect("should have a window in this context");
        let performance = window
            .performance()
            .expect("performance should be available");

        let start = performance.now();
        self.canvas.set_scene(&scene_graph.build()).expect("Failed to set scene");
        // log(&format!("self.canvas.set_scene time: {}", performance.now() - start));

        let start = performance.now();
        self.canvas.render().expect("failed to render scene");
        // log(&format!("self.canvas.render time: {}", performance.now() - start));
        Ok(())
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn origin_x(&self) -> f32 {
        self.origin[0]
    }

    pub fn origin_y(&self) -> f32 {
        self.origin[1]
    }
}
