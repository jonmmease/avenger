use avenger_render_fixtures::{parity, raster};

#[test]
fn svg_parity_scenes_render_expected_coverage() {
    let engine = avenger_typst_label::bundled_label_engine();
    for case in parity::cases()
        .into_iter()
        .filter(|case| case.resvg_reference)
    {
        let svg = avenger_svg::SvgRenderer::new()
            .render_scene_graph(&case.scene, &engine)
            .unwrap();
        case.assert_image(&raster::svg_to_png(&svg, 2.0), None, 3);
    }
}
