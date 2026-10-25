//! Queries of the hit-testing tree: which instances stand in a DE-9IM relation to a shape, and
//! which lie nearest a point.

use std::borrow::Cow;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};
use std::fmt;

use avenger_common::types::FillRule;
use avenger_scenegraph::marks::{group::Clip, mark::MarkInstance};
use avenger_scenegraph::render::fill::resolve_contours;
use geo::relate::IntersectionMatrix;
use geo::{BoundingRect, Distance, Euclidean};
use geo_types::{Geometry, MultiPolygon, Point, Rect};
use rstar::{ParentNode, RTreeNode, AABB};

use crate::relate::{holds, Target};
use crate::rtree::{GeometryInstance, Node, SceneGraphRTree};
use crate::shape::{rect_or_flat, ClipArea, Shape};

/// The shape that a query relates the tree's instances to, in scene coordinates.
#[derive(Debug, Clone)]
pub enum QueryShape<'a> {
    /// One point, such as the pointer.
    Point([f32; 2]),
    /// An axis-aligned rectangle between two corners, in either order. A rectangle with no
    /// width or height is the segment or point between its corners.
    Rect { min: [f32; 2], max: [f32; 2] },
    /// A disc.
    Circle { center: [f32; 2], radius: f32 },
    /// Closed contours, such as a lasso that crosses itself or a shape with holes, whose
    /// `fill_rule` decides which areas they enclose.
    Polygon {
        contours: Vec<Vec<[f32; 2]>>,
        fill_rule: FillRule,
    },
    /// Any geometry, such as a line string for a stroke that a drag draws, or several points.
    Geometry(Geometry<f32>),
    /// An instance's shape at the query's extent, such as the instance that a pick found.
    Instance(&'a GeometryInstance),
}

/// How a candidate instance's shape relates to the query shape, as DE-9IM defines each relation.
/// The candidate is the relation's first shape, so `Within` finds the instances inside the query
/// shape, and `Contains` the instances that hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The same point set.
    Equals,
    /// No shared point.
    Disjoint,
    /// A shared point. Shapes that only touch intersect.
    Intersects,
    /// Shared boundary points, and no shared interior point.
    Touches,
    /// Interiors that meet in a shape of lower dimension than the larger of the two, with
    /// neither shape inside the other, such as a line through an area.
    Crosses,
    /// The candidate inside the query shape, with their interiors meeting.
    Within,
    /// The query shape inside the candidate, with their interiors meeting.
    Contains,
    /// Shapes of one dimension whose interiors meet, each with a part outside the other.
    Overlaps,
    /// No point of the query shape outside the candidate.
    Covers,
    /// No point of the candidate outside the query shape.
    CoveredBy,
    /// The query shape in the candidate's interior, clear of its boundary.
    ContainsProperly,
    /// Interiors that meet, whether one shape lies inside the other or each overlaps the other.
    InteriorsIntersect,
    /// A DE-9IM pattern over the candidate's and the query shape's parts.
    Pattern(Pattern),
}

/// A DE-9IM pattern: nine cells for the candidate's interior, boundary, and exterior against the
/// query shape's, each `T` for a shared point, `F` for none, `0`, `1`, or `2` for the dimension
/// of what they share, or `*` for anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pattern([u8; 9]);

impl Pattern {
    /// The pattern that `pattern` spells, such as `T*F**F***`.
    pub fn new(pattern: &str) -> Result<Self, PatternError> {
        let cells: [u8; 9] = pattern
            .as_bytes()
            .try_into()
            .map_err(|_| PatternError(pattern.to_string()))?;
        if !cells.iter().all(|cell| b"TF*012".contains(cell)) {
            return Err(PatternError(pattern.to_string()));
        }
        Ok(Self(cells))
    }

    /// Whether shapes apart from each other can match the pattern: their interiors' and
    /// boundaries' cells are each empty or free.
    pub(crate) fn holds_apart(&self) -> bool {
        [0, 1, 3, 4]
            .iter()
            .all(|&cell| matches!(self.0[cell], b'F' | b'*'))
    }

    /// Whether `matrix` matches the pattern.
    pub(crate) fn matches(&self, matrix: &IntersectionMatrix) -> bool {
        let pattern = std::str::from_utf8(&self.0).expect("patterns are ASCII");
        matrix.matches(pattern).unwrap_or(false)
    }
}

