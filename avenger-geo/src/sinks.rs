//! Terminal stream sinks: lyon path building and polyline collection.
//! (The planar bounds sink lives in [`crate::projector::BoundsSink`].)

use crate::stream::GeoStream;
use lyon_path::math::Point;
use lyon_path::Path;

/// Builds a `lyon_path::Path` from streamed (already projected) geometry.
///
/// In `fill` mode every line becomes a closed sub-path (polygon rings);
/// in stroke mode lines end open. Points outside lines are ignored.
pub struct LyonPathSink {
    builder: lyon_path::path::Builder,
    fill: bool,
    started: bool,
    has_content: bool,
}

impl LyonPathSink {
    pub fn fill() -> Self {
        LyonPathSink {
            builder: Path::builder(),
            fill: true,
            started: false,
            has_content: false,
        }
    }

    pub fn stroke() -> Self {
        LyonPathSink {
            builder: Path::builder(),
            fill: false,
            started: false,
            has_content: false,
        }
    }

    pub fn finish(mut self) -> Path {
        if self.started {
            self.builder.end(false);
        }
        self.builder.build()
    }

    pub fn has_content(&self) -> bool {
        self.has_content
    }
}

impl GeoStream for LyonPathSink {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        let p = Point::new(x as f32, y as f32);
        if !p.x.is_finite() || !p.y.is_finite() {
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
        if self.started {
            self.builder.end(false);
            self.started = false;
        }
    }

    fn line_end(&mut self) {
        if self.started {
            self.builder.end(self.fill);
            self.started = false;
        }
    }

    fn polygon_start(&mut self) {}
    fn polygon_end(&mut self) {}
}

/// Collects streamed lines as polylines with a `defined` mask suitable for
/// `SceneLineMark`-style consumers: line boundaries insert a break.
#[derive(Debug, Default)]
pub struct PolylineSink {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub defined: Vec<bool>,
    in_line: bool,
    emitted_any_in_line: bool,
}

impl GeoStream for PolylineSink {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        if self.in_line && !self.emitted_any_in_line && !self.x.is_empty() {
            // Break between lines: repeat the previous point as undefined.
            let px = *self.x.last().unwrap();
            let py = *self.y.last().unwrap();
            self.x.push(px);
            self.y.push(py);
            self.defined.push(false);
        }
        self.x.push(x as f32);
        self.y.push(y as f32);
        self.defined.push(true);
        self.emitted_any_in_line = true;
    }

    fn line_start(&mut self) {
        self.in_line = true;
        self.emitted_any_in_line = false;
    }

    fn line_end(&mut self) {
        self.in_line = false;
    }

    fn polygon_start(&mut self) {}
    fn polygon_end(&mut self) {}
}
