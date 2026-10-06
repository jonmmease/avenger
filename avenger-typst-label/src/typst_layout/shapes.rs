//! Ported from crates/typst-layout/src/shapes.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: the styled rectangles that highlights draw. The shape elements (`line`, `curve`,
//! `polygon`, `rect`, `square`, `ellipse`, `circle`) are out of scope.

use crate::typst_library::layout::{Abs, Corner, Corners, Point, Rel, Sides, Size};
use crate::typst_library::visualize::{
    Curve, FillRule, FixedStroke, Geometry, LineCap, Paint, Shape,
};
use crate::typst_utils::Get;

/// Create a styled rectangle with shapes.
/// - use rect primitive for simple rectangles
/// - stroke sides if possible
/// - use fill for sides for best looks
pub fn styled_rect(
    size: Size,
    radius: &Corners<Rel<Abs>>,
    fill: Option<Paint>,
    stroke: &Sides<Option<FixedStroke>>,
) -> Vec<Shape> {
    if stroke.is_uniform() && radius.iter().cloned().all(Rel::is_zero) {
        simple_rect(size, fill, stroke.top.clone())
    } else {
        segmented_rect(size, radius, fill, stroke)
    }
}

/// Use rect primitive for the rectangle
fn simple_rect(
    size: Size,
    fill: Option<Paint>,
    stroke: Option<FixedStroke>,
) -> Vec<Shape> {
    vec![Shape {
        geometry: Geometry::Rect(size),
        fill,
        stroke,
        fill_rule: FillRule::default(),
    }]
}

fn corners_control_points(
    size: Size,
    radius: &Corners<Abs>,
    strokes: &Sides<Option<FixedStroke>>,
    stroke_widths: &Sides<Option<Abs>>,
) -> Corners<ControlPoints> {
    Corners {
        top_left: Corner::TopLeft,
        top_right: Corner::TopRight,
        bottom_right: Corner::BottomRight,
        bottom_left: Corner::BottomLeft,
    }
    .map(|corner| ControlPoints {
        radius: radius.get(corner),
        stroke_before: stroke_widths.get(corner.side_ccw()),
        stroke_after: stroke_widths.get(corner.side_cw()),
        corner,
        size,
        same: match (
            strokes.get_ref(corner.side_ccw()),
            strokes.get_ref(corner.side_cw()),
        ) {
            (Some(a), Some(b)) => {
                // Solid strokes can be drawn as `fill_segment`s.
                let solid =
                    a.dash.as_ref().map(|dash| dash.array.is_empty()).unwrap_or(true);

                // For solid strokes the caps are only relevant for the end of
                // the strokes, and they can be filled. For dashed strokes the
                // cap determines how the entire line is drawn, thus there
                // should be two different segments if the cap differs.
                let filled_segment_same = a.paint == b.paint && a.dash == b.dash;
                let stroke_segment_same = a.cap == b.cap && a.thickness == b.thickness;

                filled_segment_same && (solid || stroke_segment_same)
            }
            (None, None) => true,
            _ => false,
        },
    })
}

