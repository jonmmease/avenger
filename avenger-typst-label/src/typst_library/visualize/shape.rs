//! Ported from crates/typst-library/src/visualize/shape.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: the shape primitives frames carry. The shape elements (`rect`, `square`, `ellipse`,
//! `circle`) and the bounding boxes, which only exporters read, are out of scope.

use crate::typst_library::foundations::derive_cast;
use crate::typst_library::layout::{Point, Size};
use crate::typst_library::visualize::{Curve, FixedStroke, Paint};

/// A geometric shape with optional fill and stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// The shape's geometry.
    pub geometry: Geometry,
    /// The shape's background fill.
    pub fill: Option<Paint>,
    /// The shape's fill rule.
    pub fill_rule: FillRule,
    /// The shape's border stroke.
    pub stroke: Option<FixedStroke>,
}

/// A fill rule for curve drawing.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash)]
pub enum FillRule {
    /// Specifies that "inside" is computed by a non-zero sum of signed edge crossings.
    #[default]
    NonZero,
    /// Specifies that "inside" is computed by an odd number of edge crossings.
    EvenOdd,
}

derive_cast!(FillRule { NonZero, EvenOdd });

/// A shape's geometry.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Geometry {
    /// A line to a point (relative to its position).
    Line(Point),
    /// A rectangle with its origin in the topleft corner.
    Rect(Size),
    /// A curve consisting of movements, lines, and Bézier segments.
    Curve(Curve),
}

impl Geometry {
    /// Fill the geometry without a stroke.
    pub fn filled(self, fill: impl Into<Paint>) -> Shape {
        Shape {
            geometry: self,
            fill: Some(fill.into()),
            fill_rule: FillRule::default(),
            stroke: None,
        }
    }

    /// Stroke the geometry without a fill.
    pub fn stroked(self, stroke: FixedStroke) -> Shape {
        Shape {
            geometry: self,
            fill: None,
            fill_rule: FillRule::default(),
            stroke: Some(stroke),
        }
    }

    /// Set the geometry's background fill and stroke.
    pub fn filled_and_stroked(
        self,
        fill: impl Into<Paint>,
        stroke: FixedStroke,
    ) -> Shape {
        Shape {
            geometry: self,
            fill: Some(fill.into()),
            fill_rule: FillRule::default(),
            stroke: Some(stroke),
        }
    }
}
