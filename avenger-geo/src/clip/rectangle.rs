//! Planar rectangle (extent) clipping.
//!
//! Ported from d3-geo `src/clip/rectangle.js` and `src/clip/line.js` (ISC),
//! https://github.com/d3/d3-geo. Operates on projected planar coordinates
//! after the resample stage.

use super::{rejoin, BufPoint, ClipBuffer};
use crate::math::EPSILON;
use crate::stream::GeoStream;
use std::cmp::Ordering;

const CLIP_MAX: f64 = 1e9;
const CLIP_MIN: f64 = -CLIP_MAX;

/// Liang–Barsky segment clipping against a rectangle; mutates the endpoints
/// and returns true when any part of the segment is visible (d3 `clipLine`).
#[allow(clippy::too_many_arguments)]
pub fn clip_segment(
    a: &mut [f64; 2],
    b: &mut [f64; 2],
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
) -> bool {
    let ax = a[0];
    let ay = a[1];
    let bx = b[0];
    let by = b[1];
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;
    let dx = bx - ax;
    let dy = by - ay;

    macro_rules! edge {
        ($r:expr, $d:expr) => {{
            let r = $r;
            let d = $d;
            if d == 0.0 {
                if r > 0.0 {
                    return false;
                }
            } else {
                let r = r / d;
                if d < 0.0 {
                    if r < t0 {
                        return false;
                    }
                    if r < t1 {
                        t1 = r;
                    }
                } else {
                    if r > t1 {
                        return false;
                    }
                    if r > t0 {
                        t0 = r;
                    }
                }
            }
        }};
    }

    edge!(x0 - ax, dx);
    edge!(-(x1 - ax), -dx);
    edge!(y0 - ay, dy);
    edge!(-(y1 - ay), -dy);

    if t0 > 0.0 {
        a[0] = ax + t0 * dx;
        a[1] = ay + t0 * dy;
    }
    if t1 < 1.0 {
        b[0] = ax + t1 * dx;
        b[1] = ay + t1 * dy;
    }
    true
}

/// Rectangle clip stage (d3 `clipRectangle`).
pub struct ClipRectangle<S> {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    pub sink: S,
    buffer: ClipBuffer,
    /// Some(..) while inside a polygon: per-line segment lists.
    segments: Option<Vec<Vec<Vec<BufPoint>>>>,
    polygon: Option<Vec<Vec<[f64; 2]>>>,
    clean: bool,
    in_line: bool,
    first: bool,
    // first point of the current line
    x__: f64,
    y__: f64,
    v__: bool,
    // previous point
    x_: f64,
    y_: f64,
    v_: bool,
}

impl<S: GeoStream> ClipRectangle<S> {
    pub fn new(extent: [[f64; 2]; 2], sink: S) -> Self {
        ClipRectangle {
            x0: extent[0][0],
            y0: extent[0][1],
            x1: extent[1][0],
            y1: extent[1][1],
            sink,
            buffer: ClipBuffer::default(),
            segments: None,
            polygon: None,
            clean: true,
            in_line: false,
            first: true,
            x__: f64::NAN,
            y__: f64::NAN,
            v__: false,
            x_: f64::NAN,
            y_: f64::NAN,
            v_: false,
        }
    }

    fn visible(&self, x: f64, y: f64) -> bool {
        self.x0 <= x && x <= self.x1 && self.y0 <= y && y <= self.y1
    }

    fn params(&self) -> RectParams {
        RectParams {
            x0: self.x0,
            y0: self.y0,
            x1: self.x1,
            y1: self.y1,
        }
    }

    /// Winding count of the buffered polygon around the (x0, y1) corner
    /// (d3 `polygonInside`).
    fn polygon_inside(&self) -> bool {
        let Some(polygon) = &self.polygon else {
            return false;
        };
        let mut winding: i64 = 0;
        for ring in polygon {
            if ring.is_empty() {
                continue;
            }
            let mut b0 = ring[0][0];
            let mut b1 = ring[0][1];
            for p in &ring[1..] {
                let a0 = b0;
                let a1 = b1;
                b0 = p[0];
                b1 = p[1];
                if a1 <= self.y1 {
                    if b1 > self.y1 && (b0 - a0) * (self.y1 - a1) > (b1 - a1) * (self.x0 - a0) {
                        winding += 1;
                    }
                } else if b1 <= self.y1 && (b0 - a0) * (self.y1 - a1) < (b1 - a1) * (self.x0 - a0) {
                    winding -= 1;
                }
            }
        }
        winding != 0
    }

