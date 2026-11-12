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
|> AGGREGATE symbol(
     x => scaled(flipper_length_mm, scale => interval_fit(zero => FALSE)),
     y => scaled(body_mass_g, scale => interval_fit(zero => FALSE)),
     color => island) AS chart GROUP BY species
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
     size => scaled(magnitude, scale => scale(domain => [6, 9.5], range => [2, 120])),
     color => depth_km,
     coord => geo(surface => spherical(
       projection => 'equal_earth',
       fit => 'whole_sphere',
       graticule => graticule(longitude_step => 30, latitude_step => 30)))) AS chart";
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
|> AGGREGATE facet_col(chart, island) AS chart";
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
|> AGGREGATE facet_col(chart, island) AS chart GROUP BY sex
|> AGGREGATE facet_row(chart, sex) AS chart";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "facet-grid-sex-island");
}

#[tokio::test]
async fn a_histogram_of_chart() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE bar(x => bin(body_mass_g, maxbins => 20), y => count(*))";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "chart-histogram");
}

#[tokio::test]
async fn a_lump_bar_of_chart() {
    let query = "FROM penguins
|> AGGREGATE bar(x => lump(species, n => 2), y => count(*), color => lump(species, n => 2))";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "chart-lump-bar");
}

/// The panels of a facet of histograms share their bins with the default `OVER ()`, and each
/// panel has its own bins with `OVER panel`.
#[tokio::test]
async fn a_facet_of_histograms_with_shared_and_own_bins() {
    for (window, name) in [
        ("", "chart-facet-histograms"),
        (" OVER panel", "chart-facet-histograms-panel"),
    ] {
        let query = format!(
            "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE bar(x => bin(body_mass_g){window}, y => count(*)) AS chart GROUP BY island
|> AGGREGATE facet_col(chart, island) AS chart"
        );
        let charts = charts(&query).await;
        assert_eq!(charts.len(), 1);
        render(&charts[0], name);
    }
}

#[tokio::test]
async fn a_stacked_histogram_of_chart() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE bar(x => bin(body_mass_g, maxbins => 20), y => stack(count(*)), color => species)";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "chart-stacked-histogram");
}

/// A stacked area of the months of three regions, where the west region has no sales in March
/// and April, so the default imputation of a stacked area gives it a zero in those months and every
/// region has a layer at every month.
#[tokio::test]
async fn a_stacked_area_with_imputed_months() {
    let query = "FROM generate_series(0, 181)
|> SELECT to_timestamp_millis(1704067200000 + value * 86400000) AS t,
     CASE value % 3 WHEN 0 THEN 'east' WHEN 1 THEN 'west' ELSE 'north' END AS region,
     CAST((value * 7) % 11 AS DOUBLE) AS units
|> WHERE NOT (region = 'west' AND t >= TIMESTAMP '2024-03-01' AND t < TIMESTAMP '2024-05-01')
|> AGGREGATE area(x => timeunit(t, unit => 'month'), y => stack(sum(units)), color => region)";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "chart-impute-stacked-area");
}

/// A title and a subtitle are Typst markup without a syntax, and the axis titles from column
/// names stay plain.
#[tokio::test]
async fn a_typst_title_and_subtitle() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm, color => species,
     title => title(text => '*Bills* of the _Pygoscelis_ penguins',
                    subtitle => 'Length $times$ depth, in mm'))";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "chart-typst-title");
}

/// A layer of the bars of the mean body mass of each species and a rule at the mean of all, as an
/// expression item of an AGGREGATE.
#[tokio::test]
async fn a_layer_of_bars_and_a_rule_of_chart() {
    let query = "FROM penguins
|> WHERE body_mass_g IS NOT NULL
|> AGGREGATE layer(
     bar(x => species, y => avg(body_mass_g)),
     rule(y => avg(body_mass_g), color => direct('firebrick')),
   )";
    let charts = charts(query).await;
    assert_eq!(charts.len(), 1);
    render(&charts[0], "chart-layer-bars-rule");
}
