//! Generic spherical clip stage plus segment buffering and rejoin
//! machinery.
//!
//! Ported from d3-geo `src/clip/index.js`, `src/clip/buffer.js`,
//! `src/clip/rejoin.js` (ISC), https://github.com/d3/d3-geo.
//!
//! The clip stage sits between rotation and resampling. Lines are clipped
//! by a policy-supplied line clipper; polygons are collected ring by ring
//! into buffered segments, then re-joined along the clip boundary with
//! [`crate::polygon_contains`] deciding whether the boundary's start point
//! is inside.

pub mod antimeridian;
pub mod rectangle;

use crate::math::EPSILON;
use crate::stream::GeoStream;
use std::cmp::Ordering;

/// A buffered stream point; `m` carries d3's third point element, used to
/// mark endpoints created by clipping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufPoint {
    pub x: f64,
    pub y: f64,
    pub m: Option<f64>,
}

impl BufPoint {
    pub fn new(x: f64, y: f64, m: Option<f64>) -> Self {
        BufPoint { x, y, m }
    }
    fn is_marked(&self) -> bool {
        self.m.is_some_and(|v| v != 0.0)
    }
}

fn point_equal(a: &BufPoint, b: &BufPoint) -> bool {
    (a.x - b.x).abs() < EPSILON && (a.y - b.y).abs() < EPSILON
}

/// Collects streamed lines into point buffers (d3 `clipBuffer`).
#[derive(Debug, Default)]
pub struct ClipBuffer {
    lines: Vec<Vec<BufPoint>>,
}

impl ClipBuffer {
    /// Join the last and first buffered lines (used when a ring's clipped
    /// tail wraps around to its head).
    pub fn rejoin(&mut self) {
        if self.lines.len() > 1 {
            let first = self.lines.remove(0);
            if let Some(last) = self.lines.last_mut() {
                last.extend(first);
            } else {
                self.lines.push(first);
            }
        }
    }

    pub fn result(&mut self) -> Vec<Vec<BufPoint>> {
        std::mem::take(&mut self.lines)
    }
}

impl GeoStream for ClipBuffer {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        if let Some(line) = self.lines.last_mut() {
            line.push(BufPoint::new(x, y, m));
        }
    }
    fn line_start(&mut self) {
        self.lines.push(Vec::new());
    }
    fn line_end(&mut self) {}
    fn polygon_start(&mut self) {}
    fn polygon_end(&mut self) {}
}

/// Line clipper state machine supplied by a clip policy. The sink is
/// passed per call so the same clipper type can write to the downstream
/// stage or to a [`ClipBuffer`].
pub trait LineClipper {
    fn line_start(&mut self, sink: &mut dyn GeoStream);
    fn point(&mut self, lambda: f64, phi: f64, m: Option<f64>, sink: &mut dyn GeoStream);
    fn line_end(&mut self, sink: &mut dyn GeoStream);
    /// d3 `clean()`: bit 0 set when there were no intersections; bit 1 set
    /// when the first and last segments should be rejoined.
    fn clean(&self) -> u8;
}

/// A spherical clip policy. The built-in policy cuts at the antimeridian.
pub trait ClipPolicy {
    type Line: LineClipper;

    fn point_visible(&self, lambda: f64, phi: f64) -> bool;
    fn line_clipper(&self) -> Self::Line;
    /// Walk the clip boundary from `from` to `to` in `direction`
    /// (1 = clockwise), or the full boundary when `from` is `None`.
    fn interpolate(
        &self,
        from: Option<[f64; 2]>,
        to: Option<[f64; 2]>,
        direction: i32,
        sink: &mut dyn GeoStream,
    );
    /// A point guaranteed to lie on the clip boundary interior-test side
    /// (d3's `start`).
    fn start_point(&self) -> [f64; 2];
    fn compare_intersection(&self, a: &BufPoint, b: &BufPoint) -> Ordering;
}

fn valid_segment(segment: &[BufPoint]) -> bool {
    segment.len() > 1
}

/// The generic clip stage (d3 `clip(pointVisible, clipLine, interpolate, start)`).
pub struct Clip<P: ClipPolicy, S: GeoStream> {
    policy: P,
    pub sink: S,
    line: P::Line,
    ring_buffer: ClipBuffer,
    ring_sink: P::Line,
    polygon_started: bool,
    /// Per-ring segment lists accumulated during a polygon.
    segments: Vec<Vec<Vec<BufPoint>>>,
    /// Raw (unclipped) rings in spherical radians for polygon_contains.
    polygon: Vec<Vec<[f64; 2]>>,
    ring: Vec<[f64; 2]>,
    in_polygon: bool,
    in_line: bool,
}

impl<P: ClipPolicy, S: GeoStream> Clip<P, S> {
    pub fn new(policy: P, sink: S) -> Self {
        let line = policy.line_clipper();
        let ring_sink = policy.line_clipper();
        Clip {
            policy,
            sink,
            line,
            ring_buffer: ClipBuffer::default(),
            ring_sink,
            polygon_started: false,
            segments: Vec::new(),
            polygon: Vec::new(),
            ring: Vec::new(),
            in_polygon: false,
            in_line: false,
        }
    }

