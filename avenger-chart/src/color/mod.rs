//! Color space conversions and utilities
//!
//! This module provides color space conversion functions for use in the theme system.
//! Functions are ported from Mozilla's Stylo engine with adaptations for Avenger's needs.

pub mod convert;

pub use convert::{hsl_to_rgb, normalize_hue, rgb_to_hsl};