/// A DE-9IM pattern that isn't nine cells of `T`, `F`, `*`, `0`, `1`, and `2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternError(String);

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} is not a DE-9IM pattern of nine cells from T, F, *, 0, 1, and 2",
            self.0
        )
    }
}

impl std::error::Error for PatternError {}

/// Which shape of an instance a query sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Extent {
    /// The geometry grown by its reach: the fill, and everything within half the stroke width.
    #[default]
    Drawn,
    /// The area that the geometry encloses: a polygon's interior, or a circle symbol's disc.
    /// Lines, rules, and line vertices fill nothing.
    Fill,
    /// The geometry as the tree stores it: a circle symbol's center, a line's path, or a shape's
    /// polygon.
    Geometry,
    /// The centroid of the geometry.
    Centroid,
}

/// How `nearest` measures distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Metric {
    /// The distance in the plane.
    #[default]
    Euclidean,
    /// The distance along x alone, with equal distances nearest in the plane first.
    AlongX,
    /// The distance along y alone, with equal distances nearest in the plane first.
    AlongY,
}

/// A filter on the candidates of a query, by their mark and instance.
pub(crate) struct Filter<'a>(Box<dyn Fn(&MarkInstance) -> bool + 'a>);

impl Filter<'_> {
    fn keeps(&self, mark: &MarkInstance) -> bool {
        (self.0)(mark)
    }
}

impl fmt::Debug for Filter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Filter")
    }
}

/// A query for the instances that stand in a relation to a shape.
#[derive(Debug)]
pub struct Query<'a> {
    pub(crate) shape: QueryShape<'a>,
    pub(crate) relation: Relation,
    pub(crate) extent: Extent,
    pub(crate) filter: Option<Filter<'a>>,
    pub(crate) limit: Option<usize>,
}

impl<'a> Query<'a> {
    /// The instances in `relation` to `shape`, at the drawn extent.
    pub fn new(shape: QueryShape<'a>, relation: Relation) -> Self {
        Self {
            shape,
            relation,
            extent: Extent::default(),
            filter: None,
            limit: None,
        }
    }

    /// The query, comparing each instance's shape at `extent`, and an `Instance` query shape's
    /// too.
    pub fn extent(self, extent: Extent) -> Self {
        Self { extent, ..self }
    }

    /// The query, among the instances that `filter` keeps. The tree calls it with each
    /// candidate's mark and instance before any exact geometry.
    pub fn filter(self, filter: impl Fn(&MarkInstance) -> bool + 'a) -> Self {
        Self {
            filter: Some(Filter(Box::new(filter))),
            ..self
        }
    }

    /// The query, returning at most `limit` instances, the topmost of those that match.
    pub fn limit(self, limit: usize) -> Self {
        Self {
            limit: Some(limit),
            ..self
        }
    }
}

/// A query for the instances nearest a point.
#[derive(Debug)]
pub struct Nearest<'a> {
    pub(crate) point: [f32; 2],
    pub(crate) max_distance: f32,
    pub(crate) metric: Metric,
    pub(crate) extent: Extent,
    pub(crate) filter: Option<Filter<'a>>,
}

impl<'a> Nearest<'a> {
    /// The instances within `max_distance` of `point`, which can be infinite, in the plane, at
    /// the drawn extent.
    pub fn new(point: [f32; 2], max_distance: f32) -> Self {
        Self {
            point,
            max_distance,
            metric: Metric::default(),
            extent: Extent::default(),
            filter: None,
        }
    }

    /// The query, measuring distance by `metric`.
    pub fn metric(self, metric: Metric) -> Self {
        Self { metric, ..self }
    }

    /// The query, measuring the distance to each instance's shape at `extent`.
    pub fn extent(self, extent: Extent) -> Self {
        Self { extent, ..self }
    }

    /// The query, among the instances that `filter` keeps.
    pub fn filter(self, filter: impl Fn(&MarkInstance) -> bool + 'a) -> Self {
        Self {
            filter: Some(Filter(Box::new(filter))),
            ..self
        }
    }
}

/// An instance that `nearest` finds, with its distance from the query's point.
#[derive(Debug, Clone, Copy)]
pub struct Neighbor<'t> {
    pub instance: &'t GeometryInstance,
    /// The distance by the query's metric to the instance's shape.
    pub distance: f32,
}

