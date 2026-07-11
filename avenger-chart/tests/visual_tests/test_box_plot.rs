use super::helpers::assert_visual_match_default;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use avenger_chart_marks_statistical::box_plot::{
    BOX_PLOT_MEDIAN_FIELD, BOX_PLOT_Q1_FIELD, BOX_PLOT_Q3_FIELD,
};
use datafusion::arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::{
    dataframe::DataFrame,
    prelude::{SessionContext, col, lit},
    scalar::ScalarValue,
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

fn box_plot_no_outliers_data(ctx: &SessionContext) -> DataFrame {
    box_plot_data_from_group_values(
        ctx,
        &[
            (
                "Alpha",
                [4.0, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5, 8.0, 8.5, 9.0].as_slice(),
            ),
            (
                "Beta",
                [10.0, 11.0, 11.5, 12.0, 12.5, 13.0, 14.0, 14.5, 15.0, 16.0].as_slice(),
            ),
            (
                "Gamma",
                [16.0, 17.0, 17.5, 18.0, 18.5, 19.0, 20.0, 20.5, 21.0, 22.0].as_slice(),
            ),
        ],
    )
}

fn box_plot_single_observation_data(ctx: &SessionContext) -> DataFrame {
    box_plot_data_from_group_values(
        ctx,
        &[
            ("Alpha", [8.0].as_slice()),
            (
                "Beta",
                [10.0, 11.0, 11.5, 12.0, 12.5, 13.0, 14.0].as_slice(),
            ),
            (
                "Gamma",
                [16.0, 17.0, 17.5, 18.0, 18.5, 19.0, 20.0].as_slice(),
            ),
        ],
    )
}

fn box_plot_all_outliers_data(ctx: &SessionContext) -> DataFrame {
    box_plot_data_from_group_values(
        ctx,
        &[
            (
                "Alpha",
                [4.0, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5, 8.0, 8.5, 28.0, 32.0].as_slice(),
            ),
            (
                "Beta",
                [10.0, 11.0, 11.5, 12.0, 12.5, 13.0, 14.0, 14.5, 15.0, 16.0].as_slice(),
            ),
            (
                "Gamma",
                [16.0, 17.0, 17.5, 18.0, 18.5, 19.0, 20.0, 20.5, 21.0, 22.0].as_slice(),
            ),
        ],
    )
}

fn box_plot_null_values_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("group", DataType::Utf8, true),
        Field::new("value", DataType::Float64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                Some("Alpha"),
                Some("Alpha"),
                Some("Alpha"),
                Some("Alpha"),
                Some("Alpha"),
                Some("Alpha"),
                Some("Alpha"),
                Some("Beta"),
                Some("Beta"),
                Some("Beta"),
                Some("Beta"),
                Some("Beta"),
                Some("Beta"),
                Some("Beta"),
                Some("Gamma"),
                Some("Gamma"),
                Some("Gamma"),
                Some("Gamma"),
                Some("Gamma"),
                Some("Gamma"),
                Some("Gamma"),
                None,
            ])) as _,
            Arc::new(Float64Array::from(vec![
                Some(4.0),
                Some(5.0),
                None,
                Some(6.0),
                Some(6.5),
                Some(7.0),
                Some(8.0),
                Some(10.0),
                None,
                Some(11.5),
                Some(12.0),
                Some(12.5),
                Some(13.0),
                Some(14.0),
                Some(16.0),
                Some(17.0),
                Some(17.5),
                None,
                Some(18.5),
                Some(19.0),
                Some(20.0),
                Some(15.0),
            ])) as _,
        ],
    )
    .expect("nullable box plot batch");
    ctx.read_batch(batch).expect("nullable box plot dataframe")
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

