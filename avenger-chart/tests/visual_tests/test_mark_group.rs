use super::helpers::assert_visual_match_default;
use avenger_chart::marks::box_plot::{
    BOX_PLOT_MEDIAN_FIELD, BOX_PLOT_Q1_FIELD, BOX_PLOT_Q3_FIELD, BOX_PLOT_WHISKER_HIGH_FIELD,
    BOX_PLOT_WHISKER_LOW_FIELD, boxplot_fence_stats, boxplot_summary_stats, boxplot_whisker_stats,
    inlier_predicate as boxplot_inlier_predicate, outlier_predicate as boxplot_outlier_predicate,
};
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
    box_plot_data_from_group_values(
        ctx,
        &[
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
        ],
    )
}

fn box_plot_data_from_group_values(
    ctx: &SessionContext,
    observations: &[(&str, &[f64])],
) -> DataFrame {
    let mut groups = Vec::new();
    let mut values = Vec::new();

    for (group, group_values) in observations.iter().copied() {
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

fn grouped_box_plot_data(ctx: &SessionContext) -> DataFrame {
    let mut categories = Vec::new();
    let mut segments = Vec::new();
    let mut values = Vec::new();
    let observations = [
        (
            "Platform",
            "SMB",
            [12.0, 14.0, 15.0, 15.5, 16.0, 17.0, 18.5, 24.0].as_slice(),
        ),
        (
            "Platform",
            "Enterprise",
            [18.0, 19.0, 21.0, 22.0, 22.5, 23.0, 24.0, 31.0].as_slice(),
        ),
        (
            "Infrastructure",
            "SMB",
            [9.0, 10.0, 11.0, 12.0, 12.5, 13.0, 14.0, 19.0].as_slice(),
        ),
        (
            "Infrastructure",
            "Enterprise",
            [20.0, 21.0, 23.0, 24.0, 24.5, 25.0, 26.0, 34.0].as_slice(),
        ),
        (
            "Services",
            "SMB",
            [7.0, 8.0, 9.0, 9.5, 10.0, 11.0, 12.0, 18.0].as_slice(),
        ),
        (
            "Services",
            "Enterprise",
            [14.0, 15.0, 16.0, 17.0, 17.5, 18.0, 19.0, 27.0].as_slice(),
        ),
    ];

    for (category, segment, group_values) in observations {
        for value in group_values {
            categories.push(category);
            segments.push(segment);
            values.push(*value);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("segment", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(categories)) as _,
            Arc::new(StringArray::from(segments)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("grouped box plot batch");
    ctx.read_batch(batch).expect("grouped box plot dataframe")
}

fn faceted_grouped_box_plot_data(ctx: &SessionContext) -> DataFrame {
    let mut regions = Vec::new();
    let mut categories = Vec::new();
    let mut segments = Vec::new();
    let mut values = Vec::new();
    let observations = [
        (
            "Platform",
            "SMB",
            [12.0, 14.0, 15.0, 15.5, 16.0, 17.0, 18.5, 24.0].as_slice(),
        ),
        (
            "Platform",
            "Enterprise",
            [18.0, 19.0, 21.0, 22.0, 22.5, 23.0, 24.0, 31.0].as_slice(),
        ),
        (
            "Infrastructure",
            "SMB",
            [9.0, 10.0, 11.0, 12.0, 12.5, 13.0, 14.0, 19.0].as_slice(),
        ),
        (
            "Infrastructure",
            "Enterprise",
            [20.0, 21.0, 23.0, 24.0, 24.5, 25.0, 26.0, 34.0].as_slice(),
        ),
        (
            "Services",
            "SMB",
            [7.0, 8.0, 9.0, 9.5, 10.0, 11.0, 12.0, 18.0].as_slice(),
        ),
        (
            "Services",
            "Enterprise",
            [14.0, 15.0, 16.0, 17.0, 17.5, 18.0, 19.0, 27.0].as_slice(),
        ),
    ];

    for (region, offset) in [("North", 0.0), ("South", 2.5)] {
        for (category, segment, group_values) in observations {
            for value in group_values {
                regions.push(region);
                categories.push(category);
                segments.push(segment);
                values.push(*value + offset);
            }
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("segment", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(regions)) as _,
            Arc::new(StringArray::from(categories)) as _,
            Arc::new(StringArray::from(segments)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("faceted grouped box plot batch");
    ctx.read_batch(batch)
        .expect("faceted grouped box plot dataframe")
}

fn inlier_predicate() -> Expr {
    boxplot_inlier_predicate(
        col("value"),
        col(BOX_PLOT_Q1_FIELD),
        col(BOX_PLOT_Q3_FIELD),
        1.5,
    )
}

fn outlier_predicate() -> Expr {
    boxplot_outlier_predicate(
        col("value"),
        col(BOX_PLOT_Q1_FIELD),
        col(BOX_PLOT_Q3_FIELD),
        1.5,
    )
}

fn y_category_axis(value: CartesianPositionConfig) -> CartesianPositionConfig {
    value
        .scale_with::<Band>(|scale| {
            scale.domain_discrete(vec![lit("Alpha"), lit("Beta"), lit("Gamma"), lit("Delta")])
        })
        .axis(|axis| axis.title("Group").grid(false))
}

fn whisker_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().transform_no_output(Filter::new(inlier_predicate()), |group| {
        group.transform(
            boxplot_whisker_stats([col("group")], col("value")),
            |group, whiskers| {
                group
                    .mark(
                        Rule::new()
                            .id("whiskers")
                            .x(whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD))
                            .x2(whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD))
                            .y_with(col("group"), |y| y.band(0.5))
                            .y2_with(col("group"), |y| y.band(0.5))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(1),
                    )
                    .mark(
                        Rule::new()
                            .id("lower_cap")
                            .x(whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD))
                            .x2(whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD))
                            .y_with(col("group"), |y| y.band(0.32))
                            .y2_with(col("group"), |y| y.band(0.68))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(2),
                    )
                    .mark(
                        Rule::new()
                            .id("upper_cap")
                            .x(whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD))
                            .x2(whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD))
                            .y_with(col("group"), |y| y.band(0.32))
                            .y2_with(col("group"), |y| y.band(0.68))
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(2),
                    )
            },
        )
    })
}

fn box_summary_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().transform(
        boxplot_summary_stats([col("group")], col("value")),
        |group, stats| {
            group
                .mark(
                    Rect::new()
                        .id("box")
                        .x_with(stats.output(BOX_PLOT_Q1_FIELD), |x| {
                            x.scale_with::<Linear>(|scale| {
                                scale.domain_interval(lit(0.0), lit(36.0))
                            })
                            .axis(|axis| axis.title("Value").grid(true))
                        })
                        .x2(stats.output(BOX_PLOT_Q3_FIELD))
                        .y_with(col("group"), |y| y_category_axis(y).band(0.26))
                        .y2_with(col("group"), |y| y.band(0.74))
                        .fill("#bfdbfe")
                        .stroke("#2563eb")
                        .stroke_width(1.5)
                        .zindex(3),
                )
                .mark(
                    Rule::new()
                        .id("median")
                        .x(stats.output(BOX_PLOT_MEDIAN_FIELD))
                        .x2(stats.output(BOX_PLOT_MEDIAN_FIELD))
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
    MarkGroup::new().transform_no_output(Filter::new(outlier_predicate()), |group| {
        group.mark(
            Symbol::new()
                .id("outliers")
                .x(col("value"))
                .y_with(col("group"), |y| y.band(0.5))
                .fill("#f97316")
                .stroke("#ffffff")
                .stroke_width(1.25)
                .size(95.0)
                .zindex(5),
        )
    })
}

fn grouped_y_axis(value: CartesianPositionConfig, include_axis: bool) -> CartesianPositionConfig {
    let value = value
        .level(0, |level| level.padding_inner(0.38).padding_outer(0.12))
        .level(1, |level| {
            level
                .nest_scope(NestScope::Shared)
                .padding_inner(0.12)
                .axis(|axis| axis.title("Segment"))
        });
    if include_axis {
        value.axis(|axis| axis.title("Segment grouped by category").grid(false))
    } else {
        value
    }
}

fn grouped_y_band(
    value: CartesianPositionConfig,
    band: f64,
    include_axis: bool,
) -> CartesianPositionConfig {
    grouped_y_axis(value, include_axis).band(band)
}

fn grouped_whisker_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().transform_no_output(Filter::new(inlier_predicate()), |group| {
        group.transform(
            boxplot_whisker_stats([col("category"), col("segment")], col("value")),
            |group, whiskers| {
                group
                    .mark(
                        Rule::new()
                            .id("whiskers")
                            .x(whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD))
                            .x2(whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD))
                            .y_with(nested(["category", "segment"]), |y| {
                                grouped_y_band(y, 0.5, false)
                            })
                            .y2_with(nested(["category", "segment"]), |y| {
                                grouped_y_band(y, 0.5, false)
                            })
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(1),
                    )
                    .mark(
                        Rule::new()
                            .id("lower_cap")
                            .x(whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD))
                            .x2(whiskers.output(BOX_PLOT_WHISKER_LOW_FIELD))
                            .y_with(nested(["category", "segment"]), |y| {
                                grouped_y_band(y, 0.32, false)
                            })
                            .y2_with(nested(["category", "segment"]), |y| {
                                grouped_y_band(y, 0.68, false)
                            })
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(2),
                    )
                    .mark(
                        Rule::new()
                            .id("upper_cap")
                            .x(whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD))
                            .x2(whiskers.output(BOX_PLOT_WHISKER_HIGH_FIELD))
                            .y_with(nested(["category", "segment"]), |y| {
                                grouped_y_band(y, 0.32, false)
                            })
                            .y2_with(nested(["category", "segment"]), |y| {
                                grouped_y_band(y, 0.68, false)
                            })
                            .stroke("#475569")
                            .stroke_width(1.5)
                            .zindex(2),
                    )
            },
        )
    })
}

