// Visual tests for faceted legend support
// Tests various combinations of scale sharing modes with legends

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::legend::LegendPosition;
use avenger_chart::prelude::*;
use avenger_chart::scales::{Linear, Ordinal, ScaleRange};
use datafusion::prelude::*;
use palette::rgb::Srgba;

/// Test 1: Row facet with Free x/y scales + Free color legend
/// Each subplot should have its own color legend on the right side
#[tokio::test]
async fn test_facet_row_free_scales_with_free_color_legend() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.with_scale_sharing(Sharing::Free)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.with_scale_sharing(Sharing::Free)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_length"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.267, 0.004, 0.329, 1.0), // viridis start
                                    Srgba::new(0.127, 0.566, 0.550, 1.0), // viridis mid
                                    Srgba::new(0.993, 0.906, 0.144, 1.0), // viridis end
                                ]))
                            })
                            .legend(|l| l.title("Petal Length"))
                        })
                        .size(48.0),
                ),
            )
            .row(col("species")),
        )
        .canvas_size(700.0, 550.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_row_free_scales_with_free_color_legend",
    )
    .await;
}

/// Test: Row facet with Free x/y scales + Free color legend positioned on the LEFT
/// Tests that left-positioned legends align correctly across row facets
#[tokio::test]
async fn test_facet_row_free_scales_with_left_color_legend() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.with_scale_sharing(Sharing::Free)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.with_scale_sharing(Sharing::Free)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_length"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.267, 0.004, 0.329, 1.0), // viridis start
                                    Srgba::new(0.127, 0.566, 0.550, 1.0), // viridis mid
                                    Srgba::new(0.993, 0.906, 0.144, 1.0), // viridis end
                                ]))
                            })
                            .legend(|l| l.title("Petal Length").position(LegendPosition::Left))
                        })
                        .size(48.0),
                ),
            )
            .row(col("species")),
        )
        .canvas_size(700.0, 550.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_row_free_scales_with_left_color_legend",
    )
    .await;
}

/// Test 2: Row facet with Shared x/y scales
/// Since position scales are Shared, legends are NOT rendered in subplots
/// (outer-level legend rendering is not yet implemented)
#[tokio::test]
async fn test_facet_row_shared_scales_with_shared_color_legend() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.with_scale_sharing(Sharing::Shared)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.with_scale_sharing(Sharing::Shared)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_length"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.050, 0.030, 0.529, 1.0), // plasma start
                                    Srgba::new(0.790, 0.223, 0.477, 1.0), // plasma mid
                                    Srgba::new(0.940, 0.975, 0.131, 1.0), // plasma end
                                ]))
                            })
                            .legend(|l| l.title("Petal Length"))
                        })
                        .size(48.0),
                ),
            )
            .row(col("species")),
        )
        .canvas_size(700.0, 550.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_row_shared_scales_with_shared_color_legend",
    )
    .await;
}

/// Test 3: Row facet with mixed sharing modes
/// Free color legend per subplot, Shared x/y axes
#[tokio::test]
async fn test_facet_row_mixed_sharing_free_color_shared_axes() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.with_scale_sharing(Sharing::Shared)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.with_scale_sharing(Sharing::Shared)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_length"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.190, 0.071, 0.231, 1.0), // turbo start
                                    Srgba::new(0.251, 0.720, 0.945, 1.0), // turbo mid-low
                                    Srgba::new(0.984, 0.906, 0.137, 1.0), // turbo mid-high
                                    Srgba::new(0.478, 0.016, 0.000, 1.0), // turbo end
                                ]))
                            })
                            .legend(|l| l.title("Petal Length (Free)"))
                        })
                        .size(48.0),
                ),
            )
            .row(col("species")),
        )
        .canvas_size(700.0, 550.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_row_mixed_sharing_free_color_shared_axes",
    )
    .await;
}

/// Test 4: Column facet with Shared color legend
/// One legend outside the facet grid with ordinal fill channel
#[tokio::test]
async fn test_facet_col_shared_color_legend() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Register the dataframe and add a categorical column based on petal_width ranges
    ctx.register_table("iris", df.into_view())
        .expect("register iris table");

    let df = ctx
        .sql(
            "SELECT *,
                CASE
                    WHEN petal_width < 0.8 THEN 'Small'
                    WHEN petal_width < 1.8 THEN 'Medium'
                    ELSE 'Large'
                END as petal_size
            FROM iris",
        )
        .await
        .expect("add categorical column");

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.with_scale_sharing(Sharing::Shared)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.with_scale_sharing(Sharing::Shared)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_size"), |c| {
                            c.scale_with::<Ordinal>(|s| s)
                                .legend(|l| l.title("Petal Size"))
                        })
                        .size(48.0),
                ),
            )
            .column(col("species")),
        )
        .canvas_size(800.0, 400.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_col_shared_color_legend",
    )
    .await;
}

