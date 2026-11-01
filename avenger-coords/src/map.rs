//! Map projections, whose positions are the longitudes and latitudes that a map's projector draws.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::projection::{densified, short_way, PlotProjection};
use avenger_geo::projections::Equirectangular;
use avenger_geo::{polygon_contains, Projection, Projector};
use geo::{algorithm::bool_ops::FillRule, BooleanOps, InteriorPoint};
use geo_types::{Geometry, LineString, MultiPolygon, Point, Polygon};

/// The largest step between the places of a path, in degrees, as a graticule densifies its
/// parallels, so that the projection follows a parallel and not a great circle.
const PATH_STEP: f64 = 2.5;

/// The latitude where paths and polygons stop short of a pole, where meridians meet.
const POLE: f64 = 90.0 - 1e-6;

/// The projection of a map plot through the projector of its cartographic projection, from the
/// top-left corner of the plot. Its range positions are longitudes and latitudes in degrees, so a
/// map has no scales, and longitude wraps every 360 degrees.
pub struct MapProjection {
    projector: Projector,
    /// The region where the projector draws the sphere, in pixels, which the projection computes
    /// when a lasso first asks for it.
    region: OnceLock<MultiPolygon<f64>>,
}

impl MapProjection {
    pub fn new(projector: Projector) -> Self {
        Self {
            projector,
            region: OnceLock::new(),
        }
    }

    pub fn projector(&self) -> &Projector {
        &self.projector
    }
}

impl MapProjection {
    /// The ring in degrees of `piece`, a polygon in pixels where the map draws, wound with the
    /// piece on its left, or `None` for a piece too small to make one.
    fn ring(&self, piece: &Polygon<f64>) -> Option<Vec<[f64; 2]>> {
        let exterior: Vec<[f32; 2]> = (piece.exterior().coords())
            .map(|c| [c.x as f32, c.y as f32])
            .collect();
        // The exterior closes on its first vertex, which the ring leaves to its last side.
        let open = &exterior[..exterior.len().saturating_sub(1)];
        let mut ring: Vec<[f64; 2]> = (densified(open))
            .filter_map(|pixel| self.position(pixel))
            .collect();
        if ring.len() < 3 {
            return None;
        }
        let inside = piece.interior_point();
        let inside = inside.and_then(|at| self.position([at.x() as f32, at.y() as f32]));
        if inside.is_some_and(|inside| !polygon_contains(std::slice::from_ref(&ring), inside)) {
            ring.reverse();
        }
        Some(ring)
    }
}

impl std::fmt::Debug for MapProjection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MapProjection")
            .finish_non_exhaustive()
    }
}