fn grouped_box_summary_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().transform(
        boxplot_summary_stats([col("category"), col("segment")], col("value")),
        |group, stats| {
            group
                .mark(
                    Rect::new()
                        .id("box")
                        .x_with(stats.output(BOX_PLOT_Q1_FIELD), |x| {
                            x.scale_with::<Linear>(|scale| {
                                scale.domain_interval(lit(0.0), lit(36.0))
                            })
                            .axis(|axis| axis.title("Value").grid(true))
                        })
                        .x2(stats.output(BOX_PLOT_Q3_FIELD))
                        .y_with(nested(["category", "segment"]), |y| {
                            grouped_y_band(y, 0.26, true)
                        })
                        .y2_with(nested(["category", "segment"]), |y| {
                            grouped_y_band(y, 0.74, false)
                        })
                        .fill_with(col("segment"), |fill| {
                            fill.legend(|legend| legend.title("Segment"))
                        })
                        .stroke("#2563eb")
                        .stroke_width(1.5)
                        .zindex(3),
                )
                .mark(
                    Rule::new()
                        .id("median")
                        .x(stats.output(BOX_PLOT_MEDIAN_FIELD))
                        .x2(stats.output(BOX_PLOT_MEDIAN_FIELD))
                        .y_with(nested(["category", "segment"]), |y| {
                            grouped_y_band(y, 0.24, false)
                        })
                        .y2_with(nested(["category", "segment"]), |y| {
                            grouped_y_band(y, 0.76, false)
                        })
                        .stroke("#1e3a8a")
                        .stroke_width(2.2)
                        .zindex(4),
                )
        },
    )
}

