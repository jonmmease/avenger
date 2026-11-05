//! Geographic projections, whose positions are the places that a map's projector draws: the
//! longitudes and latitudes of a spherical projector, or the planar coordinates of a planar one.

use std::f64::consts::PI;
use std::sync::OnceLock;

use arrow::array::Float32Array;
use geo::{algorithm::bool_ops::FillRule, BooleanOps, InteriorPoint};
use geo_types::{Geometry, LineString, MultiPolygon, Point, Polygon};

use crate::positions::Positions;
use crate::projection::{densified, short_way, PlotProjection};
use avenger_geo::projections::Equirectangular;
use avenger_geo::{polygon_contains, Projection, Projector};

/// The largest step between the places of a path, in degrees, as a graticule densifies its
/// parallels, so that the projection follows a parallel and not a great circle.
const PATH_STEP: f64 = 2.5;

/// The latitude where paths and polygons stop short of a pole, where meridians meet.
const POLE: f64 = 90.0 - 1e-6;

/// The projection of a map plot through the projector of its cartographic projection, from the
/// top-left corner of the plot. A map has no scales: its positions are the places that the
/// projector draws.
///
/// Through a spherical projector, the positions are longitudes and latitudes in degrees, and
/// longitude wraps every 360 degrees. Paths run at a steady change of longitude and latitude, so
/// the edge of a box follows a parallel, routes follow great circles, and lines and polygons cut
/// at the antimeridian and at the clip, such as a globe's horizon.
///
/// Through a planar projector, avenger-geo's `Projection::planar`, the positions are the map's
/// planar coordinates, such as projected meters, which the projector scales, moves, and can
/// reflect onto the plot. The projection is then affine, so its lines are straight, it has no
/// period, and its extent is what the plot shows.
#[derive(Debug)]
pub struct GeoProjection {
    surface: Surface,
}

/// The projection of a spherical or a planar projector.
#[derive(Debug)]
enum Surface {
    Spherical(Spherical),
    Planar(Planar),
}

impl GeoProjection {
    /// The projection through `projector` of a map plot of `size`, whose extent a planar
    /// projector takes from the plot's corners.
    pub fn new(projector: Projector, size: [f32; 2]) -> Self {
        let surface = if projector.is_planar() {
            Surface::Planar(Planar::new(projector, size))
        } else {
            Surface::Spherical(Spherical::new(projector))
        };
        Self { surface }
    }

    /// The projector that the projection draws through.
    pub fn projector(&self) -> &Projector {
        match &self.surface {
            Surface::Spherical(spherical) => &spherical.projector,
            Surface::Planar(planar) => &planar.projector,
        }
    }

    /// The projection of the projector's surface.
    fn surface(&self) -> &dyn PlotProjection {
        match &self.surface {
            Surface::Spherical(spherical) => spherical,
            Surface::Planar(planar) => planar,
        }
    }
}

impl PlotProjection for GeoProjection {
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
        self.surface().position(pixel)
    }

    fn pixel(&self, position: [f64; 2]) -> Option<[f32; 2]> {
        self.surface().pixel(position)
    }

    fn pixels(&self, positions: [Positions<'_>; 2]) -> [Float32Array; 2] {
        self.surface().pixels(positions)
    }

    fn extent(&self) -> [[f64; 2]; 2] {
        self.surface().extent()
    }

    fn region(&self) -> Vec<[f64; 2]> {
        self.surface().region()
    }

    fn clamp(&self, position: [f64; 2]) -> [f64; 2] {
        self.surface().clamp(position)
    }

    fn shows(&self, position: [f64; 2]) -> bool {
        self.surface().shows(position)
    }

    fn outline(&self) -> Vec<Vec<[f32; 2]>> {
        self.surface().outline()
    }

    fn bounds(&self) -> Option<[[f32; 2]; 2]> {
        self.surface().bounds()
    }

    fn tangents(&self, position: [f64; 2]) -> Option<[[f32; 2]; 2]> {
        self.surface().tangents(position)
    }

    fn period(&self, dimension: usize) -> Option<f64> {
        self.surface().period(dimension)
    }

    fn is_linear(&self) -> bool {
        self.surface().is_linear()
    }

    fn path(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        self.surface().path(from, to)
    }

    fn route(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        self.surface().route(from, to)
    }

    fn iso_line(&self, dimension: usize, value: f64, span: Option<[f64; 2]>) -> Vec<Vec<[f32; 2]>> {
        self.surface().iso_line(dimension, value, span)
    }

    fn polygon(&self, ring: &[[f64; 2]]) -> Vec<Vec<[f32; 2]>> {
        self.surface().polygon(ring)
    }

    fn enclosed(&self, path: &[[f32; 2]]) -> Vec<Vec<[f64; 2]>> {
        self.surface().enclosed(path)
    }
}

