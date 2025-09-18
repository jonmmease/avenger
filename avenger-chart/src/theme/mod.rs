//! Theme system for visual styling of charts
//!
//! The theme system provides a centralized way to control the visual appearance
//! of charts, including colors, fonts, shapes, and other styling properties.

// Trait-based theme system
mod theme_interface;

// CSS theme system
pub mod css;

// Export trait and related types
pub use theme_interface::{
    ContextBuilder, LengthUnit, Rgba, Theme, ThemeContext, ThemeProperty, ThemeValue,
};
