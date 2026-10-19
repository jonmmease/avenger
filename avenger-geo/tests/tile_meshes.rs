//! Tile meshes follow the projection, meet their neighbors exactly, and end at the map's outline.

use avenger_geo::projections::{
    AzimuthalEqualArea, AzimuthalEquidistant, ConicConformal, EqualEarth, Equirectangular,
    Gnomonic, Mercator, NaturalEarth1, Orthographic,
};
use avenger_geo::tiles::{visible_tiles, TileId, TileMesh, TileMesher};
use avenger_geo::{Projection, Projector, RegionFrame, SphericalClip};
use geo_types::{Geometry, LineString, MultiPolygon, Polygon};
use std::collections::{BTreeMap, BTreeSet};

/// The projectors' precision: the default.
const PRECISION: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// The latitude where the Web Mercator square ends.
const MERCATOR_LATITUDE: f64 = 85.051_128_779_806_59;

/// The meshes of every tile that the view shows at `zoom`, from one mesher.
fn meshes(projector: &Projector, zoom: u8) -> Vec<(TileId, TileMesh)> {
    let mut mesher = TileMesher::new(projector, zoom).unwrap();
    visible_tiles(projector, zoom)
        .unwrap()
        .into_iter()
        .map(|tile| (tile, mesher.mesh(tile).unwrap()))
        .collect()
}

fn triangles(mesh: &TileMesh) -> impl Iterator<Item = [[f64; 2]; 3]> + '_ {
    mesh.indices.chunks_exact(3).map(|t| {
        t.iter()
            .map(|&i| mesh.positions[i as usize].map(f64::from))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    })
}

fn mesh_area(mesh: &TileMesh) -> f64 {
    triangles(mesh)
        .map(|[a, b, c]| {
            ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
        })
        .sum()
}

fn ring_area(ring: &LineString<f64>) -> f64 {
    ring.0
        .windows(2)
        .map(|w| w[0].x * w[1].y - w[1].x * w[0].y)
        .sum::<f64>()
        .abs()
        / 2.0
}

fn ring_length(ring: &LineString<f64>) -> f64 {
    ring.0
        .windows(2)
        .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
        .sum()
}

/// The area and the outline's length of polygons.
fn polygons_area(polygons: &MultiPolygon<f64>) -> (f64, f64) {
    polygons
        .0
        .iter()
        .fold((0.0, 0.0), |(area, length), polygon| {
            let interiors = polygon.interiors();
            (
                area + ring_area(polygon.exterior()) - interiors.iter().map(ring_area).sum::<f64>(),
                length
                    + ring_length(polygon.exterior())
                    + interiors.iter().map(ring_length).sum::<f64>(),
            )
        })
}

/// The area of a tile that the view shows, through the projector's own pipeline, and the length
/// of its outline.
fn outline_area(projector: &Projector, tile: TileId) -> (f64, f64) {
    let [[west, south], [east, north]] = tile.bounds();
    let steps = ((east - west) / 0.5).ceil() as usize;
    let along = |t: usize| west + (east - west) * t as f64 / steps as f64;
    let mut ring: Vec<(f64, f64)> = (0..=steps).map(|t| (along(t), south)).collect();
    ring.extend((0..=steps).rev().map(|t| (along(t), north)));
    ring.push(ring[0]);
    match projector.project(&Polygon::new(LineString::from(ring), vec![])) {
        Geometry::MultiPolygon(polygons) => polygons_area(&polygons),
        other => panic!("{other:?}"),
    }
}

/// The edges of a tile's outline that lie along one border of its texture, as pairs of
/// positions' bits. An edge of the outline belongs to one triangle.
fn border_edges(mesh: &TileMesh, on_border: impl Fn([f32; 2]) -> bool) -> BTreeSet<[[u32; 2]; 2]> {
    let mut uses = BTreeMap::new();
    for t in mesh.indices.chunks_exact(3) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (a, b) = (a as usize, b as usize);
            if on_border(mesh.uvs[a]) && on_border(mesh.uvs[b]) {
                let bits = |i: usize| mesh.positions[i].map(f32::to_bits);
                let (pa, pb) = (bits(a), bits(b));
                *uses
                    .entry(if pa <= pb { [pa, pb] } else { [pb, pa] })
                    .or_insert(0) += 1;
            }
        }
    }
    uses.into_iter()
        .filter(|&(_, count)| count == 1)
        .map(|(edge, _)| edge)
        .collect()
}

