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

// =============================================================================
// Three-Level Nesting Tests
// =============================================================================
// NOTE: Level(2) domain propagation for 3-level nesting requires additional
// implementation work. The current implementation only supports Level(1)
// sharing with the immediate parent. These tests demonstrate 3-level nesting
// with Level(1) at the innermost level.

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
