//! Query regions: a selection shape made valid, cut to what a clip leaves visible, and prepared
//! for testing many instances.

use avenger_scenegraph::{
    marks::group::Clip,
    path_geometry::{filled_contours, resolve_contours, FilledContours},
};
use geo::{
    BoundingRect, Centroid, CoordsIter, Distance, Euclidean, Intersects, MonotonicPolygons,
    PreparedGeometry, Relate,
};
use geo_types::{Coord, Geometry, LineString, MultiPolygon, Point, Polygon};
use i_overlay::{
    core::{fill_rule::FillRule as OverlayFillRule, overlay_rule::OverlayRule},
    float::overlay::FloatOverlay,
};
use rstar::{primitives::Line, Envelope, RTree, AABB};

use crate::rtree::{GeometryInstance, GeometryQueryHitPolicy, GeometryQueryShape};

/// The largest gap between a curve and the polygon edges that stand in for it: in clip paths,
/// and in circles that a clip cuts.
const TOLERANCE: f32 = 0.1;

/// A query shape in scene coordinates, ready to test instances against.
pub(crate) enum Region {
    Rect { min: [f32; 2], max: [f32; 2] },
    Circle { center: [f32; 2], radius: f32 },
    Polygons(Box<Polygons>),
}

/// Valid polygons, with the structures that make testing many instances fast.
pub(crate) struct Polygons {
    contours: FilledContours,
    /// The polygons, prepared for exact tests against an instance's geometry.
    prepared: PreparedGeometry<'static, f32>,
    /// The polygons, prepared for point tests.
    inside: MonotonicPolygons<f32>,
    /// The polygons' edges, to find the instances that an edge comes near.
    outline: RTree<Line<[f32; 2]>>,
    envelope: AABB<[f32; 2]>,
}

impl Region {
    /// The region that a query shape covers, or `None` when it covers no area.
    pub(crate) fn new(shape: &GeometryQueryShape) -> Option<Self> {
        match shape {
            GeometryQueryShape::Rect { x0, y0, x1, y1 } => Some(Region::Rect {
                min: [x0.min(*x1), y0.min(*y1)],
                max: [x0.max(*x1), y0.max(*y1)],
            }),
            GeometryQueryShape::Circle { cx, cy, radius } => Some(Region::Circle {
                center: [*cx, *cy],
                radius: radius.abs(),
            }),
            GeometryQueryShape::Polygon { points, fill_rule } => {
                if points.len() < 3 {
                    return None;
                }
                Polygons::new(resolve_contours(std::slice::from_ref(points), *fill_rule))
                    .map(|polygons| Region::Polygons(Box::new(polygons)))
            }
        }
    }

    /// The part of the region that `clip` leaves visible, or `None` when none of it is.
    pub(crate) fn clipped(&self, clip: &Clip) -> Option<Self> {
        let clip = match clip {
            Clip::None => {
                return match self {
                    Region::Rect { min, max } => Some(Region::Rect {
                        min: *min,
                        max: *max,
                    }),
                    Region::Circle { center, radius } => Some(Region::Circle {
                        center: *center,
                        radius: *radius,
                    }),
                    Region::Polygons(polygons) => Polygons::new(polygons.contours.clone())
                        .map(|polygons| Region::Polygons(Box::new(polygons))),
                };
            }
            Clip::Rect {
                x,
                y,
                width,
                height,
            } => {
                if *width <= 0.0 || *height <= 0.0 {
                    return None;
                }
                let (min, max) = ([*x, *y], [x + width, y + height]);
                match self {
                    // A rect cut by a rect is a rect, and a circle inside the clip is uncut.
                    Region::Rect { min: a, max: b } => {
                        let min = [a[0].max(min[0]), a[1].max(min[1])];
                        let max = [b[0].min(max[0]), b[1].min(max[1])];
                        return (min[0] <= max[0] && min[1] <= max[1])
                            .then_some(Region::Rect { min, max });
                    }
                    Region::Circle { center, radius }
                        if AABB::from_corners(min, max).contains_envelope(&self.envelope()) =>
                    {
                        return Some(Region::Circle {
                            center: *center,
                            radius: *radius,
                        });
                    }
                    _ => rect_contours(min, max),
                }
            }
            Clip::Path { path, fill_rule } => filled_contours(path, TOLERANCE, *fill_rule),
        };
        let cut = FloatOverlay::with_subj_and_clip(&self.contours(), &clip)
            .overlay(OverlayRule::Intersect, OverlayFillRule::NonZero);
        Polygons::new(cut).map(|polygons| Region::Polygons(Box::new(polygons)))
    }

