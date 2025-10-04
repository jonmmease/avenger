//! Theme system for visual styling of charts
//!
//! The theme system provides a centralized way to control the visual appearance
//! of charts using CSS-like syntax. Themes are defined using CSS stylesheets that
//! target chart elements with selectors, properties, and values.
//!
//! # Basic Usage
//!
//! ## Using Built-in Themes
//!
//! ```rust,no_run
//! use avenger_chart::theme::Theme;
//!
//! // Use a specific built-in theme
//! let theme = Theme::light();
//! let theme = Theme::dark();
//! ```
//!
//! ## Creating Custom Themes
//!
//! ```rust,no_run
//! use avenger_chart::theme::Theme;
//!
//! let css = r#"
//!     mark[type="symbol"] {
//!         size: 100px;
//!         fill-discrete: #E69F00, #56B4E9, #009E73;
//!     }
//!
//!     guide[type="cartesian"] axis label {
//!         font-size: 11px;
//!         color: #666;
//!     }
//! "#;
//!
//! let theme = Theme::from_css(css).unwrap();
//! ```
//!
//! ## Combining Themes
//!
//! ```rust,no_run
//! use avenger_chart::theme::Theme;
//!
//! // Start with a base theme
//! let mut theme = Theme::dark();
//!
//! // Add custom CSS on top
//! let custom_css = r#"
//!     mark[type="symbol"] {
//!         fill-discrete: #ff0000, #00ff00, #0000ff;
//!     }
//! "#;
//!
//! theme.append_css(custom_css).unwrap();
//! ```
//!
//! # Element Hierarchy and CSS Selectors
//!
//! The theme system uses a hierarchical element structure that can be targeted with
//! CSS-like selectors. Elements can have subtypes (via `[type="..."]` attribute selectors)
//! to enable fine-grained styling control.
//!
//! ## Complete Element Hierarchy
//!
//! ```text
//! canvas
//! ├── chart-title
//! ├── chart-subtitle
//! ├── guide[type="cartesian"|"polar"]
//! │   ├── background
//! │   └── axis[type="x"|"y"|"r"|"theta"|...]
//! │       ├── domain
//! │       ├── tick
//! │       ├── grid
//! │       ├── label
//! │       ├── title
//! │       └── ...
//! ├── legend[type="symbol"|"line"|"colorbar"|"rect"|...]
//! │   ├── background
//! │   ├── title
//! │   ├── label
//! │   ├── tick
//! │   └── ...
//! └── mark[type="symbol"|"line"|"rect"|"arc"|"area"|"path"|...]
//!     ├── fill
//!     ├── stroke
//!     ├── stroke-width
//!     ├── opacity
//!     └── ...
//! ```
//!
//! ## CSS Selector Examples
//!
//! ### Root Elements
//! ```css
//! canvas {
//!     background-color: white;
//! }
//!
//! chart-title {
//!     font-size: 18px;
//!     font-weight: 700;
//!     color: #111827;
//! }
//! ```
//!
//! ### Guides and Coordinate Systems
//! ```css
//! /* All guides */
//! guide {
//!     background-color: transparent;
//! }
//!
//! /* Cartesian-specific background */
//! guide[type="cartesian"] {
//!     background-color: #f0f0f0;
//! }
//!
//! /* Polar-specific background */
//! guide[type="polar"] {
//!     background-color: #fff8f0;
//! }
//! ```
//!
//! ### Axes (Nested under Guide)
//! ```css
//! /* All axis labels in cartesian plots */
//! guide[type="cartesian"] axis label {
//!     font-size: 11px;
//!     color: #666;
//! }
//!
//! /* X-axis title in cartesian plots */
//! guide[type="cartesian"] axis[type="x"] title {
//!     color: blue;
//!     font-weight: 600;
//! }
//!
//! /* Y-axis grid in cartesian plots */
//! guide[type="cartesian"] axis[type="y"] grid {
//!     stroke: #ddd;
//!     opacity: 0.5;
//! }
//!
//! /* Theta axis in polar plots */
//! guide[type="polar"] axis[type="theta"] title {
//!     color: orange;
//! }
//! ```
//!
//! ### Legends
//! ```css
//! /* All legend titles */
//! legend title {
//!     font-size: 14px;
//!     font-weight: 600;
//! }
//!
//! /* Symbol legend background */
//! legend[type="symbol"] background {
//!     fill: #e3f2fd;
//!     stroke: #2196f3;
//!     stroke-width: 1px;
//!     padding: 8px;
//!     corner-radius: 4px;
//! }
//!
//! /* Line legend background */
//! legend[type="line"] background {
//!     fill: #fce4ec;
//!     stroke: #e91e63;
//! }
//! ```
//!
//! ### Marks and Scale Ranges
//! ```css
//! /* All symbol marks */
//! mark[type="symbol"] {
//!     size: 100px;
//!     fill-discrete: #E69F00, #56B4E9, #009E73;
//!     fill-continuous: #deebf7, #08306b;
//! }
//!
//! /* Line marks */
//! mark[type="line"] {
//!     stroke-discrete: #1f77b4, #ff7f0e, #2ca02c;
//!     stroke-width-discrete: 1, 2, 3, 4, 5;
//! }
//! ```
//!
//! ## Mark Properties and Scale Ranges
//!
//! Marks support both direct properties and scale range properties:
//!
//! - Direct properties: `fill`, `stroke`, `size`, `opacity`, `stroke-width`
//! - Discrete scale ranges: `{channel}-discrete` (list of values)
//! - Continuous scale ranges: `{channel}-continuous` (min/max values)
//!
//! Supported channels (following CSS/SVG naming conventions):
//! - **Fill color**: `fill-discrete`, `fill-continuous` (for shape fill colors)
//! - **Stroke color**: `stroke-discrete`, `stroke-continuous` (for shape outlines)
//! - **Size**: `size-discrete`, `size-continuous`
//! - **Opacity**: `opacity-discrete`, `opacity-continuous`
//! - **Stroke width**: `stroke-width-discrete`, `stroke-width-continuous`
//! - **Stroke dash**: `stroke-dash-discrete` (discrete only)
//! - **Shape**: `shape-discrete` (discrete only)
//! - **Custom**: Any channel name (e.g., `glow-color-continuous`, `pulse-speed-discrete`)
//!
//! Channel names with underscores in Rust (e.g., `stroke_width`) automatically
//! convert to hyphens in CSS (e.g., `stroke-width-discrete`).
//!
//! ## Type Detection
//!
//! The theme system automatically detects value types:
//! - **Numbers first**: If a value parses as a number, it's treated as numeric
//! - **Colors second**: If not a number, attempts to parse as color (hex, rgb, hsl, named)
//! - **Strings**: Everything else is treated as a string value
//!
//! This enables custom channels to work without hardcoding:
//! ```css
//! mark[type="custom"] {
//!     glow-color-continuous: #ff0000, #00ff00;  /* Detected as colors */
//!     intensity-continuous: 0, 100;              /* Detected as numbers */
//! }
//! ```
//!
//! ## Limitations
//!
//! ### No Positional Pseudo-Classes
//!
//! Positional pseudo-classes (`:first-child`, `:last-child`, `:nth-child()`) are not
//! supported because `ThemeContext` instances are created independently on demand without
//! sibling relationships. The theme system uses a lazy context creation model rather than
//! a materialized tree structure.
//!
//! ### No Interactive Pseudo-Classes
//!
//! Interactive pseudo-classes (`:hover`, `:active`, `:focus`) are not supported as the
//! theme system is designed for static chart styling, not runtime interactivity.

// Core theme modules
mod calc;
mod context;
mod value;

// CSS-based theme implementation modules
pub(crate) mod color_mix;
pub(crate) mod css_value;
pub(crate) mod element;
pub(crate) mod lab_color;
pub(crate) mod parser;
pub(crate) mod selector_impl;
mod theme;

// Re-export core types
pub use context::ThemeContext;
pub use value::{AngleUnit, CssRgba, LengthUnit, ThemeValue};

// Re-export Theme as the main theme type
pub use theme::Theme;

// Re-export default constants
pub use theme::{
    DEFAULT_CATEGORICAL_COLORS, DEFAULT_DASH_NAMES, DEFAULT_FONT_FAMILY, DEFAULT_SHAPE_NAMES,
};

// Helper function for font selection (used by theme_impl.rs)
/// Select the first available font from a list of font families
/// Checks against the fonts available in the system using avenger-text
pub(crate) fn select_available_font(fonts: Vec<String>) -> String {
    use avenger_text::font_resolver::{FontResolver, default_font_resolver};

    let resolver = default_font_resolver();
    resolver.select_available_font(fonts)
}
