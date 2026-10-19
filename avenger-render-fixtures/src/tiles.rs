//! Map tile scenes: Web Mercator tiles on several projections, with OpenStreetMap tiles from
//! `data/osm-tiles` for baselines and synthetic tiles for the seam checks. The scenes name their
//! tiles by URL, and each set of tiles is the resolver that the renderers draw them with.

use std::collections::HashMap;
use std::f64::consts::{PI, TAU};
use std::path::Path;
use std::sync::Arc;

use avenger_geo::projections::{
    ConicConformal, EqualEarth, Equirectangular, Mercator, Orthographic,
};
use avenger_geo::tiles::{visible_tiles, TileId};
use avenger_geo::{Projection, Projector, RegionFrame, SphericalClip};
use avenger_guides_geo::{make_sphere_marks, SphereStyle, TileLayer, TileSource};
use avenger_image::{ImageResolver, RgbaImage};
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::scene_graph::SceneGraph;
use geo_types::{Geometry, LineString, Polygon};

/// The URL template of the fixtures' tiles.
pub const TILE_URL: &str = "https://tiles.test/{z}/{x}/{y}.png";

/// Tiles by their URLs from [`TILE_URL`]: the resolver that draws a tile scene.
pub type Tiles = Arc<HashMap<String, Arc<RgbaImage>>>;

/// The URL of a fixture tile.
pub fn tile_url(tile: TileId) -> String {
    TILE_URL
        .replace("{z}", &tile.z().to_string())
        .replace("{x}", &tile.x().to_string())
        .replace("{y}", &tile.y().to_string())
}

/// The latitude where the Web Mercator square ends.
const MERCATOR_LATITUDE: f64 = 85.051_128_779_806_59;

/// A view of tiles: its size, its projection, and the zoom of its tiles.
#[derive(Debug, Clone)]
pub struct TileCase {
    pub name: &'static str,
    pub width: f32,
    pub height: f32,
    pub zoom: u8,
    pub projection: Projection,
}

impl TileCase {
    pub fn projector(&self) -> Projector {
        self.projection.build().unwrap()
    }

    /// The tiles that the view shows.
    pub fn tiles(&self) -> Vec<TileId> {
        visible_tiles(&self.projector(), self.zoom).unwrap()
    }

    /// The view's scene: the sphere's fill in `background`, the tiles at [`TILE_URL`], which
    /// draw through the `tiles` resolver, and, when `outline` is set, the sphere's outline on
    /// top.
    pub fn scene<R: ImageResolver + 'static>(
        &self,
        tiles: &Arc<R>,
        background: [f32; 4],
        outline: bool,
    ) -> SceneGraph {
        let projector = self.projector();
        let fill = make_sphere_marks(
            &projector,
            &SphereStyle {
                fill: background,
                stroke: [0.0; 4],
                stroke_width: 0.0,
            },
        );
        let mut marks = vec![
            fill.into(),
            TileLayer::new(TileSource::new(TILE_URL, tiles.clone()).unwrap())
                .group(&projector, self.zoom)
                .unwrap()
                .into(),
        ];
        if outline {
            let stroke = make_sphere_marks(
                &projector,
                &SphereStyle {
                    fill: [0.0; 4],
                    ..Default::default()
                },
            );
            marks.push(stroke.into());
        }
        SceneGraph {
            width: self.width,
            height: self.height,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                marks,
                ..Default::default()
            }
            .into()],
        }
    }
}

/// `projection` fitted to show the whole sphere in a view of `width` by `height`, 4 units inside
/// its edges.
fn world(mut projection: Projection, width: f64, height: f64) -> Projection {
    let summary = projection.summarize_sphere().unwrap();
    projection
        .fit([[4.0, 4.0], [width - 4.0, height - 4.0]], &summary)
        .unwrap();
    projection
}

