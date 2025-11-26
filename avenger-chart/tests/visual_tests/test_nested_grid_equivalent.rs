//! Tests porting Grid Facet functionality to Nested FacetColumn + FacetRow
//!
//! These tests demonstrate that nested facets (FacetColumn containing FacetRow,
//! or vice versa) can achieve the same visual results as FacetGrid.
//!
//! Milestone 1: Free Scale Tests
//! - test_nested_free_row_free_scales: Port of test_grid_facet_free_scales
//! - test_nested_free_row_with_line_mark: Port of test_grid_facet_with_line_mark
//! - test_nested_free_row_custom_spacing: Port of test_grid_facet_custom_spacing

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
    df.with_column(
        "petal_width_bin",
        when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
            .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
            .otherwise(lit("wide"))
            .unwrap(),
    )
    .unwrap()
}

/// Build a runtime with larger stack for nested facet tests
fn build_test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(64 * 1024 * 1024) // 64 MB
        .enable_all()
        .build()
        .expect("build runtime")
}

// =============================================================================
// Milestone 1: Free Scale Tests
// =============================================================================

/// Port of test_grid_facet_free_scales
///
/// Nested facet version using FacetColumn (outer) containing FacetRow (inner).
/// With Free scale sharing, each subplot computes its own scale domain.
#[test]
fn test_nested_free_row_free_scales() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Outer: FacetColumn by petal_width_bin (3 columns)
        // Inner: FacetRow by species (3 rows per column)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
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
                        ),
                    ),
            );

        let compiled = outer.compile(&ctx).await.expect("compile nested free scales");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_free_scales",
        )
        .await;
    });
}

/// Port of test_grid_facet_with_line_mark
///
/// Tests that nested facets work correctly with Line marks.
#[test]
fn test_nested_free_row_with_line_mark() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Line::new()
                                            .x(col("sepal_length"))
                                            .y(col("sepal_width"))
                                            .stroke("#4682b4")
                                            .stroke_width(2.0),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested with line mark");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_with_line_mark",
        )
        .await;
    });
}

/// Port of test_grid_facet_custom_spacing
///
/// Tests that custom spacing is respected in nested facets.
#[test]
fn test_nested_free_row_custom_spacing() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("species"), |c| c.facet(|f| f.spacing(20.0)))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x(col("sepal_length"))
                                            .y(col("sepal_width"))
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested custom spacing");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_custom_spacing",
        )
        .await;
    });
}

// =============================================================================
// Milestone 2: Data Channel Scale Sharing (Shared scales for x/y)
// =============================================================================

/// Port of test_grid_facet_shared_both
///
/// Tests shared scale domains for both x and y channels across all subplots.
#[test]
fn test_nested_free_row_shared_both() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
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
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared both");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_both",
        )
        .await;
    });
}

/// Port of test_grid_facet_shared_x
///
/// Tests shared scale domain for x channel only, free y scales.
#[test]
fn test_nested_free_row_shared_x() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
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
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared x");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "nested_free_row_shared_x").await;
    });
}

/// Port of test_grid_facet_shared_y
///
/// Tests shared scale domain for y channel only, free x scales.
#[test]
fn test_nested_free_row_shared_y() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
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
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared y");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "nested_free_row_shared_y").await;
    });
}

// =============================================================================
// Milestone 5: Guide Ownership / Unified Rendering
// =============================================================================

/// Port of test_grid_facet_basic
///
/// Basic nested facet test without explicit scale sharing (default behavior).
#[test]
fn test_nested_free_row_basic() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
        let iris = ctx
            .read_parquet(iris_path, ParquetReadOptions::default())
            .await
            .expect("load iris dataset");

        // Add binned column
        let df = iris
            .with_column(
                "length_bin",
                when(col("sepal_length").lt(lit(5.5)), lit("short"))
                    .when(col("sepal_length").lt(lit(6.5)), lit("medium"))
                    .otherwise(lit("long"))
                    .unwrap(),
            )
            .unwrap();

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("length_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x(col("sepal_length"))
                                            .y(col("sepal_width"))
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer.compile(&ctx).await.expect("compile nested basic");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "nested_free_row_basic").await;
    });
}

/// Port of test_grid_facet_with_titles
///
/// Tests facet titles on nested facets.
#[test]
fn test_nested_free_row_with_titles() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .col_with(col("petal_width_bin"), |c| {
                        c.facet(|f| f.title("Petal Width"))
                    })
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x(col("sepal_length"))
                                            .y(col("sepal_width"))
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested with titles");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_with_titles",
        )
        .await;
    });
}

/// Port of test_grid_facet_with_unified_titles
///
/// Tests unified axis titles with shared scales in nested facets.
#[test]
fn test_nested_free_row_with_unified_titles() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .col_with(col("petal_width_bin"), |c| {
                        c.facet(|f| f.title("Petal Width"))
                    })
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("species"), |c| c.facet(|f| f.title("Species")))
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
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested with unified titles");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_with_unified_titles",
        )
        .await;
    });
}

// =============================================================================
// Milestone 8: Axis Position Variants
// =============================================================================

/// Port of test_grid_facet_x_axis_top
///
/// Tests x axis positioned at top in nested facets.
#[test]
fn test_nested_free_row_x_axis_top() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.axis(|a| a.position("top"))
                                            })
                                            .y(col("sepal_width"))
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested x axis top");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_x_axis_top",
        )
        .await;
    });
}

/// Port of test_grid_facet_y_axis_right
///
/// Tests y axis positioned at right in nested facets.
#[test]
fn test_nested_free_row_y_axis_right() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x(col("sepal_length"))
                                            .y_with(col("sepal_width"), |c| {
                                                c.axis(|a| a.position("right"))
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested y axis right");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_y_axis_right",
        )
        .await;
    });
}

