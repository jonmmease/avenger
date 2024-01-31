#[macro_use]
extern crate lazy_static;

pub mod canvas;
pub mod error;
pub mod marks;
pub use marks::text::register_font_directory;