/// The views that the baselines and seam checks draw.
pub fn cases() -> Vec<TileCase> {
    vec![
        TileCase {
            name: "web-mercator",
            width: 512.0,
            height: 512.0,
            zoom: 1,
            projection: Projection::new(Mercator).with_view(512.0 / TAU, [256.0, 256.0]),
        },
        TileCase {
            name: "globe",
            width: 400.0,
            height: 400.0,
            zoom: 2,
            projection: Projection::new(Orthographic)
                .with_view(190.0, [200.0, 200.0])
                .with_rotate([-30.0, -35.0, 0.0]),
        },
        TileCase {
            name: "rotated-equal-earth",
            width: 640.0,
            height: 320.0,
            zoom: 2,
            projection: world(
                Projection::new(EqualEarth).with_rotate([150.0, 0.0, 0.0]),
                640.0,
                320.0,
            ),
        },
        TileCase {
            name: "conic-alps",
            width: 480.0,
            height: 320.0,
            zoom: 5,
            projection: Projection::new(ConicConformal::new([40.0, 52.0]))
                .with_rotate([-10.0, 0.0, 0.0])
                .with_center(0.0, 46.5)
                .with_view(1250.0, [240.0, 160.0])
                .with_clip_extent(Some([[10.0, 10.0], [470.0, 310.0]])),
        },
        // The limb runs through the Alps along the meridian 10°E, across the view's center.
        TileCase {
            name: "alps-limb",
            width: 360.0,
            height: 240.0,
            zoom: 5,
            projection: Projection::new(Orthographic)
                .with_rotate([80.0, 0.0, 0.0])
                .with_view(4000.0, [-2599.0, 2997.0])
                .with_clip_extent(Some([[0.0, 0.0], [360.0, 240.0]])),
        },
    ]
}

/// The views that the seam checks draw: the baselines' views, and views with a region, a small
/// cap, and the poles of a world map turned by latitude, inside tiles and on their edges.
pub fn seam_cases() -> Vec<TileCase> {
    let europe = SphericalClip::lon_lat_box(RegionFrame::Earth, -10.0, 35.0, 40.0, 60.0).unwrap();
    let mut cases = cases();
    cases.extend([
        TileCase {
            name: "region",
            width: 640.0,
            height: 320.0,
            zoom: 2,
            projection: world(Projection::new(EqualEarth), 640.0, 320.0).with_clip(Some(europe)),
        },
        TileCase {
            name: "small-cap",
            width: 400.0,
            height: 400.0,
            zoom: 2,
            projection: Projection::new(Orthographic)
                .with_view(190.0, [200.0, 200.0])
                .with_rotate([-10.0, -45.0, 0.0])
                .with_clip(Some(SphericalClip::Circle { angle: 20.0 })),
        },
        TileCase {
            name: "oblique",
            width: 640.0,
            height: 320.0,
            zoom: 1,
            projection: world(
                Projection::new(Equirectangular).with_rotate([10.0, -60.0, 0.0]),
                640.0,
                320.0,
            ),
        },
        TileCase {
            name: "pole-on-an-edge",
            width: 640.0,
            height: 320.0,
            zoom: 2,
            projection: world(
                Projection::new(Equirectangular).with_rotate([0.0, -30.0, 0.0]),
                640.0,
                320.0,
            ),
        },
    ]);
    cases
}

/// The path in `data/osm-tiles` of an OpenStreetMap tile, which `tools/osm-tiles/fetch.py`
/// downloads.
pub fn osm_tile_path(tile: TileId) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "data/osm-tiles/{}/{}/{}.png",
        tile.z(),
        tile.x(),
        tile.y()
    ))
}

/// The encoded OpenStreetMap tile at a [`TILE_URL`] address, as a server would send it, or
/// `None` for a tile that `data/osm-tiles` doesn't hold.
pub fn osm_tile_bytes(url: &str) -> Option<Vec<u8>> {
    let address = url
        .strip_prefix("https://tiles.test/")?
        .strip_suffix(".png")?;
    let [z, x, y] = <[&str; 3]>::try_from(address.split('/').collect::<Vec<_>>()).ok()?;
    let tile = TileId::new(z.parse().ok()?, x.parse().ok()?, y.parse().ok()?).ok()?;
    std::fs::read(osm_tile_path(tile)).ok()
}

/// The OpenStreetMap tiles of `data/osm-tiles` for `tiles`, which `tools/osm-tiles/fetch.py`
/// downloads.
pub fn osm_tiles(tiles: &[TileId]) -> Tiles {
    tiles
        .iter()
        .map(|&tile| {
            let path = osm_tile_path(tile);
            let image = image::open(&path)
                .unwrap_or_else(|error| {
                    panic!("{}: {error}; see tools/osm-tiles/fetch.py", path.display())
                })
                .to_rgba8();
            (tile_url(tile), Arc::new(RgbaImage::from_image(&image)))
        })
        .collect::<HashMap<_, _>>()
        .into()
}

/// The color of OpenStreetMap's water, which fills the sphere where no tile draws.
pub const OSM_WATER: [f32; 4] = [170.0 / 255.0, 211.0 / 255.0, 223.0 / 255.0, 1.0];