/// The projection of a spherical projector, whose positions are longitudes and latitudes.
struct Spherical {
    projector: Projector,
    /// The region where the projector draws the sphere, in pixels, which the projection computes
    /// when a lasso first asks for it.
    region: OnceLock<MultiPolygon<f64>>,
}

impl Spherical {
    fn new(projector: Projector) -> Self {
        Self {
            projector,
            region: OnceLock::new(),
        }
    }

    /// The pieces in pixels of `line`, in longitudes and latitudes, as the projector draws it.
    fn lines(&self, line: &LineString<f64>) -> Vec<Vec<[f32; 2]>> {
        let coords = |line: &LineString<f64>| -> Vec<[f32; 2]> {
            line.coords().map(|c| [c.x as f32, c.y as f32]).collect()
        };
        match self.projector.project(line) {
            Geometry::MultiLineString(lines) => lines.0.iter().map(coords).collect(),
            Geometry::LineString(line) => vec![coords(&line)],
            _ => Vec::new(),
        }
    }

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

impl std::fmt::Debug for Spherical {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Spherical").finish_non_exhaustive()
    }
}

impl PlotProjection for Spherical {
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
        self.lines(&steps(from, to).collect())
    }

    /// The pieces of the great circle between the two places, as the projector resamples a line
    /// between them, cut at the antimeridian and at the clip.
    fn route(&self, from: [f64; 2], to: [f64; 2]) -> Vec<Vec<[f32; 2]>> {
        self.lines(&LineString::from(vec![(from[0], from[1]), (to[0], to[1])]))
    }

    /// The sphere as the projector draws it, such as a globe's disk or a flat map's rectangle.
    fn outline(&self) -> Vec<Vec<[f32; 2]>> {
        let region = self.region.get_or_init(|| self.projector.project_sphere());
        (region.iter())
            .flat_map(|polygon| std::iter::once(polygon.exterior()).chain(polygon.interiors()))
            .map(|ring| ring.coords().map(|c| [c.x as f32, c.y as f32]).collect())
            .collect()
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

/// The projection of a planar projector, whose positions are a planar map's coordinates.
struct Planar {
    projector: Projector,
    extent: [[f64; 2]; 2],
}

impl Planar {
    /// The projection through `projector` fitted to a plot of `size`, whose extent holds the
    /// positions at the plot's corners.
    fn new(projector: Projector, size: [f32; 2]) -> Self {
        let [width, height] = size.map(f64::from);
        let mut extent = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
        for (x, y) in [(0.0, 0.0), (width, 0.0), (0.0, height), (width, height)] {
            if let Some(corner) = projector.invert(x, y) {
                for (axis, value) in [corner.0, corner.1].into_iter().enumerate() {
                    extent[axis] = [extent[axis][0].min(value), extent[axis][1].max(value)];
                }
            }
        }
        if !extent.as_flattened().iter().all(|value| value.is_finite()) {
            extent = [[0.0, 0.0]; 2];
        }
        Self { projector, extent }
    }
}

impl std::fmt::Debug for Planar {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Planar")
            .field("extent", &self.extent)
            .finish_non_exhaustive()
    }
}

impl PlotProjection for Planar {
    fn position(&self, pixel: [f32; 2]) -> Option<[f64; 2]> {
        let (x, y) = (self.projector).invert(f64::from(pixel[0]), f64::from(pixel[1]))?;
        Some([x, y])
    }

    fn pixel(&self, [x, y]: [f64; 2]) -> Option<[f32; 2]> {
        let (x, y) = self.projector.project_point(x, y)?;
        (x.is_finite() && y.is_finite()).then_some([x as f32, y as f32])
    }

    fn extent(&self) -> [[f64; 2]; 2] {
        self.extent
    }

    fn is_linear(&self) -> bool {
        true
    }
}
