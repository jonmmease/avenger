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

/// Run an async block on a thread with larger stack for nested facet tests.
/// The current_thread runtime runs everything on its own thread, which we create
/// with a large stack using std::thread::Builder.
fn run_with_large_stack<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024) // 64 MB
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build runtime");
            rt.block_on(f());
        })
        .expect("failed to spawn thread")
        .join()
        .expect("thread panicked");
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Outer: FacetColumn by petal_width_bin (3 columns)
        // Inner: FacetRow by species (3 rows per column)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested free scales");
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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

        let compiled = outer.compile(&ctx).await.expect("compile nested shared x");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_x",
        )
        .await;
    });
}

/// Port of test_grid_facet_shared_y
///
/// Tests shared scale domain for y channel only, free x scales.
#[test]
fn test_nested_free_row_shared_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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

        let compiled = outer.compile(&ctx).await.expect("compile nested shared y");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_shared_y",
        )
        .await;
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
    run_with_large_stack(|| async {
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
                Facet::new().column(col("length_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_free_row_basic",
        )
        .await;
    });
}

/// Port of test_grid_facet_with_titles
///
/// Tests facet titles on nested facets.
#[test]
fn test_nested_free_row_with_titles() {
    run_with_large_stack(|| async {
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
    run_with_large_stack(|| async {
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| c.axis(|a| a.position("top")))
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x(col("sepal_length"))
                                    .y_with(col("sepal_width"), |c| c.axis(|a| a.position("right")))
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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
// Milestone 9: Level(1) Row-Based Scale Sharing (formerly SharedInRow)
// =============================================================================
// MIGRATION NOTE: SharedInRow has been replaced with Level(1) in a restructured
// FacetRow > FacetColumn layout. To achieve row-based sharing (all cells in the
// same row share scale domains), the outer facet must be FacetRow.

/// Tests Level(1) row-based scale sharing mode (migrated from SharedInRow)
///
/// RESTRUCTURED: Changed from FacetColumn > FacetRow to FacetRow > FacetColumn.
/// With Level(1) in FacetRow > FacetColumn structure, cells share scale domains
/// with their parent row:
/// - All "setosa" cells across columns share x/y domains (computed from all setosa data)
/// - All "versicolor" cells across columns share x/y domains (computed from all versicolor data)
/// - All "virginica" cells across columns share x/y domains (computed from all virginica data)
///
/// This is useful when you want to compare values across columns within each row.
#[test]
fn test_nested_free_row_shared_in_row_both() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // RESTRUCTURED: FacetRow > FacetColumn (was FacetColumn > FacetRow)
        // Outer: FacetRow by species (3 rows)
        // Inner: FacetColumn by petal_width_bin (3 columns per row)
        // Level(1): each row shares scales across columns
        let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("species")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("petal_width_bin")).subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Level(1))
                                })
                                .y_with(col("sepal_width"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Level(1))
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

/// Tests Level(1) for x channel only, free y scales (migrated from SharedInRow)
///
/// RESTRUCTURED: FacetRow > FacetColumn layout.
/// Each row shares x domain across columns, but y is free per cell.
#[test]
fn test_nested_free_row_shared_in_row_x() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("species")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("petal_width_bin")).subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Level(1))
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

/// Tests Level(1) for y channel only, free x scales (migrated from SharedInRow)
///
/// RESTRUCTURED: FacetRow > FacetColumn layout.
/// Each row shares y domain across columns, but x is free per cell.
#[test]
fn test_nested_free_row_shared_in_row_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("species")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("petal_width_bin")).subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Free)
                                })
                                .y_with(col("sepal_width"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Level(1))
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

/// Tests mixed scale sharing: x globally shared, y Level(1) row-based (migrated from SharedInRow)
///
/// RESTRUCTURED: FacetRow > FacetColumn layout.
/// This tests combining different sharing modes on different channels.
#[test]
fn test_nested_free_row_mixed_shared_and_shared_in_row() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("species")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("petal_width_bin")).subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Shared)
                                })
                                .y_with(col("sepal_width"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Level(1))
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
// Milestone 10: Level(1) Scale Sharing (formerly SharedInColumn)
// =============================================================================
// MIGRATION NOTE: SharedInColumn has been replaced with Level(1).
// In a FacetColumn > FacetRow structure, Level(1) shares scales with the
// immediate parent (the FacetColumn), achieving the same column-based sharing.

/// Tests Level(1) scale sharing mode (migrated from SharedInColumn)
///
/// With Level(1) in FacetColumn > FacetRow structure, cells share scale domains
/// with their immediate parent column:
/// - All cells in the "narrow" column share x/y domains (computed from narrow data)
/// - All cells in the "medium" column share x/y domains (computed from medium data)
/// - All cells in the "wide" column share x/y domains (computed from wide data)
///
/// This is useful when you want to compare values across rows within each column.
#[test]
fn test_nested_free_row_shared_in_column_both() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Outer: FacetColumn by petal_width_bin (3 columns)
        // Inner: FacetRow by species (3 rows per column)
        // Level(1): each column shares scales across rows (same as SharedInColumn)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
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

/// Tests Level(1) for x channel only, free y scales (migrated from SharedInColumn)
///
/// Each column shares x domain across rows, but y is free per cell.
#[test]
fn test_nested_free_row_shared_in_column_x() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
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

/// Tests Level(1) for y channel only, free x scales (migrated from SharedInColumn)
///
/// Each column shares y domain across rows, but x is free per cell.
#[test]
fn test_nested_free_row_shared_in_column_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Free)
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
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

