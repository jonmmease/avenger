//! Web Mercator map tiles: their addresses, the zoom that suits a view, and the tiles that a view
//! shows.
//!
//! A tile is a cell of the `2^z × 2^z` grid that slippy maps such as OpenStreetMap divide the
//! Web Mercator square into at zoom `z`, numbered from the northwest corner. The square spans
//! every longitude and the latitudes within ±85.0511°, where Web Mercator is square.

mod mesh;

pub use mesh::{TileMesh, TileMesher};

use crate::error::AvengerGeoError;
use crate::math::{DEGREES, PI, RADIANS};
use crate::projector::Projector;
use geo_types::{Geometry, LineString, Polygon};

/// The deepest zoom that tile addresses support.
pub const MAX_ZOOM: u8 = 30;

/// A Web Mercator tile: column `x` and row `y` of the `2^z × 2^z` grid at zoom `z`, with column
/// 0 at the antimeridian and row 0 at the north edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    z: u8,
    x: u32,
    y: u32,
}

impl TileId {
    /// The tile at column `x` and row `y` of zoom `z`, or an error for a zoom beyond
    /// [`MAX_ZOOM`] or a column or row outside the grid.
    pub fn new(z: u8, x: u32, y: u32) -> Result<Self, AvengerGeoError> {
        if z > MAX_ZOOM || u64::from(x) >= 1 << z || u64::from(y) >= 1 << z {
            return Err(AvengerGeoError::InvalidConfig(format!(
                "tile {z}/{x}/{y} is outside the grid of zooms 0 to {MAX_ZOOM}"
            )));
        }
        Ok(Self { z, x, y })
    }

    /// The tile's zoom.
    pub fn z(&self) -> u8 {
        self.z
    }

    /// The tile's column, from 0 at the antimeridian eastward.
    pub fn x(&self) -> u32 {
        self.x
    }

    /// The tile's row, from 0 at the north edge southward.
    pub fn y(&self) -> u32 {
        self.y
    }

    /// The number of columns and of rows at the tile's zoom.
    fn count(&self) -> u32 {
        1 << self.z
    }

    /// The tile's bounds `[[west, south], [east, north]]` in degrees.
    pub fn bounds(&self) -> [[f64; 2]; 2] {
        let n = f64::from(self.count());
        let [x, y] = [f64::from(self.x), f64::from(self.y)];
        [
            [longitude(x / n), latitude((y + 1.0) / n)],
            [longitude((x + 1.0) / n), latitude(y / n)],
        ]
    }

    /// The tile `dx` columns east and `dy` rows south. Columns wrap around the antimeridian, and
    /// there is no tile beyond the top or the bottom row.
    pub fn neighbor(&self, dx: i64, dy: i64) -> Option<TileId> {
        let n = i64::from(self.count());
        let y = i64::from(self.y) + dy;
        if !(0..n).contains(&y) {
            return None;
        }
        let x = (i64::from(self.x) + dx).rem_euclid(n);
        Some(TileId {
            z: self.z,
            x: x as u32,
            y: y as u32,
        })
    }

    /// The tile at the previous zoom that this one divides, or `None` at zoom 0.
    pub fn parent(&self) -> Option<TileId> {
        let z = self.z.checked_sub(1)?;
        Some(TileId {
            z,
            x: self.x / 2,
            y: self.y / 2,
        })
    }

    /// The four tiles at the next zoom that divide this one.
    fn children(&self) -> [TileId; 4] {
        let (z, x, y) = (self.z + 1, self.x * 2, self.y * 2);
        [
            TileId { z, x, y },
            TileId { z, x: x + 1, y },
            TileId { z, x, y: y + 1 },
            TileId {
                z,
                x: x + 1,
                y: y + 1,
            },
        ]
    }

    /// The tile's outline as a polygon of longitudes and latitudes, wound as RFC 7946 winds an
    /// exterior. The pipeline draws edges as great circles, so the parallels have a vertex at
    /// least every two degrees of longitude.
    fn outline(&self) -> Polygon<f64> {
        let [[west, south], [east, north]] = self.bounds();
        let steps = ((east - west) / 2.0).ceil().max(1.0) as usize;
        let along = |t: usize| west + (east - west) * t as f64 / steps as f64;
        let mut ring: Vec<(f64, f64)> = (0..=steps).map(|t| (along(t), south)).collect();
        ring.extend((0..=steps).rev().map(|t| (along(t), north)));
        ring.push(ring[0]);
        Polygon::new(LineString::from(ring), vec![])
    }
}

