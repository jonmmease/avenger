#![cfg(feature = "vendor-typst")]

use avenger_typst::{
    AvengerTypst, MathFontResource, MathFragmentOptions, MathOutputRequest, MathPathArtifact,
    MathPathCommand, MathPdfGlyphRun, MathPdfTextLayer, MathRunArtifact, PositionedTextLineRun,
    TextLineArtifact, TextLineOptions, TextLineOutputRequest, TypesetMetrics, TypstEngineBackend,
    TypstEngineConfig,
};
use std::fmt::Write;

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
        "1",
        "x",
        "y",
        "t",
        "x^2 + y^2",
        "x_i^2",
        "x'",
        "x''",
        "x_1^2",
        "sqrt(x) / (1 + x^2)",
        "root(3, x)",
        "frac(x + y, z)",
        "a / b",
        "a / (b + c)",
        "J_0(x)",
        "J_n(x)",
        "sum_(i=0)^n i",
        "lim_(x -> oo) f(x)",
        "sin(x)",
        "op(\"custom\")",
        "abs(x)",
        "norm(v)",
        "floor(x)",
        "ceil(x)",
        "round(x)",
        "alpha + beta -> gamma",
        "alpha + pi + sum",
        "x(t)",
        "x(t) = A r^t",
        "R^2 = 0.94",
        "y = sqrt(x) / (1 + x^2)",
    ] {
        let vendor_artifact = vendor
            .typeset_math_fragment(source, &options)
            .unwrap_or_else(|err| panic!("vendor failed for {source:?}: {err:?}"));
        let owned_artifact = owned
            .typeset_math_fragment(source, &options)
            .unwrap_or_else(|err| panic!("owned failed for {source:?}: {err:?}"));

        maybe_write_math_snapshot(source, &vendor_artifact, &owned_artifact);
        assert_math_artifact_matches(source, &vendor_artifact, &owned_artifact);
    }
}