/// Tests mixed scale sharing: x Level(1) (column sharing), y Shared (global)
///
/// MIGRATION NOTE: The original test used SharedInColumn for x and SharedInRow for y.
/// With Level(N), mixing column and row sharing in a single structure requires
/// different approaches. Here we use Level(1) for x (shares with parent column)
/// and Shared for y (global sharing as a simpler alternative to SharedInRow).
#[test]
fn test_nested_free_row_mixed_shared_in_column_and_shared_in_row() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        // Use Shared (global) instead of SharedInRow
                                        // True row-sharing would require FacetRow > FacetColumn structure
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
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_length_bin().await;

        // Outer: FacetColumn by length_bin (3 columns)
        // Inner: FacetRow by species with SHARED domain (grid-like)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("length_bin")).subplot(
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

/// Port of test_grid_facet_with_titles with shared row domain
#[test]
fn test_nested_shared_row_with_titles() {
    run_with_large_stack(|| async {
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
                                .row_with(col("species"), |c| {
                                    c.facet(|f| f.title("Species").share_scale())
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
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared row with titles");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_with_titles",
        )
        .await;
    });
}

/// Port of test_grid_facet_shared_both with shared row domain
#[test]
fn test_nested_shared_row_shared_both() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
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
            .expect("compile nested shared row shared both");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_shared_both",
        )
        .await;
    });
}

/// Port of test_grid_facet_free_scales with shared row domain
#[test]
fn test_nested_shared_row_free_scales() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
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

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared row free scales");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_free_scales",
        )
        .await;
    });
}

/// Port of test_grid_facet_shared_x with shared row domain
#[test]
fn test_nested_shared_row_shared_x() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
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
            .expect("compile nested shared row shared x");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_shared_x",
        )
        .await;
    });
}

/// Port of test_grid_facet_shared_y with shared row domain
#[test]
fn test_nested_shared_row_shared_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
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
            .expect("compile nested shared row shared y");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_shared_y",
        )
        .await;
    });
}

/// Port of test_grid_facet_with_unified_titles with shared row domain
#[test]
fn test_nested_shared_row_with_unified_titles() {
    run_with_large_stack(|| async {
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
                                .row_with(col("species"), |c| {
                                    c.facet(|f| f.title("Species").share_scale())
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
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared row with unified titles");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_with_unified_titles",
        )
        .await;
    });
}

/// Port of test_grid_facet_x_axis_top with shared row domain
#[test]
fn test_nested_shared_row_x_axis_top() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
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
            .expect("compile nested shared row x axis top");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_x_axis_top",
        )
        .await;
    });
}

/// Port of test_grid_facet_y_axis_right with shared row domain
#[test]
fn test_nested_shared_row_y_axis_right() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
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
            .expect("compile nested shared row y axis right");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_y_axis_right",
        )
        .await;
    });
}

/// Port of test_grid_facet_with_line_mark with shared row domain
#[test]
fn test_nested_shared_row_with_line_mark() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
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
            .expect("compile nested shared row with line mark");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_with_line_mark",
        )
        .await;
    });
}

/// Port of test_grid_facet_custom_spacing with shared row domain
#[test]
fn test_nested_shared_row_custom_spacing() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| {
                                c.facet(|f| f.spacing(20.0).share_scale())
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
                    ),
                ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile nested shared row custom spacing");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_custom_spacing",
        )
        .await;
    });
}

/// Port of test_grid_facet_hybrid_sharing with shared row domain
#[test]
fn test_nested_shared_row_hybrid_sharing() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("species"), |c| c.facet(|f| f.share_scale()))
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
            .expect("compile nested shared row hybrid sharing");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_row_hybrid_sharing",
        )
        .await;
    });
}

// =============================================================================
// Milestone 12: Row(Column) Pattern - Inverted Nesting Order
// =============================================================================
//
// These tests use FacetRow as outer and FacetColumn as inner (inverted pattern).
// The coordinated_spacing["inter_col_gap"] coordination should ensure columns align
// vertically across all rows, symmetric with the Column(Row) pattern.

/// Test Row(Column(Cartesian)) with shared column scale
///
/// This tests the symmetric case where:
/// - Outer: FacetRow by species (3 rows)
/// - Inner: FacetColumn by petal_width_bin (3 columns per row)
/// - Column scale is shared so all rows have same columns
///
/// The columns should align vertically across rows.
#[test]
fn test_nested_shared_col_shared_both() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Outer: FacetRow by species (3 rows)
        // Inner: FacetColumn by petal_width_bin with SHARED domain (grid-like)
        let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("species")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new()
                        // KEY: share_scale() causes domain to be computed from full dataset
                        .col_with(col("petal_width_bin"), |c| c.facet(|f| f.share_scale()))
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
            .expect("compile nested shared col shared both");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_shared_col_shared_both",
        )
        .await;
    });
}

// =============================================================================
// Level(N) Scale Sharing Tests
// =============================================================================

/// Test Level(1) scale sharing for Y axis in FacetColumn > FacetRow layout
///
/// Level(1) shares the scale domain with the immediate parent facet.
/// In this case, FacetColumn is the outer facet, so Level(1) on Y should
/// unify Y scale domains across all rows within each column.
#[test]
fn test_nested_level1_y_col_row() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Free)
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile nested level1 y col>row");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_level1_y_col_row",
        )
        .await;
    });
}

