#[cfg(feature = "cosmic-text")]
extern crate lazy_static;

pub mod canvas;
pub mod error;
pub mod marks;
pub mod offscreen;
pub mod renderer;
pub mod target;
pub mod util;
pub mod zindex_layers;

#[cfg(target_arch = "wasm32")]
pub mod html_canvas;
