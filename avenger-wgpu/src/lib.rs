#[cfg(feature = "cosmic-text")]
#[macro_use]
extern crate lazy_static;

pub mod canvas;
pub mod error;
pub mod marks;
pub mod util;

#[cfg(target_arch = "wasm32")]
pub mod html_canvas;

#[cfg(feature = "cosmic-text")]
pub use crate::marks::cosmic::register_font_directory;
