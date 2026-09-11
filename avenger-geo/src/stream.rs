//! The geometry stream protocol.
//!
//! Ported from d3-geo's stream contract (ISC), <https://github.com/d3/d3-geo>:
//! geometry is pushed through transform stages as a sequence of
//! `point`/`line_start`/`line_end`/`polygon_start`/`polygon_end`/`sphere`
//! calls, so no intermediate geometry is materialized between stages.
//!
//! Coordinates entering the pipeline are spherical degrees; the pipeline's
//! first stage converts to radians; after the projection/resample stage
//! points are planar output units.

/// Visitor for streamed geometry. The optional `m` slot carries clip
/// bookkeeping between stages (d3's third point element); sources pass
/// `None`.
pub trait GeoStream {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>);
    fn line_start(&mut self);
    fn line_end(&mut self);
    fn polygon_start(&mut self);
    fn polygon_end(&mut self);
    fn sphere(&mut self) {}
}

impl<T: GeoStream + ?Sized> GeoStream for Box<T> {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        (**self).point(x, y, m)
    }
    fn line_start(&mut self) {
        (**self).line_start()
    }
    fn line_end(&mut self) {
        (**self).line_end()
    }
    fn polygon_start(&mut self) {
        (**self).polygon_start()
    }
    fn polygon_end(&mut self) {
        (**self).polygon_end()
    }
    fn sphere(&mut self) {
        (**self).sphere()
    }
}

impl<T: GeoStream + ?Sized> GeoStream for &mut T {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        (**self).point(x, y, m)
    }
    fn line_start(&mut self) {
        (**self).line_start()
    }
    fn line_end(&mut self) {
        (**self).line_end()
    }
    fn polygon_start(&mut self) {
        (**self).polygon_start()
    }
    fn polygon_end(&mut self) {
        (**self).polygon_end()
    }
    fn sphere(&mut self) {
        (**self).sphere()
    }
}

/// Degrees -> radians conversion stage (d3 `transformRadians`).
pub struct TransformRadians<S> {
    pub sink: S,
}

impl<S: GeoStream> GeoStream for TransformRadians<S> {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        self.sink
            .point(x * crate::math::RADIANS, y * crate::math::RADIANS, m)
    }
    fn line_start(&mut self) {
        self.sink.line_start()
    }
    fn line_end(&mut self) {
        self.sink.line_end()
    }
    fn polygon_start(&mut self) {
        self.sink.polygon_start()
    }
    fn polygon_end(&mut self) {
        self.sink.polygon_end()
    }
    fn sphere(&mut self) {
        self.sink.sphere()
    }
}

/// A recording sink used by tests and fixture comparisons.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct RecordingSink {
    pub events: Vec<StreamEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Point(f64, f64),
    LineStart,
    LineEnd,
    PolygonStart,
    PolygonEnd,
    Sphere,
}

impl GeoStream for RecordingSink {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        self.events.push(StreamEvent::Point(x, y));
    }
    fn line_start(&mut self) {
        self.events.push(StreamEvent::LineStart);
    }
    fn line_end(&mut self) {
        self.events.push(StreamEvent::LineEnd);
    }
    fn polygon_start(&mut self) {
        self.events.push(StreamEvent::PolygonStart);
    }
    fn polygon_end(&mut self) {
        self.events.push(StreamEvent::PolygonEnd);
    }
    fn sphere(&mut self) {
        self.events.push(StreamEvent::Sphere);
    }
}

impl RecordingSink {
    /// Collect all points of all lines as flat polylines
    /// (line boundaries split the output).
    pub fn polylines(&self) -> Vec<Vec<[f64; 2]>> {
        let mut out = Vec::new();
        let mut current: Option<Vec<[f64; 2]>> = None;
        for ev in &self.events {
            match ev {
                StreamEvent::LineStart => current = Some(Vec::new()),
                StreamEvent::LineEnd => {
                    if let Some(line) = current.take() {
                        out.push(line);
                    }
                }
                StreamEvent::Point(x, y) => {
                    if let Some(line) = current.as_mut() {
                        line.push([*x, *y]);
                    } else {
                        out.push(vec![[*x, *y]]);
                    }
                }
                _ => {}
            }
        }
        out
    }
}
