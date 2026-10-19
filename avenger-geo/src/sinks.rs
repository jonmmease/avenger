//! Terminal sinks behind [`crate::paths`] and
//! [`crate::Projection::summarize`].
//!
//! `BoundsSink` is ported from d3-geo `src/path/bounds.js` (ISC),
//! <https://github.com/d3/d3-geo>

use crate::stream::GeoSink;
use lyon_path::math::Point;
use lyon_path::Path;

/// Builds a `lyon_path::Path` from streamed (already projected) geometry.
///
/// Polygon rings are closed and LineStrings remain open. Points outside
/// lines are ignored. Non-finite vertices break a line without closing it.
pub(crate) struct LyonPathSink {
    builder: lyon_path::path::Builder,
    started: bool,
    in_line: bool,
    in_polygon: bool,
    line_broken: bool,
}

impl Default for LyonPathSink {
    fn default() -> Self {
        LyonPathSink {
            builder: Path::builder(),
            started: false,
            in_line: false,
            in_polygon: false,
            line_broken: false,
        }
    }
}

impl LyonPathSink {
    /// Finish the path, leaving any unfinished line open.
    pub(crate) fn finish(mut self) -> Path {
        if self.started {
            self.builder.end(false);
        }
        self.builder.build()
    }
}

impl GeoSink for LyonPathSink {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        if !self.in_line {
            return;
        }
        let p = Point::new(x as f32, y as f32);
        if !p.x.is_finite() || !p.y.is_finite() {
            self.line_broken = true;
            if self.started {
                self.builder.end(false);
                self.started = false;
            }
            return;
        }
        if self.started {
            self.builder.line_to(p);
        } else {
            self.builder.begin(p);
            self.started = true;
        }
    }

    fn line_start(&mut self) {
        self.in_line = true;
        self.line_broken = false;
        if self.started {
            self.builder.end(false);
            self.started = false;
        }
    }

    fn line_end(&mut self) {
        self.in_line = false;
        if self.started {
            self.builder.end(self.in_polygon && !self.line_broken);
            self.started = false;
        }
    }

    fn polygon_start(&mut self) {
        self.in_polygon = true;
    }
    fn polygon_end(&mut self) {
        self.in_polygon = false;
    }
}

/// Planar bounds of the finite points it receives (d3 `path/bounds.js`).
/// Non-finite points are skipped, so one bad vertex doesn't discard the bounds
/// of everything else.
#[derive(Debug, Default)]
pub(crate) struct BoundsSink {
    x0: Option<f64>,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl BoundsSink {
    pub(crate) fn result(&self) -> Option<[[f64; 2]; 2]> {
        self.x0.map(|x0| [[x0, self.y0], [self.x1, self.y1]])
    }
}

impl GeoSink for BoundsSink {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        match self.x0 {
            None => {
                self.x0 = Some(x);
                self.x1 = x;
                self.y0 = y;
                self.y1 = y;
            }
            Some(x0) => {
                if x < x0 {
                    self.x0 = Some(x);
                }
                if x > self.x1 {
                    self.x1 = x;
                }
                if y < self.y0 {
                    self.y0 = y;
                }
                if y > self.y1 {
                    self.y1 = y;
                }
            }
        }
    }
    fn line_start(&mut self) {}
    fn line_end(&mut self) {}
    fn polygon_start(&mut self) {}
    fn polygon_end(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyon_path::Event;

    #[test]
    fn path_sink_ignores_isolated_points_and_keeps_broken_rings_open() {
        let mut sink = LyonPathSink::default();
        sink.point(99.0, 99.0, None);
        sink.polygon_start();
        sink.line_start();
        sink.point(0.0, 0.0, None);
        sink.point(1.0, 1.0, None);
        sink.point(f64::INFINITY, 2.0, None);
        sink.point(3.0, 3.0, None);
        sink.point(4.0, 4.0, None);
        sink.line_end();
        sink.polygon_end();
        sink.point(99.0, 99.0, None);
        sink.line_start();
        sink.point(5.0, 5.0, None);
        sink.line_end();
        let path = sink.finish();
        let count = |event: fn(&Event<_, _>) -> bool| path.iter().filter(event).count();
        assert_eq!(count(|e| matches!(e, Event::End { close: true, .. })), 0);
        assert_eq!(count(|e| matches!(e, Event::Begin { .. })), 3);
        assert_eq!(count(|e| matches!(e, Event::Line { .. })), 2);
    }
}