fn sparse_grouped_box_plot_data(ctx: &SessionContext) -> DataFrame {
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
            "Enterprise",
            [20.0, 21.0, 23.0, 24.0, 24.5, 25.0, 26.0, 34.0].as_slice(),
        ),
        (
            "Services",
            "SMB",
            [7.0, 8.0, 9.0, 9.5, 10.0, 11.0, 12.0, 18.0].as_slice(),
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
    .expect("sparse grouped box plot batch");
    ctx.read_batch(batch)
        .expect("sparse grouped box plot dataframe")
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

fn sparse_faceted_grouped_box_plot_data(ctx: &SessionContext) -> DataFrame {
    let mut regions = Vec::new();
    let mut categories = Vec::new();
    let mut segments = Vec::new();
    let mut values = Vec::new();
    let observations = [
        (
            "North",
            "Platform",
            "SMB",
            [12.0, 14.0, 15.0, 15.5, 16.0, 17.0, 18.5, 24.0].as_slice(),
        ),
        (
            "North",
            "Platform",
            "Enterprise",
            [18.0, 19.0, 21.0, 22.0, 22.5, 23.0, 24.0, 31.0].as_slice(),
        ),
        (
            "North",
            "Infrastructure",
            "Enterprise",
            [20.0, 21.0, 23.0, 24.0, 24.5, 25.0, 26.0, 34.0].as_slice(),
        ),
        (
            "South",
            "Platform",
            "SMB",
            [13.0, 15.0, 16.0, 16.5, 17.0, 18.0, 19.5, 25.0].as_slice(),
        ),
        (
            "South",
            "Infrastructure",
            "SMB",
            [10.0, 11.0, 12.0, 13.0, 13.5, 14.0, 15.0, 20.0].as_slice(),
        ),
        (
            "South",
            "Services",
            "Enterprise",
            [16.0, 17.0, 18.0, 19.0, 19.5, 20.0, 21.0, 29.0].as_slice(),
        ),
    ];

    for (region, category, segment, group_values) in observations {
        for value in group_values {
            regions.push(region);
            categories.push(category);
            segments.push(segment);
            values.push(*value);
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
    .expect("sparse faceted grouped box plot batch");
    ctx.read_batch(batch)
        .expect("sparse faceted grouped box plot dataframe")
}

fn facet_free_value_box_plot_data(ctx: &SessionContext) -> DataFrame {
    let mut regions = Vec::new();
    let mut groups = Vec::new();
    let mut values = Vec::new();
    let observations = [
        (
            "North",
            "Alpha",
            [4.0, 5.0, 5.5, 6.0, 6.5, 7.0, 8.0, 14.0].as_slice(),
        ),
        (
            "North",
            "Beta",
            [8.0, 9.0, 9.5, 10.0, 10.5, 11.0, 12.0, 18.0].as_slice(),
        ),
        (
            "South",
            "Alpha",
            [42.0, 45.0, 47.0, 48.0, 49.0, 50.0, 52.0, 64.0].as_slice(),
        ),
        (
            "South",
            "Beta",
            [55.0, 58.0, 60.0, 61.0, 62.0, 63.0, 66.0, 78.0].as_slice(),
        ),
    ];
    for (region, group, group_values) in observations {
        for value in group_values {
            regions.push(region);
            groups.push(group);
            values.push(*value);
        }
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(regions)) as _,
            Arc::new(StringArray::from(groups)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("facet free value box plot batch");
    ctx.read_batch(batch)
        .expect("facet free value box plot dataframe")
}

fn repeat_box_plot_data(ctx: &SessionContext) -> DataFrame {
    let mut categories = Vec::new();
    let mut segments = Vec::new();
    let mut throughput = Vec::new();
    let mut latency = Vec::new();
    let observations = [
        ("Platform", "SMB", 14.0, 42.0),
        ("Platform", "SMB", 15.0, 39.0),
        ("Platform", "SMB", 16.0, 37.0),
        ("Platform", "SMB", 18.0, 34.0),
        ("Platform", "Enterprise", 21.0, 32.0),
        ("Platform", "Enterprise", 22.5, 30.0),
        ("Platform", "Enterprise", 24.0, 28.0),
        ("Platform", "Enterprise", 31.0, 22.0),
        ("Infrastructure", "SMB", 10.0, 48.0),
        ("Infrastructure", "SMB", 12.0, 44.0),
        ("Infrastructure", "SMB", 13.0, 42.0),
        ("Infrastructure", "SMB", 19.0, 35.0),
        ("Infrastructure", "Enterprise", 23.0, 30.0),
        ("Infrastructure", "Enterprise", 24.5, 28.0),
        ("Infrastructure", "Enterprise", 26.0, 25.0),
        ("Infrastructure", "Enterprise", 34.0, 18.0),
    ];
    for (category, segment, throughput_value, latency_value) in observations {
        categories.push(category);
        segments.push(segment);
        throughput.push(throughput_value);
        latency.push(latency_value);
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("segment", DataType::Utf8, false),
        Field::new("throughput", DataType::Float64, false),
        Field::new("latency", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(categories)) as _,
            Arc::new(StringArray::from(segments)) as _,
            Arc::new(Float64Array::from(throughput)) as _,
            Arc::new(Float64Array::from(latency)) as _,
        ],
    )
    .expect("repeat box plot batch");
    ctx.read_batch(batch).expect("repeat box plot dataframe")
}

fn y_category_axis(value: CartesianPositionConfig) -> CartesianPositionConfig {
    value
        .scale_with::<Band>(|scale| {
            scale.domain_discrete(vec![lit("Alpha"), lit("Beta"), lit("Gamma"), lit("Delta")])
        })
        .axis(|axis| axis.title("Group").grid(false))
}

fn x_category_axis(value: CartesianPositionConfig) -> CartesianPositionConfig {
    value
        .scale_with::<Band>(|scale| {
            scale.domain_discrete(vec![lit("Alpha"), lit("Beta"), lit("Gamma"), lit("Delta")])
        })
        .axis(|axis| axis.title("Group").grid(false))
}

fn grouped_box_plot_mark(
    id: &str,
    inner_axis_visible: bool,
    fill_column: &'static str,
    legend_title: &'static str,
) -> BoxPlot {
    BoxPlot::new()
        .id(id)
        .x_with(col("value"), |x| {
            x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(36.0)))
                .axis(|axis| axis.title("Value").grid(true))
        })
        .y_with(nested(["category", "segment"]), |y| {
            y.axis(|axis| axis.title("Segment grouped by category").grid(false))
                .level(0, |level| level.padding_inner(0.38).padding_outer(0.12))
                .level(1, |level| {
                    let level = level.nest_scope(NestScope::Shared).padding_inner(0.12);
                    if inner_axis_visible {
                        level.axis(|axis| axis.title("Segment"))
                    } else {
                        level.axis(|axis| axis.visible(false))
                    }
                })
        })
        .fill_with(col(fill_column), |fill| {
            fill.legend(|legend| legend.title(legend_title))
        })
}

fn ordered_grouped_box_plot_mark() -> BoxPlot {
    BoxPlot::new()
        .id("ordered_grouped_box_plot")
        .x_with(col("value"), |x| {
            x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(36.0)))
                .axis(|axis| axis.title("Value").grid(true))
        })
        .y_with(nested(["category", "segment"]), |y| {
            y.axis(|axis| axis.title("Segment grouped by category").grid(false))
                .level(0, |level| {
                    level
                        .domain_values(vec![
                            lit("Services"),
                            lit("Infrastructure"),
                            lit("Platform"),
                        ])
                        .padding_inner(0.38)
                        .padding_outer(0.12)
                })
                .level(1, |level| {
                    level
                        .nest_scope(NestScope::Shared)
                        .domain_values(vec![lit("Enterprise"), lit("SMB")])
                        .padding_inner(0.12)
                        .axis(|axis| axis.title("Segment"))
                })
        })
        .fill_with(col("segment"), |fill| {
            fill.legend(|legend| legend.title("Segment"))
        })
}

