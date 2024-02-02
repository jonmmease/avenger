#[macro_use]
extern crate lazy_static;

pub mod canvas;
pub mod error;
pub mod marks;

#[cfg(feature = "text-glyphon")]
pub use crate::marks::text::register_font_directory;