/// Test Level(1) scale sharing for X axis in FacetRow > FacetColumn layout
///
/// Level(1) shares the scale domain with the immediate parent facet.
/// In this case, FacetRow is the outer facet, so Level(1) on X should
/// unify X scale domains across all columns within each row.
#[test]
fn test_nested_level1_x_row_col() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Note: FacetRow > FacetColumn layout (opposite of most other tests)
        let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("petal_width_bin")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("species")).subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile nested level1 x row>col");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_level1_x_row_col",
        )
        .await;
    });
}

/// Test Level(1) scale sharing for both axes
///
/// Both X and Y should share domains with the immediate parent facet.
#[test]
fn test_nested_level1_both_col_row() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile nested level1 both col>row");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_level1_both_col_row",
        )
        .await;
    });
}

// =============================================================================
// Level(N) Guide Visibility Integration Tests
// =============================================================================

/// Test Level(1) Y scale sharing with Y axis on LEFT (default)
///
/// Y axis labels should appear only at the leftmost column.
#[test]
fn test_nested_level1_y_left_axis() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Free)
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.axis(|a| a.position("left"))
                                            .with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile nested level1 y left axis");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_level1_y_left_axis",
        )
        .await;
    });
}

/// Test Level(1) Y scale sharing with Y axis on RIGHT
///
/// Y axis labels should appear only at the rightmost column.
#[test]
fn test_nested_level1_y_right_axis() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Free)
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.axis(|a| a.position("right"))
                                            .with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile nested level1 y right axis");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_level1_y_right_axis",
        )
        .await;
    });
}

/// Test Level(1) X scale sharing with X axis on BOTTOM (default)
///
/// X axis labels should appear only at the bottom row.
#[test]
fn test_nested_level1_x_bottom_axis() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Note: FacetRow > FacetColumn layout
        let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("petal_width_bin")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("species")).subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.axis(|a| a.position("bottom"))
                                        .with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile nested level1 x bottom axis");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_level1_x_bottom_axis",
        )
        .await;
    });
}

/// Test Level(1) X scale sharing with X axis on TOP
///
/// X axis labels should appear only at the top row.
#[test]
fn test_nested_level1_x_top_axis() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Note: FacetRow > FacetColumn layout
        let outer = Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Facet::new().row(col("petal_width_bin")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("species")).subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.axis(|a| a.position("top"))
                                        .with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile nested level1 x top axis");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_level1_x_top_axis",
        )
        .await;
    });
}

/// Test mixed sharing: X Free (all visible), Y Level(1) on LEFT
///
/// X axis should appear on all subplots, Y axis only at leftmost column.
#[test]
fn test_nested_level1_mixed_x_free_y_level1() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.axis(|a| a.position("bottom"))
                                            .with_scale_sharing(ScaleSharing::Free)
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.axis(|a| a.position("left"))
                                            .with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile nested mixed x free y level1");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "nested_level1_mixed_x_free_y_level1",
        )
        .await;
    });
}

/// Test 3-level nesting with Level(2) Y sharing and RIGHT axis position
///
/// Layout: FacetColumn(region) > FacetRow(country) > Cartesian (3 levels)
/// With Level(2) on Y and axis on RIGHT, Y axis labels should appear only
/// on the rightmost column (South), not on the left column (North).
/// This tests non-default axis positioning with Level(N) sharing.
#[test]
fn test_three_level_level2_y_right_axis() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting with Level(2) on Y, axis on RIGHT
        // Y axis should only appear on rightmost column (South)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.axis(|a| a.position("right"))
                                                    .with_scale_sharing(ScaleSharing::Level(2))
                                            })
                                            .size(40.0)
                                            .fill("#2ecc71"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 3-level level2 y right axis");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "three_level_level2_y_right_axis",
        )
        .await;
    });
}

/// Test 3-level nesting with Level(2) X sharing and TOP axis position
///
/// Layout: FacetColumn(region) > FacetRow(country) > Cartesian (3 levels)
/// With Level(2) on X and axis on TOP, X axis labels should appear only
/// on the top row (Canada, Brazil), not on the bottom row (USA, Mexico).
/// This tests non-default axis positioning with Level(N) sharing.
#[test]
fn test_three_level_level2_x_top_axis() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting with Level(2) on X, axis on TOP
        // X axis should only appear on top row (Canada, Brazil)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.axis(|a| a.position("top"))
                                                    .with_scale_sharing(ScaleSharing::Level(2))
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .size(40.0)
                                            .fill("#9b59b6"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 3-level level2 x top axis");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "three_level_level2_x_top_axis",
        )
        .await;
    });
}

// =============================================================================
// Three-Level Nesting Tests
// =============================================================================
// Level(N) domain propagation is supported for arbitrary nesting depths.
// - Level(1) shares with immediate parent (e.g., in 2-level nesting)
// - Level(2) shares with grandparent (e.g., in 3-level nesting)
// - Level(N) shares with N levels up in the hierarchy
// The following tests demonstrate 3-level nesting with Level(1) sharing.

/// Test 3-level nesting with Level(1) Y sharing at innermost level
///
/// Layout: FacetColumn > FacetRow > Cartesian (3 levels)
/// This tests that Level(1) correctly shares Y domain with the immediate
/// parent (FacetRow) in a 3-level nested structure.
#[test]
fn test_three_level_nesting_level1_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // 3-level nesting: FacetColumn(outer) > FacetRow(middle) > Cartesian(inner)
        // Level(1) on Y should share domain within each FacetRow (middle level)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
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
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .y_with(col("sepal_width"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(1))
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
            .expect("compile 3-level nesting level1 y");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "three_level_nesting_level1_y",
        )
        .await;
    });
}