/// The longitude in degrees of a normalized Web Mercator column coordinate, 0 at the
/// antimeridian's west side and 1 at its east side.
pub(crate) fn longitude(s: f64) -> f64 {
    360.0 * s - 180.0
}

/// The latitude in degrees of a normalized Web Mercator row coordinate, 0 at the north edge and
/// 1 at the south edge.
pub(crate) fn latitude(t: f64) -> f64 {
    (PI * (1.0 - 2.0 * t)).sinh().atan() * DEGREES
}

/// The normalized Web Mercator column coordinate of a longitude in degrees.
pub(crate) fn column(lon: f64) -> f64 {
    (lon + 180.0) / 360.0
}

/// The normalized Web Mercator row coordinate of a latitude in degrees.
pub(crate) fn row(lat: f64) -> f64 {
    (1.0 - (lat * RADIANS).tan().asinh() / PI) / 2.0
}

/// The fractional zoom at which tiles of `tile_size` texels show about one texel per output unit
/// at the view's center, or `None` where the view's center misses the map, a pole is there, or
/// the projection is planar. Callers round it and clamp it to their tile source's zooms. The
/// view's center is the middle of the clip extent, or without one, the view's translation,
/// where the projection's center lands.
///
/// The density is the geometric mean of the east-west and north-south densities at the place
/// under the view's center, so it matches exactly where the projection is conformal there.
pub fn tile_zoom(projector: &Projector, tile_size: f64) -> Option<f64> {
    if projector.is_planar() || !(tile_size.is_finite() && tile_size > 0.0) {
        return None;
    }
    let (lambda, phi) = projector.place_at(projector.view_center())?;
    let (lon, lat) = (lambda * DEGREES, phi * DEGREES);
    let step = 1e-4;
    let distance = |a: (f64, f64), b: (f64, f64)| {
        let (a, b) = (
            projector.project_point(a.0, a.1)?,
            projector.project_point(b.0, b.1)?,
        );
        Some((b.0 - a.0).hypot(b.1 - a.1) / (2.0 * step * RADIANS))
    };
    // Output units per radian of longitude, and per radian of latitude.
    let east = distance((lon - step, lat), (lon + step, lat))?;
    let north = distance((lon, lat - step), (lon, lat + step))?;
    // Web Mercator stretches latitude by 1 / cos φ, so a radian of latitude there spans as many
    // texels as 1 / cos φ radians of longitude.
    let density = (east * north * phi.cos()).sqrt();
    let zoom = (2.0 * PI * density / tile_size).log2();
    zoom.is_finite().then_some(zoom)
}

/// The tiles at zoom `z` that the view shows, those whose outline survives the projector's
/// pipeline: its rotation, its spherical clip, and its clip extent. A tile also shows where the
/// map shows one of its corners, its center, or a pole of the rotated frame in it, or where a
/// corner of the clip extent shows a place in it, which keeps tiles that the pipeline's
/// tolerances lose: those with an edge through the frame's pole, and those smaller than the
/// clip's tolerance at deep zooms. A planar projection shows no tiles. The tiles come in order
/// of zoom-0 descent, northwest child first.
///
/// # Errors
/// Returns an error for a zoom beyond [`MAX_ZOOM`].
pub fn visible_tiles(projector: &Projector, z: u8) -> Result<Vec<TileId>, AvengerGeoError> {
    if z > MAX_ZOOM {
        return Err(AvengerGeoError::InvalidConfig(format!(
            "zoom {z} is beyond the deepest zoom, {MAX_ZOOM}"
        )));
    }
    if projector.is_planar() {
        return Ok(Vec::new());
    }
    // The zoom 0 tile spans every longitude, so its outline would be a band whose ends meet
    // along the antimeridian. Its children are ordinary polygons, so it shows where they do.
    let root = TileId { z: 0, x: 0, y: 0 };
    if z == 0 {
        let shown = root.children().iter().any(|tile| shows(projector, tile));
        return Ok(if shown { vec![root] } else { Vec::new() });
    }
    let mut tiles = vec![root];
    for _ in 0..z {
        tiles = tiles
            .iter()
            .flat_map(TileId::children)
            .filter(|tile| shows(projector, tile))
            .collect();
    }
    Ok(tiles)
}

