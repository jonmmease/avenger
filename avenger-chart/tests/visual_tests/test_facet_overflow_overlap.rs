use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

/// Test that facet rows with varied overflow don't overlap
///
/// This test creates a FacetRow where each row has different Y-axis tick labels.
/// Row 1 uses short labels ("A", "B", "C") while Row 2 uses long labels
/// ("Very Long Category Name 1", etc). This triggers different left overflow
/// per row. The gap computation must accommodate the maximum overflow to
/// prevent axis labels from overlapping between rows.
///
/// This is a regression test for the overflow overlap bug that occurred
/// when gaps were computed from the wrong row's overflow values.
#[tokio::test]
async fn facet_row_varied_overflow() {
    let ctx = SessionContext::new();

    // Create inline data with two categories that will become facet rows.
    // row_category = "short_labels" has short Y-axis values ("A", "B", "C")
    // row_category = "long_labels" has long Y-axis values that need more space
    let sql = r#"
        SELECT * FROM (VALUES
            ('short_labels', 'A', 10.0),
            ('short_labels', 'B', 20.0),
            ('short_labels', 'C', 30.0),
            ('long_labels', 'Very Long Category Name 1', 15.0),
            ('long_labels', 'Very Long Category Name 2', 25.0),
            ('long_labels', 'Very Long Category Name 3', 35.0)
        ) AS t(row_category, y_label, bar_val)
    "#;

    let df = ctx.sql(sql).await.expect("create data");

    // Build facet row plot with ordinal Y-axis (band scale)
    // The Y-axis will show y_label which varies in length by row_category
    let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 400).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .y_with(col("y_label"), |c| {
                        c.scale_with::<Band>(|s| s)
                            .axis(|a| a.title("Y Axis").grid(false))
                    })
                    .y2_with(col(":y"), |c| c.band(1.0))
                    .x_with(lit(0.0), |c| {
                        c.scale(|s| s.domain((0.0, 40.0)))
                            .axis(|a| a.title("Value").grid(true))
                    })
                    .x2(col("bar_val"))
                    .fill("#4682b4")
                    .stroke("#2c5282")
                    .stroke_width(1.0),
            ),
        )
        .row_with(col("row_category"), |c| c.guide(|g| g.title("Category"))),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_varied_overflow").await;
}

/// Test FacetColumn with varied bottom overflow across columns
///
/// Each column has different X-axis tick labels with varied lengths.
/// This ensures the column gap computation accommodates maximum overflow.
#[tokio::test]
async fn facet_col_varied_overflow() {
    let ctx = SessionContext::new();

    // Create data where each column has different X-axis label lengths
    let sql = r#"
        SELECT * FROM (VALUES
            ('col_short', 'A', 10.0),
            ('col_short', 'B', 20.0),
            ('col_short', 'C', 30.0),
            ('col_long', 'Very Long X Label 1', 15.0),
            ('col_long', 'Very Long X Label 2', 25.0),
            ('col_long', 'Very Long X Label 3', 35.0)
        ) AS t(col_category, x_label, y_val)
    "#;

    let df = ctx.sql(sql).await.expect("create data");

    // Build facet column plot with ordinal X-axis (band scale)
    let outer = Plot::<FacetColumn>::new()
        .data(df)
        .canvas_size(600, 400)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .x_with(col("x_label"), |c| {
                            c.scale_with::<Band>(|s| s)
                                .axis(|a| a.title("X Axis").grid(false))
                        })
                        .x2_with(col(":x"), |c| c.band(1.0))
                        .y_with(lit(0.0), |c| {
                            c.scale(|s| s.domain((0.0, 40.0)))
                                .axis(|a| a.title("Value").grid(true))
                        })
                        .y2(col("y_val"))
                        .fill("#4682b4")
                        .stroke("#2c5282")
                        .stroke_width(1.0),
                ),
            )
            .col_with(col("col_category"), |c| c.guide(|g| g.title("Column"))),
        );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_varied_overflow").await;
}
