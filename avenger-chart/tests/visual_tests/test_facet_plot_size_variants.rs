use crate::visual_tests::datasets::{
    legend_sharing_hierarchy_df, numeric_hierarchy_df, sparse_hierarchy_df,
};
use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::legend::LegendPosition;
use avenger_chart::prelude::*;
use avenger_chart::scales::Linear;
use datafusion::prelude::*;
use std::future::Future;

fn run_with_large_stack<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build runtime");
            rt.block_on(f());
        })
        .expect("spawn thread")
        .join()
        .expect("join thread");
}

fn make_fill_legend_symbol(sharing: ScaleSharing, position: LegendPosition) -> Symbol<Cartesian> {
    Symbol::new()
        .x_with(col("x_val"), |c| c.with_scale_sharing(ScaleSharing::Shared))
        .y_with(col("y_val"), |c| c.with_scale_sharing(ScaleSharing::Shared))
        .fill_with(col("category"), move |c| {
            c.with_scale_sharing(sharing.clone())
                .legend(|l| l.title("Category").position(position))
        })
        .size(58.0)
}

#[test]
fn facet_plot_size_nested_col_col_col_legend_level2_right() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = legend_sharing_hierarchy_df(&ctx).await;

        let plot = Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(140.0, 100.0)
            .mark(Facet::new().column(col("division")).subplot(
                Plot::<FacetColumn>::new().mark(
                    Facet::new().column(col("department")).subplot(
                        Plot::<FacetColumn>::new().mark(Facet::new().column(col("team")).subplot(
                            Plot::<Cartesian>::new().mark(make_fill_legend_symbol(
                                ScaleSharing::Level(2),
                                LegendPosition::Right,
                            )),
                        )),
                    ),
                ),
            ));

        let compiled = plot.compile(&ctx).await.expect("compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "facet_plot_size",
            "facet_plot_size_nested_col_col_col_legend_level2_right",
        )
        .await;
    });
}

#[test]
fn facet_plot_size_nested_row_col_row_mixed_sharing() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = legend_sharing_hierarchy_df(&ctx).await;

        let plot = Plot::<FacetRow>::new()
            .data(df)
            .plot_size(110.0, 80.0)
            .mark(
                Facet::new().row(col("division")).subplot(
                    Plot::<FacetColumn>::new().mark(
                        Facet::new().column(col("department")).subplot(
                            Plot::<FacetRow>::new().mark(
                                Facet::new().row(col("team")).subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Free)
                                            })
                                            .fill_with(col("category"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(1))
                                            })
                                            .size(58.0),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            );

        let compiled = plot.compile(&ctx).await.expect("compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "facet_plot_size",
            "facet_plot_size_nested_row_col_row_mixed_sharing",
        )
        .await;
    });
}

#[test]
fn facet_plot_size_nested_col_row_col_empty_subplot_policy() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = sparse_hierarchy_df(&ctx).await;

        let plot = Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(110.0, 80.0)
            .mark(
                Facet::new().column(col("division")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new()
                            .row_with(col("dept"), |c| {
                                c.facet(|f| f.empty_cell_policy(FacetEmptyCellPolicy::EmptySubplot))
                            })
                            .subplot(
                                Plot::<FacetColumn>::new().mark(
                                    Facet::new().column(col("team")).subplot(
                                        Plot::<Cartesian>::new().mark(
                                            Symbol::new()
                                                .x_with(col("x_val"), |c| {
                                                    c.with_scale_sharing(ScaleSharing::Shared)
                                                })
                                                .y_with(col("y_val"), |c| {
                                                    c.with_scale_sharing(ScaleSharing::Shared)
                                                })
                                                .fill_with(col("category"), |c| {
                                                    c.with_scale_sharing(ScaleSharing::Level(1))
                                                })
                                                .size(58.0),
                                        ),
                                    ),
                                ),
                            ),
                    ),
                ),
            );

        let compiled = plot.compile(&ctx).await.expect("compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "facet_plot_size",
            "facet_plot_size_nested_col_row_col_empty_subplot_policy",
        )
        .await;
    });
}

#[test]
fn facet_plot_size_nested_col_col_row_numeric_domain_order() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = numeric_hierarchy_df(&ctx).await;

        let plot = Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(110.0, 80.0)
            .mark(
                Facet::new().column(col("division_id")).subplot(
                    Plot::<FacetColumn>::new().mark(
                        Facet::new().column(col("dept_id")).subplot(
                            Plot::<FacetRow>::new().mark(
                                Facet::new().row(col("team_id")).subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .size(58.0)
                                            .fill("#4682b4"),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            );

        let compiled = plot.compile(&ctx).await.expect("compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "facet_plot_size",
            "facet_plot_size_nested_col_col_row_numeric_domain_order",
        )
        .await;
    });
}

#[test]
fn facet_plot_size_nested_row_row_row_sparse_hierarchy() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = sparse_hierarchy_df(&ctx).await;

        let plot = Plot::<FacetRow>::new()
            .data(df)
            .plot_size(110.0, 80.0)
            .mark(
                Facet::new().row(col("division")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("dept")).subplot(
                            Plot::<FacetRow>::new().mark(
                                Facet::new().row(col("team")).subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .fill_with(col("category"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Level(1))
                                            })
                                            .size(58.0),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            );

        let compiled = plot.compile(&ctx).await.expect("compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "facet_plot_size",
            "facet_plot_size_nested_row_row_row_sparse_hierarchy",
        )
        .await;
    });
}

#[test]
fn facet_plot_size_nested_col_row_col_continuous_legend() {
    run_with_large_stack(|| async {
        let ctx = SessionContext::new();
        let df = legend_sharing_hierarchy_df(&ctx).await;

        let plot = Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(110.0, 80.0)
            .mark(
                Facet::new().column(col("division")).subplot(
                    Plot::<FacetRow>::new().mark(
                        Facet::new().row(col("department")).subplot(
                            Plot::<FacetColumn>::new().mark(
                                Facet::new().column(col("team")).subplot(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_scale_sharing(ScaleSharing::Shared)
                                            })
                                            .fill_with(col("y_val"), |c| {
                                                c.scale_with::<Linear>(|s| s).legend(|l| {
                                                    l.title("Score").position(LegendPosition::Right)
                                                })
                                            })
                                            .size(58.0),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            );

        let compiled = plot.compile(&ctx).await.expect("compile plot");
        assert_visual_match_default(
            &compiled,
            &ctx,
            None,
            "facet_plot_size",
            "facet_plot_size_nested_col_row_col_continuous_legend",
        )
        .await;
    });
}
