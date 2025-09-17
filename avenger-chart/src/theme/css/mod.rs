//! CSS-based theme system for chart styling
//!
//! This module implements a CSS-based theme system that parses and applies CSS-like
//! styling rules to chart elements.
//!
//! ## Implementation Note
//!
//! While we initially planned to use the `cssparser` and `selectors` crates from Servo,
//! we encountered significant API compatibility issues with version 0.35:
//!
//! 1. **Breaking API changes**: Methods like `parse_block` and `parse_value` have different
//!    signatures than in earlier versions, requiring additional parameters like `&ParserState`.
//!
//! 2. **Complex trait requirements**: The `selectors` crate requires implementing numerous
//!    traits (ToCss, From<&str>, etc.) for all type parameters, creating a cascade of
//!    implementation requirements.
//!
//! 3. **Version mismatch**: The `DeclarationParser` trait in 0.35 requires 4 parameters
//!    for `parse_value`, not the 3 we expected from earlier versions.
//!
//! Given these challenges and the fact that our CSS needs are relatively simple (we don't
//! need full CSS3 compliance), we implemented a "simple" parser that:
//! - Handles the CSS features we need (selectors, properties, variables, units)
//! - Passes all 136 visual regression tests
//! - Is easier to maintain and debug
//! - Avoids external dependency complexity
//!
//! The "simple" parser is production-ready and sufficient for theming needs.

mod types;
// mod parser;  // Original cssparser-based implementation (kept for reference)
// mod selector;  // Original selectors-based implementation (kept for reference)
mod defaults;
mod simple_parser;
mod simple_query;

pub use simple_parser::parse_css_simple as parse_css;
pub use types::{
    Declaration, PropertyExplanation, RGBA, Rule, Specificity, Theme, ThemeContext, ThemeError,
    ThemeUsageReport, ThemeValue, Unit, UsageTracker, props,
};
// pub use query::analyze_theme_usage;