    pub(crate) fn envelope(&self) -> AABB<[f32; 2]> {
        match self {
            Region::Rect { min, max } => AABB::from_corners(*min, *max),
            Region::Circle { center, radius } => AABB::from_corners(
                [center[0] - radius, center[1] - radius],
                [center[0] + radius, center[1] + radius],
            ),
            Region::Polygons(polygons) => polygons.envelope,
        }
    }

    /// Whether `instance` matches `policy` in this region.
    pub(crate) fn matches(
        &self,
        instance: &GeometryInstance,
        policy: GeometryQueryHitPolicy,
    ) -> bool {
        match policy {
            GeometryQueryHitPolicy::CentroidInside => instance
                .geometry
                .centroid()
                .is_some_and(|centroid| self.contains_point(centroid.0)),
            GeometryQueryHitPolicy::GeometryIntersects => self.intersects(instance),
            GeometryQueryHitPolicy::GeometryContained => self.contains(instance),
        }
    }

    fn contains_point(&self, point: Coord<f32>) -> bool {
        match self {
            Region::Rect { min, max } => {
                point.x >= min[0] && point.x <= max[0] && point.y >= min[1] && point.y <= max[1]
            }
            Region::Circle { center, radius } => {
                (point.x - center[0]).hypot(point.y - center[1]) <= *radius
            }
            Region::Polygons(polygons) => polygons.inside.intersects(&point),
        }
    }

    /// Whether the region comes within the instance's reach of its geometry.
    fn intersects(&self, instance: &GeometryInstance) -> bool {
        if instance.geometry.coords_iter().next().is_none() {
            return false;
        }
        match self {
            Region::Rect { min, max } => {
                let rect = AABB::from_corners(*min, *max);
                let envelope = instance.envelope();
                if !rect.intersects(&envelope) {
                    return false;
                }
                if rect.contains_envelope(&envelope) {
                    return true;
                }
                let rect = Geometry::Rect(geo_types::Rect::new(
                    Coord {
                        x: min[0],
                        y: min[1],
                    },
                    Coord {
                        x: max[0],
                        y: max[1],
                    },
                ));
                Euclidean::distance(&rect, &instance.geometry) <= instance.reach
            }
            Region::Circle { center, radius } => {
                Euclidean::distance(&instance.geometry, &Point::new(center[0], center[1]))
                    <= radius + instance.reach
            }
            Region::Polygons(polygons) => polygons.intersects(instance),
        }
    }

    /// Whether the instance's geometry lies in the region, at least its reach from the edge.
    fn contains(&self, instance: &GeometryInstance) -> bool {
        if instance.geometry.coords_iter().next().is_none() {
            return false;
        }
        match self {
            // Exact for an axis-aligned rect, since the instance's box includes its reach.
            Region::Rect { min, max } => {
                AABB::from_corners(*min, *max).contains_envelope(&instance.envelope())
            }
            // Exact, since a circle holds a shape when it holds the shape's vertices.
            Region::Circle { center, radius } => {
                let remaining = radius - instance.reach;
                remaining >= 0.0
                    && instance
                        .geometry
                        .coords_iter()
                        .all(|point| (point.x - center[0]).hypot(point.y - center[1]) <= remaining)
            }
            Region::Polygons(polygons) => polygons.contains(instance),
        }
    }

    /// The region as polygon contours, with a circle's edge within `TOLERANCE` of it.
    fn contours(&self) -> FilledContours {
        match self {
            Region::Rect { min, max } => rect_contours(*min, *max),
            Region::Circle { center, radius } => {
                let segments = if *radius > TOLERANCE {
                    (std::f32::consts::PI / (1.0 - TOLERANCE / radius).acos())
                        .ceil()
                        .max(8.0)
                } else {
                    8.0
                } as usize;
                let ring = (0..segments)
                    .map(|k| {
                        let angle = k as f32 / segments as f32 * std::f32::consts::TAU;
                        [
                            center[0] + radius * angle.cos(),
                            center[1] + radius * angle.sin(),
                        ]
                    })
                    .collect();
                vec![vec![ring]]
            }
            Region::Polygons(polygons) => polygons.contours.clone(),
        }
    }
}