/// Test 3-level nesting with global Shared (equivalent to Level(u8::MAX))
///
/// This tests that globally shared scales work correctly in a 3-level
/// nested structure, where all subplots use the same Y domain.
#[test]
fn test_three_level_nesting_shared_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // 3-level nesting with Shared (global) Y scale
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
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
            .expect("compile 3-level nesting shared y");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "three_level_nesting_shared_y",
        )
        .await;
    });
}

// =============================================================================
// Level(N > 1) Deep Nesting Tests
// =============================================================================
// These tests demonstrate Level(2) and Level(3) scale sharing with 3+ level
// nesting. Level(2) shares with grandparent, Level(3) with great-grandparent.

/// Create a hierarchical dataset for testing Level(N > 1) domain propagation
///
/// Structure: Region > Country > Value
/// - North region: X values 1-5, Y values 70-130
/// - South region: X values 10-15, Y values 10-70
/// Different X ranges per region demonstrate Free X sharing (each cell has own X domain)
/// Different Y ranges per region demonstrate Level-based Y sharing
async fn hierarchical_regional_data() -> datafusion::dataframe::DataFrame {
    let ctx = SessionContext::new();

    // Create a dataset with clear regional differences in BOTH x and y
    let batch = datafusion::arrow::array::RecordBatch::try_from_iter(vec![
        (
            "region",
            std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                "North", "North", "North", "North", "North", "North", "North", "North", "North",
                "North", "North", "North", "North", "North", "North", "North", "South", "South",
                "South", "South", "South", "South", "South", "South", "South", "South", "South",
                "South", "South", "South", "South", "South",
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
        (
            "country",
            std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                "USA", "USA", "USA", "USA", "Canada", "Canada", "Canada", "Canada", "USA", "USA",
                "USA", "USA", "Canada", "Canada", "Canada", "Canada", "Mexico", "Mexico", "Mexico",
                "Mexico", "Brazil", "Brazil", "Brazil", "Brazil", "Mexico", "Mexico", "Mexico",
                "Mexico", "Brazil", "Brazil", "Brazil", "Brazil",
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
        (
            "category",
            std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                "A", "A", "B", "B", "A", "A", "B", "B", "A", "A", "B", "B", "A", "A", "B", "B",
                "A", "A", "B", "B", "A", "A", "B", "B", "A", "A", "B", "B", "A", "A", "B", "B",
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
        (
            "x_val",
            std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                // North region: X values 1-5
                1.0, 2.0, 3.0, 4.0, 1.5, 2.5, 3.5, 4.5, 1.2, 2.2, 3.2, 4.2, 1.7, 2.7, 3.7, 4.7,
                // South region: X values 10-15 (different range!)
                10.0, 11.0, 12.0, 13.0, 10.5, 11.5, 12.5, 13.5, 10.2, 11.2, 12.2, 13.2, 10.7, 11.7,
                12.7, 13.7,
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
        (
            "y_val",
            std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                // North region: higher values (70-130)
                80.0, 95.0, 110.0, 125.0, 75.0, 90.0, 105.0, 120.0, 85.0, 100.0, 115.0, 130.0, 70.0,
                85.0, 100.0, 115.0, // South region: lower values (10-70)
                20.0, 35.0, 50.0, 65.0, 15.0, 30.0, 45.0, 60.0, 25.0, 40.0, 55.0, 70.0, 10.0, 25.0,
                40.0, 55.0,
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
    ])
    .expect("create record batch");

    ctx.read_batch(batch).expect("create dataframe")
}

/// Test 3-level nesting with Level(2) Y sharing
///
/// Layout: FacetColumn(region) > FacetRow(country) > Cartesian
/// With Level(2), Y domain should be shared with grandparent (FacetColumn),
/// meaning all cells across both regions share the same Y-axis range (0-130).
#[test]
fn test_three_level_level2_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting: FacetColumn(region) > FacetRow(country) > Cartesian
        // Level(2) on Y should share domain with grandparent (global across regions)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(2))
                                            })
                                            .size(40.0)
                                            .fill("#e74c3c"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 3-level nesting level2 y");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "three_level_level2_y")
            .await;
    });
}

/// Test 3-level nesting with Level(1) Y sharing
///
/// Layout: FacetColumn(region) > FacetRow(country) > Cartesian
/// With Level(1), Y domain should be shared with immediate parent (FacetRow),
/// meaning cells in each region share Y-axis within that region, but different
/// regions have different Y-axis ranges (North: 70-130, South: 10-70).
#[test]
fn test_three_level_level1_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting: FacetColumn(region) > FacetRow(country) > Cartesian
        // Level(1) on Y should share domain with parent (per-region sharing)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(1))
                                            })
                                            .size(40.0)
                                            .fill("#3498db"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 3-level nesting level1 y");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "three_level_level1_y")
            .await;
    });
}

/// Test 3-level nesting with Level(2) X sharing
///
/// Layout: FacetColumn(region) > FacetRow(country) > Cartesian
/// With Level(2), X domain should be shared with grandparent (FacetColumn),
/// meaning all cells across both regions share the same X-axis range (0-15).
/// The data has different X ranges per region:
/// - North: X values 1-5
/// - South: X values 10-15
/// With Level(2), both regions should show the combined range.
#[test]
fn test_three_level_level2_x() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting: FacetColumn(region) > FacetRow(country) > Cartesian
        // Level(2) on X should share domain with grandparent (global across regions)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(2))
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .size(40.0)
                                            .fill("#27ae60"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 3-level nesting level2 x");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "three_level_level2_x")
            .await;
    });
}

