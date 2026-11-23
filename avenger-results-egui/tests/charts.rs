//! The static chart of a table cell, which draws at its cell's size, on the surface of egui's
//! panels in the light visuals and the dark ones.

use std::sync::Arc;

use avenger_chart::results::{QueryResult, Shape};
use avenger_results_egui::{StaticChart, Surface};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_sql_project::charts::label_engine;
use datafusion::prelude::SessionContext;
use eframe::egui;

/// The result of the chart script `text`.
async fn run(text: &str) -> QueryResult {
    let ctx = SessionContext::new();
    avenger_sql::register(&ctx).unwrap();
    let frame = avenger_sql::sql(&ctx, text).await.unwrap();
    let schema = Arc::new(frame.schema().as_arrow().clone());
    QueryResult::new(schema, frame.collect().await.unwrap()).unwrap()
}

const BARS: &str = "FROM UNNEST([1, 2, 3]) AS n |> AGGREGATE mark_bar(x => n, y => n * 2) AS chart";

#[tokio::test]
async fn a_static_chart_draws_at_its_size() {
    let result = run(BARS).await;
    assert_eq!(result.shape(), Shape::Chart);
    let surface = Surface::default();
    let chart = result.chart(0, 0).unwrap().clone();
    let mut chart = StaticChart::new(
        chart,
        surface.cascade([]),
        surface.preferences(),
        label_engine(),
    )
    .unwrap();
    for size in [[360.0, 240.0], [520.0, 300.0]] {
        let scene = chart.scene(size).unwrap();
        assert_eq!([scene.width, scene.height], size);
    }
}

/// A chart's background is the color of egui's panels, in the light visuals and the dark ones, so
/// that a chart and the panel around it are one surface.
#[tokio::test]
async fn a_chart_s_background_is_the_panel_s_color_in_both_color_schemes() {
    let result = run(BARS).await;
    let chart = result.chart(0, 0).unwrap().clone();
    for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
        let surface = Surface::of(&visuals);
        let mut static_chart = StaticChart::new(
            chart.clone(),
            surface.cascade([]),
            surface.preferences(),
            label_engine(),
        )
        .unwrap();
        let scene = static_chart.scene([320.0, 240.0]).unwrap();
        let Some(SceneMark::Rect(rect)) = scene.marks.first() else {
            panic!("a background: {:?}", scene.marks.first());
        };
        assert_eq!(rect.name, "background");
        let fill = rect.fill.first().expect("a fill").color_or_transparent();
        let [r, g, b, _] = visuals.panel_fill.to_normalized_gamma_f32();
        for (actual, expected) in fill.iter().zip([r, g, b]) {
            assert!(
                (actual - expected).abs() < 0.01,
                "{fill:?} for {:?}",
                visuals.panel_fill
            );
        }
    }
}