/// Neighbors share the same edges along their shared border, so no T-junction opens a crack.
/// Where the map is cut at 180 degrees, the tiles on either side of it are at opposite ends of
/// the map, and `cut_at_180` skips them.
fn assert_shared_edges(label: &str, meshes: &[(TileId, TileMesh)], cut_at_180: bool) {
    let find = |tile: TileId| {
        meshes
            .iter()
            .find(|(id, _)| *id == tile)
            .map(|(_, mesh)| mesh)
    };
    let mut pairs = 0;
    for (tile, mesh) in meshes {
        let wraps = tile.x() + 1 == 1 << tile.z();
        if let Some(east) = tile
            .neighbor(1, 0)
            .filter(|_| !(wraps && cut_at_180))
            .and_then(find)
        {
            let ours = border_edges(mesh, |uv| uv[0] == 1.0);
            let theirs = border_edges(east, |uv| uv[0] == 0.0);
            assert_eq!(ours, theirs, "{label}: {tile:?} and its east neighbor");
            pairs += usize::from(!ours.is_empty());
        }
        if let Some(south) = tile.neighbor(0, 1).and_then(find) {
            let ours = border_edges(mesh, |uv| uv[1] == 1.0);
            let theirs = border_edges(south, |uv| uv[1] == 0.0);
            assert_eq!(ours, theirs, "{label}: {tile:?} and its south neighbor");
            pairs += usize::from(!ours.is_empty());
        }
    }
    assert!(pairs > 0, "{label}: no shared borders");
}

/// Each tile's mesh covers what the view shows of the tile, within the precision of its
/// outline, so the meshes leave no gap and don't overlap.
fn assert_covers(label: &str, projector: &Projector, meshes: &[(TileId, TileMesh)]) {
    for (tile, mesh) in meshes {
        let drawn = mesh_area(mesh);
        let (shown, outline) = outline_area(projector, *tile);
        assert!(
            (drawn - shown).abs() <= PRECISION * outline,
            "{label}: {tile:?} draws {drawn} where it shows {shown} within an outline of {outline}"
        );
    }
}

fn world(projection: Projection) -> Projector {
    fitted(projection).build().unwrap()
}

/// An orthographic globe of radius 200 around (250, 250), centered on 100° E, 30° N.
fn globe() -> Projection {
    Projection::new(Orthographic)
        .with_view(200.0, [250.0, 250.0])
        .with_rotate([-100.0, -30.0, 0.0])
}

/// A projection fitted to show the whole sphere in 800 by 400.
fn fitted(projection: Projection) -> Projection {
    let mut projection = projection;
    let summary = projection.summarize_sphere().unwrap();
    projection
        .fit([[0.0, 0.0], [800.0, 400.0]], &summary)
        .unwrap();
    projection
}

/// Web Mercator at a tile zoom draws each tile on its square, and from zoom 4, where cells may
/// span a whole tile, as two triangles.
#[test]
fn web_mercator_tiles_are_two_triangles() {
    let projector = Projection::new(Mercator)
        .with_view(4096.0 / std::f64::consts::TAU, [2048.0, 2048.0])
        .build()
        .unwrap();
    let meshes = meshes(&projector, 4);
    assert_eq!(meshes.len(), 256);
    for (tile, mesh) in &meshes {
        assert_eq!(mesh.indices.len(), 6, "{tile:?}");
        for (position, uv) in mesh.positions.iter().zip(&mesh.uvs) {
            let expected = [
                256.0 * (tile.x() as f32 + uv[0]),
                256.0 * (tile.y() as f32 + uv[1]),
            ];
            assert!(
                (position[0] - expected[0]).abs() < 1e-3
                    && (position[1] - expected[1]).abs() < 1e-3,
                "{tile:?}: {position:?} vs {expected:?}"
            );
        }
    }
}

#[test]
fn neighbors_share_their_borders() {
    let europe = Projection::new(ConicConformal::new([35.0, 65.0]))
        .with_rotate([-10.0, 0.0, 0.0])
        .with_view(900.0, [400.0, 300.0])
        .with_center(0.0, 50.0)
        .with_clip_extent(Some([[0.0, 0.0], [800.0, 600.0]]))
        .build()
        .unwrap();
    let globe = globe().build().unwrap();
    for (label, projector, zoom, cut_at_180) in [
        ("equal earth", world(Projection::new(EqualEarth)), 2, true),
        (
            "rotated equal earth",
            world(Projection::new(EqualEarth).with_rotate([150.0, 0.0, 0.0])),
            2,
            false,
        ),
        ("conic", europe, 4, false),
        ("globe", globe, 3, false),
    ] {
        assert_shared_edges(label, &meshes(&projector, zoom), cut_at_180);
    }
}

