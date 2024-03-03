use avenger::scene_graph::SceneGraph;
use avenger_vega::scene_graph::VegaSceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasDimensions};
use avenger_wgpu::html_canvas::HtmlCanvasCanvas;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

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
pub async fn try_avenger_wasm() {
    set_panic_hook();
    log(&format!("Hello from wasm"));

    // Load scene graph
    let scene_spec: VegaSceneGraph = serde_json::from_str(include_str!(
        "../../avenger-vega-test-data/vega-scenegraphs/gradients/symbol_radial_gradient.sg.json"
    ))
    .unwrap();

    let scale = 2.0;

    let scene_graph: SceneGraph = scene_spec
        .to_scene_graph()
        .expect("Failed to parse scene graph");

    // Save to png
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale,
    };

    log(&format!("dimensions: {dimensions:?}"));

    let canvas = web_sys::window()
        .and_then(|win| win.document())
        .map(|doc| {
            let dst = doc
                .get_element_by_id("plot-container")
                .expect("should be able to get plot-container div");

            let canvas = doc
                .create_element("canvas")
                .expect("should be able to create canvas element")
                .dyn_into::<HtmlCanvasElement>()
                .expect("should be able to cast as a canvas element");

            dst.append_child(&canvas)
                .expect("should be able to append canvas as child");
            canvas
        })
        .expect("should be able to return canvas");

    let mut avenger_canvas = HtmlCanvasCanvas::new(canvas, dimensions)
        .await
        .expect("Failed to make avenger canvas");
    avenger_canvas
        .set_scene(&scene_graph)
        .expect("Failed to set scene");
    avenger_canvas.render().expect("Failed to render");
}