/// Use stroke and fill for the rectangle
fn segmented_rect(
    size: Size,
    radius: &Corners<Rel<Abs>>,
    fill: Option<Paint>,
    strokes: &Sides<Option<FixedStroke>>,
) -> Vec<Shape> {
    let mut res = vec![];
    let stroke_widths = strokes.as_ref().map(|s| s.as_ref().map(|s| s.thickness / 2.0));

    let base_radius = size.x.abs().min(size.y.abs()) / 2.0;
    let corner_max =
        stroke_widths.map_corners(|a, b| base_radius + a.min(b).unwrap_or(Abs::zero()));

    let radius = radius
        .zip(corner_max)
        .map(|(value, max)| value.relative_to(max * 2.0).min(max));
    let corners = corners_control_points(size, &radius, strokes, &stroke_widths);

    // insert stroked sides below filled sides
    let mut stroke_insert = 0;

    // fill shape with inner curve
    if let Some(fill) = fill {
        let mut curve = Curve::new();
        let c = corners.get_ref(Corner::TopLeft);
        if c.arc() {
            curve.arc_move(c.start(), c.center(), c.end());
        } else {
            curve.move_(c.center());
        };

        for corner in [Corner::TopRight, Corner::BottomRight, Corner::BottomLeft] {
            let c = corners.get_ref(corner);
            if c.arc() {
                curve.arc_line(c.start(), c.center(), c.end());
            } else {
                curve.line(c.center());
            }
        }
        curve.close();
        res.push(Shape {
            geometry: Geometry::Curve(curve),
            fill: Some(fill),
            fill_rule: FillRule::default(),
            stroke: None,
        });
        stroke_insert += 1;
    }

    let current = corners.iter().find(|c| !c.same).map(|c| c.corner);
    if let Some(mut current) = current {
        // multiple segments
        // start at a corner with a change between sides and iterate clockwise all other corners
        let mut last = current;
        for _ in 0..4 {
            current = current.next_cw();
            if corners.get_ref(current).same {
                continue;
            }
            // create segment
            let start = last;
            let end = current;
            last = current;
            let Some(stroke) = strokes.get_ref(start.side_cw()) else { continue };
            let start_cap = stroke.cap;
            let end_cap = match strokes.get_ref(end.side_ccw()) {
                Some(stroke) => stroke.cap,
                None => start_cap,
            };
            let (shape, ontop) =
                segment(start, end, start_cap, end_cap, &corners, stroke);
            if ontop {
                res.push(shape);
            } else {
                res.insert(stroke_insert, shape);
                stroke_insert += 1;
            }
        }
    } else if let Some(stroke) = &strokes.top {
        // single segment
        let (shape, _) = segment(
            Corner::TopLeft,
            Corner::TopLeft,
            stroke.cap,
            stroke.cap,
            &corners,
            stroke,
        );
        res.push(shape);
    }
    res
}

fn curve_segment(
    start: Corner,
    end: Corner,
    corners: &Corners<ControlPoints>,
    curve: &mut Curve,
) {
    // create start corner
    let c = corners.get_ref(start);
    if start == end || !c.arc() {
        curve.move_(c.end());
    } else {
        curve.arc_move(c.mid(), c.center(), c.end());
    }

    // create corners between start and end
    let mut current = start.next_cw();
    while current != end {
        let c = corners.get_ref(current);
        if c.arc() {
            curve.arc_line(c.start(), c.center(), c.end());
        } else {
            curve.line(c.end());
        }
        current = current.next_cw();
    }

    // create end corner
    let c = corners.get_ref(end);
    if !c.arc() {
        curve.line(c.start());
    } else if start == end {
        curve.arc_line(c.start(), c.center(), c.end());
    } else {
        curve.arc_line(c.start(), c.center(), c.mid());
    }
}

/// Returns the shape for the segment and whether the shape should be drawn on top.
fn segment(
    start: Corner,
    end: Corner,
    start_cap: LineCap,
    end_cap: LineCap,
    corners: &Corners<ControlPoints>,
    stroke: &FixedStroke,
) -> (Shape, bool) {
    fn fill_corner(corner: &ControlPoints) -> bool {
        corner.stroke_before != corner.stroke_after
            || corner.radius() < corner.stroke_width_before()
    }

    fn fill_corners(
        start: Corner,
        end: Corner,
        corners: &Corners<ControlPoints>,
    ) -> bool {
        if fill_corner(corners.get_ref(start)) {
            return true;
        }
        if fill_corner(corners.get_ref(end)) {
            return true;
        }
        let mut current = start.next_cw();
        while current != end {
            if fill_corner(corners.get_ref(current)) {
                return true;
            }
            current = current.next_cw();
        }
        false
    }

    let solid = stroke.dash.as_ref().map(|dash| dash.array.is_empty()).unwrap_or(true);

    let use_fill = solid && fill_corners(start, end, corners);
    let shape = if use_fill {
        fill_segment(start, end, start_cap, end_cap, corners, stroke)
    } else {
        stroke_segment(start, end, corners, stroke.clone())
    };

    (shape, use_fill)
}