impl Polygons {
    /// Prepares valid polygons, or returns `None` when they cover no area.
    fn new(contours: FilledContours) -> Option<Self> {
        let polygons = MultiPolygon::new(contours.iter().map(|shape| polygon(shape)).collect());
        let bounds = polygons.bounding_rect()?;
        let edges = contours
            .iter()
            .flatten()
            .flat_map(|ring| {
                (0..ring.len()).map(move |i| Line::new(ring[i], ring[(i + 1) % ring.len()]))
            })
            .collect();
        Some(Self {
            prepared: PreparedGeometry::from(polygons.clone()),
            inside: MonotonicPolygons::from(polygons),
            outline: RTree::bulk_load(edges),
            envelope: AABB::from_corners(
                [bounds.min().x, bounds.min().y],
                [bounds.max().x, bounds.max().y],
            ),
            contours,
        })
    }

    /// Whether an edge comes within the instance's reach of its box. Away from every edge, the
    /// instance lies wholly inside or wholly outside, so one of its points decides.
    fn near_outline(&self, instance: &GeometryInstance) -> bool {
        self.outline
            .locate_in_envelope_intersecting(instance.envelope())
            .next()
            .is_some()
    }

    /// The distance from the instance's geometry to the nearest edge near it.
    fn outline_distance(&self, instance: &GeometryInstance) -> f32 {
        self.outline
            .locate_in_envelope_intersecting(instance.envelope())
            .map(|edge| {
                let edge = geo_types::Line::new(
                    Coord {
                        x: edge.from[0],
                        y: edge.from[1],
                    },
                    Coord {
                        x: edge.to[0],
                        y: edge.to[1],
                    },
                );
                Euclidean::distance(&Geometry::Line(edge), &instance.geometry)
            })
            .fold(f32::INFINITY, f32::min)
    }

    fn intersects(&self, instance: &GeometryInstance) -> bool {
        let Some(first) = instance.geometry.coords_iter().next() else {
            return false;
        };
        if !self.near_outline(instance) {
            return self.inside.intersects(&first);
        }
        let touches = match &instance.geometry {
            Geometry::Point(point) => self.inside.intersects(&point.0),
            geometry => self.prepared.relate(geometry).is_intersects(),
        };
        touches || (instance.reach > 0.0 && self.outline_distance(instance) <= instance.reach)
    }

    fn contains(&self, instance: &GeometryInstance) -> bool {
        let Some(first) = instance.geometry.coords_iter().next() else {
            return false;
        };
        if !self.near_outline(instance) {
            return self.inside.intersects(&first);
        }
        let covered = match &instance.geometry {
            Geometry::Point(point) => self.inside.intersects(&point.0),
            geometry => self.prepared.relate(geometry).is_covers(),
        };
        covered && (instance.reach == 0.0 || self.outline_distance(instance) >= instance.reach)
    }
}

fn rect_contours(min: [f32; 2], max: [f32; 2]) -> FilledContours {
    vec![vec![vec![min, [max[0], min[1]], max, [min[0], max[1]]]]]
}

/// A geo polygon from an exterior contour followed by its holes.
fn polygon(shape: &[Vec<[f32; 2]>]) -> Polygon<f32> {
    let ring = |contour: &Vec<[f32; 2]>| {
        let mut coords: Vec<Coord<f32>> =
            contour.iter().map(|p| Coord { x: p[0], y: p[1] }).collect();
        if let Some(first) = coords.first().copied() {
            coords.push(first);
        }
        LineString::new(coords)
    };
    let mut rings = shape.iter();
    let exterior = rings
        .next()
        .map(ring)
        .unwrap_or_else(|| LineString::new(Vec::new()));
    Polygon::new(exterior, rings.map(ring).collect())
}

#[cfg(test)]
mod tests {
    use avenger_common::types::FillRule;
    use avenger_scenegraph::marks::mark::MarkInstance;
    use geo_types::MultiLineString;

    use super::*;