impl PlotProjection for MapProjection {
    /// The longitude and the latitude under `pixel`, or `None` where the map is not: where the
    /// pixel does not invert, or inverts to a place that the projection draws elsewhere, as past
    /// the edge of a flat map, where longitudes wrap.
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
        let (lon, lat) = (self.projector).invert(f64::from(pixel[0]), f64::from(pixel[1]))?;
        let drawn = self.pixel([lon, lat])?;
        ((drawn[0] - pixel[0]).hypot(drawn[1] - pixel[1]) <= 1.0).then_some([lon, lat])
    }

    /// The pixel of the place `[longitude, latitude]`, or `None` where the projection clips it.
    fn pixel(&self, [lon, lat]: [f64; 2]) -> Option<[f32; 2]> {
        match self.projector.project(&Point::new(lon, lat)) {
            Geometry::Point(point) if point.x().is_finite() && point.y().is_finite() => {
                Some([point.x() as f32, point.y() as f32])
            }
            _ => None,
        }
    }

    fn extent(&self) -> [[f64; 2]; 2] {
        [[-180.0, 180.0], [-90.0, 90.0]]
    }

    fn period(&self, dimension: usize) -> Option<f64> {
        (dimension == 0).then_some(360.0)
    }

    /// The pieces of the path at a steady change of longitude and latitude, such as a parallel,
    /// that the projection draws, cut at the antimeridian and at the clip.
    fn path(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        let line: LineString<f64> = steps(from, to).collect();
        match self.projector.project(&line) {
            Geometry::MultiLineString(lines) => (lines.0.iter())
                .map(|line| line.coords().map(|c| [c.x as f32, c.y as f32]).collect())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The rings of the polygon whose sides run at a steady change of longitude and latitude,
    /// the short way around in longitude, as the projection draws it: cut at the antimeridian
    /// and at the clip, with exterior rings and holes.
    fn polygon(&self, ring: &[[f64; 2]]) -> Vec<Vec<[f32; 2]>> {
        if ring.len() < 3 {
            return Vec::new();
        }
        let mut places: Vec<(f64, f64)> = Vec::new();
        for (index, from) in ring.iter().enumerate() {
            let to = short_way(self, *from, ring[(index + 1) % ring.len()]);
            places.extend(
                steps(*from, to)
                    .skip(usize::from(index > 0))
                    .map(|[x, y]| (x, y)),
            );
        }
        let polygon = Polygon::new(places.into_iter().collect(), Vec::new());
        let rings = |polygon: &Polygon<f64>| -> Vec<Vec<[f32; 2]>> {
            std::iter::once(polygon.exterior())
                .chain(polygon.interiors())
                .map(|ring| ring.coords().map(|c| [c.x as f32, c.y as f32]).collect())
                .collect()
        };
        match self.projector.project(&polygon) {
            Geometry::MultiPolygon(polygons) => polygons.0.iter().flat_map(rings).collect(),
            Geometry::Polygon(polygon) => rings(&polygon),
            _ => Vec::new(),
        }
    }

    /// The polygons in degrees inside `path`, cut to the part of it that the map draws, which
    /// can leave several, and laid on the plane of longitude and latitude as an equirectangular
    /// map draws them: cut at the antimeridian, and closed through a pole that they hold. Each
    /// polygon winds with its region on its left, as RFC 7946 winds GeoJSON, so a planar test
    /// and `avenger_geo::polygon_contains` both test a place against it.
    fn enclosed(&self, path: &[[f32; 2]]) -> Vec<Vec<[f64; 2]>> {
        let region = self.region.get_or_init(|| self.projector.project_sphere());
        let drawn: LineString<f64> = (path.iter())
            .map(|[x, y]| (f64::from(*x), f64::from(*y)))
            .collect();
        let drawn = Polygon::new(drawn, Vec::new());
        let pieces = drawn.intersection_with_fill_rule(region, FillRule::NonZero);
        (pieces.iter())
            .filter_map(|piece| self.ring(piece))
            .flat_map(|ring| planar(&ring))
            .collect()
    }
}

/// The pieces of the spherical polygon `ring`, wound as RFC 7946 winds GeoJSON, on the plane of
/// longitude and latitude, through an equirectangular projection of one pixel to a degree, which
/// cuts them at the antimeridian and closes them through a pole that they hold. Each piece winds
/// with its region on its left.
fn planar(ring: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
    static PLANE: OnceLock<Option<Projector>> = OnceLock::new();
    let plane = PLANE.get_or_init(|| {
        let projection = Projection::new(Equirectangular).with_view(180.0 / PI, [0.0, 0.0]);
        projection.build().ok()
    });
    let Some(plane) = plane else {
        return Vec::new();
    };
    let polygon = Polygon::new(ring.iter().map(|[x, y]| (*x, *y)).collect(), Vec::new());
    let pieces = match plane.project(&polygon) {
        Geometry::Polygon(polygon) => vec![polygon],
        Geometry::MultiPolygon(polygons) => polygons.0,
        _ => Vec::new(),
    };
    (pieces.iter())
        .filter_map(|piece| {
            // The projection draws latitude downward.
            let mut ring: Vec<[f64; 2]> =
                (piece.exterior().coords()).map(|c| [c.x, -c.y]).collect();
            ring.pop();
            if ring.len() < 3 {
                return None;
            }
            let area: f64 = (0..ring.len())
                .map(|index| {
                    let ([x0, y0], [x1, y1]) = (ring[index], ring[(index + 1) % ring.len()]);
                    x0 * y1 - x1 * y0
                })
                .sum();
            if area < 0.0 {
                ring.reverse();
            }
            Some(ring)
        })
        .collect()
}

/// The places from `from` to `to`, at most 2.5 degrees apart in each coordinate, with both ends,
/// short of the poles.
fn steps(from: [f64; 2], to: [f64; 2]) -> impl Iterator<Item = [f64; 2]> {
    let span = (to[0] - from[0]).abs().max((to[1] - from[1]).abs());
    let count = (span / PATH_STEP).ceil().max(1.0) as usize;
    (0..=count).map(move |step| {
        let along = step as f64 / count as f64;
        let [lon, lat] = [0, 1].map(|axis| from[axis] + (to[axis] - from[axis]) * along);
        [lon, lat.clamp(-POLE, POLE)]
    })
}
