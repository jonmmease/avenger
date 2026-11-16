//! The sample tables: one table of the schema `samples` for each file, beside the tables of the
//! files that the user adds.

use std::path::Path;

use arrow::array::AsArray;
use arrow::datatypes::Int64Type;
use avenger_sql_editor::{query, samples, sources::FileKind};
use datafusion::prelude::SessionContext;

/// The number of rows of `table` in `ctx`.
async fn rows(ctx: &SessionContext, table: &str) -> i64 {
    let batches = ctx
        .sql(&format!("SELECT count(*) AS n FROM {table}"))
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let column = batches[0].column(0).as_primitive::<Int64Type>();
    assert_eq!(column.len(), 1);
    column.value(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn each_sample_is_a_table_of_the_samples_schema() {
    let ctx = query::session().unwrap();
    samples::register(&ctx).await.unwrap();
    for (table, count) in [
        ("penguins", 344),
        ("cars", 406),
        ("movies", 2259),
        ("iris", 150),
        ("seattle_weather", 1461),
        ("sales", 203),
    ] {
        assert_eq!(
            rows(&ctx, &format!("samples.{table}")).await,
            count,
            "{table}"
        );
    }
    // A file of the user named `penguins` is the table `penguins`, apart from the sample.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/iris.csv");
    query::register(&ctx, "penguins", &path, FileKind::Csv)
        .await
        .unwrap();
    assert_eq!(rows(&ctx, "penguins").await, 150);
    assert_eq!(rows(&ctx, "samples.penguins").await, 344);
}
