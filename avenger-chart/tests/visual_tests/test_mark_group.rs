use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::{
    dataframe::DataFrame,
    logical_expr::Expr,
    prelude::{SessionContext, col, lit},
};
use std::sync::Arc;

fn box_plot_data(ctx: &SessionContext) -> DataFrame {
    let mut groups = Vec::new();
    let mut values = Vec::new();
    let observations = [
        (
            "Alpha",
            [4.0, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5, 8.0, 8.5, 26.0].as_slice(),
        ),
        (
            "Beta",
            [1.0, 10.0, 11.5, 12.0, 13.0, 13.5, 14.0, 15.0, 16.0, 31.0].as_slice(),
        ),
        (
            "Gamma",
            [6.0, 7.0, 7.5, 8.0, 8.5, 9.0, 9.5, 10.0, 10.5, 15.0].as_slice(),
        ),
        (
            "Delta",
            [16.0, 17.0, 17.5, 18.0, 18.5, 19.0, 20.0, 21.0, 22.0, 34.0].as_slice(),
        ),
    ];

    for (group, group_values) in observations {
        for value in group_values {
            groups.push(group);
            values.push(*value);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("group", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(groups)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("box plot batch");
    ctx.read_batch(batch).expect("box plot dataframe")
}

fn fence_stats() -> JoinAggregate {
    JoinAggregate::new()
        .group_by([col("group")])
        .approx_percentile_cont("q1", col("value"), 0.25)
        .approx_percentile_cont("q3", col("value"), 0.75)
}

fn lower_fence() -> Expr {
    col("q1") - (col("q3") - col("q1")) * lit(1.5)
}

fn upper_fence() -> Expr {
    col("q3") + (col("q3") - col("q1")) * lit(1.5)
}

fn inlier_predicate() -> Expr {
    col("value")
        .gt_eq(lower_fence())
        .and(col("value").lt_eq(upper_fence()))
}

fn outlier_predicate() -> Expr {
    col("value")
        .lt(lower_fence())
        .or(col("value").gt(upper_fence()))
}

fn y_category_axis(value: CartesianPositionConfig) -> CartesianPositionConfig {
    value
        .scale_with::<Band>(|scale| {
            scale.domain_discrete(vec![lit("Alpha"), lit("Beta"), lit("Gamma"), lit("Delta")])
        })
        .axis(|axis| axis.title("Group").grid(false))
}

fn whisker_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().id("box_whiskers").transform_no_output(
        Filter::new(inlier_predicate()),
        |group| {
            group.transform(
                Aggregate::new()
                    .group_by([col("group")])
                    .min("whisker_low", col("value"))
                    .max("whisker_high", col("value")),
                |group, whiskers| {
                    group
                        .mark(
                            Rule::new()
                                .x(whiskers.output("whisker_low"))
                                .x2(whiskers.output("whisker_high"))
                                .y_with(col("group"), |y| y.band(0.5))
                                .y2_with(col("group"), |y| y.band(0.5))
                                .stroke("#475569")
                                .stroke_width(1.5)
                                .zindex(1),
                        )
                        .mark(
                            Rule::new()
                                .x(whiskers.output("whisker_low"))
                                .x2(whiskers.output("whisker_low"))
                                .y_with(col("group"), |y| y.band(0.32))
                                .y2_with(col("group"), |y| y.band(0.68))
                                .stroke("#475569")
                                .stroke_width(1.5)
                                .zindex(2),
                        )
                        .mark(
                            Rule::new()
                                .x(whiskers.output("whisker_high"))
                                .x2(whiskers.output("whisker_high"))
                                .y_with(col("group"), |y| y.band(0.32))
                                .y2_with(col("group"), |y| y.band(0.68))
                                .stroke("#475569")
                                .stroke_width(1.5)
                                .zindex(2),
                        )
                },
            )
        },
    )
}

fn box_summary_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().id("box_summary").transform(
        Aggregate::new()
            .group_by([col("group")])
            .approx_percentile_cont("q1", col("value"), 0.25)
            .median("median", col("value"))
            .approx_percentile_cont("q3", col("value"), 0.75),
        |group, stats| {
            group
                .mark(
                    Rect::new()
                        .x_with(stats.output("q1"), |x| {
                            x.scale_with::<Linear>(|scale| {
                                scale.domain_interval(lit(0.0), lit(36.0))
                            })
                            .axis(|axis| axis.title("Value").grid(true))
                        })
                        .x2(stats.output("q3"))
                        .y_with(col("group"), |y| y_category_axis(y).band(0.26))
                        .y2_with(col("group"), |y| y.band(0.74))
                        .fill("#bfdbfe")
                        .stroke("#2563eb")
                        .stroke_width(1.5)
                        .zindex(3),
                )
                .mark(
                    Rule::new()
                        .x(stats.output("median"))
                        .x2(stats.output("median"))
                        .y_with(col("group"), |y| y.band(0.24))
                        .y2_with(col("group"), |y| y.band(0.76))
                        .stroke("#1e3a8a")
                        .stroke_width(2.2)
                        .zindex(4),
                )
        },
    )
}

fn outlier_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().id("box_outliers").transform_no_output(
        Filter::new(outlier_predicate()),
        |group| {
            group.mark(
                Symbol::new()
                    .x(col("value"))
                    .y_with(col("group"), |y| y.band(0.5))
                    .fill("#f97316")
                    .stroke("#ffffff")
                    .stroke_width(1.25)
                    .size(95.0)
                    .zindex(5),
            )
        },
    )
}

fn fence_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new()
        .id("box_fences")
        .transform_no_output(fence_stats(), |group| {
            group.mark(whisker_branch()).mark(outlier_branch())
        })
}

fn box_plot_group() -> MarkGroup<Cartesian> {
    MarkGroup::new()
        .id("box_plot")
        .mark(fence_branch())
        .mark(box_summary_branch())
}

#[tokio::test]
async fn box_plot_from_mark_group_branches() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Box plot from MarkGroup branches")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .mark(box_plot_group());

    let compiled = plot.compile(&ctx).await.expect("compile box plot group");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_group",
        "box_plot_from_mark_group_branches",
    )
    .await;
}
