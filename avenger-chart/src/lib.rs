pub mod axis;
mod channel_resolution;
mod chart_layout;
pub mod constants;
pub mod coords;
pub mod error;
mod legend;
pub mod marks;
pub mod plot;
mod plot_legends;
mod plot_scales;
pub mod render;
pub mod scales;
pub mod utils;

// Re-export selected types for external tests and users
pub use crate::legend::LegendPosition;

#[cfg(test)]
pub mod test_utils;
