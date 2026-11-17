//! Charts end to end: a query builds a chart, which avenger-chart-eval draws.

mod common;

use std::sync::Arc;

use avenger_chart::{
    proto::{self, node::Node},
    Chart, LayerRef,
};
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};
use common::charts;

/// The label engine, with the providers of GoogleSQL's patterns, which the charts of avenger-sql
/// name.
fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
    .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()))
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
|> AGGREGATE symbol(
     x => x(flipper_length_mm, scale => scale(zero => FALSE)),
     y => y(body_mass_g, scale => scale(zero => FALSE)),
     color => island,
   ) AS chart
   GROUP BY species
|> ORDER BY species";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 3, "one chart for each species");
    render(&charts[0], "penguins-adelie-symbols");
}

#[tokio::test]
async fn a_map_of_earthquakes() {
    let query = "FROM earthquakes
|> AGGREGATE geo_symbol(
     longitude => longitude,
     latitude => latitude,
     size => size(magnitude, scale => scale(domain => [6, 9.5], range => [2, 120])),
     color => depth_km,
     coord => geo(
       projection => projection(type => 'equal_earth', fit => 'whole_sphere'),
       graticule => graticule(step => [30, 30]),
     ),
   ) AS chart";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "earthquakes-map");
}

#[tokio::test]
async fn a_facet_of_scatter_plots_by_island() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, color => species) AS chart
   GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "facet-col-islands");
}

#[tokio::test]
async fn a_grid_of_scatter_plots_by_sex_and_island() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL AND sex IS NOT NULL
|> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, color => species) AS chart
   GROUP BY sex, island
|> AGGREGATE facet_column(chart, island) AS chart GROUP BY sex
|> AGGREGATE facet_row(chart, sex) AS chart";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "facet-grid-sex-island");
}

#[tokio::test]
async fn a_facet_of_layers_by_island() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE AVG(bill_depth_mm) AS mean GROUP BY island, species
|> AGGREGATE layer(
     bar(x => species, y => mean, color => species),
     symbol(x => species, y => mean, fill => 'black')) AS chart
   GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "facet-col-layers");
}

#[tokio::test]
async fn a_histogram_of_lifted_bins() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE bar(x => bin(body_mass_g), y => COUNT(*)) AS chart";
    let charts = charts(query).await;
    render(&charts[0], "lifted-histogram");
}

#[tokio::test]
async fn a_facet_of_histograms_with_shared_bins() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE bar(x => bin(body_mass_g), y => COUNT(*)) AS chart GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart";
    let charts = charts(query).await;
    render(&charts[0], "lifted-facet-histograms");
}

#[tokio::test]
async fn a_lifted_stacked_bar() {
    let query = "FROM penguins
|> AGGREGATE bar(x => island, y => stack(COUNT(*)), color => species) AS chart";
    let charts = charts(query).await;
    render(&charts[0], "lifted-stacked-bar");
}

#[tokio::test]
async fn a_lifted_stacked_area_with_zero_fill() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> SET body_mass_g = bin(body_mass_g, maxbins => 30) OVER ()
|> SET body_mass_g = body_mass_g.start
|> AGGREGATE area(x => body_mass_g, y => stack(COUNT(*)), color => species) AS chart";
    let charts = charts(query).await;
    render(&charts[0], "lifted-stacked-area");
}

/// The texts of the scene of `chart` evaluated at 640 by 360, through the label engine with the
/// providers of GoogleSQL's patterns.
fn drawn(chart: &Chart) -> Vec<String> {
    fn walk(marks: &[SceneMark], out: &mut Vec<String>) {
        for mark in marks {
            match mark {
                SceneMark::Group(group) => walk(&group.marks, out),
                SceneMark::Text(text) => out.extend(text.text.as_vec(text.len as usize, None)),
                _ => {}
            }
        }
    }
    let message = chart.message();
    assert_eq!(message.number_format_syntax, "googlesql");
    assert_eq!(message.datetime_format_syntax, "googlesql");
    let mut evaluator =
        ChartEvaluator::new(chart.clone(), cascade(), engine(), Registry::new()).unwrap();
    let evaluated = evaluator
        .evaluate(&HostInputs::sized(640.0, 360.0))
        .unwrap();
    let mut out = Vec::new();
    walk(&evaluated.scene.marks, &mut out);
    out
}

/// The one chart of `query`.
async fn one(query: &str) -> Chart {
    let mut charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    charts.remove(0)
}

/// Whether `texts` holds `text`.
fn holds(texts: &[String], text: &str) -> bool {
    texts.iter().any(|t| t == text)
}

