use super::datasets;
use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

/// Test nested faceting with shared categorical x-axis (Band scale)
///
/// This test verifies that categorical scale sharing works correctly in nested facets:
/// - Outer FacetColumn facets by "group" (2 groups)
/// - Inner plot has categorical x-axis with Band scale
/// - Group1 has categories A, B, C
/// - Group2 has categories B, C, D
/// - With CoordinationScope::Shared, both facets should show all categories A, B, C, D
#[tokio::test]
async fn test_nested_facet_shared_categorical_x() {
    let ctx = SessionContext::new();
    let df = datasets::categorical_sharing_test_data();

    let outer = Chart::<FacetColumn>::new()
        .data(df)
        .canvas_size(600, 300)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .x_with(col("category"), |c| {
                            c.scale_with::<Band>(|s| s)
                                .with_domain_scope(CoordinationScope::Shared)
                                .axis(|a| a.title("Category"))
                        })
                        .x2_with(col(":x"), |c| c.band(1.0))
                        .y(0.0)
                        .y2_with(col("value"), |c| {
                            c.with_domain_scope(CoordinationScope::Shared)
                                .axis(|a| a.title("Value"))
                        })
                        .fill("#4682b4"),
                ),
            )
            .col_with(col("group"), |c| c.guide(|g| g.title("Group"))),
        );

    let compiled = outer.compile(&ctx).await.expect("compile nested facets");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "nested_facet_shared_categorical_x",
    )
    .await;
}

/// Test nested faceting with shared categorical y-axis (Band scale for horizontal bars)
///
/// This test verifies categorical scale sharing on the y-axis:
/// - Outer FacetRow facets by "group"
/// - Inner plot has categorical y-axis (horizontal bars)
/// - With CoordinationScope::Shared, both facets should show all categories
#[tokio::test]
async fn test_nested_facet_shared_categorical_y() {
    let ctx = SessionContext::new();
    let df = datasets::categorical_sharing_test_data();

    let outer = Chart::<FacetRow>::new()
        .data(df)
        .canvas_size(400, 400)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .y_with(col("category"), |c| {
                            c.scale_with::<Band>(|s| s)
                                .with_domain_scope(CoordinationScope::Shared)
                                .axis(|a| a.title("Category"))
                        })
                        .y2_with(col(":y"), |c| c.band(1.0))
                        .x(0.0)
                        .x2_with(col("value"), |c| {
                            c.with_domain_scope(CoordinationScope::Shared)
                                .axis(|a| a.title("Value"))
                        })
                        .fill("#4682b4"),
                ),
            )
            .row_with(col("group"), |c| c.guide(|g| g.title("Group"))),
        );

    let compiled = outer.compile(&ctx).await.expect("compile nested facets");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "nested_facet_shared_categorical_y",
    )
    .await;
}

/// Test deeply nested facets with categorical scale sharing
///
/// This is the key test for the categorical scale sharing fix.
/// It verifies that categorical scale sharing works in nested facets
/// where data flows through evaluate_shared_scale_nested_facet:
/// - Outer FacetColumn facets by "group"
/// - Inner FacetRow facets by "sub_group" (requires nested facet path)
/// - Innermost plot has categorical x-axis
/// - With CoordinationScope::Shared, all facets should show unified categories
#[tokio::test]
async fn test_deeply_nested_categorical_scale_sharing() {
    use datafusion::arrow::array::{Float64Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    let ctx = SessionContext::new();

    // Create data with:
    // - 2 outer groups (G1, G2)
    // - 2 sub_groups per outer (S1, S2)
    // - Different categories in each combination
    // G1+S1: cats A,B | G1+S2: cats B,C | G2+S1: cats C,D | G2+S2: cats A,D
    let outer_groups = StringArray::from(vec![
        "G1", "G1", "G1", "G1", // S1: A,B; S2: B,C
        "G2", "G2", "G2", "G2", // S1: C,D; S2: A,D
    ]);
    let sub_groups = StringArray::from(vec![
        "S1", "S1", "S2", "S2", // G1
        "S1", "S1", "S2", "S2", // G2
    ]);
    let categories = StringArray::from(vec![
        "A", "B", "B", "C", // G1 (S1: A,B; S2: B,C)
        "C", "D", "A", "D", // G2 (S1: C,D; S2: A,D)
    ]);
    let values = Float64Array::from(vec![
        10.0, 20.0, 15.0, 25.0, // G1
        30.0, 40.0, 35.0, 45.0, // G2
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("outer_group", DataType::Utf8, false),
        Field::new("sub_group", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(outer_groups),
            Arc::new(sub_groups),
            Arc::new(categories),
            Arc::new(values),
        ],
    )
    .expect("create batch");

    let df = ctx.read_batch(batch).expect("read batch");

    // Outer: FacetColumn by outer_group
    // Inner: FacetRow by sub_group (nested facet triggers evaluate_shared_scale_nested_facet)
    // Innermost: Bar chart with categorical x-axis
    let outer = Chart::<FacetColumn>::new()
        .data(df)
        .canvas_size(700, 500)
        .mark(
            Subplot::new(
                Plot::<FacetRow>::new().mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Rect::new()
                                .x_with(col("category"), |c| {
                                    c.scale_with::<Band>(|s| s)
                                        .with_domain_scope(CoordinationScope::Shared)
                                        .axis(|a| a.title("Category"))
                                })
                                .x2_with(col(":x"), |c| c.band(1.0))
                                .y(0.0)
                                .y2_with(col("value"), |c| {
                                    c.with_domain_scope(CoordinationScope::Shared)
                                        .axis(|a| a.title("Value"))
                                })
                                .fill("#4682b4"),
                        ),
                    )
                    .row_with(col("sub_group"), |c| c.guide(|g| g.title("Sub Group"))),
                ),
            )
            .col_with(col("outer_group"), |c| c.guide(|g| g.title("Outer Group"))),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile deeply nested facets");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "deeply_nested_categorical_sharing",
    )
    .await;
}

