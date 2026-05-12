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
//! For top-level faceted charts, fixed plot area selects fixed-subplot sizing:
//! the configured `plot_size(width, height)` is interpreted as per-leaf-subplot
//! plot-area size, and the root plot area/canvas are synthesized from the facet tree.
//!
//! ```ignore
//! Plot::new()
//!     .plot_size(400.0, 300.0)
//!     .margins(Margins::uniform(10.0))
//! ```
//!
//! # Responsive Sizing
//! Flexible mode with constraints on canvas and/or plot area. Supports partial
//! constraints like fixed width with flexible height, fixed height with flexible width, etc.
//!
//! ```ignore
//! Plot::new()
//!     .canvas_constraint(CanvasConstraint::Width(600.0))  // Fixed width, height adjusts
//!     .plot_constraint(PlotConstraint::Width(400.0))      // Fixed plot width
//! ```
//!
//! # Margin Behavior
//!
//! Margins are treated as **fixed values** that reserve space around the chart.
//! The plot area is flexible and expands/shrinks to fill remaining space. For example:
//! - Canvas size (400x400) with margins (50px all sides) = plot area gets 300x300
//! - Fixed canvas with large margins = plot area shrinks to fit
//!
//! When both canvas and plot dimensions are fixed in the same direction, margins become
//! expandable and will grow to center the plot within the canvas.

use datafusion::prelude::{Expr, lit};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{
    maybe::{Maybe, MaybeOptionalExpr},
    serialization::{LogicalExprNodeExt, SerializableExpr},
};

/// Trait for types that can be converted to Expr (for dimensions)
pub trait IntoExprDimension {
    fn into_expr_dim(self) -> Expr;
}

impl IntoExprDimension for Expr {
    fn into_expr_dim(self) -> Expr {
        self
    }
}

impl IntoExprDimension for f32 {
    fn into_expr_dim(self) -> Expr {
        lit(self)
    }
}

impl IntoExprDimension for f64 {
    fn into_expr_dim(self) -> Expr {
        lit(self)
    }
}

impl IntoExprDimension for i32 {
    fn into_expr_dim(self) -> Expr {
        lit(self)
    }
}

impl IntoExprDimension for i64 {
    fn into_expr_dim(self) -> Expr {
        lit(self)
    }
}

/// Constraints that can be applied to the canvas
///
/// Dimensions accept numeric literals (e.g., `800.0`), `Expr` values, or column references
#[derive(Clone, Debug, Default)]
pub enum CanvasConstraint {
    /// No constraint - canvas shrinks to fit content
    #[default]
    None,

    /// Fixed width, height adjusts to content
    Width(Expr),

    /// Fixed height, width adjusts to content
    Height(Expr),
}

impl CanvasConstraint {
    /// Create a width constraint from a numeric value
    pub fn width<T: IntoExprDimension>(value: T) -> Self {
        Self::Width(value.into_expr_dim())
    }

    /// Create a height constraint from a numeric value
    pub fn height<T: IntoExprDimension>(value: T) -> Self {
        Self::Height(value.into_expr_dim())
    }
}

/// Constraints that can be applied to the plot area
#[derive(Clone, Debug, Default)]
pub enum PlotConstraint {
    /// Plot area fills available space in canvas
    #[default]
    Auto,

    /// Fixed plot width, height adjusts
    Width(Expr),

    /// Fixed plot height, width adjusts
    Height(Expr),
}

impl PlotConstraint {
    /// Create a width constraint from a numeric value
    pub fn width<T: IntoExprDimension>(value: T) -> Self {
        Self::Width(value.into_expr_dim())
    }

    /// Create a height constraint from a numeric value
    pub fn height<T: IntoExprDimension>(value: T) -> Self {
        Self::Height(value.into_expr_dim())
    }
}

// Internal representation for layout computation
// This stores dimension expressions that are evaluated at render time
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum SizeMode {
    Fixed {
        width: SerializableExpr,
        height: SerializableExpr,
    },
    Width(SerializableExpr),
    Height(SerializableExpr),
    Auto,
}

// Evaluated version of SizeMode with concrete f32 values for layout computation
#[derive(Clone, Debug)]
pub(crate) enum EvaluatedSizeMode {
    Fixed { width: f32, height: f32 },
    Width(f32),
    Height(f32),
    Auto,
}

impl From<CanvasConstraint> for SizeMode {
    fn from(constraint: CanvasConstraint) -> Self {
        match constraint {
            CanvasConstraint::None => SizeMode::Auto,
            CanvasConstraint::Width(w) => SizeMode::Width(w.into()),
            CanvasConstraint::Height(h) => SizeMode::Height(h.into()),
        }
    }
}

impl From<PlotConstraint> for SizeMode {
    fn from(constraint: PlotConstraint) -> Self {
        match constraint {
            PlotConstraint::Auto => SizeMode::Auto,
            PlotConstraint::Width(w) => SizeMode::Width(w.into()),
            PlotConstraint::Height(h) => SizeMode::Height(h.into()),
        }
    }
}