/// Whether the view shows any of the tile.
fn shows(projector: &Projector, tile: &TileId) -> bool {
    let [[west, south], [east, north]] = tile.bounds();
    let inside =
        |(lon, lat): (f64, f64)| (west..=east).contains(&lon) && (south..=north).contains(&lat);
    let middle = (
        (west + east) / 2.0,
        latitude((f64::from(tile.y) + 0.5) / f64::from(tile.count())),
    );
    let corners = [
        (west, south),
        (east, south),
        (east, north),
        (west, north),
        middle,
    ];
    let poles = projector
        .frame_poles()
        .into_iter()
        .filter(|&pole| inside(pole));
    // A place under a corner of the clip extent shows where the clip keeps it.
    let extent = projector
        .clip_extent()
        .into_iter()
        .flat_map(|[[x0, y0], [x1, y1]]| {
            [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
                .into_iter()
                .filter_map(|corner| projector.place_at(corner))
                .map(|(lambda, phi)| (lambda * DEGREES, phi * DEGREES))
        });
    corners
        .into_iter()
        .chain(poles)
        .any(|(lon, lat)| projector.shows_place(lon, lat))
        || extent.into_iter().any(inside)
        || matches!(projector.project(&tile.outline()), Geometry::MultiPolygon(polygons) if !polygons.0.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projections::{Mercator, Orthographic};
    use crate::Projection;

    #[test]
    fn a_tile_divides_its_parent() {
        assert_eq!(TileId::new(0, 0, 0).unwrap().parent(), None);
        let tile = TileId::new(3, 5, 2).unwrap();
        let parent = tile.parent().unwrap();
        assert_eq!(parent, TileId::new(2, 2, 1).unwrap());
        assert!(parent.children().contains(&tile));
        let [[west, south], [east, north]] = tile.bounds();
        let [[p_west, p_south], [p_east, p_north]] = parent.bounds();
        assert!(p_west <= west && east <= p_east && p_south <= south && north <= p_north);
    }

    #[test]
    fn tiles_have_slippy_map_bounds() {
        let [[west, south], [east, north]] = TileId::new(0, 0, 0).unwrap().bounds();
        assert_eq!([west, east], [-180.0, 180.0]);
        assert!((north - 85.051_128_779_806_59).abs() < 1e-9 && (south + north).abs() < 1e-9);
        // Zoom 3, column 4, row 2: from 0° to 45° east, and from 40.98° to 66.51° north.
        let [[west, south], [east, north]] = TileId::new(3, 4, 2).unwrap().bounds();
        assert_eq!([west, east], [0.0, 45.0]);
        assert!((south - 40.979_898_069_620_134).abs() < 1e-9, "{south}");
        assert!((north - 66.513_260_443_111_86).abs() < 1e-9, "{north}");
        assert!(TileId::new(3, 8, 0).is_err() && TileId::new(31, 0, 0).is_err());
    }

    #[test]
    fn neighbors_wrap_across_the_antimeridian_and_stop_at_the_poles() {
        let tile = TileId::new(2, 3, 0).unwrap();
        assert_eq!(tile.neighbor(1, 0), Some(TileId::new(2, 0, 0).unwrap()));
        assert_eq!(tile.neighbor(-1, 1), Some(TileId::new(2, 2, 1).unwrap()));
        assert_eq!(tile.neighbor(0, -1), None);
        assert_eq!(TileId::new(2, 0, 3).unwrap().neighbor(-1, 1), None);
    }

    /// A Web Mercator view whose scale puts one tile texel on one output unit sits at a whole
    /// zoom, at any latitude.
    #[test]
    fn web_mercator_views_match_tile_zooms() {
        for (z, lat) in [(0.0, 0.0), (3.0, 45.0), (10.0, -60.0)] {
            let scale = 256.0 * f64::powf(2.0, z) / (2.0 * PI);
            let projector = Projection::new(Mercator)
                .with_view(scale, [0.0, 0.0])
                .with_center(10.0, lat)
                .build()
                .unwrap();
            let zoom = tile_zoom(&projector, 256.0).unwrap();
            assert!((zoom - z).abs() < 1e-6, "{z} at {lat}: {zoom}");
        }
        let planar = Projection::planar(false)
            .with_view(1.0, [0.0, 0.0])
            .build()
            .unwrap();
        assert_eq!(tile_zoom(&planar, 256.0), None);
    }

    /// The zoom follows the place under the view's center, not the projection's own center: on
    /// an orthographic globe, a place 60 degrees from the projection's center draws at half the
    /// scale in each direction, and Web Mercator draws it at twice the scale.
    #[test]
    fn tile_zooms_follow_the_view_center() {
        let scale = 256.0 * 8.0 / (2.0 * PI);
        let globe = Projection::new(Orthographic).with_view(scale, [200.0, 200.0]);
        let center = tile_zoom(&globe.clone().build().unwrap(), 256.0).unwrap();
        assert!((center - 3.0).abs() < 1e-6, "{center}");
        let north = globe.with_center(0.0, 60.0).build().unwrap();
        let zoom = tile_zoom(&north, 256.0).unwrap();
        assert!((zoom - 1.5).abs() < 1e-6, "{zoom}");
    }

    /// A globe at one texel per output unit shows the tiles under its view at deep zooms, where
    /// tiles are smaller than the circle clip's tolerance.
    #[test]
    fn deep_zooms_show_the_tiles_under_the_view() {
        for z in [22u8, 25, 30] {
            let scale = 256.0 * 2f64.powi(i32::from(z)) / (2.0 * PI);
            let projector = Projection::new(Orthographic)
                .with_rotate([-7.45, -46.95, 0.0])
                .with_view(scale, [200.0, 150.0])
                .with_clip_extent(Some([[0.0, 0.0], [400.0, 300.0]]))
                .build()
                .unwrap();
            let tiles = visible_tiles(&projector, z).unwrap();
            assert!((4..=16).contains(&tiles.len()), "zoom {z}: {}", tiles.len());
        }
    }

    #[test]
    fn a_globe_shows_the_tiles_on_its_near_side() {
        let projector = Projection::new(Orthographic)
            .with_view(100.0, [0.0, 0.0])
            .with_rotate([-10.0, -20.0, 0.0])
            .build()
            .unwrap();
        let tiles = visible_tiles(&projector, 3).unwrap();
        let has = |x, y| tiles.contains(&TileId::new(3, x, y).unwrap());
        // The tile under the center, from 0° to 45° east and 40.98° to 66.51° north, shows; the
        // one opposite it doesn't.
        assert!(has(4, 2));
        assert!(!has(0, 5));
        // A tile whose west edge, at 90° east, lies half a degree inside the horizon shows as a
        // sliver, and it doesn't when the edge lies half a degree beyond.
        let sliver = |center: f64| {
            let projector = Projection::new(Orthographic)
                .with_view(100.0, [0.0, 0.0])
                .with_rotate([-center, 0.0, 0.0])
                .build()
                .unwrap();
            visible_tiles(&projector, 3)
                .unwrap()
                .contains(&TileId::new(3, 6, 3).unwrap())
        };
        assert!(sliver(0.5));
        assert!(!sliver(-0.5));
    }

    #[test]
    fn a_clip_extent_limits_the_tiles() {
        // Web Mercator at zoom 2 on a 1024 square, clipped to its northwest quarter.
        let scale = 1024.0 / (2.0 * PI);
        let projection = Projection::new(Mercator).with_view(scale, [512.0, 512.0]);
        let world = visible_tiles(&projection.clone().build().unwrap(), 2).unwrap();
        assert_eq!(world.len(), 16);
        let quarter = projection
            .with_clip_extent(Some([[0.0, 0.0], [511.0, 511.0]]))
            .build()
            .unwrap();
        let mut tiles = visible_tiles(&quarter, 2).unwrap();
        tiles.sort();
        let expected: Vec<TileId> = [(0, 0), (0, 1), (1, 0), (1, 1)]
            .iter()
            .map(|&(x, y)| TileId::new(2, x, y).unwrap())
            .collect();
        assert_eq!(tiles, expected);
        assert!(visible_tiles(&quarter, MAX_ZOOM + 1).is_err());
    }
}
