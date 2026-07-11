use std::sync::Arc;

use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::*,
};

use crate::visual_tests::helpers::assert_visual_match_default;

fn axis_policy_test_data(ctx: &SessionContext) -> DataFrame {
    let facets = StringArray::from(vec![
        "Group A", "Group A", "Group A", "Group B", "Group B", "Group B", "Group C", "Group C",
        "Group C",
    ]);
    let x = Float64Array::from(vec![0.0, 1.0, 2.0, 10.0, 11.0, 12.0, 100.0, 101.0, 102.0]);
    let y = Float64Array::from(vec![1.0, 2.0, 1.5, 2.5, 3.0, 2.75, 1.8, 2.4, 3.1]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("facet", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(facets), Arc::new(x), Arc::new(y)])
        .expect("axis policy test batch");

    ctx.read_batch(batch).expect("axis policy test data")
}

fn axis_policy_cell_data(ctx: &SessionContext, x_offset: f64, y_offset: f64) -> DataFrame {
    let x = Float64Array::from(vec![x_offset, x_offset + 1.0, x_offset + 2.0]);
    let y = Float64Array::from(vec![y_offset + 1.0, y_offset + 2.0, y_offset + 1.5]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(x), Arc::new(y)])
        .expect("axis policy cell batch");

    ctx.read_batch(batch).expect("axis policy cell data")
}

fn axis_policy_cell_plot(ctx: &SessionContext, x_offset: f64, y_offset: f64) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .data(axis_policy_cell_data(ctx, x_offset, y_offset))
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.axis(|a| a.title("local x")))
                .y_with(col("y"), |c| c.axis(|a| a.title("local y")))
                .size(56.0)
                .fill("#2f7ed8"),
        )
}

fn axis_policy_named_domain_cell_plot(
    ctx: &SessionContext,
    x_group: &'static str,
    y_group: &'static str,
    x_offset: f64,
    y_offset: f64,
) -> Plot<Cartesian> {
    Plot::<Cartesian>::new()
        .data(axis_policy_cell_data(ctx, x_offset, y_offset))
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.with_domain_group(x_group)
                        .share_domain()
                        .axis(|a| a.title(x_group))
                })
                .y_with(col("y"), |c| {
                    c.with_domain_group(y_group)
                        .share_domain()
                        .axis(|a| a.title(y_group))
                })
                .size(56.0)
                .fill("#2f7ed8"),
        )
}

fn facet_row_axis_policy_plot(df: DataFrame, policy: AxisGuideVisibilityPolicy) -> Chart<FacetRow> {
    Chart::<FacetRow>::new().data(df).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x_with(col("x"), |c| {
                        c.with_domain_scope(CoordinationScope::Free)
                            .axis(|a| a.title("local x"))
                    })
                    .y_with(col("y"), |c| c.axis(|a| a.title("value")))
                    .size(56.0)
                    .fill("#2f7ed8"),
            ),
        )
        .row_with(col("facet"), |c| c.axis_guide_visibility(policy)),
    )
}

#[tokio::test]
async fn facet_axis_visibility_all() {
    let ctx = SessionContext::new();
    let df = axis_policy_test_data(&ctx);
    let plot = facet_row_axis_policy_plot(df, AxisGuideVisibilityPolicy::All);
    let compiled = plot.compile(&ctx).await.expect("compile axis policy plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "axis_guide_visibility",
        "facet_axis_visibility_all",
    )
    .await;
}

#[tokio::test]
async fn facet_axis_visibility_outer_edges() {
    let ctx = SessionContext::new();
    let df = axis_policy_test_data(&ctx);
    let plot = facet_row_axis_policy_plot(df, AxisGuideVisibilityPolicy::OuterEdges);
    let compiled = plot.compile(&ctx).await.expect("compile axis policy plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "axis_guide_visibility",
        "facet_axis_visibility_outer_edges",
    )
    .await;
}

#[tokio::test]
async fn grid_concat_axis_visibility_outer_edges() {
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .configure_coord(|c| {
            c.rows(2)
                .columns(2)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 0.0, 0.0)).at(0, 0))
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 10.0, 10.0)).at(0, 1))
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 100.0, 100.0)).at(1, 0))
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 200.0, 200.0)).at(1, 1));
    let compiled = plot.compile(&ctx).await.expect("compile grid policy plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "axis_guide_visibility",
        "grid_concat_axis_visibility_outer_edges",
    )
    .await;
}

#[tokio::test]
async fn grid_concat_axis_visibility_equivalent_domain_groups() {
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .configure_coord(|c| {
            c.rows(2)
                .columns(2)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups)
        })
        .mark(
            Subplot::new(axis_policy_named_domain_cell_plot(
                &ctx, "length", "length", 0.0, 0.0,
            ))
            .at(0, 0),
        )
        .mark(
            Subplot::new(axis_policy_named_domain_cell_plot(
                &ctx, "width", "length", 10.0, 10.0,
            ))
            .at(0, 1),
        )
        .mark(
            Subplot::new(axis_policy_named_domain_cell_plot(
                &ctx, "length", "width", 100.0, 100.0,
            ))
            .at(1, 0),
        )
        .mark(
            Subplot::new(axis_policy_named_domain_cell_plot(
                &ctx, "width", "width", 200.0, 200.0,
            ))
            .at(1, 1),
        );
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile grid semantic policy plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "axis_guide_visibility",
        "grid_concat_axis_visibility_equivalent_domain_groups",
    )
    .await;
}

#[tokio::test]
async fn wrap_concat_axis_visibility_outer_edges() {
    let ctx = SessionContext::new();
    let plot = Chart::<WrapConcat>::new()
        .configure_coord(|c| {
            c.columns(3.0)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 0.0, 0.0)))
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 10.0, 10.0)))
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 20.0, 20.0)))
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 100.0, 100.0)))
        .mark(Subplot::new(axis_policy_cell_plot(&ctx, 200.0, 200.0)));
    let compiled = plot.compile(&ctx).await.expect("compile wrap policy plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "axis_guide_visibility",
        "wrap_concat_axis_visibility_outer_edges",
    )
    .await;
}