fn box_plot_test_mark() -> BoxPlot {
    BoxPlot::new()
        .id("my_box_plot")
        .x_with(col("value"), |x| {
            x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(36.0)))
                .axis(|axis| axis.title("Value").grid(true))
        })
        .y_with(col("group"), |y| y_category_axis(y))
}

fn box_plot_auto_group_mark(id: &str, domain_max: f64) -> BoxPlot {
    BoxPlot::new()
        .id(id)
        .x_with(col("value"), move |x| {
            x.scale_with::<Linear>(move |scale| scale.domain_interval(lit(0.0), lit(domain_max)))
                .axis(|axis| axis.title("Value").grid(true))
        })
        .y_with(col("group"), |y| {
            y.scale_with::<Band>(|scale| scale)
                .axis(|axis| axis.title("Group").grid(false))
        })
}

#[tokio::test]
async fn box_plot_compound_matches_mark_group_baseline() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Box plot from MarkGroup branches")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .mark(box_plot_test_mark())
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("my_box_plot.outliers"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ));

    let compiled = plot.compile(&ctx).await.expect("compile box plot");
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
async fn box_plot_compound_event_targets_resolve_standard_parts() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .data(box_plot_data(&ctx))
        .mark(box_plot_test_mark())
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("my_box_plot"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ))
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("my_box_plot.box"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ))
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("my_box_plot.outliers"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile box plot target paths");
    let resolved_start_paths = compiled
        .event_bindings()
        .iter()
        .map(|binding| {
            binding
                .between
                .as_ref()
                .expect("between binding")
                .start
                .resolved_mark_paths()
                .map(|paths| paths.to_vec())
                .expect("resolved mark paths")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        resolved_start_paths,
        vec![
            vec![
                vec![0usize],
                vec![1usize],
                vec![2usize],
                vec![3usize],
                vec![4usize],
                vec![5usize],
            ],
            vec![vec![4usize]],
            vec![vec![3usize]],
        ]
    );
}

#[tokio::test]
async fn box_plot_compound_unrooted_part_target_errors() {
    let ctx = SessionContext::new();
    let err = match Chart::<Cartesian>::new()
        .data(box_plot_data(&ctx))
        .mark(box_plot_test_mark())
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("outliers"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ))
        .compile(&ctx)
        .await
    {
        Ok(_) => panic!("unrooted generated box plot part target should fail"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("Unknown mark target 'outliers'"),
        "{err}"
    );
}

#[tokio::test]
async fn box_plot_compound_part_event_datums_reflect_branch_rows() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .data(box_plot_data(&ctx))
        .mark(box_plot_test_mark())
        .event_binding(
            ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("my_box_plot.box"),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(avenger_chart::event::datum(BOX_PLOT_Q1_FIELD).is_not_null())
            .filter(avenger_chart::event::datum(BOX_PLOT_MEDIAN_FIELD).is_not_null())
            .filter(avenger_chart::event::datum(BOX_PLOT_Q3_FIELD).is_not_null()),
        )
        .event_binding(
            ChartEventBinding::on_between_end(
                ChartEventStream::on(ChartEventType::MouseDown).mark("my_box_plot.outliers"),
                ChartEventStream::on(ChartEventType::MouseUp),
            )
            .filter(avenger_chart::event::datum("value").is_not_null())
            .filter(avenger_chart::event::datum(BOX_PLOT_Q1_FIELD).is_not_null())
            .filter(avenger_chart::event::datum(BOX_PLOT_Q3_FIELD).is_not_null()),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile box plot datum bindings");
    let event_datum_types = compiled.event_datum_types();
    assert_eq!(
        event_datum_types.get(BOX_PLOT_Q1_FIELD),
        Some(&DataType::Float64)
    );
    assert_eq!(
        event_datum_types.get(BOX_PLOT_MEDIAN_FIELD),
        Some(&DataType::Float64)
    );
    assert_eq!(
        event_datum_types.get(BOX_PLOT_Q3_FIELD),
        Some(&DataType::Float64)
    );
    assert_eq!(event_datum_types.get("value"), Some(&DataType::Float64));

    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate plot");
    let rows_for_path = |path: &[usize]| {
        evaluated
            .event_datums
            .rows
            .iter()
            .find(|rows| rows.mark_path == path || rows.mark_path.ends_with(path))
            .unwrap_or_else(|| {
                let paths = evaluated
                    .event_datums
                    .rows
                    .iter()
                    .map(|rows| rows.mark_path.clone())
                    .collect::<Vec<_>>();
                panic!("missing retained event datum rows for path {path:?}; available: {paths:?}")
            })
    };

    let box_rows = rows_for_path(&[4]);
    assert_eq!(box_rows.rows.num_rows(), 4);
    assert!(box_rows.rows.column_by_name(BOX_PLOT_Q1_FIELD).is_some());
    assert!(
        box_rows
            .rows
            .column_by_name(BOX_PLOT_MEDIAN_FIELD)
            .is_some()
    );
    assert!(box_rows.rows.column_by_name(BOX_PLOT_Q3_FIELD).is_some());
    assert!(
        box_rows.rows.column_by_name("value").is_none(),
        "summary box rows should not expose raw observation values"
    );

    let outlier_rows = rows_for_path(&[3]);
    assert!(outlier_rows.rows.num_rows() > 0);
    assert!(outlier_rows.rows.column_by_name("value").is_some());
    assert!(
        outlier_rows
            .rows
            .column_by_name(BOX_PLOT_Q1_FIELD)
            .is_some()
    );
    assert!(
        outlier_rows
            .rows
            .column_by_name(BOX_PLOT_Q3_FIELD)
            .is_some()
    );
    assert!(
        outlier_rows
            .rows
            .column_by_name(BOX_PLOT_MEDIAN_FIELD)
            .is_none(),
        "outlier rows are raw rows with joined fence stats, not summary rows"
    );
}

#[tokio::test]
async fn box_plot_compound_scene_query_targets_resolve_part_path() {
    let ctx = SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .data(box_plot_data(&ctx))
        .mark(box_plot_test_mark())
        .selection(Selection::new("picked"))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click).set_selection(
                "picked",
                SelectionUpdate::replace_all_from_scene_query(SelectionSceneQuery::new(
                    SceneGeometryQuery::rect(lit(0.0), lit(0.0), lit(10.0), lit(10.0))
                        .mark("my_box_plot.outliers")
                        .datum_field(SceneQueryDatumField::new("value")),
                )),
            ),
        )
        .compile(&ctx)
        .await
        .expect("compile box plot scene query target");

    let binding = compiled.event_bindings().first().expect("event binding");
    let SelectionUpdate::ReplaceAllFromSceneQuery { query } =
        &binding.selection_assignments[0].update
    else {
        panic!("expected scene query selection update");
    };
    assert_eq!(
        query.query.target.resolved_mark_paths(),
        Some(&[vec![3usize]][..])
    );
}