impl GeometryInstance {
    /// The instance's shape at `extent`, or `None` when it has none, such as a line's fill, or
    /// when `clip` shows none of it. A shape that grows, such as a drawn circle, comes as
    /// polygons whose round edges lie within 0.01 pixels of the shape's for radii up to 20,000
    /// pixels.
    pub fn shape(&self, extent: Extent, clip: &Clip) -> Option<Geometry<f32>> {
        Shape::of(self, extent)
            .visible(&ClipArea::new(clip))
            .into_geometry()
    }
}

impl SceneGraphRTree {
    /// The clip of the mark at `mark_path`, in scene coordinates, or `Clip::None` when nothing
    /// clips it.
    pub fn clip(&self, mark_path: &[usize]) -> &Clip {
        static NONE: Clip = Clip::None;
        self.clips.get(mark_path).unwrap_or(&NONE)
    }

    /// The instances whose shapes at the query's extent stand in its relation to its shape,
    /// topmost first. An instance that its clip shows none of has no shape, and a query of
    /// `Intersects`, `Disjoint`, `Touches`, or `InteriorsIntersect` whose shape a mark's clip
    /// shows none of, such as the pointer over an axis beside a plot, sees none of the mark.
    pub fn query(&self, query: &Query) -> Vec<&GeometryInstance> {
        let shape = match &query.shape {
            QueryShape::Instance(instance) => Shape::of(instance, query.extent)
                .visible(self.clip_area(&instance.mark_instance.mark_path)),
            shape => shape_of(shape),
        };
        let target = Target::new(shape);
        let keep = |node: &Node| {
            let mark = &self.instances[node.draw_index].mark_instance;
            (query.filter.as_ref()).is_none_or(|filter| filter.keeps(mark))
        };
        let neighborhood = target.shape.envelope();
        // A relation that can hold between shapes apart, such as `Disjoint`, searches every
        // instance, and any other searches near the query shape.
        let candidates: Vec<usize> = match neighborhood {
            _ if apart(&query.relation) => (self.rtree.iter())
                .filter(|node| keep(node))
                .map(|node| node.draw_index)
                .collect(),
            None => return Vec::new(),
            Some(neighborhood) => (self.rtree.locate_in_envelope_intersecting(neighborhood))
                .filter(|node| keep(node))
                .map(|node| node.draw_index)
                .collect(),
        };
        // The heap yields the candidates topmost first, so a query with a limit tests only as
        // many as it needs.
        let mut candidates = BinaryHeap::from(candidates);
        // These relations see a candidate only inside the query shape's box, so a clip that shows
        // none of that box leaves nothing of its mark for them to see.
        let local = matches!(
            query.relation,
            Relation::Intersects
                | Relation::Disjoint
                | Relation::Touches
                | Relation::InteriorsIntersect
        );
        let mut shown: HashMap<&[usize], bool> = HashMap::new();
        let limit = query.limit.unwrap_or(usize::MAX);
        let mut found = Vec::new();
        while let Some(draw_index) = candidates.pop() {
            if found.len() >= limit {
                break;
            }
            let instance = &self.instances[draw_index];
            let path = instance.mark_instance.mark_path.as_slice();
            let clip = self.clip_area(path);
            let shows_query = match neighborhood {
                Some(neighborhood) if local => {
                    *(shown.entry(path)).or_insert_with(|| clip.meets(&neighborhood))
                }
                _ => true,
            };
            let shape = if shows_query {
                Shape::of(instance, query.extent).visible(clip)
            } else {
                Shape::Empty
            };
            if holds(&query.relation, &shape, &target) {
                found.push(instance);
            }
        }
        found
    }

    /// The instances within the query's largest distance of its point, nearest first, by the
    /// distance to each instance's shape at the query's extent, with their distances. An
    /// instance that its clip shows none of isn't found.
    pub fn nearest<'t, 'q>(&'t self, query: Nearest<'q>) -> impl Iterator<Item = Neighbor<'t>> + 'q
    where
        't: 'q,
    {
        let mut heap = BinaryHeap::new();
        heap.push(Reverse(Entry::Parent {
            bound: 0.0,
            node: self.rtree.root(),
        }));
        NearestIter {
            tree: self,
            query,
            heap,
        }
    }
}

