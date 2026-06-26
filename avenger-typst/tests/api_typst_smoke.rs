#![cfg(feature = "vendor-typst")]

use avenger_typst::{
    AvengerTypst, MathFragmentOptions, MathOutputRequest, MathStringRun, PositionedTextLineRunKind,
    RasterRequest, TextLineOutputRequest, TypstEngineBackend, TypstEngineConfig,
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

    for source in ["x^2 + y^2", "sqrt(x^2 + y^2)", "sum_(i=1)^n x_i"] {
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

    for source in ["x^2 + y^2", "sqrt(x^2 + y^2)", "sum_(i=1)^n x_i"] {
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
fn vendor_backend_layouts_mixed_text_line_with_one_baseline() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = avenger_typst::TextLineOptions::default();
    options.outputs.paths = false;

    let artifact = engine
        .typeset_text_line("Price \\$7, score $R^2$ = 0.94", &options)
        .unwrap();

    assert!(artifact.metrics.width > 0.0);
    assert!(artifact.metrics.height > 0.0);
    assert!(artifact.metrics.ascent > 0.0);
    assert!(artifact.metrics.descent >= 0.0);
    assert!(artifact.paths.is_none());
    assert!(artifact.raster.is_none());
    assert!(artifact.pdf_text.is_none());
}

#[test]
fn vendor_backend_lowers_mixed_text_line_to_paths() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();

    let artifact = engine
        .typeset_text_line("Order $J_0(x)$", &Default::default())
        .unwrap();
    let paths = artifact.paths.as_ref().unwrap();

    assert_eq!(paths.logical_width, artifact.metrics.width);
    assert_eq!(paths.logical_height, artifact.metrics.height);
    assert!(!paths.items.is_empty());
}

#[test]
fn vendor_backend_returns_positioned_runs_for_mixed_text_line() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = avenger_typst::TextLineOptions::default();
    options.outputs = TextLineOutputRequest {
        paths: false,
        raster: None,
        pdf_text_layer: true,
        positioned_runs: true,
    };

    let artifact = engine
        .typeset_text_line("speed $v^2$ now", &options)
        .unwrap();

    assert_eq!(artifact.positioned_runs.len(), 3);
    assert_eq!(
        artifact.positioned_runs[0].kind,
        PositionedTextLineRunKind::Plain
    );
    assert_eq!(artifact.positioned_runs[0].text, "speed ");
    assert_eq!(
        artifact.positioned_runs[1].kind,
        PositionedTextLineRunKind::Math
    );
    assert!(artifact.positioned_runs[1]
        .paths
        .as_ref()
        .is_some_and(|paths| !paths.items.is_empty()));
    assert!(artifact.positioned_runs[1]
        .pdf_text
        .as_ref()
        .is_some_and(|layer| !layer.glyph_runs.is_empty()));
    assert!(!artifact.positioned_runs[1].font_resources.is_empty());
    assert_eq!(
        artifact.positioned_runs[2].kind,
        PositionedTextLineRunKind::Plain
    );
    assert_eq!(artifact.positioned_runs[2].text, " now");
    assert!(artifact.positioned_runs[0].x < artifact.positioned_runs[1].x);
    assert!(artifact.positioned_runs[1].x < artifact.positioned_runs[2].x);
    assert!(artifact.metrics.width > artifact.positioned_runs[2].x);
}

#[test]
fn vendor_backend_returns_pdf_glyph_layer_for_text_line() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = avenger_typst::TextLineOptions::default();
    options.outputs = TextLineOutputRequest {
        paths: false,
        raster: None,
        pdf_text_layer: true,
        positioned_runs: false,
    };

    let artifact = engine.typeset_text_line("score $R^2$", &options).unwrap();
    let pdf_text = artifact.pdf_text.as_ref().unwrap();

    assert_eq!(pdf_text.semantic_text, "score $R^2$");
    assert_eq!(pdf_text.logical_width, artifact.metrics.width);
    assert_eq!(pdf_text.logical_height, artifact.metrics.height);
    assert!(!pdf_text.glyph_runs.is_empty());
    assert!(!artifact.font_resources.is_empty());
}