/// Complete layout specification with dimension expressions
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayoutSpec {
    /// Canvas sizing mode (internal representation)
    pub(crate) canvas: SizeMode,

    /// Plot area sizing mode (internal representation)
    pub(crate) plot_area: SizeMode,

    /// Fixed margins around entire chart
    pub margins: Margins,
}

/// Evaluated margins with concrete f32 values
#[derive(Clone, Debug)]
pub(crate) struct EvaluatedMargins {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// Evaluated layout specification with concrete f32 values for layout computation
#[derive(Clone, Debug)]
pub(crate) struct EvaluatedLayoutSpec {
    /// Canvas sizing mode with evaluated dimensions
    pub(crate) canvas: EvaluatedSizeMode,

    /// Plot area sizing mode with evaluated dimensions
    pub(crate) plot_area: EvaluatedSizeMode,

    /// Fixed margins around entire chart
    pub margins: EvaluatedMargins,
}

impl EvaluatedLayoutSpec {
    /// Determine if horizontal margins should be expandable
    pub(crate) fn should_expand_margins_horizontal(&self) -> bool {
        let canvas_width_fixed = matches!(
            self.canvas,
            EvaluatedSizeMode::Fixed { .. } | EvaluatedSizeMode::Width(_)
        );
        let plot_width_constrained = matches!(
            self.plot_area,
            EvaluatedSizeMode::Fixed { .. } | EvaluatedSizeMode::Width(_)
        );
        canvas_width_fixed && plot_width_constrained
    }

    /// Determine if vertical margins should be expandable
    pub(crate) fn should_expand_margins_vertical(&self) -> bool {
        let canvas_height_fixed = matches!(
            self.canvas,
            EvaluatedSizeMode::Fixed { .. } | EvaluatedSizeMode::Height(_)
        );
        let plot_height_constrained = matches!(
            self.plot_area,
            EvaluatedSizeMode::Fixed { .. } | EvaluatedSizeMode::Height(_)
        );
        canvas_height_fixed && plot_height_constrained
    }
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
    pub fn fixed_canvas(width: Expr, height: Expr, margins: Margins) -> Self {
        Self {
            canvas: SizeMode::Fixed {
                width: width.into(),
                height: height.into(),
            },
            plot_area: SizeMode::Auto,
            margins,
        }
    }

    /// Create a layout spec with fixed plot area size
    pub fn fixed_plot_area(width: Expr, height: Expr, margins: Margins) -> Self {
        Self {
            canvas: SizeMode::Auto,
            plot_area: SizeMode::Fixed {
                width: width.into(),
                height: height.into(),
            },
            margins,
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    #[serde_as(as = "MaybeOptionalExpr")]
    pub top: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub right: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub bottom: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub left: Maybe<Option<LogicalExprNode>>,
}

impl Default for Margins {
    fn default() -> Self {
        Self::uniform(10.0) // Sensible default
    }
}

impl Margins {
    /// Create margins with uniform size
    pub fn uniform(size: impl IntoExprDimension) -> Self {
        let expr = size.into_expr_dim();
        let node = LogicalExprNode::from_expr(expr).expect("Failed to serialize margin expr");
        Self {
            top: Maybe::Set(Some(node.clone())),
            right: Maybe::Set(Some(node.clone())),
            bottom: Maybe::Set(Some(node.clone())),
            left: Maybe::Set(Some(node)),
        }
    }

    /// Create margins with symmetric vertical and horizontal values
    pub fn symmetric(vertical: impl IntoExprDimension, horizontal: impl IntoExprDimension) -> Self {
        let v_expr = vertical.into_expr_dim();
        let h_expr = horizontal.into_expr_dim();
        let v_node = LogicalExprNode::from_expr(v_expr).expect("Failed to serialize margin expr");
        let h_node = LogicalExprNode::from_expr(h_expr).expect("Failed to serialize margin expr");
        Self {
            top: Maybe::Set(Some(v_node.clone())),
            right: Maybe::Set(Some(h_node.clone())),
            bottom: Maybe::Set(Some(v_node)),
            left: Maybe::Set(Some(h_node)),
        }
    }

    /// Create margins with no space
    pub fn none() -> Self {
        Self::uniform(0.0)
    }

    /// Set the top margin
    pub fn top(mut self, value: impl IntoExprDimension) -> Self {
        let expr = value.into_expr_dim();
        self.top = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize margin expr"),
        ));
        self
    }

    /// Set the right margin
    pub fn right(mut self, value: impl IntoExprDimension) -> Self {
        let expr = value.into_expr_dim();
        self.right = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize margin expr"),
        ));
        self
    }

    /// Set the bottom margin
    pub fn bottom(mut self, value: impl IntoExprDimension) -> Self {
        let expr = value.into_expr_dim();
        self.bottom = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize margin expr"),
        ));
        self
    }

    /// Set the left margin
    pub fn left(mut self, value: impl IntoExprDimension) -> Self {
        let expr = value.into_expr_dim();
        self.left = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize margin expr"),
        ));
        self
    }
}