/// Port of test_grid_facet_hybrid_sharing
///
/// Tests hybrid scale sharing: x shared, y free.
#[test]
fn test_nested_free_row_hybrid_sharing() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
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
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested hybrid sharing");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_hybrid_sharing",
        )
        .await;
    });
}

// =============================================================================
// Milestone 9: SharedInRow Scale Sharing
// =============================================================================

/// Tests SharedInRow scale sharing mode
///
/// With SharedInRow, cells in the same row share scale domains across columns.
/// For FacetColumn(FacetRow(Cartesian)), this means:
/// - All "setosa" cells across columns share x/y domains (computed from all setosa data)
/// - All "versicolor" cells across columns share x/y domains (computed from all versicolor data)
/// - All "virginica" cells across columns share x/y domains (computed from all virginica data)
///
/// This is useful when you want to compare values across columns within each row.
#[test]
fn test_nested_free_row_shared_in_row_both() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Outer: FacetColumn by petal_width_bin (3 columns)
        // Inner: FacetRow by species (3 rows per column)
        // SharedInRow: each row (species) shares scales across columns
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInRow)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInRow)
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared in row both");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_in_row_both",
        )
        .await;
    });
}

/// Tests SharedInRow for x channel only, free y scales
///
/// Each row shares x domain across columns, but y is free per cell.
#[test]
fn test_nested_free_row_shared_in_row_x() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInRow)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared in row x");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_in_row_x",
        )
        .await;
    });
}

/// Tests SharedInRow for y channel only, free x scales
///
/// Each row shares y domain across columns, but x is free per cell.
#[test]
fn test_nested_free_row_shared_in_row_y() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInRow)
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared in row y");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_in_row_y",
        )
        .await;
    });
}

/// Tests mixed scale sharing: x globally shared, y shared in row
///
/// This tests combining different sharing modes on different channels.
#[test]
fn test_nested_free_row_mixed_shared_and_shared_in_row() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInRow)
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested mixed shared and shared in row");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_mixed_shared_and_shared_in_row",
        )
        .await;
    });
}

// =============================================================================
// Milestone 10: SharedInColumn Scale Sharing
// =============================================================================

/// Tests SharedInColumn scale sharing mode
///
/// With SharedInColumn, cells in the same column share scale domains.
/// For FacetColumn(FacetRow(Cartesian)), this means:
/// - All cells in the "narrow" column share x/y domains (computed from narrow data)
/// - All cells in the "medium" column share x/y domains (computed from medium data)
/// - All cells in the "wide" column share x/y domains (computed from wide data)
///
/// This is useful when you want to compare values across rows within each column.
#[test]
fn test_nested_free_row_shared_in_column_both() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Outer: FacetColumn by petal_width_bin (3 columns)
        // Inner: FacetRow by species (3 rows per column)
        // SharedInColumn: each column shares scales across rows
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInColumn)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInColumn)
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared in column both");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_in_column_both",
        )
        .await;
    });
}

/// Tests SharedInColumn for x channel only, free y scales
///
/// Each column shares x domain across rows, but y is free per cell.
#[test]
fn test_nested_free_row_shared_in_column_x() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInColumn)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared in column x");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_in_column_x",
        )
        .await;
    });
}

/// Tests SharedInColumn for y channel only, free x scales
///
/// Each column shares y domain across rows, but x is free per cell.
#[test]
fn test_nested_free_row_shared_in_column_y() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInColumn)
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared in column y");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_in_column_y",
        )
        .await;
    });
}

/// Tests mixed scale sharing: x shared in column, y shared in row
///
/// This tests combining SharedInColumn and SharedInRow on different channels.
#[test]
fn test_nested_free_row_mixed_shared_in_column_and_shared_in_row() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("petal_width_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row(col("species"))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("sepal_length"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInColumn)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::SharedInRow)
                                            })
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested mixed shared in column and shared in row");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_mixed_shared_in_column_and_shared_in_row",
        )
        .await;
    });
}

// =============================================================================
// Milestone 11: Shared Row Tests (Grid-equivalent behavior)
// =============================================================================
//
// These tests use `.share_scale()` on the row facet variable, which causes the
// inner facet to use a shared domain computed from the full dataset. This produces
// grid-like behavior where all columns show the same rows (species), even if some
// cells have no data. This should match the corresponding FacetGrid baselines.

/// Helper to load iris with length_bin column (matching grid_facet_basic)
async fn iris_with_length_bin() -> datafusion::dataframe::DataFrame {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Create length_bin column matching grid facet tests
    df.with_column(
        "length_bin",
        when(col("sepal_length").lt(lit(5.5)), lit("short"))
            .when(col("sepal_length").lt(lit(6.5)), lit("medium"))
            .otherwise(lit("long"))
            .unwrap(),
    )
    .unwrap()
}

/// Test nested facets with shared row domain (grid-like behavior)
///
/// This should produce output matching grid_facet_basic.png:
/// - All columns show all three species (setosa, versicolor, virginica)
/// - Cells with no data show empty plots with axes
#[test]
fn test_nested_shared_row_basic() {
    let rt = build_test_runtime();

    rt.block_on(async {
        let ctx = SessionContext::new();
        let df = iris_with_length_bin().await;

        // Outer: FacetColumn by length_bin (3 columns)
        // Inner: FacetRow by species with SHARED domain (grid-like)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new()
                    .column(col("length_bin"))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                // KEY: share_scale() causes domain to be computed from full dataset
                                .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x(col("sepal_length"))
                                            .y(col("sepal_width"))
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared row basic");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_basic",
        )
        .await;
    });
}
