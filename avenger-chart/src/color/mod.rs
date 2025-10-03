//! Color space support for CSS color functions
//!
//! This module provides color space conversions and color mixing functionality
//! needed for advanced CSS color functions like `color-mix()`, `oklch()`, etc.
//!
//! ## Architecture
//!
//! - `types.rs`: Core color types (AbsoluteColor, ColorSpace)
//! - `convert.rs`: Color space conversion functions
//! - `mix.rs`: Color mixing/interpolation

pub mod convert;
pub mod mix;
pub mod types;

// Re-export public API
pub use convert::{normalize_hue, orthogonal_to_polar, polar_to_orthogonal};
pub use mix::{HueInterpolationMethod, mix_colors};
pub use types::{AbsoluteColor, ColorSpace};
