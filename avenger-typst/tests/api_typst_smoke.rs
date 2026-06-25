#![cfg(feature = "vendor-typst")]

use avenger_typst::{AvengerTypst, MathFragmentOptions, TypstEngineBackend, TypstEngineConfig};
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