    fn ring_end(&mut self) {
        if self.ring.is_empty() {
            self.ring_sink.line_end(&mut self.ring_buffer);
            self.ring_buffer.result();
            self.polygon.push(Vec::new());
            return;
        }
        // Close the ring through the clipper. (d3 re-pushes ring[0] into the
        // ring and pops it afterwards; we stream it without recording so
        // `self.ring` stays the open ring for polygon_contains.)
        let first = self.ring[0];
        self.ring_sink
            .point(first[0], first[1], None, &mut self.ring_buffer);
        self.ring_sink.line_end(&mut self.ring_buffer);

        let clean = self.ring_sink.clean();
        let mut ring_segments = self.ring_buffer.result();

        self.polygon.push(std::mem::take(&mut self.ring));

        let n = ring_segments.len();
        if n == 0 {
            return;
        }

        // No intersections.
        if clean & 1 != 0 {
            let segment = &ring_segments[0];
            let m = segment.len() - 1;
            if m > 0 {
                if !self.polygon_started {
                    self.sink.polygon_start();
                    self.polygon_started = true;
                }
                self.sink.line_start();
                for p in &segment[..m] {
                    self.sink.point(p.x, p.y, p.m);
                }
                self.sink.line_end();
            }
            return;
        }

        // Rejoin connected segments (the ring was cut open at the boundary).
        if n > 1 && clean & 2 != 0 {
            let tail = ring_segments.pop().unwrap();
            let head = ring_segments.remove(0);
            let mut joined = tail;
            joined.extend(head);
            ring_segments.push(joined);
        }

        ring_segments.retain(|s| valid_segment(s));
        if !ring_segments.is_empty() {
            self.segments.push(ring_segments);
        }
    }
}

impl<P: ClipPolicy, S: GeoStream> GeoStream for Clip<P, S> {
    fn point(&mut self, lambda: f64, phi: f64, m: Option<f64>) {
        if self.in_polygon {
            self.ring.push([lambda, phi]);
            self.ring_sink.point(lambda, phi, m, &mut self.ring_buffer);
        } else if self.in_line {
            self.line.point(lambda, phi, m, &mut self.sink);
        } else if self.policy.point_visible(lambda, phi) {
            self.sink.point(lambda, phi, m);
        }
    }

    fn line_start(&mut self) {
        if self.in_polygon {
            self.ring.clear();
            self.ring_sink.line_start(&mut self.ring_buffer);
        } else {
            self.in_line = true;
            self.line.line_start(&mut self.sink);
        }
    }

    fn line_end(&mut self) {
        if self.in_polygon {
            self.ring_end();
        } else {
            self.in_line = false;
            self.line.line_end(&mut self.sink);
        }
    }

    fn polygon_start(&mut self) {
        self.in_polygon = true;
        self.segments.clear();
        self.polygon.clear();
    }

    fn polygon_end(&mut self) {
        self.in_polygon = false;
        let segments: Vec<Vec<BufPoint>> = std::mem::take(&mut self.segments)
            .into_iter()
            .flatten()
            .collect();
        let policy = &self.policy;
        let sink = &mut self.sink;
        let start_inside =
            crate::polygon_contains::polygon_contains(&self.polygon, policy.start_point());
        if !segments.is_empty() {
            if !self.polygon_started {
                sink.polygon_start();
                self.polygon_started = true;
            }
            rejoin(
                segments,
                |a, b| policy.compare_intersection(a, b),
                start_inside,
                |from, to, dir, s| policy.interpolate(from, to, dir, s),
                sink,
            );
        } else if start_inside {
            if !self.polygon_started {
                sink.polygon_start();
                self.polygon_started = true;
            }
            sink.line_start();
            policy.interpolate(None, None, 1, sink);
            sink.line_end();
        }
        if self.polygon_started {
            self.sink.polygon_end();
            self.polygon_started = false;
        }
        self.polygon.clear();
    }

    fn sphere(&mut self) {
        self.sink.polygon_start();
        self.sink.line_start();
        self.policy.interpolate(None, None, 1, &mut self.sink);
        self.sink.line_end();
        self.sink.polygon_end();
    }
}

// ---------------------------------------------------------------------------
// Rejoin (d3 clip/rejoin.js)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Node {
    x: BufPoint,
    /// Segment points (subject entry nodes only).
    z: Option<std::sync::Arc<Vec<BufPoint>>>,
    /// Index of the paired node in the other list.
    o: usize,
    /// True if this is an entry into the visible region.
    e: bool,
    /// Visited flag.
    v: bool,
    n: usize,
    p: usize,
}

impl Node {
    fn new(x: BufPoint, z: Option<std::sync::Arc<Vec<BufPoint>>>, o: usize, e: bool) -> Self {
        Node {
            x,
            z,
            o,
            e,
            v: false,
            n: usize::MAX,
            p: usize::MAX,
        }
    }
}

