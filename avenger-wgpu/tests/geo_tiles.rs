//! Map tiles on several projections, drawn by the GPU: seam checks with synthetic tiles, and
//! baselines with OpenStreetMap tiles.

mod support;

use avenger_common::canvas::CanvasDimensions;
use avenger_render_fixtures::tiles;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use support::assert_baseline;

/// The page outside the map, and the sphere's fill where no tile draws.
const BACKGROUND: [f32; 4] = [1.0, 0.0, 1.0, 1.0];

/// Draw `scene`, whose tiles draw through `tiles`.
fn render(scene: &SceneGraph, tiles: &tiles::Tiles) -> image::RgbaImage {
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 1.0,
        },
        CanvasConfig {
            images: Some(tiles.clone()),
            ..Default::default()
        },
    ))
    .unwrap();
    canvas
        .set_scene(scene, &avenger_typst_label::bundled_label_engine())
        .unwrap();
    pollster::block_on(canvas.render()).unwrap()
}

/// Flat-colored tiles draw one color everywhere inside them: no crack, antialiased line, or
/// gap along the map's edge shows the sphere's fill through them.
#[test]
fn flat_tiles_show_no_seams() {
    for case in tiles::seam_cases() {
        let color = [40, 120, 200, 255];
        let tiles = tiles::flat_tiles(&case.tiles(), 16, color);
        let image = render(&case.scene(&tiles, BACKGROUND, false), &tiles);
        tiles::assert_flat(&case, &image, color, 2);
    }
}

/// Tiles of a gradient draw the gradient as each tile samples it, repeating its edge texels at
/// its border.
#[test]
fn gradient_tiles_follow_their_textures() {
    for case in tiles::seam_cases() {
        let tiles = tiles::gradient_tiles(&case.tiles(), 16);
        let image = render(&case.scene(&tiles, BACKGROUND, false), &tiles);
        tiles::assert_smooth(&case, &image, 16, 3.0, 2);
    }
}

/// OpenStreetMap tiles on each view, under the sphere's outline.
#[test]
fn osm_tiles_match_baselines() {
    for case in tiles::cases() {
        let osm = tiles::osm_tiles(&case.tiles());
        let image = render(&case.scene(&osm, tiles::OSM_WATER, true), &osm);
        assert_baseline(&image, &format!("geo-tiles-{}", case.name));
    }
}

/// A fetcher of the fixtures' OpenStreetMap tiles, which answers once a test opens its gate, and
/// with a 404 for a tile that the fixtures don't hold.
#[derive(Default)]
struct FixtureFetcher {
    open: std::sync::Mutex<bool>,
    opened: std::sync::Condvar,
}

impl avenger_image::ImageFetcher for FixtureFetcher {
    fn fetch(
        &self,
        url: &str,
        _: &avenger_image::Validators,
    ) -> Result<avenger_image::FetchResponse, avenger_image::error::AvengerImageError> {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.opened.wait(open).unwrap();
        }
        Ok(match tiles::osm_tile_bytes(url) {
            Some(body) => avenger_image::FetchResponse {
                status: 200,
                body,
                ..Default::default()
            },
            None => avenger_image::FetchResponse {
                status: 404,
                ..Default::default()
            },
        })
    }
}

/// Tiles fetched through an image cache draw end to end: the first frame shows the sphere's fill,
/// the frame after the tiles arrive draws them in place, without installing the scene again, as
/// the fixture tiles draw, and an identical frame after it uploads nothing.
#[test]
fn fetched_tiles_draw_as_they_arrive() {
    use std::sync::Arc;

    use avenger_image::{ImageCache, ImageCacheOptions};
    let case = tiles::cases()
        .into_iter()
        .find(|case| case.name == "globe")
        .unwrap();
    let fetcher = Arc::new(FixtureFetcher::default());
    let cache = Arc::new(ImageCache::with_options(ImageCacheOptions {
        fetcher: fetcher.clone(),
        ..Default::default()
    }));
    let scene = case.scene(&cache, tiles::OSM_WATER, true);
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 1.0,
        },
        CanvasConfig {
            images: Some(cache.clone()),
            ..Default::default()
        },
    ))
    .unwrap();
    canvas
        .set_scene(&scene, &avenger_typst_label::bundled_label_engine())
        .unwrap();
    let center = [(scene.width / 2.0) as u32, (scene.height / 2.0) as u32];
    let water = tiles::OSM_WATER.map(|channel| (channel * 255.0).round() as u8);

    let first = pollster::block_on(canvas.render()).unwrap();
    assert_eq!(first.get_pixel(center[0], center[1]).0, water);
    assert_eq!(canvas.image_status().pending.len(), case.tiles().len());

    *fetcher.open.lock().unwrap() = true;
    fetcher.opened.notify_all();
    let urls = scene.image_urls();
    let _held = cache
        .wait(&urls, std::time::Duration::from_secs(10))
        .unwrap();
    let arrived = pollster::block_on(canvas.render()).unwrap();
    // The tiles draw in place: the frame neither installs the scene nor builds its buffers.
    let stats = canvas.frame_stats();
    assert!(!stats.installed && !stats.prepared, "{stats:?}");
    assert!(canvas.image_status().is_complete());
    let osm = tiles::osm_tiles(&case.tiles());
    assert_eq!(
        arrived,
        render(&case.scene(&osm, tiles::OSM_WATER, true), &osm)
    );

    pollster::block_on(canvas.render()).unwrap();
    let stats = canvas.frame_stats();
    assert!(
        !stats.installed && stats.texture_upload_bytes == 0,
        "{stats:?}"
    );
}