/// The meshes don't depend on the order of the tiles.
#[test]
fn meshes_do_not_depend_on_order() {
    let projector = globe().build().unwrap();
    let tiles = visible_tiles(&projector, 3).unwrap();
    let mut forward = TileMesher::new(&projector, 3).unwrap();
    let mut backward = TileMesher::new(&projector, 3).unwrap();
    let ahead: Vec<TileMesh> = tiles
        .iter()
        .map(|&tile| forward.mesh(tile).unwrap())
        .collect();
    let mut behind: Vec<TileMesh> = tiles
        .iter()
        .rev()
        .map(|&tile| backward.mesh(tile).unwrap())
        .collect();
    behind.reverse();
    assert_eq!(ahead, behind);
}

/// A globe's tiles cover its near side, and reach past its horizon by at most the precision.
#[test]
fn globe_tiles_end_at_the_horizon() {
    let projector = globe().build().unwrap();
    let meshes = meshes(&projector, 3);
    assert_covers("globe", &projector, &meshes);
    let reach = 200.0 + PRECISION + 1e-3;
    for (tile, mesh) in &meshes {
        for &[x, y] in &mesh.positions {
            let distance = (f64::from(x) - 250.0).hypot(f64::from(y) - 250.0);
            assert!(distance <= reach, "{tile:?}: {distance}");
        }
    }
}

/// The worst distance, in output units, between a triangle's centroid and where its texture
/// coordinates' place projects: a triangle that draws a place far from where the map shows it
/// draws an unrelated part of the map.
fn worst_texture_offset(projector: &Projector, zoom: u8, meshes: &[(TileId, TileMesh)]) -> f64 {
    let n = f64::from(1u32 << zoom);
    let mut worst = 0.0f64;
    for (tile, mesh) in meshes {
        for t in mesh.indices.chunks_exact(3) {
            let centroid = |values: &[[f32; 2]]| {
                let [a, b, c] = [0, 1, 2].map(|i| values[t[i] as usize].map(f64::from));
                [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0]
            };
            let ([x, y], [u, v]) = (centroid(&mesh.positions), centroid(&mesh.uvs));
            let lon = (f64::from(tile.x()) + u) / n * 360.0 - 180.0;
            let lat = (std::f64::consts::PI * (1.0 - 2.0 * (f64::from(tile.y()) + v) / n))
                .sinh()
                .atan()
                .to_degrees();
            if let Some((px, py)) = projector.project_point(lon, lat) {
                worst = worst.max((px - x).hypot(py - y));
            }
        }
    }
    worst
}

/// An azimuthal projection that shows nearly the whole sphere spreads the cells near its
/// antipode around the map's edge. The mesh stops short of the antipode, so every triangle draws
/// its own place, and the tiles still cover most of the map.
#[test]
fn whole_sphere_azimuthal_tiles_stop_short_of_the_antipode() {
    for (label, projection, covered) in [
        ("equal-area", Projection::new(AzimuthalEqualArea), 0.96),
        ("equidistant", Projection::new(AzimuthalEquidistant), 0.9),
    ] {
        for (rotate, scale, zoom) in [
            ([10.0, 20.0, 30.0], 100.0, 3),
            ([-100.0, -30.0, 0.0], 100.0, 4),
            ([45.0, -60.0, 10.0], 300.0, 3),
            ([170.0, 5.0, 0.0], 300.0, 4),
        ] {
            let projector = projection
                .clone()
                .with_view(scale, [0.0, 0.0])
                .with_rotate(rotate)
                .build()
                .unwrap();
            let meshes = meshes(&projector, zoom);
            let offset = worst_texture_offset(&projector, zoom, &meshes);
            assert!(
                offset <= 2.0 * PRECISION,
                "{label} {rotate:?} at {scale}, zoom {zoom}: a triangle draws {offset} away from its place"
            );
            let drawn: f64 = meshes.iter().map(|(_, mesh)| mesh_area(mesh)).sum();
            let sphere = polygons_area(&projector.project_sphere()).0;
            assert!(
                drawn >= covered * sphere,
                "{label} {rotate:?} at {scale}, zoom {zoom}: the tiles draw {drawn} of {sphere}"
            );
        }
    }
}

/// A region that reaches past a projection's own circle, as one 65° to 70° east of a gnomonic
/// map's center reaches past its 60° cap, still gets tiles under every place it shows.
#[test]
fn a_region_past_the_projections_circle_gets_tiles() {
    let projector = Projection::new(Gnomonic)
        .with_view(100.0, [0.0, 0.0])
        .with_clip(Some(
            SphericalClip::lon_lat_box(RegionFrame::View, 65.0, -5.0, 70.0, 5.0).unwrap(),
        ))
        .build()
        .unwrap();
    let meshes = meshes(&projector, 5);
    for lon in [65.5, 67.0, 69.5] {
        for lat in [-4.5, 0.0, 2.0, 4.5] {
            let [x, y] = projector
                .project_point(lon, lat)
                .map(|(x, y)| [x, y])
                .unwrap();
            let covered = meshes.iter().any(|(_, mesh)| {
                triangles(mesh).any(|[a, b, c]| {
                    let side = |p: [f64; 2], q: [f64; 2]| {
                        (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0])
                    };
                    let sides = [side(a, b), side(b, c), side(c, a)];
                    sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0)
                })
            });
            assert!(covered, "({lon}, {lat}) at ({x}, {y}) has no tile");
        }
    }
}

