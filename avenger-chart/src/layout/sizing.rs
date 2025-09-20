//! Flexible layout sizing API for charts

/// Defines how a rectangular area should be sized
#[derive(Clone, Debug, PartialEq)]
pub enum SizeMode {
    /// Exact dimensions specified
    Fixed { width: f32, height: f32 },

    /// Width specified, height determined by constraints
    Width(f32),

    /// Height specified, width determined by constraints
    Height(f32),

    /// Maintain aspect ratio (width/height)
    AspectRatio(f32),

    /// Automatic sizing:
    /// - For canvas: shrink to fit content
    /// - For plot area: expand to fill available space
    Auto,
}

impl Default for SizeMode {
    fn default() -> Self {
        Self::Auto
    }
}

/// Complete layout specification
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSpec {
    /// Size of the overall canvas/viewport
    pub canvas: SizeMode,

    /// Size of the plot area (where data is rendered)
    pub plot_area: SizeMode,

    /// Fixed margins around entire chart
    pub margins: Margins,
}

impl Default for LayoutSpec {
    fn default() -> Self {
        Self {
            canvas: SizeMode::Auto,
            plot_area: SizeMode::Auto,
            margins: Margins::default(),
        }
    }
}

impl LayoutSpec {
    /// Create a layout spec with fixed canvas size (traditional mode)
    pub fn with_canvas_size(width: f32, height: f32) -> Self {
        Self {
            canvas: SizeMode::Fixed { width, height },
            plot_area: SizeMode::Auto,
            margins: Margins::default(),
        }
    }

    /// Create a layout spec with fixed plot area size
    pub fn with_plot_size(width: f32, height: f32) -> Self {
        Self {
            canvas: SizeMode::Auto,
            plot_area: SizeMode::Fixed { width, height },
            margins: Margins::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Margins {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Default for Margins {
    fn default() -> Self {
        Self::uniform(10.0) // Sensible default
    }
}

impl Margins {
    pub fn uniform(size: f32) -> Self {
        Self {
            top: size,
            right: size,
            bottom: size,
            left: size,
        }
    }

    pub fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    pub fn none() -> Self {
        Self::uniform(0.0)
    }
}

/// Result of computing layout with flexible sizing
#[derive(Debug, Clone)]
pub struct ComputeResult {
    /// The computed layout bounds for all components
    pub layout: crate::layout::LayoutResult,
    /// Actual canvas dimensions (may differ from requested in Auto mode)
    pub canvas_size: (f32, f32),
}