/// Stroke the sides from `start` to `end` clockwise.
fn stroke_segment(
    start: Corner,
    end: Corner,
    corners: &Corners<ControlPoints>,
    stroke: FixedStroke,
) -> Shape {
    // Create start corner.
    let mut curve = Curve::new();
    curve_segment(start, end, corners, &mut curve);

    Shape {
        geometry: Geometry::Curve(curve),
        stroke: Some(stroke),
        fill: None,
        fill_rule: FillRule::default(),
    }
}

/// Fill the sides from `start` to `end` clockwise.
fn fill_segment(
    start: Corner,
    end: Corner,
    start_cap: LineCap,
    end_cap: LineCap,
    corners: &Corners<ControlPoints>,
    stroke: &FixedStroke,
) -> Shape {
    let mut curve = Curve::new();

    // create the start corner
    // begin on the inside and finish on the outside
    // no corner if start and end are equal
    // half corner if different
    if start == end {
        let c = corners.get_ref(start);
        curve.move_(c.end_inner());
        curve.line(c.end_outer());
    } else {
        let c = corners.get_ref(start);

        if c.arc_inner() {
            curve.arc_move(c.end_inner(), c.center_inner(), c.mid_inner());
        } else {
            curve.move_(c.end_inner());
        }

        c.start_cap(&mut curve, start_cap);
        if c.arc_outer() {
            curve.arc_line(c.mid_outer(), c.center_outer(), c.end_outer());
        }
    }

    // create the clockwise outside curve for the corners between start and end
    let mut current = start.next_cw();
    while current != end {
        let c = corners.get_ref(current);
        if c.arc_outer() {
            curve.arc_line(c.start_outer(), c.center_outer(), c.end_outer());
        } else {
            curve.line(c.outer());
        }
        current = current.next_cw();
    }

    // create the end corner
    // begin on the outside and finish on the inside
    // full corner if start and end are equal
    // half corner if different
    if start == end {
        let c = corners.get_ref(end);
        if c.arc_outer() {
            curve.arc_line(c.start_outer(), c.center_outer(), c.end_outer());
        } else {
            curve.line(c.outer());
            curve.line(c.end_outer());
        }
        if c.arc_inner() {
            curve.arc_line(c.end_inner(), c.center_inner(), c.start_inner());
        } else {
            curve.line(c.center_inner());
        }
    } else {
        let c = corners.get_ref(end);
        if c.arc_outer() {
            curve.arc_line(c.start_outer(), c.center_outer(), c.mid_outer());
        } else {
            curve.line(c.outer());
        }
        c.end_cap(&mut curve, end_cap);
        if c.arc_inner() {
            curve.arc_line(c.mid_inner(), c.center_inner(), c.start_inner());
        }
    }

    // create the counterclockwise inside curve for the corners between start and end
    let mut current = end.next_ccw();
    while current != start {
        let c = corners.get_ref(current);
        if c.arc_inner() {
            curve.arc_line(c.end_inner(), c.center_inner(), c.start_inner());
        } else {
            curve.line(c.center_inner());
        }
        current = current.next_ccw();
    }

    curve.close();

    Shape {
        geometry: Geometry::Curve(curve),
        stroke: None,
        fill: Some(stroke.paint.clone()),
        fill_rule: FillRule::default(),
    }
}

/// Helper to calculate different control points for the corners.
/// Clockwise orientation from start to end.
/// ```text
/// O-------------------EO  ---   - Z: Zero/Origin ({x: 0, y: 0} for top left corner)
/// |\   ___----'''     |    |    - O: Outer: intersection between the straight outer lines
/// | \ /               |    |    - S_: start
/// |  MO               |    |    - M_: midpoint
/// | /Z\  __-----------E    |    - E_: end
/// |/   \M             |    ro   - r_: radius
/// |    /\             |    |    - middle of the stroke
/// |   /  \            |    |      - arc from S through M to E with center C and radius r
/// |  |    MI--EI-------    |    - outer curve
/// |  |  /  \               |      - arc from SO through MO to EO with center CO and radius ro
/// SO | |    \         CO  ---   - inner curve
/// |  | |     \                    - arc from SI through MI to EI with center CI and radius ri
/// |--S-SI-----CI      C
///      |--ri--|
///    |-------r--------|
/// ```
struct ControlPoints {
    radius: Abs,
    stroke_after: Option<Abs>,
    stroke_before: Option<Abs>,
    corner: Corner,
    size: Size,
    same: bool,
}

