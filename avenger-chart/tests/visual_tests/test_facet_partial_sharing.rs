use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::channel::config_traits::ScaleSharing;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

/// Helper to create a dataset with varying scales per facet
///
/// Creates a dataset where different groups have different value ranges,
/// making scale sharing effects clearly visible.
async fn create_varying_data() -> datafusion::dataframe::DataFrame {
    use arrow::array::{Float64Array, StringArray, UInt32Array};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    let ctx = SessionContext::new();

    // Create synthetic data with varying ranges per group
    // Group A: x in [0, 10], y in [0, 100]
    // Group B: x in [20, 30], y in [200, 300]
    // Group C: x in [40, 50], y in [400, 500]
    // Group D: x in [0, 10], y in [200, 300]

    let schema = Schema::new(vec![
        Field::new("group", DataType::Utf8, false),
        Field::new("row_facet", DataType::Utf8, false),
        Field::new("col_facet", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("size", DataType::UInt32, false),
    ]);

    let group = StringArray::from(vec![
        "A", "A", "A", "A", "A", "A",
        "B", "B", "B", "B", "B", "B",
        "C", "C", "C", "C", "C", "C",
        "D", "D", "D", "D", "D", "D",
    ]);

    let row_facet = StringArray::from(vec![
        "R1", "R1", "R1", "R1", "R1", "R1",
        "R1", "R1", "R1", "R1", "R1", "R1",
        "R2", "R2", "R2", "R2", "R2", "R2",
        "R2", "R2", "R2", "R2", "R2", "R2",
    ]);

    let col_facet = StringArray::from(vec![
        "C1", "C1", "C1", "C1", "C1", "C1",
        "C2", "C2", "C2", "C2", "C2", "C2",
        "C1", "C1", "C1", "C1", "C1", "C1",
        "C2", "C2", "C2", "C2", "C2", "C2",
    ]);

    let x = Float64Array::from(vec![
        0.0, 2.0, 4.0, 6.0, 8.0, 10.0,
        20.0, 22.0, 24.0, 26.0, 28.0, 30.0,
        40.0, 42.0, 44.0, 46.0, 48.0, 50.0,
        0.0, 2.0, 4.0, 6.0, 8.0, 10.0,
    ]);

    let y = Float64Array::from(vec![
        10.0, 25.0, 40.0, 55.0, 70.0, 90.0,
        210.0, 225.0, 240.0, 255.0, 270.0, 290.0,
        410.0, 425.0, 440.0, 455.0, 470.0, 490.0,
        210.0, 225.0, 240.0, 255.0, 270.0, 290.0,
    ]);

    let size = UInt32Array::from(vec![
        50, 60, 70, 80, 90, 100,
        50, 60, 70, 80, 90, 100,
        50, 60, 70, 80, 90, 100,
        50, 60, 70, 80, 90, 100,
    ]);

    let batch = RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(group),
            Arc::new(row_facet),
            Arc::new(col_facet),
            Arc::new(x),
            Arc::new(y),
            Arc::new(size),
        ],
    )
    .expect("create record batch");

    ctx.read_batch(batch).expect("create dataframe")
}

#[tokio::test]
async fn test_grid_facet_shared_in_row() {
    let ctx = SessionContext::new();
    let df = create_varying_data().await;

    // SharedInRow: y-scale shared within each row, independent across rows
    // This means:
    // - Row 1 (R1): Both C1 and C2 share the same y-scale [0-300]
    // - Row 2 (R2): Both C1 and C2 share the same y-scale [200-500]
    // - x-scale is free (each cell has its own scale)
    let outer = Plot::<GridFacet>::new()
        .data(df)
        .canvas_size(600, 400)
        .mark(
            Facet::new()
                .row(col("row_facet"))
                .col(col("col_facet"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x"))
                            .y_with(col("y"), |c| c.share_in_rows())  // Share within rows
                            .size(80.0)
                            .fill("#4682b4")
                    )
                )
        );

    let compiled = outer.compile(&ctx).await.expect("compile grid facet shared in row");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_shared_in_row").await;
}

#[tokio::test]
async fn test_grid_facet_shared_in_column() {
    let ctx = SessionContext::new();
    let df = create_varying_data().await;

    // SharedInColumn: x-scale shared within each column, independent across columns
    // This means:
    // - Column 1 (C1): Both R1 and R2 share the same x-scale [0-50]
    // - Column 2 (C2): Both R1 and R2 share the same x-scale [0-30]
    // - y-scale is free (each cell has its own scale)
    let outer = Plot::<GridFacet>::new()
        .data(df)
        .canvas_size(600, 400)
        .mark(
            Facet::new()
                .row(col("row_facet"))
                .col(col("col_facet"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("x"), |c| c.share_in_columns())  // Share within columns
                            .y(col("y"))
                            .size(80.0)
                            .fill("#4682b4")
                    )
                )
        );

    let compiled = outer.compile(&ctx).await.expect("compile grid facet shared in column");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_shared_in_column").await;
}