/// Test 3-level nesting with Level(1) X sharing
///
/// Layout: FacetColumn(region) > FacetRow(country) > Cartesian
/// With Level(1), X domain should be shared with immediate parent (FacetRow),
/// meaning cells in each region share X-axis within that region:
/// - North: X range 1-5
/// - South: X range 10-15
#[test]
fn test_three_level_level1_x() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting: FacetColumn(region) > FacetRow(country) > Cartesian
        // Level(1) on X should share domain with parent (per-region sharing)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(1))
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .size(40.0)
                                            .fill("#9b59b6"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 3-level nesting level1 x");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "three_level_level1_x")
            .await;
    });
}

/// Test 3-level nesting with mixed X=Shared, Y=Level(2)
///
/// Layout: FacetColumn(region) > FacetRow(country) > Cartesian
/// X=Shared and Y=Level(2) should both produce global domains in 3-level nesting.
/// - X: Shared → global (0-15)
/// - Y: Level(2) → grandparent = global (0-140)
/// Both charts should show the same unified global domain for both axes.
#[test]
fn test_three_level_mixed_x_shared_y_level2() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting: FacetColumn(region) > FacetRow(country) > Cartesian
        // X=Shared, Y=Level(2) - both should produce global domains
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(2))
                                            })
                                            .size(40.0)
                                            .fill("#f39c12"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 3-level mixed x_shared y_level2");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "three_level_mixed_x_shared_y_level2",
        )
        .await;
    });
}

/// Test 3-level nesting with mixed X=Level(2), Y=Shared
///
/// Layout: FacetColumn(region) > FacetRow(country) > Cartesian
/// X=Level(2) and Y=Shared should both produce global domains in 3-level nesting.
/// - X: Level(2) → grandparent = global (0-15)
/// - Y: Shared → global (0-140)
/// Both charts should show the same unified global domain for both axes.
#[test]
fn test_three_level_mixed_x_level2_y_shared() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting: FacetColumn(region) > FacetRow(country) > Cartesian
        // X=Level(2), Y=Shared - both should produce global domains
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(2))
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .size(40.0)
                                            .fill("#1abc9c"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 3-level mixed x_level2 y_shared");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "three_level_mixed_x_level2_y_shared",
        )
        .await;
    });
}

