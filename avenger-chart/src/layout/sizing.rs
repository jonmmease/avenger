//! Chart layout sizing system
//!
//! This module provides a type-safe API for configuring chart layouts with three
//! primary sizing strategies:
//!
//! # Fixed Canvas
//! Traditional mode where the overall chart dimensions are fixed and the plot area
//! fills the available space after accounting for margins, axes, and legends.
//!
//! ```ignore
//! Plot::new()
//!     .canvas_size(800.0, 600.0)
//!     .margins(Margins::uniform(20.0))
//! ```
//!
//! # Fixed Plot Area
//! Data-first mode where the plot area (where data is rendered) has fixed dimensions
//! and the canvas expands to accommodate margins, axes, legends, and titles.
//!
//! ```ignore
//! Plot::new()
//!     .plot_size(400.0, 300.0)
//!     .margins(Margins::uniform(10.0))
//! ```
//!
//! # Responsive Sizing
//! Flexible mode with constraints on canvas and/or plot area. Supports partial
//! constraints like fixed width with flexible height, aspect ratios, etc.
//!
//! ```ignore
//! Plot::new()
//!     .canvas_constraint(CanvasConstraint::Width(600.0))  // Fixed width, height adjusts
//!     .plot_constraint(PlotConstraint::AspectRatio(16.0/9.0))  // Plot maintains aspect ratio
//! ```
//!
//! # Margin Behavior
//!
//! Margins are treated as **fixed values** that reserve space around the chart.
//! The plot area is flexible and expands/shrinks to fill remaining space. For example:
//! - Canvas size (400x400) with margins (50px all sides) = plot area gets 300x300
//! - Fixed canvas with large margins = plot area shrinks to fit
//!
//! # Limitations
//!
//! - Canvas aspect ratio is "preferred" - it may be overridden if content requires more space
//! - Canvas and plot aspect ratios cannot be used together (creates conflicting constraints)

use serde::{Deserialize, Serialize};

/// Constraints that can be applied to the canvas
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasConstraint {
    /// No constraint - canvas shrinks to fit content
    None,

    /// Fixed width, height adjusts to content
    Width(f32),

    /// Fixed height, width adjusts to content
    Height(f32),

    /// Preferred aspect ratio (width/height)
    /// Note: This is a "preferred" ratio that may be overridden if content requires more space
    PreferredAspectRatio(f32),
}

impl Default for CanvasConstraint {
    fn default() -> Self {
        Self::None
    }
}

/// Constraints that can be applied to the plot area
#[derive(Clone, Debug, PartialEq)]
pub enum PlotConstraint {
    /// Plot area fills available space in canvas
    Auto,

    /// Plot area maintains aspect ratio within available space
    AspectRatio(f32),

    /// Fixed plot width, height adjusts
    Width(f32),

    /// Fixed plot height, width adjusts
    Height(f32),
}

impl Default for PlotConstraint {
    fn default() -> Self {
        Self::Auto
    }
}

// Internal representation for layout computation
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum SizeMode {
    Fixed { width: f32, height: f32 },
    Width(f32),
    Height(f32),
    AspectRatio(f32),
    Auto,
}

impl From<CanvasConstraint> for SizeMode {
    fn from(constraint: CanvasConstraint) -> Self {
        match constraint {
            CanvasConstraint::None => SizeMode::Auto,
            CanvasConstraint::Width(w) => SizeMode::Width(w),
            CanvasConstraint::Height(h) => SizeMode::Height(h),
            CanvasConstraint::PreferredAspectRatio(r) => SizeMode::AspectRatio(r),
        }
    }
}

impl From<PlotConstraint> for SizeMode {
    fn from(constraint: PlotConstraint) -> Self {
        match constraint {
            PlotConstraint::Auto => SizeMode::Auto,
            PlotConstraint::AspectRatio(r) => SizeMode::AspectRatio(r),
            PlotConstraint::Width(w) => SizeMode::Width(w),
            PlotConstraint::Height(h) => SizeMode::Height(h),
        }
    }
}

/// Complete layout specification
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutSpec {
    /// Canvas sizing mode (internal representation)
    pub(crate) canvas: SizeMode,

    /// Plot area sizing mode (internal representation)
    pub(crate) plot_area: SizeMode,

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
    pub fn fixed_canvas(width: f32, height: f32, margins: Margins) -> Self {
        Self {
            canvas: SizeMode::Fixed { width, height },
            plot_area: SizeMode::Auto,
            margins,
        }
    }

    /// Create a layout spec with fixed plot area size
    pub fn fixed_plot_area(width: f32, height: f32, margins: Margins) -> Self {
        Self {
            canvas: SizeMode::Auto,
            plot_area: SizeMode::Fixed { width, height },
            margins,
        }
    }

    /// Determine if horizontal margins should be expandable
    ///
    /// Margins expand horizontally when canvas width is fixed AND plot width is constrained
    /// (either directly fixed, or can be computed from height + aspect ratio)
    pub(crate) fn should_expand_margins_horizontal(&self) -> bool {
        let canvas_width_fixed = matches!(
            self.canvas,
            SizeMode::Fixed { .. } | SizeMode::Width(_)
        );

        let plot_width_constrained = matches!(
            self.plot_area,
            SizeMode::Fixed { .. } | SizeMode::Width(_)
        );

        canvas_width_fixed && plot_width_constrained
    }

    /// Determine if vertical margins should be expandable
    ///
    /// Margins expand vertically when canvas height is fixed AND plot height is constrained
    /// (either directly fixed, or can be computed from width + aspect ratio)
    pub(crate) fn should_expand_margins_vertical(&self) -> bool {
        let canvas_height_fixed = matches!(
            self.canvas,
            SizeMode::Fixed { .. } | SizeMode::Height(_)
        );

        let plot_height_constrained = matches!(
            self.plot_area,
            SizeMode::Fixed { .. } | SizeMode::Height(_)
        );

        canvas_height_fixed && plot_height_constrained
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
