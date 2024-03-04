use avenger::scene_graph::SceneGraph;
use avenger_vega::scene_graph::VegaSceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasDimensions};
use avenger_wgpu::html_canvas::HtmlCanvasCanvas;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use avenger::marks::group::SceneGroup;
use avenger_vega::marks::group::VegaGroupItem;
use avenger_vega::marks::mark::VegaMarkContainer;

pub fn set_panic_hook() {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "console_error_panic_hook")]
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
    canvas: HtmlCanvasCanvas,
    width: f32,
    height: f32,
    origin: [f32; 2],
}

#[wasm_bindgen]
impl AvengerCanvas {
    #[wasm_bindgen(constructor)]
    pub async fn new(canvas: HtmlCanvasElement, width: f32, height: f32, origin_x: f32, origin_y: f32) -> Result<AvengerCanvas, JsError> {
        let dimensions = CanvasDimensions {
            size: [width, height],
            scale: 1.0,
        };
        let Ok(canvas) = HtmlCanvasCanvas::new(canvas, dimensions).await else {
            return Err(JsError::new("Failed to construct Avenger Canvas"))
        };
        Ok(AvengerCanvas { canvas, width, height, origin: [origin_x, origin_y] })
    }

    pub fn set_scene(&mut self, scene_groups: JsValue) -> Result<(), JsError> {
        let scenegraph: VegaMarkContainer<VegaGroupItem> = serde_wasm_bindgen::from_value(scene_groups)?;
        let vega_scene_graph = VegaSceneGraph {
            width: self.width,
            height: self.height,
            origin: self.origin,
            scenegraph
        };

        // TODO: don't panic
        let scene_graph = vega_scene_graph.to_scene_graph().expect("Failed to import vega scene graph");
        self.canvas.set_scene(&scene_graph).expect("Failed to set scene");
        self.canvas.render().expect("failed to render scene");
        Ok(())
    }
}
