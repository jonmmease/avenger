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
//! - `contrast.rs`: WCAG 2.1 contrast ratio calculations

pub mod contrast;
pub mod convert;
pub mod mix;
pub mod types;

pub use self::{
    contrast::{
        choose_best_contrast, choose_contrast_color, contrast_ratio, relative_luminance_srgb,
    },
    convert::{normalize_hue, orthogonal_to_polar, polar_to_orthogonal},
    mix::{HueInterpolationMethod, mix_colors},
    types::{AbsoluteColor, ColorSpace},
};
