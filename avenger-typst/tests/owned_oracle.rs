#![cfg(feature = "vendor-typst")]

use avenger_typst::{
    AvengerTypst, MathFragmentOptions, MathOutputRequest, MathPathArtifact, MathPdfTextLayer,
    MathRunArtifact, PositionedTextLineRun, TextLineArtifact, TextLineOptions,
    TextLineOutputRequest, TypesetMetrics, TypstEngineBackend, TypstEngineConfig,
};

const TOLERANCE: f32 = 0.01;

fn engine(backend: TypstEngineBackend) -> AvengerTypst {
    AvengerTypst::new(TypstEngineConfig {
        backend,
        ..Default::default()
    })
    .unwrap()
}

fn vendor() -> AvengerTypst {
    engine(TypstEngineBackend::VendorTypst)
}

fn owned() -> AvengerTypst {
    engine(TypstEngineBackend::OwnedTypst)
}

#[test]
fn owned_backend_matches_vendor_for_fragment_metrics_and_artifacts() {
    let vendor = vendor();
    let owned = owned();
    let mut options = MathFragmentOptions::default();
    options.outputs = MathOutputRequest {
        paths: true,
        raster: None,
        pdf_text_layer: true,
    };

    for source in [
        "x^2 + y^2",
        "sqrt(x) / (1 + x^2)",
        "root(3, x)",
        "J_0(x)",
        "sum_(i=0)^n i",
        "lim_(x -> oo) f(x)",
        "alpha + beta -> gamma",
    ] {
        let vendor_artifact = vendor
            .typeset_math_fragment(source, &options)
            .unwrap_or_else(|err| panic!("vendor failed for {source:?}: {err:?}"));
        let owned_artifact = owned
            .typeset_math_fragment(source, &options)
            .unwrap_or_else(|err| panic!("owned failed for {source:?}: {err:?}"));

        assert_math_artifact_matches(source, &vendor_artifact, &owned_artifact);
    }
}

#[test]
fn owned_backend_matches_vendor_for_text_line_metrics_and_runs() {
    let vendor = vendor();
    let owned = owned();
    let mut options = TextLineOptions::default();
    options.outputs = TextLineOutputRequest {
        paths: true,
        raster: None,
        pdf_text_layer: true,
        positioned_runs: true,
    };

    for source in [
        "Hello",
        "Axis Tick Spacing",
        "Price \\$7, score $R^2$ = 0.94",
        "Order $J_0(x)$",
        "Bessel equation $x^2 y + x y + (x^2 - n^2)y = 0$",
        "Revenue 🚀",
        "Family 👨‍👩‍👧‍👦",
        "שלום world $x^2$",
        "السعر $R^2$ = 0.94",
        "温度 $T^2$",
    ] {
        let vendor_artifact = vendor
            .typeset_text_line(source, &options)
            .unwrap_or_else(|err| panic!("vendor failed for {source:?}: {err:?}"));
        let owned_artifact = owned
            .typeset_text_line(source, &options)
            .unwrap_or_else(|err| panic!("owned failed for {source:?}: {err:?}"));

        assert_text_line_matches(source, &vendor_artifact, &owned_artifact);
    }
}

#[test]
fn owned_backend_matches_vendor_for_errors() {
    let vendor = vendor();
    let owned = owned();

    for source in ["#let x = 1", "#box(x)", "x^"] {
        let vendor_err = vendor
            .typeset_math_fragment(source, &MathFragmentOptions::default())
            .unwrap_err();
        let owned_err = owned
            .typeset_math_fragment(source, &MathFragmentOptions::default())
            .unwrap_err();

        assert_eq!(owned_err, vendor_err, "{source:?}");
    }

    let source = "before $x^$ after";
    let vendor_err = vendor
        .typeset_text_line(source, &TextLineOptions::default())
        .unwrap_err();
    let owned_err = owned
        .typeset_text_line(source, &TextLineOptions::default())
        .unwrap_err();
    assert_eq!(owned_err, vendor_err, "{source:?}");
}

fn assert_text_line_matches(source: &str, vendor: &TextLineArtifact, owned: &TextLineArtifact) {
    assert_eq!(owned.source, vendor.source, "{source:?}");
    assert_metrics_close(source, "line metrics", vendor.metrics, owned.metrics);
    assert_path_artifact_matches(source, vendor.paths.as_ref(), owned.paths.as_ref());
    assert_pdf_layer_matches(source, vendor.pdf_text.as_ref(), owned.pdf_text.as_ref());
    assert_eq!(
        owned.font_resources.len(),
        vendor.font_resources.len(),
        "{source:?} font resource count"
    );
    assert_eq!(
        owned.positioned_runs.len(),
        vendor.positioned_runs.len(),
        "{source:?} positioned run count"
    );

    for (index, (vendor_run, owned_run)) in vendor
        .positioned_runs
        .iter()
        .zip(&owned.positioned_runs)
        .enumerate()
    {
        assert_positioned_run_matches(source, index, vendor_run, owned_run);
    }
}

