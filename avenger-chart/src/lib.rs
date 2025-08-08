pub mod adjust;
pub mod axis;
mod chart_layout;
pub mod constants;
pub mod controllers;
pub mod coords;
pub mod derive;
pub mod error;
mod legend;
pub mod marks;
pub mod params;
pub mod plot;
mod plot_legends;
mod plot_scales;
pub mod render;
pub mod scales;
pub mod transforms;
pub mod utils;

// Re-export selected types for external tests and users
pub use crate::legend::LegendPosition;

#[cfg(test)]
pub mod test_utils;