#[test]
fn vendor_backend_keeps_text_line_math_digits_out_of_text_font() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = avenger_typst::TextLineOptions::default();
    options.outputs = TextLineOutputRequest {
        paths: false,
        raster: None,
        pdf_text_layer: true,
        positioned_runs: false,
    };

    let artifact = engine
        .typeset_text_line("Bessel equation $x^2 y + x y + (x^2 - n^2)y = 0$", &options)
        .unwrap();
    let atkinson = artifact
        .font_resources
        .iter()
        .find(|resource| resource.family == "Atkinson Hyperlegible Next")
        .expect("bundled text font should be used for the plain prefix")
        .id;
    let pdf_text = artifact.pdf_text.as_ref().unwrap();
    let mut saw_math_zero = false;

    for run in &pdf_text.glyph_runs {
        let text = run
            .glyphs
            .iter()
            .map(|glyph| glyph.unicode.as_str())
            .collect::<String>();

        if text == "Bessel equation " {
            assert_eq!(run.font, atkinson);
        } else {
            assert_ne!(run.font, atkinson, "math run {text:?} used text font");
            saw_math_zero |= text == "0";
        }
    }

    assert!(
        saw_math_zero,
        "test expression did not expose the math zero"
    );
}

#[test]
#[cfg(not(feature = "raster"))]
fn vendor_backend_reports_raster_requires_feature_without_raster_feature() {
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
        avenger_typst::MathTypesetError::UnsupportedOutput(
            "avenger-typst raster output requires the raster feature"
        )
    );
}

#[test]
#[cfg(feature = "raster")]
fn vendor_backend_rasterizes_common_fragment_from_paths() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = MathFragmentOptions::default();
    options.outputs.paths = false;
    options.outputs.raster = Some(RasterRequest { scale: 2.0 });

    let artifact = engine
        .typeset_math_fragment("sqrt(x^2 + y^2)", &options)
        .unwrap();
    let raster = artifact.raster.as_ref().unwrap();

    assert!(artifact.paths.is_none());
    assert_eq!(raster.scale, 2.0);
    assert_eq!(raster.logical_width, artifact.metrics.width);
    assert_eq!(raster.logical_height, artifact.metrics.height);
    assert!(raster.image.width > 1);
    assert!(raster.image.height > 1);
    assert_eq!(
        raster.image.data.len(),
        raster.image.width as usize * raster.image.height as usize * 4
    );
    assert!(raster.origin_x.is_finite());
    assert!(raster.origin_y.is_finite());
    assert!(raster.origin_x < raster.logical_width);
    assert!(raster.origin_y < raster.logical_height);
    assert!(
        raster.image.data.chunks_exact(4).any(|pixel| pixel[3] > 0),
        "raster should contain non-transparent pixels"
    );
    assert_eq!(
        &raster.image.data[0..4],
        &[0, 0, 0, 0],
        "antialias padding should leave the first pixel transparent"
    );
}

#[test]
#[cfg(feature = "raster")]
fn vendor_backend_rasterizes_math_string_runs() {
    let engine = AvengerTypst::new(TypstEngineConfig {
        backend: TypstEngineBackend::VendorTypst,
        ..Default::default()
    })
    .unwrap();
    let mut options = avenger_typst::MathStringOptions::default();
    options.outputs.paths = false;
    options.outputs.raster = Some(RasterRequest { scale: 1.5 });

    let artifact = engine
        .typeset_math_string("$x^2$ + $y^2$", &options)
        .unwrap();
    let rasters = artifact
        .runs
        .iter()
        .filter_map(|run| match run {
            MathStringRun::Math(math_run) => math_run.artifact.raster.as_ref(),
            MathStringRun::Plain(_) => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(rasters.len(), 2);
    assert!(rasters.iter().all(|raster| raster.scale == 1.5));
    assert!(rasters.iter().all(|raster| raster.image.width > 1));
    assert!(rasters.iter().all(|raster| raster.image.height > 1));
}
