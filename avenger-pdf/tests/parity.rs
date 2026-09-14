#[path = "../../tests/render_fixtures/parity.rs"]
mod parity;
#[path = "support/pdf_raster.rs"]
mod pdf_raster;
#[path = "../../tests/render_fixtures/raster.rs"]
#[allow(dead_code)]
mod raster;

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn shared_scene_rules_survive_pdf_export() {
    let renderer = avenger_pdf::PdfRenderer::new();
    let output = std::env::var_os("AVENGER_PARITY_OUTPUT").map(std::path::PathBuf::from);
    // PDF viewers control image interpolation; this check covers shared geometry and paints.
    for case in parity::cases()
        .into_iter()
        .filter(|case| !case.name.starts_with("image-"))
    {
        let pdf = renderer.render_scene_graph(&case.scene).unwrap();
        let image = pdf_raster::pdf_to_png(&pdf, case.scene.width, case.scene.height);
        if let Some(output) = &output {
            std::fs::create_dir_all(output).unwrap();
            std::fs::write(output.join(format!("{}.pdf", case.name)), &pdf).unwrap();
            image
                .save(output.join(format!("{}-pdf.png", case.name)))
                .unwrap();
        }
        if !case.browser_only {
            let svg = avenger_svg::SvgRenderer::new()
                .render_scene_graph(&case.scene)
                .unwrap();
            let reference = raster::svg_to_png(&svg, 2.0);
            let large = image
                .pixels()
                .zip(reference.pixels())
                .filter(|(a, b)| a.0.into_iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > 20))
                .count();
            assert!(
                large < 840 * 480 / 100,
                "{}: {large} pixels differ beyond edge tolerance",
                case.name
            );
        }
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
