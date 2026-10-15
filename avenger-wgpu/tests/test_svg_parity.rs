use avenger_render_fixtures::{parity, raster};

use avenger_common::canvas::CanvasDimensions;
use avenger_svg::SvgRenderer;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

/// Draws each case with WGPU, and compares it with the samples and with resvg's image of its SVG.
/// The SVGs and WGPU images are written to the ignored `tests/output/parity`, to compare in a
/// browser where resvg isn't a reference.
#[test]
fn svg_parity_scenes_render_expected_coverage() {
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/output/parity");
    std::fs::create_dir_all(&output).unwrap();
    let engine = avenger_typst_label::bundled_label_engine();
    let new_canvas = |size| {
        pollster::block_on(PngCanvas::new(
            CanvasDimensions { size, scale: 2.0 },
            CanvasConfig::default(),
        ))
        .unwrap()
    };
    // Cases of one size share a canvas, so they also exercise its caches.
    let mut canvas = new_canvas([420.0, 240.0]);
    for case in parity::cases() {
        let size = [case.scene.width, case.scene.height];
        if canvas.dimensions().size != size {
            canvas = new_canvas(size);
        }
        canvas.set_scene(&case.scene, &engine).unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();
        let svg = SvgRenderer::new()
            .render_scene_graph(&case.scene, &engine)
            .unwrap();
        std::fs::write(output.join(format!("{}.svg", case.name)), &svg).unwrap();
        image
            .save(output.join(format!("{}-wgpu.png", case.name)))
            .unwrap();
        let reference = case.resvg_reference.then(|| raster::svg_to_png(&svg, 2.0));
        case.assert_image(&image, reference.as_ref(), 3);
    }
}