/// Create a 4-level hierarchical dataset for Level(3) testing
///
/// Structure: Division > Department > Team > Values
async fn hierarchical_4level_data() -> datafusion::dataframe::DataFrame {
    let ctx = SessionContext::new();

    let batch = datafusion::arrow::array::RecordBatch::try_from_iter(vec![
        (
            "division",
            std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                // Engineering division (higher values)
                "Eng", "Eng", "Eng", "Eng", "Eng", "Eng", "Eng", "Eng", "Eng", "Eng", "Eng", "Eng",
                "Eng", "Eng", "Eng", "Eng", // Operations division (lower values)
                "Ops", "Ops", "Ops", "Ops", "Ops", "Ops", "Ops", "Ops", "Ops", "Ops", "Ops", "Ops",
                "Ops", "Ops", "Ops", "Ops",
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
        (
            "department",
            std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                "Frontend", "Frontend", "Frontend", "Frontend", "Backend", "Backend", "Backend",
                "Backend", "Frontend", "Frontend", "Frontend", "Frontend", "Backend", "Backend",
                "Backend", "Backend", "Support", "Support", "Support", "Support", "DevOps",
                "DevOps", "DevOps", "DevOps", "Support", "Support", "Support", "Support", "DevOps",
                "DevOps", "DevOps", "DevOps",
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
        (
            "team",
            std::sync::Arc::new(datafusion::arrow::array::StringArray::from(vec![
                "Alpha", "Alpha", "Beta", "Beta", "Alpha", "Alpha", "Beta", "Beta", "Alpha",
                "Alpha", "Beta", "Beta", "Alpha", "Alpha", "Beta", "Beta", "Alpha", "Alpha",
                "Beta", "Beta", "Alpha", "Alpha", "Beta", "Beta", "Alpha", "Alpha", "Beta", "Beta",
                "Alpha", "Alpha", "Beta", "Beta",
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
        (
            "x_val",
            std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0,
                1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0, 2.0,
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
        (
            "y_val",
            std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                // Engineering: higher values (70-150)
                90.0, 100.0, 85.0, 95.0, 110.0, 120.0, 105.0, 115.0, 95.0, 105.0, 90.0, 100.0,
                115.0, 125.0, 110.0, 120.0, // Operations: lower values (20-70)
                40.0, 50.0, 35.0, 45.0, 55.0, 65.0, 50.0, 60.0, 45.0, 55.0, 40.0, 50.0, 60.0, 70.0,
                55.0, 65.0,
            ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
        ),
    ])
    .expect("create record batch");

    ctx.read_batch(batch).expect("create dataframe")
}

/// Test 4-level nesting with Level(2) Y sharing
///
/// Layout: FacetColumn(division) > FacetRow(department) > FacetColumn(team) > Cartesian
/// With Level(2), Y domain shares with grandparent (FacetRow/department level).
#[test]
fn test_four_level_level2_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_4level_data().await;

        // 4-level nesting: Division > Department > Team > Cartesian
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(1000, 800)
            .mark(
                Facet::new()
                    .col_with(col("division"), |c| c.facet(|f| f.title("Division")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("department"), |c| c.facet(|f| f.title("Dept")))
                                .subplot(
                                    Plot::<FacetColumn>::new().mark(
                                        Facet::new()
                                            .col_with(col("team"), |c| c.facet(|f| f.title("Team")))
                                            .subplot(
                                                Plot::<Cartesian>::new().mark(
                                                    Symbol::new()
                                                        .x_with(col("x_val"), |c| {
                                                            c.with_scale_sharing(ScaleSharing::Free)
                                                        })
                                                        .y_with(col("y_val"), |c| {
                                                            c.with_scale_sharing(
                                                                ScaleSharing::Level(2),
                                                            )
                                                        })
                                                        .size(40.0)
                                                        .fill("#3498db"),
                                                ),
                                            ),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 4-level nesting level2 y");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "four_level_level2_y")
            .await;
    });
}

/// Test 4-level nesting with Level(3) Y sharing
///
/// Layout: FacetColumn(division) > FacetRow(department) > FacetColumn(team) > Cartesian
/// With Level(3), Y domain shares with great-grandparent (outermost FacetColumn/division).
/// This means all cells across the entire chart share the same Y-axis range.
#[test]
fn test_four_level_level3_y() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_4level_data().await;

        // 4-level nesting with Level(3) - shares with great-grandparent (global)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(1000, 800)
            .mark(
                Facet::new()
                    .col_with(col("division"), |c| c.facet(|f| f.title("Division")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("department"), |c| c.facet(|f| f.title("Dept")))
                                .subplot(
                                    Plot::<FacetColumn>::new().mark(
                                        Facet::new()
                                            .col_with(col("team"), |c| c.facet(|f| f.title("Team")))
                                            .subplot(
                                                Plot::<Cartesian>::new().mark(
                                                    Symbol::new()
                                                        .x_with(col("x_val"), |c| {
                                                            c.with_scale_sharing(ScaleSharing::Free)
                                                        })
                                                        .y_with(col("y_val"), |c| {
                                                            c.with_scale_sharing(
                                                                ScaleSharing::Level(3),
                                                            )
                                                        })
                                                        .size(40.0)
                                                        .fill("#9b59b6"),
                                                ),
                                            ),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile 4-level nesting level3 y");
        assert_visual_match_default(&compiled, &ctx, None, "nested_grid", "four_level_level3_y")
            .await;
    });
}

// =============================================================================
// Scale Sharing Equivalence Tests
// =============================================================================
// These tests verify that Level(0) is equivalent to Free and Level(255) is
// equivalent to Shared. Rather than comparing to baselines, these tests render
// two charts with supposedly equivalent configurations and verify they produce
// identical visual output using image similarity comparison.

use crate::visual_tests::helpers::DEFAULT_SCALE;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use image::RgbaImage;

/// Helper function to render a CompiledPlot to an RgbaImage for comparison
async fn render_to_image(
    compiled: &avenger_chart::plot::CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
) -> RgbaImage {
    let evaluated = compiled
        .evaluate(ctx, None)
        .await
        .expect("Failed to evaluate plot");

    let dimensions = CanvasDimensions {
        size: [evaluated.scene_graph.width, evaluated.scene_graph.height],
        scale: DEFAULT_SCALE,
    };

    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("Failed to create canvas");
    canvas
        .set_scene(&evaluated.scene_graph)
        .expect("Failed to set scene");

    canvas.render().await.expect("Failed to render image")
}

/// Compare two images and assert they are visually identical (99.99%+ similarity)
fn assert_images_identical(img1: &RgbaImage, img2: &RgbaImage, msg: &str) {
    assert_eq!(
        img1.dimensions(),
        img2.dimensions(),
        "{}: Image dimensions don't match. First: {:?}, Second: {:?}",
        msg,
        img1.dimensions(),
        img2.dimensions()
    );

    let result = image_compare::rgba_hybrid_compare(img1, img2).expect("Image comparison failed");

    // Require 99.99% similarity for "identical" output
    assert!(
        result.score >= 0.9999,
        "{}: Images are not identical. Similarity: {:.4}% (expected 99.99%+)",
        msg,
        result.score * 100.0
    );
}

/// Test that Level(0) produces identical output to Free
///
/// Level(0) should be semantically equivalent to Free (per-cell scale domains).
/// Both configurations should produce visually identical rendered output.
#[test]
fn test_level0_equivalent_to_free() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Configuration 1: Level(0) on both channels
        let level0_plot = Plot::<FacetColumn>::new()
            .data(df.clone())
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(0))
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(0))
                                    })
                                    .size(25.0)
                                    .fill("#4682b4"),
                            ),
                        ),
                    ),
                ),
            );

        // Configuration 2: Free on both channels (should be identical)
        let free_plot = Plot::<FacetColumn>::new()
            .data(df.clone())
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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

        // Compile and render both
        let level0_compiled = level0_plot
            .compile(&ctx)
            .await
            .expect("compile level0 plot");
        let free_compiled = free_plot.compile(&ctx).await.expect("compile free plot");

        let level0_img = render_to_image(&level0_compiled, &ctx).await;
        let free_img = render_to_image(&free_compiled, &ctx).await;

        // Images should be visually identical
        assert_images_identical(
            &level0_img,
            &free_img,
            "Level(0) and Free should produce identical output",
        );
    });
}

// =============================================================================
// Advanced Feature Tests (Group C)
// =============================================================================
// These tests verify that Level(N) sharing works with advanced features like
// explicit domain configuration and non-position channels (color).