#[tokio::test]
async fn box_plot_compound_part_ids_are_scoped_by_root() {
    let ctx = SessionContext::new();
    let compiled = Chart::<Cartesian>::new()
        .data(box_plot_data(&ctx))
        .mark(box_plot_test_mark().id("first_box_plot"))
        .mark(box_plot_test_mark().id("second_box_plot"))
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown)
                .marks(["first_box_plot.outliers", "second_box_plot.outliers"]),
            ChartEventStream::on(ChartEventType::MouseUp),
        ))
        .compile(&ctx)
        .await
        .expect("compile sibling box plot part targets");

    let binding = compiled.event_bindings().first().expect("event binding");
    let between = binding.between.as_ref().expect("between binding");
    assert_eq!(
        between.start.resolved_mark_paths(),
        Some(&[vec![3usize], vec![9usize]][..])
    );
}

#[tokio::test]
async fn box_plot_compound_with_pre_filter_transform_matches_baseline() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Box plot from MarkGroup branches")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .mark(BoxPlot::new().id("filtered_box_plot").transform_no_output(
            Filter::new(col("value").gt(lit(0.0))),
            |box_plot| {
                box_plot
                    .x_with(col("value"), |x| {
                        x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(36.0)))
                            .axis(|axis| axis.title("Value").grid(true))
                    })
                    .y_with(col("group"), |y| y_category_axis(y))
            },
        ));

    let compiled = plot.compile(&ctx).await.expect("compile filtered box plot");
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
async fn box_plot_compound_horizontal() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Horizontal box plot")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .mark(box_plot_test_mark());

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile horizontal box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_compound_horizontal",
    )
    .await;
}

