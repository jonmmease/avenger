// Test data generation utilities for visual tests

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

/// Create a dataset for testing categorical scale sharing in nested facets
///
/// This dataset has:
/// - 2 groups (outer facet dimension)
/// - Different categories present in each group (inner facet tests sharing)
/// - Categories A, B, C in Group1; categories B, C, D in Group2
/// - This tests that shared categorical scales show all categories (A, B, C, D) in both facets
pub fn categorical_sharing_test_data() -> DataFrame {
    // Group1 has categories A, B, C
    // Group2 has categories B, C, D
    // With sharing, both should show A, B, C, D on the axis
    let groups = StringArray::from(vec![
        "Group1", "Group1", "Group1", // A, B, C
        "Group2", "Group2", "Group2", // B, C, D
    ]);
    let categories = StringArray::from(vec![
        "A", "B", "C", // Group1
        "B", "C", "D", // Group2
    ]);
    let values = Float64Array::from(vec![
        10.0, 20.0, 30.0, // Group1
        25.0, 35.0, 45.0, // Group2
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("group", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(groups), Arc::new(categories), Arc::new(values)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}

/// Create a simple categorical bar chart dataset
pub fn simple_categories() -> DataFrame {
    let categories = StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H", "I"]);
    let values = Float64Array::from(vec![28.0, 55.0, 43.0, 91.0, 81.0, 53.0, 19.0, 87.0, 52.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(categories), Arc::new(values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}

/// Load iris and add a categorical bin column used in nested facet tests.
pub async fn iris_with_petal_width_bin(ctx: &SessionContext) -> DataFrame {
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    df.with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap(),
    )
    .unwrap()
}

/// Dataset with 3-level categorical hierarchy and color legend category.
pub async fn legend_sharing_hierarchy_df(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "CREATE TABLE legend_sharing_hierarchy AS VALUES
        ('DivA', 'Dept1', 'Team1', 1.0, 1.0, 'Low'),
        ('DivA', 'Dept1', 'Team1', 1.4, 1.3, 'High'),
        ('DivA', 'Dept1', 'Team2', 2.0, 1.1, 'Low'),
        ('DivA', 'Dept1', 'Team2', 2.3, 1.4, 'High'),
        ('DivA', 'Dept2', 'Team1', 1.1, 2.0, 'Low'),
        ('DivA', 'Dept2', 'Team1', 1.5, 2.3, 'High'),
        ('DivA', 'Dept2', 'Team2', 2.1, 2.1, 'Low'),
        ('DivA', 'Dept2', 'Team2', 2.4, 2.4, 'High'),
        ('DivB', 'Dept1', 'Team1', 3.0, 1.0, 'Low'),
        ('DivB', 'Dept1', 'Team1', 3.4, 1.3, 'High'),
        ('DivB', 'Dept1', 'Team2', 4.0, 1.1, 'Low'),
        ('DivB', 'Dept1', 'Team2', 4.3, 1.4, 'High'),
        ('DivB', 'Dept2', 'Team1', 3.1, 2.0, 'Low'),
        ('DivB', 'Dept2', 'Team1', 3.5, 2.3, 'High'),
        ('DivB', 'Dept2', 'Team2', 4.1, 2.1, 'Low'),
        ('DivB', 'Dept2', 'Team2', 4.4, 2.4, 'High')",
    )
    .await
    .expect("create legend_sharing_hierarchy test data");

    ctx.sql(
        "SELECT
            column1 AS division,
            column2 AS department,
            column3 AS team,
            column4 AS x_val,
            column5 AS y_val,
            column6 AS category
         FROM legend_sharing_hierarchy",
    )
    .await
    .expect("load legend_sharing_hierarchy test data")
}

/// Sparse hierarchy dataset with missing combinations and uneven branch shape.
pub async fn sparse_hierarchy_df(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "CREATE TABLE sparse_hierarchy AS VALUES
        ('North', 'Eng', 'Alpha', 1.2, 10.0, 'Low'),
        ('North', 'Eng', 'Beta', 2.2, 12.0, 'High'),
        ('North', 'Ops', 'Alpha', 1.2, 8.0, 'Low'),
        ('South', 'Eng', 'Beta', 4.2, 11.0, 'High'),
        ('South', 'Ops', 'Alpha', 3.2, 3.0, 'Low'),
        ('South', 'Support', 'Beta', 4.2, 2.8, 'High')",
    )
    .await
    .expect("create sparse_hierarchy test data");

    ctx.sql(
        "SELECT
            column1 AS division,
            column2 AS dept,
            column3 AS team,
            column4 AS x_val,
            column5 AS y_val,
            column6 AS category
         FROM sparse_hierarchy",
    )
    .await
    .expect("load sparse_hierarchy test data")
}

/// Numeric hierarchy dataset to exercise ordering for numeric facet fields.
pub async fn numeric_hierarchy_df(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "CREATE TABLE numeric_hierarchy AS VALUES
        (1, 10, 100, 0.5, 2.0),
        (1, 10, 200, 1.2, 2.2),
        (1, 20, 100, 0.7, 1.8),
        (1, 20, 200, 1.5, 2.4),
        (2, 10, 100, 2.5, 2.1),
        (2, 10, 200, 3.2, 2.3),
        (2, 20, 100, 2.7, 1.9),
        (2, 20, 200, 3.5, 2.6)",
    )
    .await
    .expect("create numeric_hierarchy test data");

    ctx.sql(
        "SELECT
            column1 AS division_id,
            column2 AS dept_id,
            column3 AS team_id,
            column4 AS x_val,
            column5 AS y_val
         FROM numeric_hierarchy",
    )
    .await
    .expect("load numeric_hierarchy test data")
}
