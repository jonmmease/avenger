//! Default stroke dash patterns for scales

/// The default dash pattern names and their patterns
/// These are used both when setting default ranges for stroke_dash scales
/// and when creating line legends
///
/// Optimized for width 2 lines with rounded caps/joins:
/// - With rounded caps, each dash extends by width/2 on each end
/// - For width 2, this means gaps appear 2 units smaller than specified
/// - Minimum gap of 4 units needed for clear visibility at width 2
pub const DEFAULT_DASH_PATTERNS: &[(&str, &[f32])] = &[
    ("solid", &[]),                         // Solid line
    ("dashed", &[6.0, 6.0]),                // Dashed - balanced dash and gap
    ("dotted", &[2.0, 6.0]),                // Dotted - small dash, clear gap
    ("long-dash", &[12.0, 4.0]),            // Long dash - long dash, visible gap
    ("dash-dot", &[6.0, 4.0, 1.0, 4.0]),    // Dash-dot - dash and dot
    ("long-short", &[12.0, 4.0, 2.0, 4.0]), // Long-short - alternating lengths
];