#[tokio::test]
async fn box_plot_grouped_nested_band_fill_by_segment() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Grouped box plot")
        .canvas_size(780.0, 460.0)
        .data(grouped_box_plot_data(&ctx))
        .mark(grouped_box_plot_mark(
            "grouped_box_plot",
            true,
            "segment",
            "Segment",
        ))
        .event_binding(ChartEventBinding::on_between_end(
            ChartEventStream::on(ChartEventType::MouseDown).mark("grouped_box_plot.outliers"),
            ChartEventStream::on(ChartEventType::MouseUp),
        ));

    let compiled = plot.compile(&ctx).await.expect("compile grouped box plot");
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
        "boxplot",
        "box_plot_grouped_nested_band_fill_by_segment",
    )
    .await;
}

#[tokio::test]
async fn box_plot_grouped_inner_axis_hidden() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Grouped box plot")
        .canvas_size(780.0, 420.0)
        .data(grouped_box_plot_data(&ctx))
        .mark(grouped_box_plot_mark(
            "grouped_box_plot",
            false,
            "segment",
            "Segment",
        ));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile grouped box plot with hidden inner axis");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_grouped_inner_axis_hidden",
    )
    .await;
}

#[tokio::test]
async fn box_plot_grouped_nested_band_sparse_segments() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Sparse grouped box plot")
        .canvas_size(780.0, 460.0)
        .data(sparse_grouped_box_plot_data(&ctx))
        .mark(grouped_box_plot_mark(
            "sparse_grouped_box_plot",
            true,
            "segment",
            "Segment",
        ));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile sparse grouped box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_grouped_nested_band_sparse_segments",
    )
    .await;
}

