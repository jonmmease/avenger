#![cfg(feature = "vendor-typst")]

use avenger_typst::{
    AvengerTypst, MathFragmentOptions, MathOutputRequest, MathStringRun, MathTypesetError,
    RasterRequest, TypstEngineBackend, TypstEngineConfig,
};
use typst_syntax::{parse_math, SyntaxKind};

#[test]
fn vendored_typst_syntax_parses_common_math_fragment() {
    let root = parse_math("sqrt(x^2 + y^2)");
    let (errors, warnings) = root.errors_and_warnings();

    assert_eq!(root.kind(), SyntaxKind::Math);
    assert!(errors.is_empty(), "unexpected parse errors: {errors:?}");
    assert!(
        warnings.is_empty(),
        "unexpected parse warnings: {warnings:?}"
    );
}

#[test]
fn vendored_typst_syntax_reports_invalid_math_fragment() {
    let root = parse_math("x^");
    assert!(root.diagnosis().errors);
}

#[test]
fn vendor_backend_returns_nonzero_metrics_for_common_fragments() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();

    for source in [
        "x^2 + y^2",
        "sqrt(x^2 + y^2)",
        "sum_(i=1)^n x_i",
        "mat(1, 2; 3, 4)",
    ] {
        let artifact = engine
            .typeset_math_fragment(source, &metrics_only_options())
            .unwrap();

        assert!(artifact.metrics.width > 0.0, "{source} has zero width");
        assert!(artifact.metrics.height > 0.0, "{source} has zero height");
        assert!(
            artifact.metrics.baseline > 0.0,
            "{source} has zero baseline"
        );
        assert!(artifact.paths.is_none());
        assert!(artifact.raster.is_none());
        assert!(artifact.pdf_text.is_none());
    }
}

fn metrics_only_options() -> MathFragmentOptions {
    let mut options = MathFragmentOptions::default();
    options.outputs.paths = false;
    options
}

#[test]
fn vendor_backend_lowers_common_fragments_to_paths() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();

    for source in [
        "x^2 + y^2",
        "sqrt(x^2 + y^2)",
        "sum_(i=1)^n x_i",
        "mat(1, 2; 3, 4)",
    ] {
        let artifact = engine
            .typeset_math_fragment(source, &Default::default())
            .unwrap();
        let paths = artifact
            .paths
            .as_ref()
            .unwrap_or_else(|| panic!("{source} did not produce paths"));

        assert_eq!(paths.logical_width, artifact.metrics.width);
        assert_eq!(paths.logical_height, artifact.metrics.height);
        assert!(!paths.items.is_empty(), "{source} produced no path items");
    }
}

#[test]
fn vendor_backend_returns_pdf_glyph_layer_and_font_bytes() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = MathFragmentOptions::default();
    options.outputs.paths = false;
    options.outputs.pdf_text_layer = true;

    let artifact = engine.typeset_math_fragment("x^2 + y^2", &options).unwrap();
    let pdf_text = artifact.pdf_text.as_ref().unwrap();

    assert_eq!(pdf_text.semantic_text, "x^2 + y^2");
    assert_eq!(pdf_text.logical_width, artifact.metrics.width);
    assert_eq!(pdf_text.logical_height, artifact.metrics.height);
    assert!(!pdf_text.glyph_runs.is_empty());
    assert!(pdf_text.glyph_runs.iter().any(|run| !run.glyphs.is_empty()));
    assert!(!artifact.font_resources.is_empty());
    assert!(artifact
        .font_resources
        .iter()
        .all(|resource| !resource.data.is_empty()));
}

#[test]
fn vendor_string_artifact_deduplicates_pdf_font_resources() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = avenger_typst::MathStringOptions::default();
    options.outputs = MathOutputRequest {
        paths: false,
        raster: None,
        pdf_text_layer: true,
    };

    let artifact = engine.typeset_math_string("$x$ + $y$", &options).unwrap();
    assert_eq!(artifact.font_resources.len(), 1);

    let math_runs = artifact
        .runs
        .iter()
        .filter_map(|run| match run {
            MathStringRun::Math(math_run) => Some(math_run),
            MathStringRun::Plain(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(math_runs.len(), 2);

    for math_run in math_runs {
        let run_font = math_run.artifact.pdf_text.as_ref().unwrap().glyph_runs[0].font;
        assert_eq!(run_font, artifact.font_resources[0].id);
        assert_eq!(
            math_run.artifact.font_resources[0].id,
            artifact.font_resources[0].id
        );
    }
}

#[test]
fn vendor_backend_reports_raster_unavailable_until_path_rasterization_exists() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = MathFragmentOptions::default();
    options.outputs.raster = Some(RasterRequest { scale: 2.0 });

    let err = engine.typeset_math_fragment("x", &options).unwrap_err();
    assert_eq!(
        err,
        MathTypesetError::UnsupportedOutput("vendor-typst raster output is not wired yet")
    );
}