    /// A small deterministic random source.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    fn instance(geometry: Geometry<f32>, reach: f32) -> GeometryInstance {
        GeometryInstance {
            mark_instance: MarkInstance {
                name: "probe".into(),
                mark_path: vec![0],
                instance_index: Some(0),
            },
            interactive: true,
            geometry,
            reach,
        }
    }

    /// The region's polygons and edges, for exact answers.
    fn reference(polygons: &Polygons) -> (MultiPolygon<f32>, Geometry<f32>) {
        let region = MultiPolygon::new(
            polygons
                .contours
                .iter()
                .map(|shape| polygon(shape))
                .collect(),
        );
        let edges = Geometry::MultiLineString(MultiLineString::new(
            region
                .iter()
                .flat_map(|polygon| {
                    std::iter::once(polygon.exterior().clone())
                        .chain(polygon.interiors().iter().cloned())
                })
                .collect(),
        ));
        (region, edges)
    }

    /// The answers for each policy without the shortcut away from the outline: an exact
    /// `relate` and the distance to every edge.
    fn exact(
        polygons: &Polygons,
        (region, edges): &(MultiPolygon<f32>, Geometry<f32>),
        instance: &GeometryInstance,
    ) -> [bool; 3] {
        let matrix = polygons.prepared.relate(&instance.geometry);
        let edge_distance = Euclidean::distance(edges, &instance.geometry);
        let reach = instance.reach;
        [
            instance
                .geometry
                .centroid()
                .is_some_and(|centroid| region.intersects(&centroid)),
            matrix.is_intersects() || edge_distance <= reach,
            matrix.is_covers() && (reach == 0.0 || edge_distance >= reach),
        ]
    }

    /// Random self-crossing lassos, under both fill rules, against random points, rects, triangles
    /// and lines: the fast classification gives the exact answer.
    #[test]
    fn classification_agrees_with_exact_tests() {
        let mut random = Random(7);
        let mut checked = 0;
        for _ in 0..20 {
            let count = 5 + (random.next() * 30.0) as usize;
            let points: Vec<[f32; 2]> = (0..count)
                .map(|_| {
                    let angle = random.next() * std::f32::consts::TAU;
                    let radius = 10.0 + random.next() * 40.0;
                    [50.0 + radius * angle.cos(), 50.0 + radius * angle.sin()]
                })
                .collect();
            for fill_rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let Some(polygons) =
                    Polygons::new(resolve_contours(std::slice::from_ref(&points), fill_rule))
                else {
                    continue;
                };
                let reference = reference(&polygons);
                let region = Region::Polygons(Box::new(polygons));
                let Region::Polygons(polygons) = &region else {
                    unreachable!()
                };
                for _ in 0..200 {
                    let [x, y] = [random.next() * 100.0, random.next() * 100.0];
                    let size = 0.5 + random.next() * 8.0;
                    let reach = if random.next() < 0.5 {
                        0.0
                    } else {
                        random.next() * 3.0
                    };
                    let geometry = match (random.next() * 4.0) as usize {
                        0 => Geometry::Point(Point::new(x, y)),
                        1 => Geometry::Rect(geo_types::Rect::new(
                            Coord { x, y },
                            Coord {
                                x: x + size,
                                y: y + size * 0.6,
                            },
                        )),
                        2 => Geometry::Polygon(Polygon::new(
                            LineString::from(vec![(x, y), (x + size, y), (x, y + size), (x, y)]),
                            vec![],
                        )),
                        _ => Geometry::LineString(LineString::from(vec![
                            (x, y),
                            (x + size, y + size * 0.3),
                        ])),
                    };
                    let reach = if matches!(geometry, Geometry::Point(_)) {
                        reach.max(0.5)
                    } else {
                        reach
                    };
                    let instance = instance(geometry, reach);
                    let exact = exact(polygons, &reference, &instance);
                    for (policy, exact) in [
                        GeometryQueryHitPolicy::CentroidInside,
                        GeometryQueryHitPolicy::GeometryIntersects,
                        GeometryQueryHitPolicy::GeometryContained,
                    ]
                    .into_iter()
                    .zip(exact)
                    {
                        assert_eq!(
                            region.matches(&instance, policy),
                            exact,
                            "{policy:?} {fill_rule:?} {:?}",
                            instance.geometry
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 20_000, "{checked}");
    }
}