impl ControlPoints {
    /// Move and rotate the point from top-left to the required corner.
    fn rotate(&self, point: Point) -> Point {
        match self.corner {
            Corner::TopLeft => Point {
                x: self.size.x.signum() * point.x,
                y: self.size.y.signum() * point.y,
            },
            Corner::TopRight => Point {
                x: self.size.x - self.size.x.signum() * point.y,
                y: self.size.y.signum() * point.x,
            },
            Corner::BottomRight => Point {
                x: self.size.x - self.size.x.signum() * point.x,
                y: self.size.y - self.size.y.signum() * point.y,
            },
            Corner::BottomLeft => Point {
                x: self.size.x.signum() * point.y,
                y: self.size.y - self.size.y.signum() * point.x,
            },
        }
    }

    /// Whether to use the [`Self::stroke_after`] if [`Self::stroke_before`] is
    /// missing to compute the control points. If the radius is too small, caps
    /// other than the [`LineCap::Butt`] might be misshaped.
    fn reuse_stroke_after_for_cap(&self) -> Option<Abs> {
        self.stroke_after.filter(|s| 2.0 * *s < self.radius)
    }

    /// Either fall back to [`Self::reuse_stroke_after_for_cap`] or zero.
    fn stroke_width_before(&self) -> Abs {
        self.stroke_before
            .or(self.reuse_stroke_after_for_cap())
            .unwrap_or(Abs::zero())
    }

    /// Whether to use the [`Self::stroke_before`] if [`Self::stroke_after`] is
    /// missing to compute the control points. If the radius is too small, caps
    /// other than the [`LineCap::Butt`] might be misshaped.
    fn reuse_stroke_before_for_cap(&self) -> Option<Abs> {
        self.stroke_before.filter(|s| 2.0 * *s < self.radius)
    }

    /// Either fall back to [`Self::reuse_stroke_before_for_cap`] or zero.
    fn stroke_width_after(&self) -> Abs {
        self.stroke_after
            .or(self.reuse_stroke_before_for_cap())
            .unwrap_or(Abs::zero())
    }

    /// Outside intersection of the sides.
    pub fn outer(&self) -> Point {
        self.rotate(Point {
            x: -self.stroke_width_before(),
            y: -self.stroke_width_after(),
        })
    }

    /// Center for the outer arc.
    pub fn center_outer(&self) -> Point {
        let r = self.radius_outer();
        self.rotate(Point {
            x: r - self.stroke_width_before(),
            y: r - self.stroke_width_after(),
        })
    }

    /// Center for the middle arc.
    pub fn center(&self) -> Point {
        let r = self.radius();
        self.rotate(Point { x: r, y: r })
    }

    /// Center for the inner arc.
    pub fn center_inner(&self) -> Point {
        let r = self.radius_inner();

        self.rotate(Point {
            x: self.stroke_width_before() + r,
            y: self.stroke_width_after() + r,
        })
    }

    /// Radius of the outer arc.
    pub fn radius_outer(&self) -> Abs {
        self.radius
    }

    /// Radius of the middle arc.
    pub fn radius(&self) -> Abs {
        (self.radius - self.stroke_width_before().min(self.stroke_width_after()))
            .max(Abs::zero())
    }

    /// Radius of the inner arc.
    pub fn radius_inner(&self) -> Abs {
        (self.radius - 2.0 * self.stroke_width_before().max(self.stroke_width_after()))
            .max(Abs::zero())
    }

