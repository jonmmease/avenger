#[path = "support/pdf_raster.rs"]
mod pdf_raster;

use avenger_render_fixtures::{parity, raster};

#[test]
fn exports_every_parity_case() {
    let engine = avenger_typst_label::bundled_label_engine();
    for case in parity::cases() {
        let pdf = avenger_pdf::PdfRenderer::new()
            .render_scene_graph(&case.scene, &engine)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF-"), "{}", case.name);
    }
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn shared_scene_rules_survive_pdf_export() {
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/output/parity");
    std::fs::create_dir_all(&output).unwrap();
    let engine = avenger_typst_label::bundled_label_engine();
    for case in parity::cases() {
        let pdf = avenger_pdf::PdfRenderer::new()
            .render_scene_graph(&case.scene, &engine)
            .unwrap();
        let image = pdf_raster::pdf_to_png(&pdf, case.scene.width, case.scene.height);
        image
            .save(output.join(format!("{}-pdf.png", case.name)))
            .unwrap();
        let reference = case.resvg_reference.then(|| {
            let svg = avenger_svg::SvgRenderer::new()
                .render_scene_graph(&case.scene, &engine)
                .unwrap();
            raster::svg_to_png(&svg, 2.0)
        });
        // Native radial shading colors vary slightly between PDF viewers.
        let tolerance = if case.resvg_reference { 3 } else { 5 };
        case.assert_image(&image, reference.as_ref(), tolerance);
    }
}
