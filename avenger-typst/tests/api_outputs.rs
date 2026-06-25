use avenger_typst::{
    AvengerTypst, MathOutputRequest, MathStringRun, RasterRequest, TextLineOutputRequest,
    TypstEngineConfig,
};

fn engine() -> AvengerTypst {
    AvengerTypst::new(TypstEngineConfig::default()).unwrap()
}

#[test]
fn paths_can_be_requested() {
    let artifact = engine()
        .typeset_math_fragment("x", &Default::default())
        .unwrap();
    assert!(artifact.paths.is_some());
}

#[test]
fn paths_can_be_disabled() {
    let mut options = avenger_typst::MathFragmentOptions::default();
    options.outputs.paths = false;

    let artifact = engine().typeset_math_fragment("x", &options).unwrap();
    assert!(artifact.paths.is_none());
}

#[test]
fn raster_payload_can_be_requested_from_mock_engine() {
    let mut options = avenger_typst::MathFragmentOptions::default();
    options.outputs.raster = Some(RasterRequest { scale: 2.0 });

    let artifact = engine().typeset_math_fragment("x", &options).unwrap();
    let raster = artifact.raster.unwrap();
    assert_eq!(raster.scale, 2.0);
    assert_eq!(raster.image.width, 1);
    assert_eq!(raster.image.height, 1);
    assert_eq!(raster.origin_x, 0.0);
    assert_eq!(raster.origin_y, 0.0);
}

#[test]
fn pdf_text_layer_can_be_requested() {
    let mut options = avenger_typst::MathFragmentOptions::default();
    options.outputs.pdf_text_layer = true;

    let artifact = engine().typeset_math_fragment("xy", &options).unwrap();
    let pdf_text = artifact.pdf_text.unwrap();
    assert_eq!(pdf_text.semantic_text, "xy");
    assert_eq!(pdf_text.glyph_runs.len(), 1);
    assert_eq!(pdf_text.glyph_runs[0].glyphs.len(), 2);
    assert_eq!(artifact.font_resources.len(), 1);
}

#[test]
fn string_artifact_deduplicates_mock_font_resources() {
    let mut options = avenger_typst::MathStringOptions::default();
    options.outputs = MathOutputRequest {
        paths: true,
        raster: None,
        pdf_text_layer: true,
    };

    let artifact = engine().typeset_math_string("$x$ + $y$", &options).unwrap();

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
        assert_eq!(
            math_run.artifact.pdf_text.as_ref().unwrap().glyph_runs[0].font,
            artifact.font_resources[0].id
        );
        assert_eq!(
            math_run.artifact.font_resources[0].id,
            artifact.font_resources[0].id
        );
    }
}

#[test]
fn text_line_outputs_can_be_requested_from_mock_engine() {
    let mut options = avenger_typst::TextLineOptions::default();
    options.outputs = TextLineOutputRequest {
        paths: true,
        raster: Some(RasterRequest { scale: 2.0 }),
        pdf_text_layer: true,
    };

    let artifact = engine()
        .typeset_text_line("Price \\$7, score $R^2$ = 0.94", &options)
        .unwrap();

    assert_eq!(artifact.source, "Price \\$7, score $R^2$ = 0.94");
    assert!(artifact.paths.is_some());
    assert!(artifact.raster.is_some());
    assert!(artifact.pdf_text.is_some());
    assert_eq!(artifact.font_resources.len(), 1);
}

#[test]
fn text_line_metrics_only_disables_heavy_outputs() {
    let mut options = avenger_typst::TextLineOptions::default();
    options.outputs.paths = false;

    let artifact = engine().typeset_text_line("plain $x$", &options).unwrap();

    assert!(artifact.metrics.width > 0.0);
    assert!(artifact.paths.is_none());
    assert!(artifact.raster.is_none());
    assert!(artifact.pdf_text.is_none());
}
