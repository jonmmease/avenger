//! The shapes of results, the rows of their batches, and the static charts of their cells.

use std::path::Path;
use std::sync::Arc;

use arrow::array::{Int64Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_sql_editor::charts::{engine, ChartTheme, StaticChart};
use avenger_sql_editor::query::{self, Outcome, QueryError};
use avenger_sql_editor::result::{QueryResult, Shape};
use datafusion::prelude::{CsvReadOptions, SessionContext};
use eframe::egui;

/// A session with avenger-sql's functions and the penguins of the sample project as the table
/// `penguins`.
async fn session() -> SessionContext {
    let ctx = SessionContext::new();
    avenger_sql::register(&ctx).unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/data/files/penguins.csv");
    (ctx.register_csv("penguins", path.to_str().unwrap(), CsvReadOptions::new())
        .await)
        .unwrap();
    ctx
}

/// The outcome of the query `text` in `ctx`, as a run of a chart's tab gives it.
async fn outcome(ctx: &SessionContext, text: &str) -> Result<Outcome, QueryError> {
    match avenger_sql::sql(ctx, text).await {
        Ok(frame) => query::outcome(frame, text).await,
        Err(error) => Err(QueryError::new(&error, text)),
    }
}

async fn run(ctx: &SessionContext, text: &str) -> QueryResult {
    match outcome(ctx, text).await {
        Ok(Outcome { result, .. }) => result,
        Err(error) => panic!("{}", error.message),
    }
}

#[tokio::test]
async fn each_result_has_its_shape() {
    let ctx = session().await;

    let single = run(
        &ctx,
        "FROM penguins
|> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm) AS chart",
    )
    .await;
    assert_eq!(single.shape(), Shape::Chart);
    assert!(single.chart(0, 0).is_some());

    let groups = run(
        &ctx,
        "FROM penguins
|> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm) AS chart GROUP BY species
|> ORDER BY species",
    )
    .await;
    assert_eq!(groups.shape(), Shape::Table);
    assert_eq!(groups.num_rows(), 3, "one row for each species");
    assert_eq!(groups.chart_columns(), [1]);
    assert!((0..3).all(|row| groups.chart(row, 1).is_some()));

    let two = run(
        &ctx,
        "FROM penguins
|> AGGREGATE
     mark_symbol(x => bill_length_mm, y => bill_depth_mm) AS bills,
     mark_symbol(x => flipper_length_mm, y => body_mass_g) AS sizes
   GROUP BY species",
    )
    .await;
    assert_eq!(two.shape(), Shape::Table);
    assert_eq!(two.chart_columns(), [1, 2]);

    let rows = run(
        &ctx,
        "FROM penguins |> AGGREGATE count(*) AS n GROUP BY island",
    )
    .await;
    assert_eq!(rows.shape(), Shape::Table);
    assert!(rows.chart_columns().is_empty());
    assert_eq!(rows.num_rows(), 3);
}

/// A query error points to the text of the unknown column.
#[tokio::test]
async fn an_error_points_to_its_text() {
    let ctx = session().await;
    let text = "FROM penguins\n|> WHERE body_mass > 4000";
    let Err(error) = outcome(&ctx, text).await else {
        panic!("the query has an unknown column");
    };
    let range = error
        .range
        .unwrap_or_else(|| panic!("the error has a range: {}", error.message));
    let found: String = text.chars().skip(range.start).take(range.len()).collect();
    assert_eq!(found, "body_mass", "{}", error.message);
}

/// A row is found in its batch across batches of different lengths.
#[test]
fn a_row_is_found_in_its_batch() {
    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    let batch = |values: Vec<i64>| {
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap()
    };
    let result = QueryResult::new(
        schema.clone(),
        vec![
            batch(vec![0, 1]),
            batch(vec![]),
            batch(vec![2, 3, 4]),
            batch(vec![5]),
        ],
    )
    .unwrap();
    assert_eq!(result.num_rows(), 6);
    for row in 0..6 {
        let (batch, offset) = result.locate(row).unwrap();
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(values.value(offset), row as i64);
    }
    assert!(result.locate(6).is_none());
}

/// The text of a value, and `None` for a null.
#[tokio::test]
async fn a_value_has_its_text() {
    let ctx = session().await;
    let result = run(
        &ctx,
        "FROM penguins |> WHERE sex IS NULL |> ORDER BY species |> LIMIT 1 |> SELECT species, sex",
    )
    .await;
    assert_eq!(result.value_text(0, 0).as_deref(), Some("Adelie"));
    assert_eq!(result.value_text(0, 1), None);
    assert_eq!(result.value_text(1, 0), None, "past the last row");
}

/// A static chart draws at the size that the editor gives it.
#[tokio::test]
async fn a_static_chart_draws_at_its_size() {
    let ctx = session().await;
    let result = run(
        &ctx,
        "FROM penguins
|> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm) AS chart",
    )
    .await;
    let chart = result.chart(0, 0).unwrap().clone();
    let mut chart = StaticChart::new(
        chart,
        ChartTheme::default().cascade(None),
        ChartTheme::default().preferences(),
        engine(),
    )
    .unwrap();
    for size in [[360.0, 240.0], [520.0, 300.0]] {
        let started = std::time::Instant::now();
        let scene = chart.scene(size).unwrap();
        eprintln!("a {size:?} cell evaluated in {:?}", started.elapsed());
        assert_eq!([scene.width, scene.height], size);
    }
}

/// A chart's background is the color of the editor's panels, in the light theme and the dark one,
/// so that a chart and the panel around it are one surface.
#[tokio::test]
async fn a_charts_background_is_the_panels_color_in_both_themes() {
    let ctx = session().await;
    let result = run(
        &ctx,
        "FROM penguins |> AGGREGATE mark_bar(x => species, y => count(*)) AS chart",
    )
    .await;
    let chart = result.chart(0, 0).unwrap().clone();
    for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
        let theme = ChartTheme::of(&visuals);
        let mut static_chart = StaticChart::new(
            chart.clone(),
            theme.cascade(None),
            theme.preferences(),
            engine(),
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