/// Test numeric-coded categorical sharing (Int32 category IDs with Band scale)
///
/// This test verifies that numeric columns used as categorical values share correctly:
/// - Uses Int32 column for category IDs (1, 2, 3, 4) instead of strings
/// - Configures Band scale explicitly to indicate categorical treatment
/// - With CoordinationScope::Shared, both facets should show all category IDs
///
/// This was a bug where numeric columns were always treated as numeric scales
/// (computing min/max intervals) instead of categorical scales (computing DISTINCT values).
/// The fix checks the scale's domain_kind() before falling back to Arrow type detection.
#[tokio::test]
async fn test_numeric_coded_categorical_sharing() {
    use datafusion::arrow::array::{Float64Array, Int32Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    let ctx = SessionContext::new();

    // Create data with numeric category IDs:
    // - Group1 has category IDs 1, 2, 3
    // - Group2 has category IDs 2, 3, 4
    // With sharing, both should show 1, 2, 3, 4 on the axis
    let groups = StringArray::from(vec![
        "Group1", "Group1", "Group1", // IDs 1, 2, 3
        "Group2", "Group2", "Group2", // IDs 2, 3, 4
    ]);
    let category_ids = Int32Array::from(vec![
        1, 2, 3, // Group1
        2, 3, 4, // Group2
    ]);
    let values = Float64Array::from(vec![
        10.0, 20.0, 30.0, // Group1
        25.0, 35.0, 45.0, // Group2
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("group", DataType::Utf8, false),
        Field::new("category_id", DataType::Int32, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(groups), Arc::new(category_ids), Arc::new(values)],
    )
    .expect("create batch");

    let df = ctx.read_batch(batch).expect("read batch");

    let outer = Chart::<FacetColumn>::new()
        .data(df)
        .canvas_size(600, 300)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .x_with(col("category_id"), |c| {
                            // Explicitly configure as Band scale (categorical)
                            // This triggers the scale-driven categorical detection
                            c.scale_with::<Band>(|s| s)
                                .with_domain_scope(CoordinationScope::Shared)
                                .axis(|a| a.title("Category ID"))
                        })
                        .x2_with(col(":x"), |c| c.band(1.0))
                        .y(0.0)
                        .y2_with(col("value"), |c| {
                            c.with_domain_scope(CoordinationScope::Shared)
                                .axis(|a| a.title("Value"))
                        })
                        .fill("#4682b4"),
                ),
            )
            .col_with(col("group"), |c| c.guide(|g| g.title("Group"))),
        );

    let compiled = outer.compile(&ctx).await.expect("compile nested facets");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "numeric_coded_categorical_sharing",
    )
    .await;
}

/// Test Level(1) sharing on categorical channel
///
/// This test verifies that Level-based scale sharing works for categorical scales:
/// - Uses Level(1) instead of Shared
/// - Should produce the same result as Shared for single-level nesting
#[tokio::test]
async fn test_nested_facet_level1_categorical() {
    let ctx = SessionContext::new();
    let df = datasets::categorical_sharing_test_data();

    let outer = Chart::<FacetColumn>::new()
        .data(df)
        .canvas_size(600, 300)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .x_with(col("category"), |c| {
                            c.scale_with::<Band>(|s| s)
                                .with_domain_scope(CoordinationScope::Level(1))
                                .axis(|a| a.title("Category"))
                        })
                        .x2_with(col(":x"), |c| c.band(1.0))
                        .y(0.0)
                        .y2_with(col("value"), |c| {
                            c.with_domain_scope(CoordinationScope::Level(1))
                                .axis(|a| a.title("Value"))
                        })
                        .fill("#4682b4"),
                ),
            )
            .col_with(col("group"), |c| c.guide(|g| g.title("Group"))),
        );

    let compiled = outer.compile(&ctx).await.expect("compile nested facets");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "nested_facet_level1_categorical",
    )
    .await;
}
