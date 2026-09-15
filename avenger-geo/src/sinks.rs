//! Terminal stream sinks: lyon path building and polyline collection.
//! (The planar bounds sink lives in [`crate::projector::BoundsSink`].)

use crate::stream::GeoStream;
use lyon_path::math::Point;
use lyon_path::Path;

/// Builds a `lyon_path::Path` from streamed (already projected) geometry.
///
/// Polygon rings are closed and LineStrings remain open. Points outside
/// lines are ignored. Non-finite vertices break a line without closing it.
pub struct LyonPathSink {
    builder: lyon_path::path::Builder,
    started: bool,
    in_line: bool,
    in_polygon: bool,
    line_broken: bool,
    has_content: bool,
}

impl Default for LyonPathSink {
    fn default() -> Self {
        LyonPathSink {
            builder: Path::builder(),
            started: false,
            in_line: false,
            in_polygon: false,
            line_broken: false,
            has_content: false,
        }
    }
}

impl LyonPathSink {
    /// Create an empty path sink that preserves line and polygon boundaries.
    pub fn new() -> Self {
        Self::default()
    }

    /// Finish the path, leaving any unfinished line open.
    pub fn finish(mut self) -> Path {
        if self.started {
            self.builder.end(false);
        }
        self.builder.build()
    }

    /// Whether the sink has accepted any finite line vertices.
    pub fn has_content(&self) -> bool {
        self.has_content
    }
}

impl GeoStream for LyonPathSink {
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
        self.has_content = true;
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

/// Collects streamed lines as polylines with a `defined` mask suitable for
/// `SceneLineMark`-style consumers: line boundaries insert a break.
/// Polygon rings repeat their first point to close the boundary. Isolated
/// points are ignored. Coordinates that are not finite as `f32` break the
/// line and are omitted, and the broken ring remains open.
#[derive(Debug, Default)]
pub struct PolylineSink {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub defined: Vec<bool>,
    in_line: bool,
    in_polygon: bool,
    line_broken: bool,
    first: Option<[f32; 2]>,
    emitted_any_in_line: bool,
}

impl GeoStream for PolylineSink {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        if !self.in_line {
            return;
        }
        let (x, y) = (x as f32, y as f32);
        if !x.is_finite() || !y.is_finite() {
            self.line_broken = true;
            self.emitted_any_in_line = false;
            return;
        }
        if !self.emitted_any_in_line && !self.x.is_empty() {
            // Break between lines: repeat the previous point as undefined.
            let px = *self.x.last().unwrap();
            let py = *self.y.last().unwrap();
            self.x.push(px);
            self.y.push(py);
            self.defined.push(false);
        }
        self.x.push(x);
        self.y.push(y);
        self.defined.push(true);
        self.first.get_or_insert([x, y]);
        self.emitted_any_in_line = true;
    }

    fn line_start(&mut self) {
        self.in_line = true;
        self.line_broken = false;
        self.first = None;
        self.emitted_any_in_line = false;
    }

    fn line_end(&mut self) {
        if self.in_polygon && !self.line_broken {
            if let Some([x, y]) = self.first {
                if self.x.last() != Some(&x) || self.y.last() != Some(&y) {
                    self.point(x as f64, y as f64, None);
                }
            }
        }
        self.in_line = false;
    }

    fn polygon_start(&mut self) {
        self.in_polygon = true;
    }
    fn polygon_end(&mut self) {
        self.in_polygon = false;
    }
}
