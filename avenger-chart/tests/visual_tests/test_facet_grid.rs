use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

/// Helper to load iris with binned petal_width column
async fn iris_with_binned_petal_width() -> datafusion::dataframe::DataFrame {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Create binned petal_width column
    let binned_df = df
        .with_column(
            "petal_width_bin",
            when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
                .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
                .otherwise(lit("wide"))
                .unwrap(),
        )
        .unwrap();

    binned_df
}

#[tokio::test]
async fn test_grid_facet_basic() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let iris = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Add binned column using when/otherwise
    let df = iris
        .with_column(
            "length_bin",
            when(col("sepal_length").lt(lit(5.5)), lit("short"))
                .when(col("sepal_length").lt(lit(6.5)), lit("medium"))
                .otherwise(lit("long"))
                .unwrap(),
        )
        .unwrap();

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("length_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("sepal_length"))
                            .y(col("sepal_width"))
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer.compile(&ctx).await.expect("compile grid facet");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_basic").await;
}

#[tokio::test]
async fn test_grid_facet_with_titles() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
                .col_with(col("petal_width_bin"), |c| {
                    c.facet(|f| f.title("Petal Width"))
                })
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("sepal_length"))
                            .y(col("sepal_width"))
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet with titles");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_with_titles").await;
}

#[tokio::test]
async fn test_grid_facet_shared_both() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("sepal_length"), |c| {
                                c.with_scale_sharing(ScaleSharing::Shared)
                            })
                            .y_with(col("sepal_width"), |c| {
                                c.with_scale_sharing(ScaleSharing::Shared)
                            })
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet shared both");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_shared_both").await;
}

#[tokio::test]
async fn test_grid_facet_free_scales() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("sepal_length"), |c| {
                                c.with_scale_sharing(ScaleSharing::Free)
                            })
                            .y_with(col("sepal_width"), |c| {
                                c.with_scale_sharing(ScaleSharing::Free)
                            })
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet free scales");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_free_scales").await;
}

#[tokio::test]
async fn test_grid_facet_shared_x() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("sepal_length"), |c| {
                                c.with_scale_sharing(ScaleSharing::Shared)
                            })
                            .y_with(col("sepal_width"), |c| {
                                c.with_scale_sharing(ScaleSharing::Free)
                            })
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet shared x");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_shared_x").await;
}

#[tokio::test]
async fn test_grid_facet_shared_y() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("sepal_length"), |c| {
                                c.with_scale_sharing(ScaleSharing::Free)
                            })
                            .y_with(col("sepal_width"), |c| {
                                c.with_scale_sharing(ScaleSharing::Shared)
                            })
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet shared y");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_shared_y").await;
}

#[tokio::test]
async fn test_grid_facet_with_unified_titles() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
                .col_with(col("petal_width_bin"), |c| {
                    c.facet(|f| f.title("Petal Width"))
                })
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("sepal_length"), |c| {
                                c.with_scale_sharing(ScaleSharing::Shared)
                                    .axis(|a| a.title("Sepal Length (cm)"))
                            })
                            .y_with(col("sepal_width"), |c| {
                                c.with_scale_sharing(ScaleSharing::Shared)
                                    .axis(|a| a.title("Sepal Width (cm)"))
                            })
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet with unified titles");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "grid_facet_with_unified_titles",
    )
    .await;
}

#[tokio::test]
async fn test_grid_facet_x_axis_top() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("sepal_length"), |c| c.axis(|a| a.position("top")))
                            .y(col("sepal_width"))
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet x axis top");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_x_axis_top").await;
}

#[tokio::test]
#[ignore] // TODO: Support degenerate GridFacet with single value in row dimension
// Issue: When filtering to one species, no 'row' band scale is created (needs >=2 values)
// This causes "Scale 'row' not found for channel 'row'" error during evaluation
async fn test_grid_facet_single_row() {
    let ctx = SessionContext::new();
    // Degenerate case: filter to just one species
    let df = iris_with_binned_petal_width()
        .await
        .filter(col("species").eq(lit("setosa")))
        .unwrap();

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("sepal_length"))
                            .y(col("sepal_width"))
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet single row");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_single_row").await;
}

#[tokio::test]
async fn test_grid_facet_with_line_mark() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Line::new()
                            .x(col("sepal_length"))
                            .y(col("sepal_width"))
                            .stroke("#4682b4")
                            .stroke_width(2.0),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet with line mark");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_with_line_mark").await;
}

#[tokio::test]
async fn test_grid_facet_custom_spacing() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row_with(col("species"), |c| c.facet(|f| f.spacing(20.0)))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("sepal_length"))
                            .y(col("sepal_width"))
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet custom spacing");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_custom_spacing").await;
}

#[tokio::test]
async fn test_grid_facet_hybrid_sharing() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    // Test hybrid scale sharing: x shared, y free
    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x_with(col("sepal_length"), |c| {
                                c.with_scale_sharing(ScaleSharing::Shared)
                            })
                            .y_with(col("sepal_width"), |c| {
                                c.with_scale_sharing(ScaleSharing::Free)
                            })
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet hybrid sharing");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_hybrid_sharing").await;
}

#[tokio::test]
async fn test_grid_facet_y_axis_right() {
    let ctx = SessionContext::new();
    let df = iris_with_binned_petal_width().await;

    let outer = Plot::<FacetGrid>::new()
        .data(df)
        .canvas_size(600, 600)
        .mark(
            Facet::new()
                .row(col("species"))
                .col(col("petal_width_bin"))
                .subplot(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("sepal_length"))
                            .y_with(col("sepal_width"), |c| c.axis(|a| a.position("right")))
                            .size(25.0)
                            .fill("#4682b4"),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile grid facet y axis right");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "grid_facet_y_axis_right").await;
}