/// Test explicit domain configuration with Level(1) scale sharing
///
/// Verifies that when an explicit domain is specified via `.scale(|s| s.domain(...))`,
/// it is properly respected across all cells within a Level(1) sharing group.
#[test]
fn test_explicit_domain_with_level1() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // FacetColumn > FacetRow with Level(1) on Y plus explicit domain
        // The explicit domain (1.5, 5.0) should be applied within each column
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Free)
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                            .scale(|s| s.domain((1.5, 5.0)))
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
            .expect("compile explicit domain with level1");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "explicit_domain_with_level1",
        )
        .await;
    });
}

/// Test explicit domain configuration with Level(2) scale sharing
///
/// Verifies that when an explicit domain is specified via `.scale(|s| s.domain(...))`,
/// it takes precedence over Level(2) computed domain. The explicit domain should
/// be applied to all cells (global sharing at Level(2) in 3-level nesting).
#[test]
fn test_explicit_domain_with_level2() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 3-level nesting: FacetColumn > FacetRow > Cartesian
        // Level(2) on Y should share globally, but explicit domain (0.0, 100.0)
        // should override the computed domain (10-130)
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(800, 600)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("country"), |c| c.facet(|f| f.title("Country")))
                                .subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(2))
                                                    .scale(|s| s.domain((0.0, 100.0)))
                                            })
                                            .size(40.0)
                                            .fill("#1abc9c"),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile explicit domain with level2");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "explicit_domain_with_level2",
        )
        .await;
    });
}

/// Test Level(N) when nesting depth < N (clamp to max available)
///
/// In a 2-level nesting (FacetColumn > Cartesian), Level(3) should clamp
/// to the maximum available depth, behaving like Shared (global domain).
#[test]
fn test_level_exceeds_nesting_depth() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_regional_data().await;

        // 2-level nesting: FacetColumn > Cartesian (no FacetRow)
        // Level(3) on Y exceeds nesting depth, should clamp to global
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 400)
            .mark(
                Facet::new()
                    .col_with(col("region"), |c| c.facet(|f| f.title("Region")))
                    .subplot(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x(col("x_val"))
                                .y_with(col("y_val"), |c| {
                                    c.with_scale_sharing(ScaleSharing::Level(3))
                                })
                                .size(40.0)
                                .fill("#c0392b"),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile level exceeds nesting depth");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "level_exceeds_nesting_depth",
        )
        .await;
    });
}

/// Test Level(N) in non-nested context (simple Cartesian plot)
///
/// When Level(N) is used in a non-faceted context (just Plot::<Cartesian>),
/// it should gracefully degrade to Free behavior since there's no
/// facet hierarchy to share across.
#[test]
fn test_level_non_nested_context() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();

        // Simple dataset for non-nested test
        let batch = datafusion::arrow::array::RecordBatch::try_from_iter(vec![
            (
                "x",
                std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                    1.0, 2.0, 3.0, 4.0, 5.0,
                ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
            ),
            (
                "y",
                std::sync::Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                    10.0, 20.0, 15.0, 25.0, 30.0,
                ])) as std::sync::Arc<dyn datafusion::arrow::array::Array>,
            ),
        ])
        .expect("create record batch");

        let df = ctx.read_batch(batch).expect("create dataframe");

        // Non-faceted plot with Level(1) - should degrade to Free
        let plot = Plot::<Cartesian>::new()
            .data(df)
            .canvas_size(400, 300)
            .mark(
                Symbol::new()
                    .x(col("x"))
                    .y_with(col("y"), |c| c.with_scale_sharing(ScaleSharing::Level(1)))
                    .size(50.0)
                    .fill("#3498db"),
            );

        let compiled = plot.compile(&ctx).await.expect("compile non-nested level1");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "level_non_nested_context",
        )
        .await;
    });
}

/// Test color channel with Level(1) scale sharing
///
/// Verifies that Level(N) scale sharing works correctly for color channels,
/// not just position channels. The color scale domain should be shared
/// within each column group.
#[test]
fn test_color_channel_with_level1() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // FacetColumn > FacetRow with Level(1) on fill color
        // The color scale domain should be shared within each column
        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x(col("sepal_length"))
                                    .y(col("sepal_width"))
                                    .fill_with(col("petal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(1))
                                    })
                                    .size(35.0),
                            ),
                        ),
                    ),
                ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile color channel with level1");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "color_channel_with_level1",
        )
        .await;
    });
}

/// Test that Level(255) produces identical output to Shared
///
/// Level(255) should be semantically equivalent to Shared (global scale domains).
/// Both configurations should produce visually identical rendered output.
///
/// Both Shared and Level(255) now use unified UNION semantics via `extend_with_shared_extents`,
/// and all internal checks use `is_fully_shared()` which treats them identically.
#[test]
fn test_level255_equivalent_to_shared() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = iris_with_binned_petal_width().await;

        // Configuration 1: Level(255) on both channels
        let level255_plot = Plot::<FacetColumn>::new()
            .data(df.clone())
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("sepal_length"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(255))
                                    })
                                    .y_with(col("sepal_width"), |c| {
                                        c.with_scale_sharing(ScaleSharing::Level(255))
                                    })
                                    .size(25.0)
                                    .fill("#4682b4"),
                            ),
                        ),
                    ),
                ),
            );

        // Configuration 2: Shared on both channels (should be identical)
        let shared_plot = Plot::<FacetColumn>::new()
            .data(df.clone())
            .canvas_size(600, 600)
            .mark(
                Facet::new().column(col("petal_width_bin")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("species")).subplot(
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

        // Compile and render both
        let level255_compiled = level255_plot
            .compile(&ctx)
            .await
            .expect("compile level255 plot");
        let shared_compiled = shared_plot
            .compile(&ctx)
            .await
            .expect("compile shared plot");

        let level255_img = render_to_image(&level255_compiled, &ctx).await;
        let shared_img = render_to_image(&shared_compiled, &ctx).await;

        // Images should be visually identical
        assert_images_identical(
            &level255_img,
            &shared_img,
            "Level(255) and Shared should produce identical output",
        );
    });
}

