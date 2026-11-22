//! The shapes of results, the rows of their batches, and the static charts of their cells.

use std::path::Path;
use std::sync::Arc;

use arrow::array::{Int64Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};
use avenger_sql_editor::charts::{cascade, engine, StaticChart};
use avenger_sql_editor::query::{self, Outcome};
use avenger_sql_editor::result::{QueryResult, Shape};
use avenger_sql_editor::sources::FileKind;
use datafusion::prelude::SessionContext;

/// A session with the penguins of `samples` as the table `penguins`.
async fn session() -> SessionContext {
    let ctx = query::session().unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/penguins.csv");
    let source = query::register(&ctx, "penguins", &path, FileKind::Csv)
        .await
        .unwrap();
    assert_eq!(source.columns.len(), 7, "{:?}", source.columns);
    ctx
}

async fn run(ctx: &SessionContext, text: &str) -> QueryResult {
    match query::run(ctx, text).await {
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
    let Err(error) = query::run(&ctx, text).await else {
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
    let mut chart = StaticChart::new(chart, cascade(), engine()).unwrap();
    for size in [[360.0, 240.0], [520.0, 300.0]] {
        let started = std::time::Instant::now();
        let scene = chart.scene(size).unwrap();
        eprintln!("a {size:?} cell evaluated in {:?}", started.elapsed());
        assert_eq!([scene.width, scene.height], size);
    }
}