#[tokio::test]
async fn an_axis_formats_its_labels_with_a_googlesql_pattern() {
    let chart = one("FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE symbol(
     x => x(body_mass_g, axis => axis(labels => labels(format => \"%'.0f\"))),
     y => y(bill_length_mm, axis => axis(labels => '%.1f mm')),
   ) AS chart")
    .await;
    let texts = drawn(&chart);
    assert!(holds(&texts, "4,000"), "{texts:?}");
    assert!(holds(&texts, "40.0 mm"), "{texts:?}");
}

#[tokio::test]
async fn a_legend_formats_its_labels_with_a_googlesql_pattern() {
    let chart = one("FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE symbol(
     x => bill_length_mm,
     y => bill_depth_mm,
     color => color(body_mass_g, legend => legend(labels => \"%'.1f\")),
   ) AS chart")
    .await;
    let texts = drawn(&chart);
    assert!(holds(&texts, "4,000.0"), "{texts:?}");
}

#[tokio::test]
async fn a_text_mark_formats_its_values_with_a_googlesql_pattern() {
    let chart = one("FROM penguins
|> AGGREGATE COUNT(*) AS n GROUP BY species
|> AGGREGATE text(x => species, y => n, text => n, format => '%05d') AS chart")
    .await;
    let texts = drawn(&chart);
    for count in ["00152", "00068", "00124"] {
        assert!(holds(&texts, count), "{texts:?}");
    }
}

#[tokio::test]
async fn a_tooltip_field_takes_a_googlesql_pattern() {
    let chart = one("FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE symbol(
     x => bill_length_mm,
     y => body_mass_g,
     tools => [tooltip(fields => [field(column => 'body_mass_g', format => \"%'.0f g\")])],
   ) AS chart")
    .await;
    // The tooltip formats its values in the app, when it shows, so the chart reads and evaluates
    // with the pattern.
    let Some(Node::Plot(plot)) = &chart.root().node else {
        panic!("a plot");
    };
    let layer = LayerRef::of(plot).remove(0);
    let Some(proto::layer_tool::Kind::Tooltip(tooltip)) = &layer.tools()[0].kind else {
        panic!("a tooltip");
    };
    assert_eq!(tooltip.fields[0].format, "%'.0f g");
    drawn(&chart);
}

#[tokio::test]
async fn a_date_axis_formats_its_labels_with_format_date_elements() {
    let days = "FROM UNNEST([
     STRUCT(DATE '2024-01-01' AS day, 3 AS n),
     STRUCT(DATE '2024-04-01' AS day, 5 AS n),
     STRUCT(DATE '2024-09-01' AS day, 4 AS n),
   ])";
    let chart = one(&format!(
        "{days}
|> AGGREGATE line(x => x(day, axis => axis(labels => '%Y-%m')), y => n ORDER BY day) AS chart"
    ))
    .await;
    let texts = drawn(&chart);
    assert!(holds(&texts, "2024-04"), "{texts:?}");

    // Without a pattern, the provider's default ticks.
    let chart = one(&format!(
        "{days}\n|> AGGREGATE line(x => day, y => n ORDER BY day) AS chart"
    ))
    .await;
    assert!(!drawn(&chart).is_empty());
}

#[tokio::test]
async fn compositions_of_marks_with_patterns_name_googlesql() {
    let marks = "bar(x => species, y => y(n, axis => axis(labels => '%.1f'))),
     text(x => species, y => n, text => n, format => '%05d')";
    let counts = "FROM penguins |> AGGREGATE COUNT(*) AS n GROUP BY species, island";
    let layered = one(&format!(
        "{counts}
|> AGGREGATE layer(
     {marks},
   ) AS chart"
    ))
    .await;
    let texts_of_layer = drawn(&layered);
    assert!(holds(&texts_of_layer, "100.0"), "{texts_of_layer:?}");

    let concat = one(&format!(
        "{counts}
|> AGGREGATE bar(x => species, y => y(n, axis => axis(labels => '%.1f'))) AS chart GROUP BY island
|> AGGREGATE hconcat_agg(chart ORDER BY island) AS chart"
    ))
    .await;
    assert!(holds(&drawn(&concat), "40.0"));

    let facet = one(&format!(
        "{counts}
|> AGGREGATE layer({marks}) AS chart GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart"
    ))
    .await;
    let texts = drawn(&facet);
    assert!(holds(&texts, "00052"), "{texts:?}");
    assert!(holds(&texts, "40.0"), "{texts:?}");
}

#[tokio::test]
async fn a_chart_without_patterns_takes_the_default_ticks() {
    let chart = one("FROM penguins
|> AGGREGATE COUNT(*) AS n GROUP BY species
|> AGGREGATE bar(x => species, y => n) AS chart")
    .await;
    let texts = drawn(&chart);
    assert!(holds(&texts, "0") && holds(&texts, "Adelie"), "{texts:?}");
}