fn grouped_outlier_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().transform_no_output(Filter::new(outlier_predicate()), |group| {
        group.mark(
            Symbol::new()
                .id("outliers")
                .x(col("value"))
                .y_with(nested(["category", "segment"]), |y| {
                    grouped_y_band(y, 0.5, false)
                })
                .fill("#f97316")
                .stroke("#ffffff")
                .stroke_width(1.25)
                .size(95.0)
                .zindex(5),
        )
    })
}

fn grouped_fence_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new().transform_no_output(
        boxplot_fence_stats([col("category"), col("segment")], col("value")),
        |group| {
            group
                .mark(grouped_whisker_branch())
                .mark(grouped_outlier_branch())
        },
    )
}

fn fence_branch() -> MarkGroup<Cartesian> {
    MarkGroup::new()
        .transform_no_output(boxplot_fence_stats([col("group")], col("value")), |group| {
            group.mark(whisker_branch()).mark(outlier_branch())
        })
}

fn box_plot_group() -> MarkGroup<Cartesian> {
    MarkGroup::new()
        .id("manual_box_plot")
        .mark(fence_branch())
        .mark(box_summary_branch())
}

fn grouped_box_plot_group() -> MarkGroup<Cartesian> {
    MarkGroup::new()
        .id("manual_grouped_box_plot")
        .mark(grouped_fence_branch())
        .mark(grouped_box_summary_branch())
}

#[tokio::test]
async fn box_plot_from_mark_group_branches() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Box plot from MarkGroup branches")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .mark(box_plot_group())
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("manual_box_plot.outliers"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ));

    let compiled = plot.compile(&ctx).await.expect("compile box plot group");
    let event_binding = compiled.event_bindings().first().expect("event binding");
    let between = event_binding.between.as_ref().expect("between binding");
    assert_eq!(
        between.start.resolved_mark_paths(),
        Some(&[vec![3usize]][..])
    );
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_group",
        "box_plot_from_mark_group_branches",
    )
    .await;
}

#[tokio::test]
async fn box_plot_from_mark_group_nested_band() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Grouped box plot from MarkGroup branches")
        .canvas_size(780.0, 460.0)
        .data(grouped_box_plot_data(&ctx))
        .mark(grouped_box_plot_group())
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown)
                .mark("manual_grouped_box_plot.outliers"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile grouped box plot group");
    let event_binding = compiled.event_bindings().first().expect("event binding");
    let between = event_binding.between.as_ref().expect("between binding");
    assert_eq!(
        between.start.resolved_mark_paths(),
        Some(&[vec![3usize]][..])
    );
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_group",
        "box_plot_from_mark_group_nested_band",
    )
    .await;
}

#[tokio::test]
async fn box_plot_from_mark_group_faceted() {
    let ctx = SessionContext::new();
    let cell = Plot::<Cartesian>::new().mark(grouped_box_plot_group());
    let plot = Plot::<FacetColumn>::new()
        .title("Faceted box plot from MarkGroup branches")
        .canvas_size(980.0, 500.0)
        .data(faceted_grouped_box_plot_data(&ctx))
        .mark(Subplot::new(cell).column(col("region")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile faceted manual box plot group");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "mark_group",
        "box_plot_from_mark_group_faceted",
    )
    .await;
}