    /// Middle of the corner on the outside of the stroke.
    pub fn mid_outer(&self) -> Point {
        let c_i = self.center_inner();
        let c_o = self.center_outer();
        let o = self.outer();
        let r = self.radius_outer();

        // https://math.stackexchange.com/a/311956
        // intersection between the line from inner center to outside and the outer arc
        let a = (o.x - c_i.x).scalar().powi(2) + (o.y - c_i.y).scalar().powi(2);
        let b = 2.0 * (o.x - c_i.x).scalar() * (c_i.x - c_o.x).scalar()
            + 2.0 * (o.y - c_i.y).scalar() * (c_i.y - c_o.y).scalar();
        let c = (c_i.x - c_o.x).scalar().powi(2) + (c_i.y - c_o.y).scalar().powi(2)
            - r.scalar().powi(2);
        let t = (-b + (b * b - 4.0 * a * c).max(0.0.into()).sqrt()) / (2.0 * a);
        c_i + t.get() * (o - c_i)
    }

    /// Middle of the corner in the middle of the stroke.
    pub fn mid(&self) -> Point {
        let center = self.center();
        let outer = self.outer();
        let diff = outer - center;
        center + diff / diff.hypot().to_raw() * self.radius().to_raw()
    }

    /// Middle of the corner on the inside of the stroke.
    pub fn mid_inner(&self) -> Point {
        let center = self.center_inner();
        let outer = self.outer();
        let diff = outer - center;
        center + diff / diff.hypot().to_raw() * self.radius_inner().to_raw()
    }

    /// If an outer arc is required.
    pub fn arc_outer(&self) -> bool {
        self.radius_outer() > Abs::zero()
    }

    pub fn arc(&self) -> bool {
        self.radius() > Abs::zero()
    }

    /// If an inner arc is required.
    pub fn arc_inner(&self) -> bool {
        self.radius_inner() > Abs::zero()
    }

    /// Start of the corner on the outside of the stroke.
    pub fn start_outer(&self) -> Point {
        self.rotate(Point {
            x: -self.stroke_width_before(),
            y: self.radius_outer() - self.stroke_width_after(),
        })
    }

    /// Start of the corner in the center of the stroke.
    pub fn start(&self) -> Point {
        self.rotate(Point::with_y(self.radius()))
    }

    /// Start of the corner on the inside of the stroke.
    pub fn start_inner(&self) -> Point {
        self.rotate(Point {
            x: self.stroke_width_before(),
            y: self.stroke_width_after() + self.radius_inner(),
        })
    }

    /// End of the corner on the outside of the stroke.
    pub fn end_outer(&self) -> Point {
        self.rotate(Point {
            x: self.radius_outer() - self.stroke_width_before(),
            y: -self.stroke_width_after(),
        })
    }

    /// End of the corner in the center of the stroke.
    pub fn end(&self) -> Point {
        self.rotate(Point::with_x(self.radius()))
    }

    /// End of the corner on the inside of the stroke.
    pub fn end_inner(&self) -> Point {
        self.rotate(Point {
            x: self.stroke_width_before() + self.radius_inner(),
            y: self.stroke_width_after(),
        })
    }

    /// Draw the cap at the beginning of the segment.
    ///
    /// If this corner has a stroke before it,
    /// a default "butt" cap is used.
    pub fn start_cap(&self, curve: &mut Curve, cap_type: LineCap) {
        // Avoid misshaped caps on small radii.
        let small_radius = self.reuse_stroke_after_for_cap().is_none();
        if cap_type == LineCap::Butt
            || self.stroke_before.is_some()
            || self.radius != Abs::zero() && small_radius
        {
            // Just the default cap.
            curve.line(self.mid_outer());
        } else if cap_type == LineCap::Square {
            let butt_start = self.mid_inner();
            let butt_end = self.mid_outer();
            // Extend by the stroke width.
            let offset_dir = line_normal(butt_start, butt_end);
            let offset = self.stroke_width_after().to_raw() * offset_dir;
            curve.line(butt_start + offset);
            curve.line(butt_end + offset);
            curve.line(butt_end);
        } else if cap_type == LineCap::Round {
            let arc_start = self.mid_inner();
            let arc_end = self.mid_outer();
            // We push the center by a little bit to ensure the correct
            // half of the circle gets drawn. If it is perfectly centered
            // the `arc` function just degenerates into a line, which we
            // do not want in this case.
            let offset_dir = -line_normal(arc_start, arc_end);
            let arc_center = (arc_start + arc_end) / 2.0 + offset_dir;
            curve.arc(arc_start, arc_center, arc_end);
        }
    }

