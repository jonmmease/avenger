//! Map tiles on several projections, drawn by the GPU: seam checks with synthetic tiles, and
//! baselines with OpenStreetMap tiles.

use avenger_common::canvas::CanvasDimensions;
use avenger_render_fixtures::tiles;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use dssim::Dssim;
use std::path::Path;

/// The page outside the map, and the sphere's fill where no tile draws.
const BACKGROUND: [f32; 4] = [1.0, 0.0, 1.0, 1.0];

fn render(scene: &SceneGraph) -> image::RgbaImage {
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 1.0,
        },
        CanvasConfig::default(),
    ))
    .unwrap();
    canvas
        .set_scene(scene, &avenger_typst_label::bundled_label_engine())
        .unwrap();
    pollster::block_on(canvas.render()).unwrap()
}

/// Asserts that `image` matches `tests/baselines/{name}.png`, and writes it to the ignored
/// `tests/output/{name}.png`, so that an intended change is accepted by copying it.
fn assert_baseline(image: &image::RgbaImage, name: &str) {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let output = tests.join(format!("output/{name}.png"));
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    image.save(&output).unwrap();
    let baseline = tests.join(format!("baselines/{name}.png"));
    let attr = Dssim::new();
    let expected = dssim::load_image(&attr, &baseline).expect("the baseline exists");
    let actual = dssim::load_image(&attr, &output).unwrap();
    let (diff, _) = attr.compare(&expected, actual);
    assert!(
        diff < 0.0001,
        "DSSIM {diff}; to accept {}, copy it to {}",
        output.display(),
        baseline.display()
    );
}

/// Flat-colored tiles draw one color everywhere inside them: no crack, antialiased line, or
/// gap along the map's edge shows the sphere's fill through them.
#[test]
fn flat_tiles_show_no_seams() {
    for case in tiles::seam_cases() {
        let color = [40, 120, 200, 255];
        let tiles = tiles::flat_tiles(&case.tiles(), 16, color);
        let image = render(&case.scene(&tiles, BACKGROUND, false));
        tiles::assert_flat(&case, &image, color, 2);
    }
}

/// Tiles of a gradient draw the gradient without a step at their borders.
#[test]
fn gradient_tiles_show_no_steps() {
    for case in tiles::seam_cases() {
        let tiles = tiles::gradient_tiles(&case.tiles(), 16);
        let image = render(&case.scene(&tiles, BACKGROUND, false));
        tiles::assert_smooth(&case, &image, 16, 3.0, 2);
    }
}

/// OpenStreetMap tiles on each view, under the sphere's outline.
#[test]
fn osm_tiles_match_baselines() {
    for case in tiles::cases() {
        let osm = tiles::osm_tiles(&case.tiles());
        let image = render(&case.scene(&osm, tiles::OSM_WATER, true));
        assert_baseline(&image, &format!("geo-tiles-{}", case.name));
    }
}