/// Tiles of `size` texels a side in one color.
pub fn flat_tiles(tiles: &[TileId], size: u32, color: [u8; 4]) -> Tiles {
    synthetic(tiles, size, |_| color)
}

/// Tiles of `size` texels a side whose color follows [`gradient`] at each texel's center.
pub fn gradient_tiles(tiles: &[TileId], size: u32) -> Tiles {
    synthetic(tiles, size, |[s, t]| {
        let [r, g, b] = gradient(s, t);
        [r.round() as u8, g.round() as u8, b.round() as u8, 255]
    })
}

/// A color that varies continuously over the Web Mercator square and across the antimeridian,
/// from its normalized coordinates.
pub fn gradient(s: f64, t: f64) -> [f64; 3] {
    [
        128.0 + 120.0 * (TAU * s).sin(),
        128.0 + 120.0 * (TAU * t).sin(),
        128.0 + 120.0 * (TAU * (s + t)).cos(),
    ]
}

fn synthetic(tiles: &[TileId], size: u32, color: impl Fn([f64; 2]) -> [u8; 4]) -> Tiles {
    tiles
        .iter()
        .map(|&tile| {
            let n = f64::from(1u32 << tile.z());
            let mut data = Vec::with_capacity((size * size * 4) as usize);
            for j in 0..size {
                for i in 0..size {
                    let s = (f64::from(tile.x()) + (f64::from(i) + 0.5) / f64::from(size)) / n;
                    let t = (f64::from(tile.y()) + (f64::from(j) + 0.5) / f64::from(size)) / n;
                    data.extend(color([s, t]));
                }
            }
            let image = RgbaImage {
                width: size,
                height: size,
                data,
            };
            (tile_url(tile), Arc::new(image))
        })
        .collect::<HashMap<_, _>>()
        .into()
}

/// The pixels of an image of the view at `scale` pixels per unit whose centers lie at least
/// `margin` pixels inside what the view shows of the Web Mercator square, as a mask in rows.
fn tile_interior(case: &TileCase, scale: f64, margin: i64) -> Vec<bool> {
    let width = (f64::from(case.width) * scale).round() as i64;
    let height = (f64::from(case.height) * scale).round() as i64;
    let covered = coverage(case, scale, width as usize, height as usize);
    let at = |x: i64, y: i64| {
        (0..width).contains(&x) && (0..height).contains(&y) && covered[(y * width + x) as usize]
    };
    let mut interior = vec![false; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            interior[(y * width + x) as usize] =
                (-margin..=margin).all(|dy| (-margin..=margin).all(|dx| at(x + dx, y + dy)));
        }
    }
    interior
}