    /// Draw the cap at the end of the segment.
    ///
    /// If this corner has a stroke before it,
    /// a default "butt" cap is used.
    pub fn end_cap(&self, curve: &mut Curve, cap_type: LineCap) {
        // Avoid misshaped caps on small radii.
        let small_radius = self.reuse_stroke_before_for_cap().is_none();
        if cap_type == LineCap::Butt
            || self.stroke_after.is_some()
            || self.radius != Abs::zero() && small_radius
        {
            // Just the default cap.
            curve.line(self.mid_inner());
        } else if cap_type == LineCap::Square {
            let butt_start = self.mid_outer();
            let butt_end = self.mid_inner();
            // Extend by the stroke width.
            let offset_dir = line_normal(butt_start, butt_end);
            let offset = self.stroke_width_before().to_raw() * offset_dir;
            curve.line(butt_start + offset);
            curve.line(butt_end + offset);
            curve.line(butt_end);
        } else if cap_type == LineCap::Round {
            let arc_start = self.mid_outer();
            let arc_end = self.mid_inner();
            // We push the center by a little bit to ensure the correct
            // half of the circle gets drawn. If it is perfectly centered
            // the `arc` function just degenerates into a line, which we
            // do not want in this case.
            let arc_center_offset = -line_normal(arc_start, arc_end);
            let arc_center = (arc_start + arc_end) / 2.0 + arc_center_offset;
            curve.arc(arc_start, arc_center, arc_end);
        }
    }
}

/// Computes the normal vector towards the left of the line.
fn line_normal(start: Point, end: Point) -> Point {
    (end - start).rot90ccw().normalized()
}

/// Helper to draw arcs with Bézier curves.
trait CurveExt {
    fn arc(&mut self, start: Point, center: Point, end: Point);
    fn arc_move(&mut self, start: Point, center: Point, end: Point);
    fn arc_line(&mut self, start: Point, center: Point, end: Point);
}

impl CurveExt for Curve {
    fn arc(&mut self, start: Point, center: Point, end: Point) {
        let arc = bezier_arc_control(start, center, end);
        self.cubic(arc[0], arc[1], end);
    }

    fn arc_move(&mut self, start: Point, center: Point, end: Point) {
        self.move_(start);
        self.arc(start, center, end);
    }

    fn arc_line(&mut self, start: Point, center: Point, end: Point) {
        self.line(start);
        self.arc(start, center, end);
    }
}

/// Get the control points for a Bézier curve that approximates a circular arc for
/// a start point, an end point and a center of the circle whose arc connects
/// the two.
fn bezier_arc_control(start: Point, center: Point, end: Point) -> [Point; 2] {
    // https://stackoverflow.com/a/44829356/1567835
    let a = start - center;
    let b = end - center;

    let q1 = a.x.to_raw() * a.x.to_raw() + a.y.to_raw() * a.y.to_raw();
    let q2 = q1 + a.x.to_raw() * b.x.to_raw() + a.y.to_raw() * b.y.to_raw();
    let k2 = (4.0 / 3.0) * ((2.0 * q1 * q2).sqrt() - q2)
        / (a.x.to_raw() * b.y.to_raw() - a.y.to_raw() * b.x.to_raw());

    let control_1 = Point::new(center.x + a.x - k2 * a.y, center.y + a.y + k2 * a.x);
    let control_2 = Point::new(center.x + b.x + k2 * b.y, center.y + b.y - k2 * b.x);

    [control_1, control_2]
}