#[tokio::test]
async fn test_grid_facet_mixed_sharing() {
    let ctx = SessionContext::new();
    let df = create_varying_data().await;

    // Mixed sharing: x shared in columns, y shared in rows
    // This is the most interesting case:
    // - Each row has a shared y-scale (comparable across columns within a row)
    // - Each column has a shared x-scale (comparable across rows within a column)
    // - All subplots show axis labels only on edges
    let outer = Plot::<GridFacet>::new()
        .data(df)
        .canvas_size(600, 400)
        .mark(
            Facet::new()
                .row(col("row_facet"))
                .col(col("col_facet"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("x"), |c| c.share_in_columns())
                            .y_with(col("y"), |c| c.share_in_rows())
                            .size(80.0)
                            .fill("#4682b4")
                    )
                )
        );

    let compiled = outer.compile(&ctx).await.expect("compile grid facet mixed sharing");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_mixed_sharing").await;
}

#[tokio::test]
async fn test_grid_facet_compare_sharing_modes() {
    // This test creates a 2x2 comparison grid showing all four sharing combinations
    let ctx = SessionContext::new();
    let df = create_varying_data().await;

    // Free scales (default)
    let free_plot = Plot::<GridFacet>::new()
        .data(df.clone())
        .canvas_size(280, 280)
        .mark(
            Facet::new()
                .row(col("row_facet"))
                .col(col("col_facet"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x"))
                            .y(col("y"))
                            .size(80.0)
                            .fill("#4682b4")
                    )
                )
        );

    let compiled_free = free_plot.compile(&ctx).await.expect("compile free scales");
    assert_visual_match_default(&compiled_free, &ctx, None, "facet", "grid_facet_comparison_free").await;

    // Shared scales (both x and y shared)
    let shared_plot = Plot::<GridFacet>::new()
        .data(df.clone())
        .canvas_size(280, 280)
        .mark(
            Facet::new()
                .row(col("row_facet"))
                .col(col("col_facet"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("x"), |c| c.share())
                            .y_with(col("y"), |c| c.share())
                            .size(80.0)
                            .fill("#4682b4")
                    )
                )
        );

    let compiled_shared = shared_plot.compile(&ctx).await.expect("compile shared scales");
    assert_visual_match_default(&compiled_shared, &ctx, None, "facet", "grid_facet_comparison_shared").await;
}

#[tokio::test]
async fn test_grid_facet_axis_label_visibility() {
    // Test that verifies axis labels are hidden correctly for partial sharing
    let ctx = SessionContext::new();
    let df = create_varying_data().await;

    // With SharedInRow: y-axis labels should only show on left and right edges
    // With free x-scale: x-axis labels show on all bottom cells
    let outer = Plot::<GridFacet>::new()
        .data(df)
        .canvas_size(600, 400)
        .mark(
            Facet::new()
                .row(col("row_facet"))
                .col(col("col_facet"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x"))
                            .y_with(col("y"), |c| c.share_in_rows())
                            .size(80.0)
                            .fill("#4682b4")
                    )
                )
        );

    let compiled = outer.compile(&ctx).await.expect("compile for label visibility test");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_label_visibility").await;
}

#[tokio::test]
async fn test_grid_facet_shared_in_row_with_varying_sizes() {
    // Test SharedInRow with symbols of varying sizes to verify radius-aware domain inference
    let ctx = SessionContext::new();
    let df = create_varying_data().await;

    let outer = Plot::<GridFacet>::new()
        .data(df)
        .canvas_size(600, 400)
        .mark(
            Facet::new()
                .row(col("row_facet"))
                .col(col("col_facet"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("x"))
                            .y_with(col("y"), |c| c.share_in_rows())
                            .size(80.0)  // Varying sizes
                            .fill("#4682b4")
                    )
                )
        );

    let compiled = outer.compile(&ctx).await.expect("compile with varying sizes");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_shared_in_row_varying_sizes").await;
}

#[tokio::test]
async fn test_grid_facet_using_enum_directly() {
    // Test using ScaleSharing enum directly via share_scale() method
    let ctx = SessionContext::new();
    let df = create_varying_data().await;

    let outer = Plot::<GridFacet>::new()
        .data(df)
        .canvas_size(600, 400)
        .mark(
            Facet::new()
                .row(col("row_facet"))
                .col(col("col_facet"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("x"), |c| c.share_scale(ScaleSharing::SharedInColumn))
                            .y_with(col("y"), |c| c.share_scale(ScaleSharing::SharedInRow))
                            .size(80.0)
                            .fill("#4682b4")
                    )
                )
        );

    let compiled = outer.compile(&ctx).await.expect("compile using enum directly");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_enum_direct").await;
}
