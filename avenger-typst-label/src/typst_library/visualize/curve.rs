//! Ported from crates/typst-library/src/visualize/curve.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: the curve geometry frames carry. The `curve` element and its components, and the
//! bounding box and hit tests, which only exporters and the IDE read, are out of scope.

use typst_utils::Numeric;

use crate::typst_library::layout::{Abs, Point, Size};

/// A curve consisting of movements, lines, and Bézier segments.
#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
pub struct Curve(pub Vec<CurveItem>);

/// An item in a curve.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum CurveItem {
    Move(Point),
    Line(Point),
    Cubic(Point, Point, Point),
    Close,
}

impl Curve {
    /// Creates an empty curve.
    pub const fn new() -> Self {
        Self(vec![])
    }

    /// Creates a curve that describes a rectangle.
    pub fn rect(size: Size) -> Self {
        let z = Abs::zero();
        let point = Point::new;
        let mut curve = Self::new();
        curve.move_(point(z, z));
        curve.line(point(size.x, z));
        curve.line(point(size.x, size.y));
        curve.line(point(z, size.y));
        curve.close();
        curve
    }

    /// Creates a curve that describes an axis-aligned ellipse.
    pub fn ellipse(size: Size) -> Self {
        // https://stackoverflow.com/a/2007782
        let z = Abs::zero();
        let rx = size.x / 2.0;
        let ry = size.y / 2.0;
        let m = 0.551784;
        let mx = m * rx;
        let my = m * ry;
        let point = |x, y| Point::new(x + rx, y + ry);

        let mut curve = Curve::new();
        curve.move_(point(-rx, z));
        curve.cubic(point(-rx, -my), point(-mx, -ry), point(z, -ry));
        curve.cubic(point(mx, -ry), point(rx, -my), point(rx, z));
        curve.cubic(point(rx, my), point(mx, ry), point(z, ry));
        curve.cubic(point(-mx, ry), point(-rx, my), point(-rx, z));
        curve
    }

    /// Push a [`Move`](CurveItem::Move) item.
    pub fn move_(&mut self, p: Point) {
        self.0.push(CurveItem::Move(p));
    }

    /// Push a [`Line`](CurveItem::Line) item.
    pub fn line(&mut self, p: Point) {
        self.0.push(CurveItem::Line(p));
    }

    /// Push a [`Cubic`](CurveItem::Cubic) item.
    pub fn cubic(&mut self, p1: Point, p2: Point, p3: Point) {
        self.0.push(CurveItem::Cubic(p1, p2, p3));
    }

    /// Push a [`Close`](CurveItem::Close) item.
    pub fn close(&mut self) {
        self.0.push(CurveItem::Close);
    }

    /// Check if the curve is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Translate all points in this curve by the given offset.
    pub fn translate(&mut self, offset: Point) {
        if offset.is_zero() {
            return;
        }
        for item in self.0.iter_mut() {
            match item {
                CurveItem::Move(p) => *p += offset,
                CurveItem::Line(p) => *p += offset,
                CurveItem::Cubic(p1, p2, p3) => {
                    *p1 += offset;
                    *p2 += offset;
                    *p3 += offset;
                }
                CurveItem::Close => (),
            }
        }
    }
}