/// The pixels whose centers lie inside what the view shows of the Web Mercator square.
fn coverage(case: &TileCase, scale: f64, width: usize, height: usize) -> Vec<bool> {
    let projector = case.projector();
    let steps = 720;
    let along = |i: usize| -180.0 + 360.0 * i as f64 / steps as f64;
    let mut ring: Vec<(f64, f64)> = (0..=steps)
        .map(|i| (along(i), -MERCATOR_LATITUDE))
        .collect();
    ring.extend((0..=steps).rev().map(|i| (along(i), MERCATOR_LATITUDE)));
    ring.push(ring[0]);
    let projected = projector.project(&Polygon::new(LineString::from(ring), vec![]));
    let Geometry::MultiPolygon(polygons) = projected else {
        panic!("{projected:?}");
    };
    let edges: Vec<[[f64; 2]; 2]> = polygons
        .0
        .iter()
        .flat_map(|polygon| std::iter::once(polygon.exterior()).chain(polygon.interiors()))
        .flat_map(|ring| {
            ring.0.windows(2).map(|w| {
                [
                    [w[0].x * scale, w[0].y * scale],
                    [w[1].x * scale, w[1].y * scale],
                ]
            })
        })
        .collect();
    let mut covered = vec![false; width * height];
    for y in 0..height {
        let center = y as f64 + 0.5;
        let mut crossings: Vec<f64> = edges
            .iter()
            .filter(|[a, b]| (a[1] <= center) != (b[1] <= center))
            .map(|[a, b]| a[0] + (center - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
            .collect();
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.chunks_exact(2) {
            let start = (pair[0] - 0.5).ceil().max(0.0) as usize;
            let end = ((pair[1] - 0.5).floor() + 1.0).clamp(0.0, width as f64) as usize;
            for x in start..end.max(start) {
                covered[y * width + x] = true;
            }
        }
    }
    covered
}

/// The pixels per unit of an image of the view.
fn scale_of(case: &TileCase, image: &image::RgbaImage) -> f64 {
    f64::from(image.width()) / f64::from(case.width)
}

/// Asserts that every pixel at least `margin` pixels inside the tiles of flat-colored tiles has
/// the tiles' color, so no crack, line, or gap shows between tiles or along the map's edge. The
/// image may draw the view at any scale.
pub fn assert_flat(case: &TileCase, image: &image::RgbaImage, color: [u8; 4], margin: i64) {
    let interior = tile_interior(case, scale_of(case, image), margin);
    let mut wrong = Vec::new();
    for (x, y, pixel) in image.enumerate_pixels() {
        let inside = interior[(y * image.width() + x) as usize];
        if inside && pixel.0.iter().zip(color).any(|(a, b)| a.abs_diff(b) > 1) {
            wrong.push((x, y, pixel.0));
        }
    }
    assert!(
        wrong.is_empty(),
        "{}: {} pixels differ from {color:?}, such as {:?}",
        case.name,
        wrong.len(),
        &wrong[..wrong.len().min(8)]
    );
}

/// Asserts that every pixel at least `margin` pixels inside the tiles of gradient tiles of `size`
/// texels shows [`gradient`] at the place under its center within `tolerance`, as the tile there
/// samples it: each tile repeats its edge texels beyond their centers. A pixel whose footprint
/// meets several tiles blends their samples, so any color between theirs passes. The image may
/// draw the view at any scale.
pub fn assert_smooth(
    case: &TileCase,
    image: &image::RgbaImage,
    size: u32,
    tolerance: f64,
    margin: i64,
) {
    // How far from a pixel's center, in pixels, its samples and its meshes' edges can reach.
    const FOOTPRINT: f64 = 1.25;
    let projector = case.projector();
    let scale = scale_of(case, image);
    let interior = tile_interior(case, scale, margin);
    let tiles = f64::from(1u32 << case.zoom);
    let half_texel = 0.5 / f64::from(size);
    let place = |x: f64, y: f64| {
        let (lon, lat) = projector.invert(x / scale, y / scale)?;
        Some([
            (lon + 180.0) / 360.0,
            (1.0 - lat.to_radians().tan().asinh() / PI) / 2.0,
        ])
    };
    // The gradient where the tile at column and row `tile` samples the place `[s, t]`.
    let sampled = |[column, row]: [f64; 2], [s, t]: [f64; 2]| {
        // The place's column coordinate, by whole turns, nearest the tile.
        let s = s + ((column + 0.5) / tiles - s).round();
        let clamp = |coordinate: f64, index: f64| {
            (coordinate * tiles - index).clamp(half_texel, 1.0 - half_texel) + index
        };
        gradient(clamp(s, column) / tiles, clamp(t, row) / tiles)
    };
    let mut worst = (0.0f64, 0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if !interior[(y * image.width() + x) as usize] {
            continue;
        }
        let center = [f64::from(x) + 0.5, f64::from(y) + 0.5];
        let Some(at) = place(center[0], center[1]) else {
            continue;
        };
        let mut met: Vec<[f64; 2]> = Vec::new();
        for dy in [-FOOTPRINT, 0.0, FOOTPRINT] {
            for dx in [-FOOTPRINT, 0.0, FOOTPRINT] {
                if let Some([s, t]) = place(center[0] + dx, center[1] + dy) {
                    let tile = [(s * tiles).floor().rem_euclid(tiles), (t * tiles).floor()];
                    if (0.0..tiles).contains(&tile[1]) && !met.contains(&tile) {
                        met.push(tile);
                    }
                }
            }
        }
        let samples: Vec<[f64; 3]> = met.iter().map(|&tile| sampled(tile, at)).collect();
        let error = (0..3)
            .map(|c| {
                let low = samples.iter().map(|v| v[c]).fold(f64::INFINITY, f64::min);
                let high = samples
                    .iter()
                    .map(|v| v[c])
                    .fold(f64::NEG_INFINITY, f64::max);
                let value = f64::from(pixel.0[c]);
                (low - value).max(value - high).max(0.0)
            })
            .fold(0.0, f64::max);
        if error > worst.0 {
            worst = (error, x, y);
        }
    }
    assert!(
        worst.0 <= tolerance,
        "{}: the color at ({}, {}) is {} from the gradient",
        case.name,
        worst.1,
        worst.2,
        worst.0
    );
}