    fn active_point(&mut self, x: f64, y: f64) {
        if self.segments.is_some() {
            self.buffer.point(x, y, None);
        } else {
            self.sink.point(x, y, None);
        }
    }
    fn active_line_start(&mut self) {
        if self.segments.is_some() {
            self.buffer.line_start();
        } else {
            self.sink.line_start();
        }
    }
    fn active_line_end(&mut self) {
        if self.segments.is_some() {
            self.buffer.line_end();
        } else {
            self.sink.line_end();
        }
    }

    fn line_point(&mut self, x: f64, y: f64) {
        let v = self.visible(x, y);
        if let Some(polygon) = self.polygon.as_mut() {
            if let Some(ring) = polygon.last_mut() {
                ring.push([x, y]);
            }
        }
        if self.first {
            self.x__ = x;
            self.y__ = y;
            self.v__ = v;
            self.first = false;
            if v {
                self.active_line_start();
                self.active_point(x, y);
            }
        } else if v && self.v_ {
            self.active_point(x, y);
        } else {
            let mut a = [
                self.x_.clamp(CLIP_MIN, CLIP_MAX),
                self.y_.clamp(CLIP_MIN, CLIP_MAX),
            ];
            let mut b = [x.clamp(CLIP_MIN, CLIP_MAX), y.clamp(CLIP_MIN, CLIP_MAX)];
            if clip_segment(&mut a, &mut b, self.x0, self.y0, self.x1, self.y1) {
                if !self.v_ {
                    self.active_line_start();
                    self.active_point(a[0], a[1]);
                }
                self.active_point(b[0], b[1]);
                if !v {
                    self.active_line_end();
                }
                self.clean = false;
            } else if v {
                self.active_line_start();
                self.active_point(x, y);
                self.clean = false;
            }
        }
        self.x_ = x;
        self.y_ = y;
        self.v_ = v;
    }
}

impl<S: GeoStream> GeoStream for ClipRectangle<S> {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        if self.in_line {
            self.line_point(x, y);
        } else if self.visible(x, y) {
            self.active_point(x, y);
        }
    }

    fn line_start(&mut self) {
        self.in_line = true;
        if let Some(polygon) = self.polygon.as_mut() {
            polygon.push(Vec::new());
        }
        self.first = true;
        self.v_ = false;
        self.x_ = f64::NAN;
        self.y_ = f64::NAN;
    }

    fn line_end(&mut self) {
        if self.segments.is_some() {
            self.line_point(self.x__, self.y__);
            if self.v__ && self.v_ {
                self.buffer.rejoin();
            }
            let result = self.buffer.result();
            self.segments.as_mut().expect("in polygon").push(result);
        }
        self.in_line = false;
        if self.v_ {
            self.active_line_end();
        }
    }

    fn polygon_start(&mut self) {
        self.segments = Some(Vec::new());
        self.polygon = Some(Vec::new());
        self.clean = true;
    }

    fn polygon_end(&mut self) {
        let start_inside = self.polygon_inside();
        let clean_inside = self.clean && start_inside;
        let segments: Vec<Vec<BufPoint>> = self
            .segments
            .take()
            .expect("in polygon")
            .into_iter()
            .flatten()
            .filter(|s| s.len() > 1)
            .collect();
        let visible = !segments.is_empty();
        if clean_inside || visible {
            let params = self.params();
            self.sink.polygon_start();
            if clean_inside {
                self.sink.line_start();
                params.interpolate(None, None, 1, &mut self.sink);
                self.sink.line_end();
            }
            if visible {
                rejoin(
                    segments,
                    |a, b| params.compare_intersection(a, b),
                    start_inside,
                    |from, to, dir, sink| params.interpolate(from, to, dir, sink),
                    &mut self.sink,
                );
            }
            self.sink.polygon_end();
        }
        self.polygon = None;
    }
}

/// Boundary parameters split out so rejoin closures don't borrow the whole
/// stage.
#[derive(Clone, Copy)]
struct RectParams {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl RectParams {
    fn corner(&self, p: &[f64; 2], direction: i32) -> i32 {
        if (p[0] - self.x0).abs() < EPSILON {
            if direction > 0 {
                0
            } else {
                3
            }
        } else if (p[0] - self.x1).abs() < EPSILON {
            if direction > 0 {
                2
            } else {
                1
            }
        } else if (p[1] - self.y0).abs() < EPSILON {
            if direction > 0 {
                1
            } else {
                0
            }
        } else if direction > 0 {
            3
        } else {
            2
        }
    }