// =============================================================================
// 4-Level Nesting Permutation Tests
// =============================================================================
// These tests verify facet guide positioning with different row/col orderings
// at 4 levels of nesting.

/// Test 4-level nesting: Row > Col > Row > Col (Row as outermost)
///
/// Layout: FacetRow(division) > FacetColumn(department) > FacetRow(team) > Cartesian
/// This tests whether the fix for FacetColGuide also applies when FacetRow is outermost.
#[test]
fn test_four_level_row_col_row_col() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_4level_data().await;

        let outer = Plot::<FacetRow>::new()
            .data(df)
            .canvas_size(1000, 800)
            .mark(
                Facet::new()
                    .row_with(col("division"), |c| c.facet(|f| f.title("Division")))
                    .subplot(
                        Plot::<FacetColumn>::new().mark(
                            Facet::new()
                                .col_with(col("department"), |c| c.facet(|f| f.title("Dept")))
                                .subplot(
                                    Plot::<FacetRow>::new().mark(
                                        Facet::new()
                                            .row_with(col("team"), |c| c.facet(|f| f.title("Team")))
                                            .subplot(
                                                Plot::<Cartesian>::new().mark(
                                                    Symbol::new()
                                                        .x_with(col("x_val"), |c| {
                                                            c.with_scale_sharing(ScaleSharing::Free)
                                                        })
                                                        .y_with(col("y_val"), |c| {
                                                            c.with_scale_sharing(
                                                                ScaleSharing::Level(3),
                                                            )
                                                        })
                                                        .size(40.0)
                                                        .fill("#e74c3c"),
                                                ),
                                            ),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile row>col>row>col nesting");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "four_level_row_col_row_col",
        )
        .await;
    });
}

/// Test 4-level nesting: Col > Col > Row > Row (Column-heavy at top)
///
/// Layout: FacetColumn(division) > FacetColumn(department) > FacetRow(team) > Cartesian
/// This tests positioning with consecutive columns at outer levels.
#[test]
#[ignore = "crashes with subtraction overflow in visibility.rs:308 - consecutive same-type facets"]
fn test_four_level_col_col_row_row() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_4level_data().await;

        let outer = Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(1000, 800)
            .mark(
                Facet::new()
                    .col_with(col("division"), |c| c.facet(|f| f.title("Division")))
                    .subplot(
                        Plot::<FacetColumn>::new().mark(
                            Facet::new()
                                .col_with(col("department"), |c| c.facet(|f| f.title("Dept")))
                                .subplot(
                                    Plot::<FacetRow>::new().mark(
                                        Facet::new()
                                            .row_with(col("team"), |c| c.facet(|f| f.title("Team")))
                                            .subplot(
                                                Plot::<Cartesian>::new().mark(
                                                    Symbol::new()
                                                        .x_with(col("x_val"), |c| {
                                                            c.with_scale_sharing(ScaleSharing::Free)
                                                        })
                                                        .y_with(col("y_val"), |c| {
                                                            c.with_scale_sharing(
                                                                ScaleSharing::Level(3),
                                                            )
                                                        })
                                                        .size(40.0)
                                                        .fill("#27ae60"),
                                                ),
                                            ),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile col>col>row>row nesting");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "four_level_col_col_row_row",
        )
        .await;
    });
}

/// Test 4-level nesting: Row > Row > Col > Col (Row-heavy at top)
///
/// Layout: FacetRow(division) > FacetRow(department) > FacetColumn(team) > Cartesian
/// This tests positioning with consecutive rows at outer levels.
#[test]
#[ignore = "crashes with subtraction overflow in visibility.rs:109 - consecutive same-type facets"]
fn test_four_level_row_row_col_col() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = hierarchical_4level_data().await;

        let outer = Plot::<FacetRow>::new()
            .data(df)
            .canvas_size(1000, 800)
            .mark(
                Facet::new()
                    .row_with(col("division"), |c| c.facet(|f| f.title("Division")))
                    .subplot(
                        Plot::<FacetRow>::new().mark(
                            Facet::new()
                                .row_with(col("department"), |c| c.facet(|f| f.title("Dept")))
                                .subplot(
                                    Plot::<FacetColumn>::new().mark(
                                        Facet::new()
                                            .col_with(col("team"), |c| c.facet(|f| f.title("Team")))
                                            .subplot(
                                                Plot::<Cartesian>::new().mark(
                                                    Symbol::new()
                                                        .x_with(col("x_val"), |c| {
                                                            c.with_scale_sharing(ScaleSharing::Free)
                                                        })
                                                        .y_with(col("y_val"), |c| {
                                                            c.with_scale_sharing(
                                                                ScaleSharing::Level(3),
                                                            )
                                                        })
                                                        .size(40.0)
                                                        .fill("#f39c12"),
                                                ),
                                            ),
                                    ),
                                ),
                        ),
                    ),
            );

        let compiled = outer
            .compile(&ctx)
            .await
            .expect("compile row>row>col>col nesting");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "nested_grid",
            "four_level_row_row_col_col",
        )
        .await;
    });
}
