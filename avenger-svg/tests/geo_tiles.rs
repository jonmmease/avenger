//! Map tiles on several projections, exported to SVG: seam checks with synthetic tiles, and
//! baselines with OpenStreetMap tiles.

use avenger_render_fixtures::{raster, scene, tiles};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};

/// The page outside the map, and the sphere's fill where no tile draws.
const BACKGROUND: [f32; 4] = [1.0, 0.0, 1.0, 1.0];

fn render(scene: &SceneGraph) -> image::RgbaImage {
    let engine = LabelEngine::new(EngineOptions {
        fonts: scene::fonts(),
    });
    let svg = SvgRenderer::new()
        .render_scene_graph(scene, &engine)
        .unwrap();
    raster::svg_to_png(&svg, 1.0)
}

/// Flat-colored tiles export one color everywhere inside them: the instances' single image has
/// no crack, line, or gap along the map's edge that shows the sphere's fill.
#[test]
fn flat_tiles_show_no_seams() {
    for case in tiles::seam_cases() {
        let color = [40, 120, 200, 255];
        let tiles = tiles::flat_tiles(&case.tiles(), 16, color);
        let image = render(&case.scene(&tiles, BACKGROUND, false));
        tiles::assert_flat(&case, &image, color, 2);
    }
}

/// Tiles of a gradient export the gradient as each tile samples it, repeating its edge texels
/// at its border.
#[test]
fn gradient_tiles_follow_their_textures() {
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
        let name = format!("geo-tiles-{}", case.name);
        raster::assert_baseline(env!("CARGO_MANIFEST_DIR"), &image, &name);
    }
}
