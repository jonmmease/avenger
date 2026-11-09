//! Charts end to end: a query builds a chart, which avenger-chart-eval draws.

mod common;

use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};
use common::charts;

fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
    .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The cascade of the base stylesheet.
fn cascade() -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(1),
        options,
        &mut no_files,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

/// Renders `chart` at 320 by 240 and at 640 by 360 and compares each image with its baseline,
/// after writing both.
fn render(chart: &Chart, name: &str) {
    let mut evaluator =
        ChartEvaluator::new(chart.clone(), cascade(), engine(), Registry::new()).unwrap();
    let mut failures = Vec::new();
    for [width, height] in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluator
            .evaluate(&HostInputs::sized(width, height))
            .unwrap();
        let svg = SvgRenderer::new()
            .render_scene_graph(&evaluated.scene, &engine())
            .unwrap();
        let png = svg_to_png(&svg, 2.0);
        let name = format!("{name}-{width}x{height}");
        if let Err(failure) =
            std::panic::catch_unwind(|| assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, &name))
        {
            failures.push(failure);
        }
    }
    if let Some(failure) = failures.pop() {
        std::panic::resume_unwind(failure);
    }
}

#[tokio::test]
async fn a_bar_chart_of_counts() {
    let query = "FROM penguins
|> AGGREGATE count(*) AS n GROUP BY species
|> AGGREGATE bar(x => species, y => n ORDER BY species) AS chart";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "penguins-bar");
}

#[tokio::test]
async fn a_chart_for_each_group() {
    let query = "FROM penguins
|> AGGREGATE symbol(x => flipper_length_mm, y => body_mass_g, color => island) AS chart GROUP BY species
|> ORDER BY species";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 3, "one chart for each species");
    render(&charts[0], "penguins-adelie-symbols");
}