/// Test: Column facet with legend on LEFT
/// Verifies spacing between facet cells accounts for left-positioned legend
#[tokio::test]
async fn test_facet_col_legend_left() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_length"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.267, 0.004, 0.329, 1.0),
                                    Srgba::new(0.127, 0.566, 0.550, 1.0),
                                    Srgba::new(0.993, 0.906, 0.144, 1.0),
                                ]))
                            })
                            .legend(|l| l.title("Petal Length").position(LegendPosition::Left))
                        })
                        .size(48.0),
                ),
            )
            .column(col("species")),
        )
        .canvas_size(800.0, 400.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_col_legend_left",
    )
    .await;
}

/// Test: Column facet with legend on RIGHT
/// Verifies spacing between facet cells accounts for right-positioned legend
#[tokio::test]
async fn test_facet_col_legend_right() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_length"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.267, 0.004, 0.329, 1.0),
                                    Srgba::new(0.127, 0.566, 0.550, 1.0),
                                    Srgba::new(0.993, 0.906, 0.144, 1.0),
                                ]))
                            })
                            .legend(|l| l.title("Petal Length").position(LegendPosition::Right))
                        })
                        .size(48.0),
                ),
            )
            .column(col("species")),
        )
        .canvas_size(800.0, 400.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_col_legend_right",
    )
    .await;
}

/// Test: Column facet with legend on TOP
/// Verifies spacing between facet cells accounts for top-positioned legend
#[tokio::test]
async fn test_facet_col_legend_top() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_length"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.267, 0.004, 0.329, 1.0),
                                    Srgba::new(0.127, 0.566, 0.550, 1.0),
                                    Srgba::new(0.993, 0.906, 0.144, 1.0),
                                ]))
                            })
                            .legend(|l| l.title("Petal Length").position(LegendPosition::Top))
                        })
                        .size(48.0),
                ),
            )
            .column(col("species")),
        )
        .canvas_size(800.0, 450.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_col_legend_top",
    )
    .await;
}

/// Test: Column facet with legend on BOTTOM
/// Verifies spacing between facet cells accounts for bottom-positioned legend
#[tokio::test]
async fn test_facet_col_legend_bottom() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .fill_with(col("petal_length"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.267, 0.004, 0.329, 1.0),
                                    Srgba::new(0.127, 0.566, 0.550, 1.0),
                                    Srgba::new(0.993, 0.906, 0.144, 1.0),
                                ]))
                            })
                            .legend(|l| l.title("Petal Length").position(LegendPosition::Bottom))
                        })
                        .size(48.0),
                ),
            )
            .column(col("species")),
        )
        .canvas_size(800.0, 550.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_col_legend_bottom",
    )
    .await;
}

/// Test 5: Small 2-row facet with Free legends
/// Tests with fewer facets
#[tokio::test]
async fn test_facet_row_two_rows_free_legend() {
    let ctx = SessionContext::new();

    // Create simple test data with just 2 categories
    ctx.sql(
        "CREATE TABLE test_data AS VALUES
        (1.0, 2.0, 3.0, 'A'),
        (1.5, 2.5, 3.5, 'A'),
        (2.0, 3.0, 4.0, 'A'),
        (2.5, 3.5, 4.5, 'A'),
        (4.0, 5.0, 6.0, 'B'),
        (4.5, 5.5, 6.5, 'B'),
        (5.0, 6.0, 7.0, 'B'),
        (5.5, 6.5, 7.5, 'B')",
    )
    .await
    .expect("create test data");

    let df = ctx.table("test_data").await.expect("load test data");

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("column1"), |c| {
                            c.with_scale_sharing(Sharing::Free)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("X"))
                        })
                        .y_with(col("column2"), |c| {
                            c.with_scale_sharing(Sharing::Free)
                                .scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Y"))
                        })
                        .fill_with(col("column3"), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.range(ScaleRange::new_color(vec![
                                    Srgba::new(0.968, 0.984, 1.000, 1.0), // blues light
                                    Srgba::new(0.419, 0.682, 0.839, 1.0), // blues mid
                                    Srgba::new(0.031, 0.188, 0.420, 1.0), // blues dark
                                ]))
                            })
                            .legend(|l| l.title("Value"))
                        })
                        .size(60.0),
                ),
            )
            .row(col("column4")),
        )
        .canvas_size(600.0, 400.0);

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legends",
        "facet_row_two_rows_free_legend",
    )
    .await;
}
