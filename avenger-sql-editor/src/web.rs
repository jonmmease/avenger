//! The editor in a web page: it starts on the page's canvas `editor` when the module loads.

use wasm_bindgen::{prelude::*, JsCast};

use crate::{EditorApp, Options};

/// Starts the editor on the canvas `editor`, and logs to the browser's console.
#[wasm_bindgen(start)]
pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("editor"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("the page has a canvas `editor`");
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(EditorApp::new(cc, Options::default())?))),
            )
            .await;
        if let Err(error) = started {
            log::error!("the editor failed to start: {error:?}");
        }
    });
}