/// Return whether `point` is visible through `clip`, both in scene coordinates.
/// Clip paths use the nonzero fill rule, as in rendering and hit testing.
pub fn clip_shows(clip: &Clip, point: [f32; 2]) -> bool {
    match clip {
        Clip::None => true,
        Clip::Rect {
            x,
            y,
            width,
            height,
        } => {
            *width > 0.0
                && *height > 0.0
                && (*x..=x + width).contains(&point[0])
                && (*y..=y + height).contains(&point[1])
        }
        Clip::Path(path) => lyon_algorithms::hit_test::hit_test_path(
            &lyon_path::math::point(point[0], point[1]),
            path.iter(),
            lyon_path::FillRule::NonZero,
            0.01,
        ),
    }
}

/// Whether `relation` can hold between shapes apart from each other: `Disjoint`, and a pattern
/// that leaves the cells of both shapes' interiors and boundaries empty or free.
fn apart(relation: &Relation) -> bool {
    match relation {
        Relation::Disjoint => true,
        Relation::Pattern(pattern) => pattern.holds_apart(),
        _ => false,
    }
}

/// The shape of a query shape other than an instance.
fn shape_of<'q>(shape: &'q QueryShape) -> Shape<'q> {
    match shape {
        QueryShape::Point(point) => Shape::exact(Cow::Owned(Point::from(*point).into())),
        QueryShape::Rect { min, max } => Shape::exact(Cow::Owned(rect_or_flat(*min, *max))),
        QueryShape::Circle { center, radius } => {
            Shape::grown(Cow::Owned(Point::from(*center).into()), radius.abs())
        }
        QueryShape::Polygon {
            contours,
            fill_rule,
        } => {
            let resolved = resolve_contours(contours, *fill_rule);
            let polygons = MultiPolygon::new(
                (resolved.iter())
                    .map(|shape| crate::shape::polygon(shape))
                    .collect(),
            );
            Shape::exact(Cow::Owned(polygons.into()))
        }
        QueryShape::Geometry(geometry) => Shape::exact(Cow::Borrowed(geometry)),
        QueryShape::Instance(_) => Shape::Empty,
    }
}

/// A step of the nearest search: a subtree or instance at a lower bound of its distance, or an
/// instance at its distance.
enum Entry<'t> {
    Parent {
        bound: f32,
        node: &'t ParentNode<Node>,
    },
    Leaf {
        bound: f32,
        node: Node,
    },
    Found {
        distance: f32,
        plane: f32,
        draw_index: usize,
    },
}

impl Entry<'_> {
    /// The order of the search: by distance, with bounds before instances found at the same
    /// distance, so a tie resolves only once every candidate for it is found, then by the
    /// distance in the plane, then topmost first.
    fn key(&self) -> (f32, u8, f32, Reverse<usize>) {
        match self {
            Entry::Parent { bound, .. } | Entry::Leaf { bound, .. } => (*bound, 0, 0.0, Reverse(0)),
            Entry::Found {
                distance,
                plane,
                draw_index,
            } => (*distance, 1, *plane, Reverse(*draw_index)),
        }
    }
}

impl PartialEq for Entry<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Entry<'_> {}

impl PartialOrd for Entry<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        let (a, b) = (self.key(), other.key());
        (a.0.total_cmp(&b.0))
            .then(a.1.cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.cmp(&b.3))
    }
}

struct NearestIter<'t, 'q> {
    tree: &'t SceneGraphRTree,
    query: Nearest<'q>,
    heap: BinaryHeap<Reverse<Entry<'t>>>,
}

impl NearestIter<'_, '_> {
    /// A lower bound of the distance to anything inside `envelope`, a few units in the last
    /// place under the distance to the box, so rounding never puts it above an instance's
    /// measured distance and an instance found at a tie waits for the others at it.
    fn bound(&self, envelope: &AABB<[f32; 2]>) -> f32 {
        let point = self.query.point;
        let distance = match self.query.metric {
            Metric::Euclidean => envelope.distance_2(&point).sqrt(),
            Metric::AlongX => interval_gap(envelope.lower()[0], envelope.upper()[0], point[0]),
            Metric::AlongY => interval_gap(envelope.lower()[1], envelope.upper()[1], point[1]),
        };
        distance * (1.0 - 4.0 * f32::EPSILON)
    }