#[tokio::test]
async fn box_plot_grouped_nested_band_fill_by_category() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Grouped box plot by category")
        .canvas_size(780.0, 460.0)
        .data(grouped_box_plot_data(&ctx))
        .mark(grouped_box_plot_mark(
            "category_filled_box_plot",
            true,
            "category",
            "Category",
        ));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile category-filled grouped box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_grouped_nested_band_fill_by_category",
    )
    .await;
}

#[tokio::test]
async fn box_plot_grouped_nested_band_ordered_segments() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Ordered grouped box plot")
        .canvas_size(780.0, 460.0)
        .data(grouped_box_plot_data(&ctx))
        .mark(ordered_grouped_box_plot_mark());

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile ordered grouped box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_grouped_nested_band_ordered_segments",
    )
    .await;
}

#[tokio::test]
async fn box_plot_invalid_fill_column_errors() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .data(grouped_box_plot_data(&ctx))
        .mark(
            BoxPlot::new()
                .id("invalid_box_plot")
                .x(col("value"))
                .y(nested(["category", "segment"]))
                .fill(col("region")),
        );

    let err = match plot.compile(&ctx).await {
        Ok(_) => panic!("fill by an ungrouped column should fail"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("region"),
        "error should name the invalid fill column: {err}"
    );
    assert!(
        err.to_string().contains("category") && err.to_string().contains("segment"),
        "error should list preserved grouping columns: {err}"
    );
}

#[tokio::test]
async fn box_plot_outlier_symbol_styling() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Styled box plot outliers")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .mark(
            BoxPlot::new()
                .id("styled_outlier_box_plot")
                .x_with(col("value"), |x| {
                    x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(36.0)))
                        .axis(|axis| axis.title("Value").grid(true))
                })
                .y_with(col("group"), |y| y_category_axis(y))
                .outliers(|outliers| {
                    outliers
                        .size(150.0)
                        .fill("#ec4899")
                        .stroke("#111827")
                        .stroke_width(2.0)
                        .shape("diamond")
                        .angle(45.0)
                        .opacity(0.85)
                }),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile outlier-styled box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_outlier_symbol_styling",
    )
    .await;
}

#[tokio::test]
async fn box_plot_part_styling() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Styled box plot parts")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .mark(
            BoxPlot::new()
                .id("styled_part_box_plot")
                .x_with(col("value"), |x| {
                    x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(36.0)))
                        .axis(|axis| axis.title("Value").grid(true))
                })
                .y_with(col("group"), |y| y_category_axis(y))
                .box_body(|body| {
                    body.fill("#fde68a")
                        .stroke("#92400e")
                        .stroke_width(2.0)
                        .opacity(0.92)
                        .band(0.20, 0.80)
                })
                .median(|median| median.stroke("#dc2626").stroke_width(3.0).band(0.16, 0.84))
                .whiskers(|whiskers| whiskers.stroke("#0f766e").stroke_width(2.0).opacity(0.9))
                .caps(|caps| caps.stroke("#0f766e").stroke_width(2.0).band(0.26, 0.74)),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile part-styled box plot");
    assert_visual_match_default(&compiled, &ctx, None, "boxplot", "box_plot_part_styling").await;
}