fn assert_math_artifact_matches(source: &str, vendor: &MathRunArtifact, owned: &MathRunArtifact) {
    assert_metrics_close(source, "fragment metrics", vendor.metrics, owned.metrics);
    assert_path_artifact_matches(source, vendor.paths.as_ref(), owned.paths.as_ref());
    assert_pdf_layer_matches(source, vendor.pdf_text.as_ref(), owned.pdf_text.as_ref());
    assert_eq!(
        owned.font_resources.len(),
        vendor.font_resources.len(),
        "{source:?} font resource count"
    );
}

fn assert_positioned_run_matches(
    source: &str,
    index: usize,
    vendor: &PositionedTextLineRun,
    owned: &PositionedTextLineRun,
) {
    assert_eq!(owned.kind, vendor.kind, "{source:?} run {index} kind");
    assert_eq!(owned.text, vendor.text, "{source:?} run {index} text");
    assert_eq!(
        owned.byte_range, vendor.byte_range,
        "{source:?} run {index} byte range"
    );
    assert_close(source, &format!("run {index} x"), vendor.x, owned.x);
    assert_close(source, &format!("run {index} y"), vendor.y, owned.y);
    assert_metrics_close(
        source,
        &format!("run {index} metrics"),
        vendor.metrics,
        owned.metrics,
    );
    assert_path_artifact_matches(source, vendor.paths.as_ref(), owned.paths.as_ref());
    assert_pdf_layer_matches(source, vendor.pdf_text.as_ref(), owned.pdf_text.as_ref());
    assert_eq!(
        owned.font_resources.len(),
        vendor.font_resources.len(),
        "{source:?} run {index} font resource count"
    );
}

fn assert_metrics_close(source: &str, label: &str, vendor: TypesetMetrics, owned: TypesetMetrics) {
    assert_close(source, &format!("{label} width"), vendor.width, owned.width);
    assert_close(
        source,
        &format!("{label} height"),
        vendor.height,
        owned.height,
    );
    assert_close(
        source,
        &format!("{label} baseline"),
        vendor.baseline,
        owned.baseline,
    );
    assert_close(
        source,
        &format!("{label} ascent"),
        vendor.ascent,
        owned.ascent,
    );
    assert_close(
        source,
        &format!("{label} descent"),
        vendor.descent,
        owned.descent,
    );
}

fn assert_path_artifact_matches(
    source: &str,
    vendor: Option<&MathPathArtifact>,
    owned: Option<&MathPathArtifact>,
) {
    assert_eq!(
        owned.is_some(),
        vendor.is_some(),
        "{source:?} path presence"
    );
    let (Some(vendor), Some(owned)) = (vendor, owned) else {
        return;
    };

    assert_close(
        source,
        "path logical width",
        vendor.logical_width,
        owned.logical_width,
    );
    assert_close(
        source,
        "path logical height",
        vendor.logical_height,
        owned.logical_height,
    );
    assert_eq!(
        owned.items.len(),
        vendor.items.len(),
        "{source:?} path items"
    );
    for (index, (vendor_item, owned_item)) in vendor.items.iter().zip(&owned.items).enumerate() {
        assert_eq!(
            owned_item.path.commands.len(),
            vendor_item.path.commands.len(),
            "{source:?} path item {index} command count"
        );
        assert_eq!(
            owned_item.fill.is_some(),
            vendor_item.fill.is_some(),
            "{source:?} path item {index} fill presence"
        );
        assert_eq!(
            owned_item.stroke.is_some(),
            vendor_item.stroke.is_some(),
            "{source:?} path item {index} stroke presence"
        );
    }
}

fn assert_pdf_layer_matches(
    source: &str,
    vendor: Option<&MathPdfTextLayer>,
    owned: Option<&MathPdfTextLayer>,
) {
    assert_eq!(
        owned.is_some(),
        vendor.is_some(),
        "{source:?} PDF layer presence"
    );
    let (Some(vendor), Some(owned)) = (vendor, owned) else {
        return;
    };

    assert_eq!(
        owned.semantic_text, vendor.semantic_text,
        "{source:?} semantic text"
    );
    assert_close(
        source,
        "PDF logical width",
        vendor.logical_width,
        owned.logical_width,
    );
    assert_close(
        source,
        "PDF logical height",
        vendor.logical_height,
        owned.logical_height,
    );
    assert_eq!(
        owned.glyph_runs.len(),
        vendor.glyph_runs.len(),
        "{source:?} PDF glyph run count"
    );
    for (index, (vendor_run, owned_run)) in
        vendor.glyph_runs.iter().zip(&owned.glyph_runs).enumerate()
    {
        assert_close(
            source,
            &format!("PDF run {index} font size"),
            vendor_run.font_size,
            owned_run.font_size,
        );
        assert_eq!(
            owned_run.glyphs.len(),
            vendor_run.glyphs.len(),
            "{source:?} PDF run {index} glyph count"
        );
    }
}

fn assert_close(source: &str, label: &str, expected: f32, actual: f32) {
    let difference = (expected - actual).abs();
    assert!(
        difference <= TOLERANCE,
        "{source:?} {label}: expected {expected}, got {actual}, diff {difference}"
    );
}
