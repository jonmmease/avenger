//! What the tests share: the label engine, the cascades of the base stylesheet, and the evaluation
//! and rendering of charts against image baselines.

use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_eval::{ChartEvaluator, EvaluatedChart, Registry};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};

mod data;
mod scenes;

pub(crate) use data::*;
pub(crate) use scenes::*;

pub(crate) fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
    .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The cascade of a table whose stylesheet imports the base stylesheet, then holds `css`.
pub(crate) fn cascade_with(css: &str) -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let text = format!("@import 'std:base.css';\n{css}");
    let (sheet, diagnostics) = Sheet::load(&text, None, SourceId(1), options, &mut no_files);
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

pub(crate) fn cascade() -> Arc<Cascade> {
    cascade_with("")
}

pub(crate) fn evaluator(chart: &Chart) -> ChartEvaluator {
    ChartEvaluator::new(chart.clone(), cascade(), engine(), Registry::new()).unwrap()
}

pub(crate) fn host([width, height]: [f32; 2]) -> HostInputs {
    HostInputs::sized(width, height)
}

/// Evaluates `chart` at `size`.
pub(crate) fn evaluate(chart: &Chart, size: [f32; 2]) -> EvaluatedChart {
    evaluator(chart).evaluate(&host(size)).unwrap()
}

/// Renders an evaluation and compares it with the baseline `name`.
pub(crate) fn assert_image(evaluated: &EvaluatedChart, name: &str) {
    let svg = SvgRenderer::new()
        .render_scene_graph(&evaluated.scene, &engine())
        .unwrap();
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &svg_to_png(&svg, 2.0), name);
}

/// Evaluates `chart` at `size`, renders it, and compares it with the baseline `name`.
pub(crate) fn render(chart: &Chart, size: [f32; 2], name: &str) -> EvaluatedChart {
    let evaluated = evaluate(chart, size);
    assert_image(&evaluated, name);
    evaluated
}

/// Renders `chart` at 320 by 240 and at 640 by 360, writing both images before a failure of
/// either.
pub(crate) fn render_both(chart: &Chart, name: &str) {
    let results: Vec<_> = [[320.0, 240.0], [640.0, 360.0]]
        .iter()
        .map(|size| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render(chart, *size, &format!("{name}-{}x{}", size[0], size[1]));
            }))
        })
        .collect();
    for result in results {
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