#[test]
fn owned_backend_matches_vendor_for_single_atom_fragment_metrics_only_fast_path() {
    let vendor = vendor();
    let owned = owned();
    let mut options = MathFragmentOptions::default();
    options.outputs = MathOutputRequest {
        paths: false,
        raster: None,
        pdf_text_layer: false,
    };

    for source in ["1", "0.94", "x", "R", "alpha", "pi", "sum", "+", "->"] {
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
fn owned_backend_matches_vendor_for_simple_row_fragment_metrics_only_fast_path() {
    let vendor = vendor();
    let owned = owned();
    let mut options = MathFragmentOptions::default();
    options.outputs = MathOutputRequest {
        paths: false,
        raster: None,
        pdf_text_layer: false,
    };

    for source in [
        "x + y",
        "alpha + beta -> gamma",
        "alpha + pi + sum",
        "x <= y => y >= x",
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
fn owned_backend_matches_vendor_for_simple_row_fragment_paths_fast_path() {
    let vendor = vendor();
    let owned = owned();
    let mut options = MathFragmentOptions::default();
    options.outputs = MathOutputRequest {
        paths: true,
        raster: None,
        pdf_text_layer: false,
    };

    for source in [
        "1",
        "x",
        "0.94",
        "alpha + beta -> gamma",
        "alpha + pi + sum",
        "x <= y => y >= x",
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

#[cfg(feature = "raster")]
#[test]
fn owned_backend_matches_vendor_for_simple_row_fragment_raster_fast_path() {
    let vendor = vendor();
    let owned = owned();
    let mut options = MathFragmentOptions::default();
    options.outputs = MathOutputRequest {
        paths: false,
        raster: Some(avenger_typst::RasterRequest { scale: 2.0 }),
        pdf_text_layer: false,
    };

    for source in ["x", "alpha + beta -> gamma", "alpha + pi + sum"] {
        let vendor_artifact = vendor
            .typeset_math_fragment(source, &options)
            .unwrap_or_else(|err| panic!("vendor failed for {source:?}: {err:?}"));
        let owned_artifact = owned
            .typeset_math_fragment(source, &options)
            .unwrap_or_else(|err| panic!("owned failed for {source:?}: {err:?}"));

        assert_metrics_close(
            source,
            "raster fragment metrics",
            vendor_artifact.metrics,
            owned_artifact.metrics,
        );
        let vendor_raster = vendor_artifact
            .raster
            .as_ref()
            .unwrap_or_else(|| panic!("vendor raster missing for {source:?}"));
        let owned_raster = owned_artifact
            .raster
            .as_ref()
            .unwrap_or_else(|| panic!("owned raster missing for {source:?}"));

        assert_eq!(
            owned_raster.image.width, vendor_raster.image.width,
            "{source:?} raster width"
        );
        assert_eq!(
            owned_raster.image.height, vendor_raster.image.height,
            "{source:?} raster height"
        );
        assert_close(
            source,
            "raster origin x",
            vendor_raster.origin_x,
            owned_raster.origin_x,
        );
        assert_close(
            source,
            "raster origin y",
            vendor_raster.origin_y,
            owned_raster.origin_y,
        );
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
        "",
        "Hello",
        "Axis Tick Spacing",
        "Price \\$7, score $R^2$ = 0.94",
        "Price \\$7, score $R^2 = 0.94$",
        "Order $J_0(x)$",
        "Bessel functions $J_n(x)$",
        "Bessel equation $x^2 y + x y + (x^2 - n^2)y = 0$",
        "Damped oscillator $x(t) = A r^t$",
        "Frequency $omega$",
        "Root fraction $y = sqrt(x) / (1 + x^2)$",
        "Cost is \\$5, score is $R^2$",
        "peak $x_i^2$",
        "Math z-order $x_i^2$",
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

        maybe_write_text_line_snapshot(source, &vendor_artifact, &owned_artifact);
        assert_text_line_matches(source, &vendor_artifact, &owned_artifact);
    }
}

#[test]
fn owned_backend_matches_vendor_for_plain_text_metrics_only_fast_path() {
    let vendor = vendor();
    let owned = owned();
    let mut options = TextLineOptions::default();
    options.outputs = TextLineOutputRequest {
        paths: false,
        raster: None,
        pdf_text_layer: false,
        positioned_runs: true,
    };

    for source in ["Hello", "Axis Tick Spacing", "Using count() aggregation"] {
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

#[test]
fn owned_backend_rejects_matrix_math_as_unsupported_subset() {
    let vendor = vendor();
    let owned = owned();
    let source = "mat(1, 2; 3, 4)";

    assert!(vendor
        .typeset_math_fragment(source, &MathFragmentOptions::default())
        .is_ok());

    let owned_err = owned
        .typeset_math_fragment(source, &MathFragmentOptions::default())
        .unwrap_err();
    assert_eq!(
        owned_err,
        avenger_typst::MathTypesetError::UnsupportedSyntax {
            position: 0,
            message: "matrix/table math is not supported in owned Typst subset"
        }
    );
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

fn maybe_write_math_snapshot(source: &str, vendor: &MathRunArtifact, owned: &MathRunArtifact) {
    if !oracle_snapshots_enabled() {
        return;
    }

    let mut output = String::new();
    writeln!(&mut output, "source: {source:?}").unwrap();
    append_math_artifact_snapshot(&mut output, "vendor", vendor);
    append_math_artifact_snapshot(&mut output, "owned", owned);
    write_snapshot_file("fragment", source, output);
}

fn maybe_write_text_line_snapshot(
    source: &str,
    vendor: &TextLineArtifact,
    owned: &TextLineArtifact,
) {
    if !oracle_snapshots_enabled() {
        return;
    }

    let mut output = String::new();
    writeln!(&mut output, "source: {source:?}").unwrap();
    append_text_line_snapshot(&mut output, "vendor", vendor);
    append_text_line_snapshot(&mut output, "owned", owned);
    write_snapshot_file("text-line", source, output);
}

fn oracle_snapshots_enabled() -> bool {
    std::env::var_os("AVENGER_TYPST_ORACLE_SNAPSHOTS").is_some()
}

fn write_snapshot_file(kind: &str, source: &str, output: String) {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("target")
        });
    let dir = target_dir.join("avenger-typst-owned-oracle").join(kind);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.txt", snapshot_name(source)));
    std::fs::write(path, output).unwrap();
}

fn snapshot_name(source: &str) -> String {
    let mut name = String::new();
    for ch in source.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            name.push(ch);
        } else if !name.ends_with('_') {
            name.push('_');
        }
    }
    let name = name.trim_matches('_').chars().take(80).collect::<String>();
    if name.is_empty() {
        "empty".to_string()
    } else {
        name
    }
}

fn append_text_line_snapshot(output: &mut String, label: &str, artifact: &TextLineArtifact) {
    writeln!(output, "\n[{label}]").unwrap();
    append_metrics_snapshot(output, "metrics", artifact.metrics);
    append_path_snapshot(output, "paths", artifact.paths.as_ref());
    append_pdf_snapshot(output, "pdf_text", artifact.pdf_text.as_ref());
    append_font_resources_snapshot(output, "font_resources", &artifact.font_resources);
    writeln!(
        output,
        "positioned_runs: {}",
        artifact.positioned_runs.len()
    )
    .unwrap();
    for (index, run) in artifact.positioned_runs.iter().enumerate() {
        writeln!(
            output,
            "  run {index}: kind={:?} text={:?} range={:?} x={:.4} y={:.4}",
            run.kind, run.text, run.byte_range, run.x, run.y
        )
        .unwrap();
        append_metrics_snapshot(output, "    metrics", run.metrics);
        append_path_snapshot(output, "    paths", run.paths.as_ref());
        append_pdf_snapshot(output, "    pdf_text", run.pdf_text.as_ref());
        append_font_resources_snapshot(output, "    font_resources", &run.font_resources);
    }
}

fn append_math_artifact_snapshot(output: &mut String, label: &str, artifact: &MathRunArtifact) {
    writeln!(output, "\n[{label}]").unwrap();
    append_metrics_snapshot(output, "metrics", artifact.metrics);
    append_path_snapshot(output, "paths", artifact.paths.as_ref());
    append_pdf_snapshot(output, "pdf_text", artifact.pdf_text.as_ref());
    append_font_resources_snapshot(output, "font_resources", &artifact.font_resources);
}

fn append_metrics_snapshot(output: &mut String, label: &str, metrics: TypesetMetrics) {
    writeln!(
        output,
        "{label}: width={:.4} height={:.4} baseline={:.4} ascent={:.4} descent={:.4}",
        metrics.width, metrics.height, metrics.baseline, metrics.ascent, metrics.descent
    )
    .unwrap();
}

fn append_path_snapshot(output: &mut String, label: &str, paths: Option<&MathPathArtifact>) {
    let Some(paths) = paths else {
        writeln!(output, "{label}: none").unwrap();
        return;
    };

    let command_count = paths
        .items
        .iter()
        .map(|item| item.path.commands.len())
        .sum::<usize>();
    writeln!(
        output,
        "{label}: logical_width={:.4} logical_height={:.4} items={} commands={}",
        paths.logical_width,
        paths.logical_height,
        paths.items.len(),
        command_count
    )
    .unwrap();

    if let Some(bounds) = path_bounds(paths) {
        writeln!(
            output,
            "{label}_bounds: min_x={:.4} min_y={:.4} max_x={:.4} max_y={:.4}",
            bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y
        )
        .unwrap();
    }
}

fn append_pdf_snapshot(output: &mut String, label: &str, pdf: Option<&MathPdfTextLayer>) {
    let Some(pdf) = pdf else {
        writeln!(output, "{label}: none").unwrap();
        return;
    };

    writeln!(
        output,
        "{label}: semantic={:?} logical_width={:.4} logical_height={:.4} glyph_runs={}",
        pdf.semantic_text,
        pdf.logical_width,
        pdf.logical_height,
        pdf.glyph_runs.len()
    )
    .unwrap();
    for (index, run) in pdf.glyph_runs.iter().enumerate() {
        append_pdf_glyph_run_snapshot(output, index, run);
    }
}

fn append_pdf_glyph_run_snapshot(output: &mut String, index: usize, run: &MathPdfGlyphRun) {
    let glyph_ids = run
        .glyphs
        .iter()
        .map(|glyph| glyph.glyph_id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let unicode = run
        .glyphs
        .iter()
        .map(|glyph| glyph.unicode.as_str())
        .collect::<String>();
    let positions = run
        .glyphs
        .iter()
        .map(|glyph| {
            format!(
                "({:.4},{:.4};adv={:.4})",
                glyph.transform.dx, glyph.transform.dy, glyph.x_advance
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    writeln!(
        output,
        "  glyph_run {index}: font={:?} font_size={:.4} glyphs={} glyph_ids=[{}] unicode={:?} pos=[{}]",
        run.font,
        run.font_size,
        run.glyphs.len(),
        glyph_ids,
        unicode,
        positions
    )
    .unwrap();
}

fn append_font_resources_snapshot(
    output: &mut String,
    label: &str,
    resources: &[MathFontResource],
) {
    writeln!(output, "{label}: {}", resources.len()).unwrap();
    for resource in resources {
        writeln!(
            output,
            "  font {:?}: family={:?} postscript={:?} face_index={} units_per_em={:.1} bytes={}",
            resource.id,
            resource.family,
            resource.postscript_name,
            resource.face_index,
            resource.units_per_em,
            resource.data.len()
        )
        .unwrap();
    }
}

#[derive(Debug, Clone, Copy)]
struct PathBounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl PathBounds {
    fn new(x: f32, y: f32) -> Self {
        Self {
            min_x: x,
            min_y: y,
            max_x: x,
            max_y: y,
        }
    }

    fn include(&mut self, x: f32, y: f32) {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }
}

fn path_bounds(paths: &MathPathArtifact) -> Option<PathBounds> {
    let mut bounds: Option<PathBounds> = None;
    for item in &paths.items {
        for command in &item.path.commands {
            for (x, y) in command_points(command) {
                let transformed_x =
                    item.transform.xx * x + item.transform.xy * y + item.transform.dx;
                let transformed_y =
                    item.transform.yx * x + item.transform.yy * y + item.transform.dy;
                match &mut bounds {
                    Some(bounds) => bounds.include(transformed_x, transformed_y),
                    None => bounds = Some(PathBounds::new(transformed_x, transformed_y)),
                }
            }
        }
    }
    bounds
}

fn command_points(command: &MathPathCommand) -> Vec<(f32, f32)> {
    match *command {
        MathPathCommand::MoveTo { x, y } | MathPathCommand::LineTo { x, y } => vec![(x, y)],
        MathPathCommand::QuadTo { x1, y1, x, y } => vec![(x1, y1), (x, y)],
        MathPathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => vec![(x1, y1), (x2, y2), (x, y)],
        MathPathCommand::Close => Vec::new(),
    }
}
