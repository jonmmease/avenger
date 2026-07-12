//! Boundary baseline for the child-frame facet-band alignment adapter
//! (`apply_facet_band_grid_requirements`).
//!
//! A `GridConcat` of two template-equivalent fixed-plot-size `FacetColumn`
//! children whose inner charts plot different y metrics with FREE y scales:
//! every facet cell carries its own y axis, so the wide-magnitude metric's
//! band genuinely demands more interior track spacing (the alignment
//! diagnostics report ~25-40px track/slab deltas between the siblings at
//! the alignment-apply stage).
//!
//! This is the closest any end-to-end spec gets to the facet-band apply
//! adapter. Concat-nested facet bands use canvas-constrained current facet
//! geometry rather than content-driven placement, so the apply skips and the
//! two bands keep their own spacing. The baseline pins that unaligned
//! rendering so any change to the adapter's reachability or behavior shows
//! up as an image diff.

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

async fn alignment_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS x,
            column2 AS y_small,
            column3 AS y_large,
            column4 AS group_name
         FROM (VALUES
            (1.0, 1.2, 12000.0, 'Alpha'),
            (2.0, 2.6, 26000.0, 'Alpha'),
            (3.0, 1.8, 18000.0, 'Alpha'),
            (4.0, 3.1, 31000.0, 'Alpha'),
            (1.0, 2.2, 22000.0, 'Beta'),
            (2.0, 1.4, 14000.0, 'Beta'),
            (3.0, 2.9, 29000.0, 'Beta'),
            (4.0, 1.1, 11000.0, 'Beta')
         )",
    )
    .await
    .expect("facet grid alignment data")
}

fn facet_child(y_column: &str) -> Plot<FacetColumn> {
    let y_column = y_column.to_string();
    Plot::<FacetColumn>::new().mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Line::new()
                    .x(col("x"))
                    .y_with(col(&y_column), |c| {
                        c.scale_with::<Linear>(|s| s)
                            .with_domain_scope(CoordinationScope::Free)
                    })
                    .stroke("#4682b4"),
            ),
        )
        .column(col("group_name")),
    )
}

#[tokio::test]
async fn concat_grid_facet_track_alignment() {
    let ctx = SessionContext::new();
    let df = alignment_data(&ctx).await;

    let plot = Chart::<GridConcat>::new()
        .data(df)
        .configure_coord(|c| c.rows(1).columns(2))
        .mark(
            Subplot::new(facet_child("y_small"))
                .size(120.0, 90.0)
                .at(0, 0)
                .name("metric_small"),
        )
        .mark(
            Subplot::new(facet_child("y_large"))
                .size(120.0, 90.0)
                .at(0, 1)
                .name("metric_large"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "concat_grid_facet_track_alignment",
    )
    .await;
}