    /// The distances by the query's metric and in the plane to the instance at `draw_index`'s
    /// shape, or `None` when it has none or its clip shows none of it.
    fn measure(&self, draw_index: usize) -> Option<(f32, f32)> {
        let instance = &self.tree.instances[draw_index];
        let clip = self.tree.clip_area(&instance.mark_instance.mark_path);
        let shape = Shape::of(instance, self.query.extent).visible(clip);
        shape_distance(&shape, self.query.point, self.query.metric)
    }
}

impl<'t> Iterator for NearestIter<'t, '_> {
    type Item = Neighbor<'t>;

    fn next(&mut self) -> Option<Self::Item> {
        let max = self.query.max_distance;
        while let Some(Reverse(entry)) = self.heap.pop() {
            match entry {
                Entry::Found {
                    distance,
                    draw_index,
                    ..
                } => {
                    return Some(Neighbor {
                        instance: &self.tree.instances[draw_index],
                        distance,
                    })
                }
                Entry::Parent { node, .. } => {
                    for child in node.children() {
                        let entry = match child {
                            RTreeNode::Parent(parent) => Entry::Parent {
                                bound: self.bound(&parent.envelope()),
                                node: parent,
                            },
                            RTreeNode::Leaf(leaf) => Entry::Leaf {
                                bound: self.bound(&leaf.envelope),
                                node: *leaf,
                            },
                        };
                        if entry.key().0 <= max {
                            self.heap.push(Reverse(entry));
                        }
                    }
                }
                Entry::Leaf { node, .. } => {
                    let mark = &self.tree.instances[node.draw_index].mark_instance;
                    if !(self.query.filter.as_ref()).is_none_or(|filter| filter.keeps(mark)) {
                        continue;
                    }
                    let Some((distance, plane)) = self.measure(node.draw_index) else {
                        continue;
                    };
                    if distance <= max {
                        self.heap.push(Reverse(Entry::Found {
                            distance,
                            plane,
                            draw_index: node.draw_index,
                        }));
                    }
                }
            }
        }
        None
    }
}

/// The distances from `point` to `shape` by `metric` and in the plane, or `None` for an empty
/// shape.
fn shape_distance(shape: &Shape, point: [f32; 2], metric: Metric) -> Option<(f32, f32)> {
    let (geometry, by) = shape.parts()?;
    let plane = (Euclidean.distance(geometry, &Point::from(point)) - by).max(0.0);
    let distance = match metric {
        Metric::Euclidean => plane,
        Metric::AlongX => (axis_gap(geometry, 0, point[0]) - by).max(0.0),
        Metric::AlongY => (axis_gap(geometry, 1, point[1]) - by).max(0.0),
    };
    Some((distance, plane))
}

/// The distance along `axis` from `value` to `geometry`: to the nearest of its connected parts,
/// each of which spans its box along the axis.
fn axis_gap(geometry: &Geometry<f32>, axis: usize, value: f32) -> f32 {
    let gap = |part: Option<Rect<f32>>| {
        part.map_or(f32::INFINITY, |rect| {
            let [low, high] = [rect.min(), rect.max()].map(|corner| corner.x_y());
            let [low, high] = [low, high].map(|(x, y)| if axis == 0 { x } else { y });
            interval_gap(low, high, value)
        })
    };
    let nearest = |gaps: &mut dyn Iterator<Item = f32>| gaps.fold(f32::INFINITY, f32::min);
    match geometry {
        Geometry::MultiPoint(points) => {
            nearest(&mut points.iter().map(|point| gap(Some(point.bounding_rect()))))
        }
        Geometry::MultiLineString(lines) => {
            nearest(&mut lines.iter().map(|line| gap(line.bounding_rect())))
        }
        Geometry::MultiPolygon(polygons) => {
            nearest(&mut polygons.iter().map(|polygon| gap(polygon.bounding_rect())))
        }
        Geometry::GeometryCollection(collection) => {
            nearest(&mut collection.iter().map(|part| axis_gap(part, axis, value)))
        }
        part => gap(part.bounding_rect()),
    }
}

/// The distance from `value` to the interval from `low` to `high`, 0 inside it.
fn interval_gap(low: f32, high: f32, value: f32) -> f32 {
    (low - value).max(value - high).max(0.0)
}
