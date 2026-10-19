use avenger_render_fixtures::{parity, raster};
use avenger_svg::{SvgRenderOptions, SvgRenderer};

/// An exporter that draws the parity cases' unready resources as their placeholders.
fn renderer() -> SvgRenderer {
    SvgRenderer::new().with_options(SvgRenderOptions {
        allow_unready_images: true,
        ..Default::default()
    })
}

#[test]
fn svg_parity_scenes_render_expected_coverage() {
    let engine = avenger_typst_label::bundled_label_engine();
    for case in parity::cases()
        .into_iter()
        .filter(|case| case.resvg_reference)
    {
        let svg = renderer().render_scene_graph(&case.scene, &engine).unwrap();
        case.assert_image(&raster::svg_to_png(&svg, 2.0), None, 3);
    }
}