    fn compare_point(&self, a: &[f64; 2], b: &[f64; 2]) -> f64 {
        let ca = self.corner(a, 1);
        let cb = self.corner(b, 1);
        if ca != cb {
            (ca - cb) as f64
        } else {
            match ca {
                0 => b[1] - a[1],
                1 => a[0] - b[0],
                2 => a[1] - b[1],
                _ => b[0] - a[0],
            }
        }
    }

    fn compare_intersection(&self, a: &BufPoint, b: &BufPoint) -> Ordering {
        self.compare_point(&[a.x, a.y], &[b.x, b.y])
            .partial_cmp(&0.0)
            .unwrap_or(Ordering::Equal)
    }

    fn interpolate(
        &self,
        from: Option<[f64; 2]>,
        to: Option<[f64; 2]>,
        direction: i32,
        sink: &mut dyn GeoStream,
    ) {
        match (from, to) {
            (Some(from), Some(to)) => {
                let a = self.corner(&from, direction);
                let a1 = self.corner(&to, direction);
                if a != a1 || (self.compare_point(&from, &to) < 0.0) ^ (direction > 0) {
                    let mut a = a;
                    loop {
                        sink.point(
                            if a == 0 || a == 3 { self.x0 } else { self.x1 },
                            if a > 1 { self.y1 } else { self.y0 },
                            None,
                        );
                        a = (a + direction + 4) % 4;
                        if a == a1 {
                            break;
                        }
                    }
                } else {
                    sink.point(to[0], to[1], None);
                }
            }
            _ => {
                // Full boundary.
                let mut a = 0;
                for _ in 0..4 {
                    sink.point(
                        if a == 0 || a == 3 { self.x0 } else { self.x1 },
                        if a > 1 { self.y1 } else { self.y0 },
                        None,
                    );
                    a = (a + direction + 4) % 4;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::RecordingSink;

    #[test]
    fn clips_line_to_extent() {
        let mut sink = RecordingSink::default();
        {
            let mut clip = ClipRectangle::new([[0.0, 0.0], [10.0, 10.0]], &mut sink);
            clip.line_start();
            clip.point(-5.0, 5.0, None);
            clip.point(15.0, 5.0, None);
            clip.line_end();
        }
        let lines = sink.polylines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].first().unwrap(), &[0.0, 5.0]);
        assert_eq!(lines[0].last().unwrap(), &[10.0, 5.0]);
    }

    #[test]
    fn drops_outside_line() {
        let mut sink = RecordingSink::default();
        {
            let mut clip = ClipRectangle::new([[0.0, 0.0], [10.0, 10.0]], &mut sink);
            clip.line_start();
            clip.point(-5.0, 20.0, None);
            clip.point(15.0, 20.0, None);
            clip.line_end();
        }
        assert!(sink.polylines().is_empty());
    }

    #[test]
    fn clips_polygon_overlapping_corner() {
        // Square from (-5,-5) to (5,5) clipped by [0,10]^2 -> [0,5]^2.
        let mut sink = RecordingSink::default();
        {
            let mut clip = ClipRectangle::new([[0.0, 0.0], [10.0, 10.0]], &mut sink);
            clip.polygon_start();
            clip.line_start();
            for p in [
                [-5.0, -5.0],
                [5.0, -5.0],
                [5.0, 5.0],
                [-5.0, 5.0],
                [-5.0, -5.0],
            ] {
                clip.point(p[0], p[1], None);
            }
            clip.line_end();
            clip.polygon_end();
        }
        let lines = sink.polylines();
        assert_eq!(lines.len(), 1, "events: {:?}", sink.events);
        for p in &lines[0] {
            assert!(p[0] >= -1e-9 && p[0] <= 5.0 + 1e-9);
            assert!(p[1] >= -1e-9 && p[1] <= 5.0 + 1e-9);
        }
    }

    #[test]
    fn polygon_containing_extent_emits_boundary() {
        let mut sink = RecordingSink::default();
        {
            let mut clip = ClipRectangle::new([[0.0, 0.0], [10.0, 10.0]], &mut sink);
            clip.polygon_start();
            clip.line_start();
            for p in [
                [-100.0, -100.0],
                [100.0, -100.0],
                [100.0, 100.0],
                [-100.0, 100.0],
                [-100.0, -100.0],
            ] {
                clip.point(p[0], p[1], None);
            }
            clip.line_end();
            clip.polygon_end();
        }
        let lines = sink.polylines();
        assert_eq!(lines.len(), 1, "events: {:?}", sink.events);
        assert_eq!(lines[0].len(), 4, "the four extent corners");
    }
}
