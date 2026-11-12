#![doc = include_str!("../README.md")]

pub mod fonts;
pub mod input;
pub mod styles;
#[cfg(target_arch = "wasm32")]
mod view;

#[cfg(target_arch = "wasm32")]
pub use view::{load_fonts, ChartView};

#[cfg(test)]
mod tests;