/// A world turned so that its antimeridian cuts through tiles reaches both sides of the map, and
/// no triangle spans it.
#[test]
fn rotated_world_tiles_end_at_the_cut() {
    let projector = world(Projection::new(EqualEarth).with_rotate([150.0, 0.0, 0.0]));
    let meshes = meshes(&projector, 2);
    assert_covers("rotated world", &projector, &meshes);
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for (tile, mesh) in &meshes {
        for [a, b, c] in triangles(mesh) {
            for (p, q) in [(a, b), (b, c), (c, a)] {
                assert!((p[0] - q[0]).abs() < 200.0, "{tile:?}: {p:?} to {q:?}");
            }
        }
        for &[x, _] in &mesh.positions {
            low = low.min(f64::from(x));
            high = high.max(f64::from(x));
        }
    }
    assert!(low <= 0.01 && high >= 799.99, "{low} to {high}");
}

/// A circle or region clip lies within the projection's own edge, and the map's outline clips
/// the tiles to it, so their meshes follow the projection's own edge: a cap's tiles reach the
/// globe's horizon, and a region's tiles cover them whole.
#[test]
fn clips_draw_through_the_outline() {
    let region = SphericalClip::lon_lat_box(RegionFrame::Earth, -40.0, -30.0, 60.0, 50.0).unwrap();
    for (label, clipped, own) in [
        (
            "cap",
            globe()
                .with_clip(Some(SphericalClip::Circle { angle: 60.0 }))
                .build()
                .unwrap(),
            globe().build().unwrap(),
        ),
        (
            "region",
            fitted(Projection::new(EqualEarth))
                .with_clip(Some(region))
                .build()
                .unwrap(),
            world(Projection::new(EqualEarth)),
        ),
    ] {
        let meshes = meshes(&clipped, 3);
        assert!(!meshes.is_empty(), "{label}");
        assert_covers(label, &own, &meshes);
    }
}

/// A world map turned by latitude draws each pole of its rotated frame as a line, and the tiles
/// reach it from every side, with the pole inside a tile or on a tile's edge. Turned by latitude
/// alone, the map's cut runs along the meridians through the poles, which are tiles' edges, so
/// tiles on either side of it don't share those edges.
#[test]
fn oblique_world_tiles_reach_the_frame_poles() {
    for (label, projection, cut_along_edges) in [
        (
            "equirectangular",
            Projection::new(Equirectangular).with_rotate([10.0, -60.0, 0.0]),
            false,
        ),
        (
            "pole on an edge",
            Projection::new(Equirectangular).with_rotate([0.0, -30.0, 0.0]),
            true,
        ),
        (
            "natural earth",
            Projection::new(NaturalEarth1).with_rotate([10.0, -60.0, 20.0]),
            false,
        ),
    ] {
        let projector = world(projection);
        for zoom in [1, 2] {
            let meshes = meshes(&projector, zoom);
            assert_eq!(meshes.len(), 1 << (2 * zoom), "{label}: zoom {zoom}");
            if !cut_along_edges {
                assert_shared_edges(label, &meshes, false);
            }
            // The tiles cover the map but the caps beyond Web Mercator's latitudes.
            let (sphere, outline) = polygons_area(&projector.project_sphere());
            let (caps, cap_outlines) = [MERCATOR_LATITUDE, -MERCATOR_LATITUDE]
                .iter()
                .map(|&lat| {
                    let ring: Vec<(f64, f64)> =
                        (0..=360).map(|i| (f64::from(i) - 180.0, lat)).collect();
                    let ring = if lat > 0.0 {
                        ring
                    } else {
                        ring.into_iter().rev().collect()
                    };
                    match projector.project(&Polygon::new(LineString::from(ring), vec![])) {
                        Geometry::MultiPolygon(polygons) => polygons_area(&polygons),
                        other => panic!("{other:?}"),
                    }
                })
                .fold((0.0, 0.0), |(area, length), (a, l)| (area + a, length + l));
            let drawn: f64 = meshes.iter().map(|(_, mesh)| mesh_area(mesh)).sum();
            let shown = sphere - caps;
            // The outline's chords lie within the precision inside the map's edge, and the
            // meshes' edges within the precision outside it.
            assert!(
                (drawn - shown).abs() <= 2.0 * PRECISION * (outline + cap_outlines),
                "{label}: zoom {zoom} draws {drawn} where the map shows {shown}"
            );
        }
    }
}