/// Rejoin clipped polygon segments along the clip boundary
/// (d3 `clipRejoin`). `interpolate` walks the boundary between two
/// intersection points; `compare` orders intersections along the boundary.
pub fn rejoin<C, I>(
    segments: Vec<Vec<BufPoint>>,
    compare: C,
    start_inside: bool,
    interpolate: I,
    sink: &mut dyn GeoStream,
) where
    C: Fn(&BufPoint, &BufPoint) -> Ordering,
    I: Fn(Option<[f64; 2]>, Option<[f64; 2]>, i32, &mut dyn GeoStream),
{
    let mut subject: Vec<Node> = Vec::new();
    let mut clip: Vec<Node> = Vec::new();

    for segment in segments {
        let n = segment.len() - 1;
        if n == 0 {
            continue;
        }
        let p0 = segment[0];
        let mut p1 = segment[n];

        if point_equal(&p0, &p1) {
            if !p0.is_marked() && !p1.is_marked() {
                sink.line_start();
                for p in &segment[..n] {
                    sink.point(p.x, p.y, p.m);
                }
                sink.line_end();
                continue;
            }
            // handle degenerate cases by moving the point
            p1.x += 2.0 * EPSILON;
        }

        let seg = std::sync::Arc::new(segment);

        let si = subject.len();
        let ci = clip.len();
        subject.push(Node::new(p0, Some(seg.clone()), ci, true));
        clip.push(Node::new(p0, None, si, false));
        let si = subject.len();
        let ci = clip.len();
        subject.push(Node::new(p1, Some(seg.clone()), ci, false));
        clip.push(Node::new(p1, None, si, true));
    }

    if subject.is_empty() {
        return;
    }

    // Sort the clip list along the boundary, then link both lists.
    let mut order: Vec<usize> = (0..clip.len()).collect();
    order.sort_by(|&a, &b| compare(&clip[a].x, &clip[b].x));
    link_in_order(&mut subject, None);
    link_in_order(&mut clip, Some(&order));

    let mut inside = start_inside;
    for &i in &order {
        inside = !inside;
        clip[i].e = inside;
    }

    // Traverse.
    let start = 0usize; // subject[0]
    loop {
        // Find an unvisited subject entry.
        let mut current = start;
        let mut looped = false;
        while subject[current].v {
            current = subject[current].n;
            if current == start {
                looped = true;
                break;
            }
        }
        if looped {
            return;
        }

        let mut is_subject = true;
        let mut side_subject = true; // current index refers to subject list
        sink.line_start();

        loop {
            // Mark current and its pair visited.
            if side_subject {
                let o = subject[current].o;
                subject[current].v = true;
                clip[o].v = true;
            } else {
                let o = clip[current].o;
                clip[current].v = true;
                subject[o].v = true;
            }

            let (entry, next_idx, prev_idx) = if side_subject {
                (subject[current].e, subject[current].n, subject[current].p)
            } else {
                (clip[current].e, clip[current].n, clip[current].p)
            };

            if entry {
                if is_subject {
                    let points = subject[current]
                        .z
                        .clone()
                        .expect("subject entry has points");
                    for p in points.iter() {
                        sink.point(p.x, p.y, p.m);
                    }
                } else {
                    let from = node_x(&subject, &clip, side_subject, current);
                    let to = node_x(&subject, &clip, side_subject, next_idx);
                    interpolate(Some(from), Some(to), 1, sink);
                }
                current = next_idx;
            } else {
                if is_subject {
                    let points = subject[prev_idx]
                        .z
                        .clone()
                        .expect("subject prev has points");
                    for p in points.iter().rev() {
                        sink.point(p.x, p.y, p.m);
                    }
                } else {
                    let from = node_x(&subject, &clip, side_subject, current);
                    let to = node_x(&subject, &clip, side_subject, prev_idx);
                    interpolate(Some(from), Some(to), -1, sink);
                }
                current = prev_idx;
            }

            // Jump to the paired node in the other list.
            let o = if side_subject {
                subject[current].o
            } else {
                clip[current].o
            };
            side_subject = !side_subject;
            current = o;
            is_subject = !is_subject;

            let visited = if side_subject {
                subject[current].v
            } else {
                clip[current].v
            };
            if visited {
                break;
            }
        }
        sink.line_end();
    }
}

fn node_x(subject: &[Node], clip: &[Node], side_subject: bool, idx: usize) -> [f64; 2] {
    let node = if side_subject {
        &subject[idx]
    } else {
        &clip[idx]
    };
    [node.x.x, node.x.y]
}

fn link_in_order(nodes: &mut [Node], order: Option<&[usize]>) {
    if nodes.is_empty() {
        return;
    }
    match order {
        None => {
            let n = nodes.len();
            for (i, node) in nodes.iter_mut().enumerate() {
                node.n = (i + 1) % n;
                node.p = (i + n - 1) % n;
            }
        }
        Some(order) => {
            let n = order.len();
            for k in 0..n {
                let i = order[k];
                nodes[i].n = order[(k + 1) % n];
                nodes[i].p = order[(k + n - 1) % n];
            }
        }
    }
}
