//! Default stroke dash pattern names for scales

/// The default dash pattern names used for stroke_dash scales
/// The actual numeric patterns are defined in the Coercer to avoid duplication
///
/// Pattern characteristics (optimized for width 2 lines with rounded caps/joins):
/// - solid: No dashing
/// - dashed: Balanced dash and gap pattern
/// - dotted: Small dots with clear gaps
/// - long-dash: Long dashes with visible gaps
/// - dash-dot: Dash with two dots
/// - long-short: Long dash followed by short dash
/// - double-dash: Regular medium dashes
/// - even-short: Dash-dot-short-dot-dash symmetric pattern
pub const DEFAULT_DASH_PATTERN_NAMES: &[&str] = &[
    "solid",
    "dashed",
    "dotted",
    "long-dash",
    "dash-dot",
    "long-short",
    "even-short",
    "double-dash",
];
