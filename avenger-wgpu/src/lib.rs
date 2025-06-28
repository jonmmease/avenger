#[cfg(feature = "cosmic-text")]
extern crate lazy_static;

pub mod canvas;
pub mod error;
pub mod marks;
pub mod util;

#[cfg(target_arch = "wasm32")]
pub mod html_canvas;