#[tokio::test]
async fn box_plot_scalar_param_style() {
    let ctx = SessionContext::new();
    let box_fill = Param::new("box_fill", ScalarValue::Utf8(Some("#e0f2fe".to_string())));
    let median_width = Param::new("median_width", ScalarValue::Float32(Some(3.5)));
    let plot = Chart::<Cartesian>::new()
        .title("Param-styled box plot")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .param(box_fill.clone())
        .param(median_width.clone())
        .mark(
            BoxPlot::new()
                .id("param_styled_box_plot")
                .x_with(col("value"), |x| {
                    x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(36.0)))
                        .axis(|axis| axis.title("Value").grid(true))
                })
                .y_with(col("group"), |y| y_category_axis(y))
                .box_body(|body| {
                    body.fill_with(box_fill.expr(), |fill| fill.no_scale())
                        .stroke("#0369a1")
                        .stroke_width(1.75)
                })
                .median(|median| {
                    median
                        .stroke("#7c3aed")
                        .stroke_width_with(median_width.expr(), |stroke_width| {
                            stroke_width.no_scale()
                        })
                }),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile param-styled box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_scalar_param_style",
    )
    .await;
}

#[tokio::test]
async fn box_plot_compound_vertical() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Vertical box plot")
        .canvas_size(720.0, 420.0)
        .data(box_plot_data(&ctx))
        .mark(
            BoxPlot::new()
                .id("vertical_box_plot")
                .vertical()
                .x_with(col("group"), |x| x_category_axis(x))
                .y_with(col("value"), |y| {
                    y.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(36.0)))
                        .axis(|axis| axis.title("Value").grid(true))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("compile vertical box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_compound_vertical",
    )
    .await;
}

#[tokio::test]
async fn box_plot_no_outliers() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Box plot with no outliers")
        .canvas_size(720.0, 390.0)
        .data(box_plot_no_outliers_data(&ctx))
        .mark(box_plot_auto_group_mark("no_outliers_box_plot", 22.0));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile no-outlier box plot");
    assert_visual_match_default(&compiled, &ctx, None, "boxplot", "box_plot_no_outliers").await;
}

#[tokio::test]
async fn box_plot_single_observation_group() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Box plot with a single-observation group")
        .canvas_size(720.0, 390.0)
        .data(box_plot_single_observation_data(&ctx))
        .mark(box_plot_auto_group_mark(
            "single_observation_box_plot",
            22.0,
        ));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile single-observation box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_single_observation_group",
    )
    .await;
}

#[tokio::test]
async fn box_plot_all_outliers_in_one_group() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Box plot with separated outliers")
        .canvas_size(720.0, 390.0)
        .data(box_plot_all_outliers_data(&ctx))
        .mark(box_plot_auto_group_mark("all_outliers_box_plot", 36.0));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile all-outliers box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_all_outliers_in_one_group",
    )
    .await;
}

#[tokio::test]
async fn box_plot_null_values_ignored() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Box plot with null values")
        .canvas_size(720.0, 390.0)
        .data(box_plot_null_values_data(&ctx))
        .mark(box_plot_auto_group_mark("nullable_box_plot", 24.0));

    let compiled = plot.compile(&ctx).await.expect("compile nullable box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_null_values_ignored",
    )
    .await;
}

#[tokio::test]
async fn box_plot_facet_nested_band_shared_segments() {
    let ctx = SessionContext::new();
    let cell = Plot::<Cartesian>::new().mark(
        BoxPlot::new()
            .id("facet_grouped_box_plot")
            .x_with(col("value"), |x| {
                x.with_domain_scope(CoordinationScope::Shared)
                    .scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(40.0)))
                    .axis(|axis| axis.title("Value").grid(true))
            })
            .y_with(nested(["category", "segment"]), |y| {
                y.with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("Segment grouped by category").grid(false))
                    .level(0, |level| {
                        level
                            .domain_scope(CoordinationScope::Shared)
                            .padding_inner(0.38)
                            .padding_outer(0.12)
                    })
                    .level(1, |level| {
                        level
                            .domain_scope(CoordinationScope::Shared)
                            .nest_scope(NestScope::Shared)
                            .padding_inner(0.12)
                            .axis(|axis| axis.visible(false))
                    })
            })
            .fill_with(col("segment"), |fill| {
                fill.legend(|legend| legend.title("Segment"))
            }),
    );
    let plot = Chart::<FacetColumn>::new()
        .title("Faceted grouped box plot")
        .canvas_size(980.0, 500.0)
        .data(faceted_grouped_box_plot_data(&ctx))
        .mark(Subplot::new(cell).column(col("region")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile faceted grouped box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_facet_nested_band_shared_segments",
    )
    .await;
}

