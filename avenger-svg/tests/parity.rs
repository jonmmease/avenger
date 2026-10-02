#[path = "../../tests/render_fixtures/parity.rs"]
mod parity;
#[path = "../../tests/render_fixtures/raster.rs"]
#[allow(dead_code)]
mod raster;

#[test]
fn svg_parity_scenes_render_expected_coverage() {
    for case in parity::cases()
        .into_iter()
        .filter(|case| !case.browser_only)
    {
        let svg = avenger_svg::SvgRenderer::new()
            .render_scene_graph(&case.scene)
            .unwrap();
        let image = raster::svg_to_png(&svg, 2.0);
        for (point, expected) in case.samples {
            let actual = image.get_pixel(point[0] * 2, point[1] * 2).0;
            assert!(
                actual
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(b) <= 3),
                "{} {point:?}: {actual:?} != {expected:?}",
                case.name
            );
        }
    }
}