#[tokio::test]
async fn box_plot_facet_free_value_domains() {
    let ctx = SessionContext::new();
    let cell = Plot::<Cartesian>::new().mark(
        BoxPlot::new()
            .id("facet_free_box_plot")
            .x_with(col("value"), |x| {
                x.with_domain_scope(CoordinationScope::Free)
                    .axis(|axis| axis.title("Value").grid(true))
            })
            .y_with(col("group"), |y| {
                y.scale_with::<Band>(|scale| scale)
                    .axis(|axis| axis.title("Group").grid(false))
            }),
    );
    let plot = Chart::<FacetColumn>::new()
        .title("Faceted box plot with free value domains")
        .canvas_size(920.0, 450.0)
        .data(facet_free_value_box_plot_data(&ctx))
        .mark(Subplot::new(cell).column(col("region")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile free-domain faceted box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_facet_free_value_domains",
    )
    .await;
}

#[tokio::test]
async fn box_plot_facet_sparse_nested_segments() {
    let ctx = SessionContext::new();
    let cell = Plot::<Cartesian>::new().mark(
        BoxPlot::new()
            .id("facet_sparse_box_plot")
            .x_with(col("value"), |x| {
                x.with_domain_scope(CoordinationScope::Shared)
                    .scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(40.0)))
                    .axis(|axis| axis.title("Value").grid(true))
            })
            .y_with(nested(["category", "segment"]), |y| {
                y.with_domain_scope(CoordinationScope::Shared)
                    .axis(|axis| axis.title("Segment grouped by category").grid(false))
                    .level(0, |level| {
                        level
                            .domain_scope(CoordinationScope::Shared)
                            .padding_inner(0.38)
                            .padding_outer(0.12)
                    })
                    .level(1, |level| {
                        level
                            .domain_scope(CoordinationScope::Shared)
                            .nest_scope(NestScope::Shared)
                            .padding_inner(0.12)
                            .axis(|axis| axis.visible(false))
                    })
            })
            .fill_with(col("segment"), |fill| {
                fill.legend(|legend| legend.title("Segment"))
            }),
    );
    let plot = Chart::<FacetColumn>::new()
        .title("Sparse faceted grouped box plot")
        .canvas_size(980.0, 500.0)
        .data(sparse_faceted_grouped_box_plot_data(&ctx))
        .mark(Subplot::new(cell).column(col("region")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile sparse faceted grouped box plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_facet_sparse_nested_segments",
    )
    .await;
}

#[tokio::test]
async fn box_plot_repeat_nested_band() {
    let ctx = SessionContext::new();
    let cell = Plot::<Cartesian>::new().mark(
        BoxPlot::new()
            .id("repeat_box_plot")
            .x_with(repeat::column(), |x| {
                x.scale_with::<Linear>(|scale| scale.domain_interval(lit(0.0), lit(55.0)))
                    .axis(|axis| axis.title(repeat::column_title()).grid(true))
            })
            .y_with(
                nested(["category".to_string(), "segment".to_string()]),
                |y| {
                    y.with_domain_scope(CoordinationScope::Shared)
                        .axis(|axis| axis.title("Segment grouped by category").grid(false))
                        .level(0, |level| {
                            level
                                .domain_scope(CoordinationScope::Shared)
                                .padding_inner(0.38)
                                .padding_outer(0.12)
                        })
                        .level(1, |level| {
                            level
                                .domain_scope(CoordinationScope::Shared)
                                .nest_scope(NestScope::Shared)
                                .padding_inner(0.12)
                                .axis(|axis| axis.visible(false))
                        })
                },
            )
            .fill_with(col("segment"), |fill| {
                fill.legend(|legend| legend.title("Segment"))
            }),
    );
    let plot = Chart::<RepeatColumns>::new()
        .title("Repeated grouped box plots")
        .canvas_size(980.0, 480.0)
        .data(repeat_box_plot_data(&ctx))
        .configure_coord(|c| {
            c.columns(vec![
                RepeatVariable::field("throughput").title("Throughput"),
                RepeatVariable::field("latency").title("Latency"),
            ])
            .cell(cell)
        });

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile repeated grouped box plots");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "boxplot",
        "box_plot_repeat_nested_band",
    )
    .await;
}
