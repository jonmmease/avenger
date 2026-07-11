use std::sync::Arc;

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{
            ArrayRef, Float32Array, Float64Array, Int32Array, StringArray,
            TimestampMillisecondArray,
        },
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::SessionContext,
};
use palette::Srgba;

fn record_batch(fields: Vec<Field>, columns: Vec<ArrayRef>) -> RecordBatch {
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).expect("record batch")
}

fn nested_x(outer: &str, inner: &str) -> ChannelExpr {
    nested([outer, inner])
}

fn grouped_bar_df(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let batch = record_batch(
        vec![
            Field::new("quarter", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "Q1", "Q1", "Q1", "Q2", "Q2", "Q3", "Q3", "Q3",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "North", "South", "East", "North", "East", "North", "South", "East",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                42.0, 30.0, 34.0, 47.0, 38.0, 51.0, 39.0, 44.0,
            ])) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn grouped_stacked_bar_df(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let rows = [
        ("Q1", "North", "Hardware", 18.0),
        ("Q1", "North", "Software", 12.0),
        ("Q1", "North", "Services", 9.0),
        ("Q1", "South", "Hardware", 14.0),
        ("Q1", "South", "Software", 10.0),
        ("Q1", "South", "Services", 7.0),
        ("Q1", "East", "Hardware", 16.0),
        ("Q1", "East", "Software", 11.0),
        ("Q1", "East", "Services", 8.0),
        ("Q2", "North", "Hardware", 20.0),
        ("Q2", "North", "Software", 15.0),
        ("Q2", "North", "Services", 11.0),
        ("Q2", "South", "Hardware", 12.0),
        ("Q2", "South", "Software", 13.0),
        ("Q2", "South", "Services", 8.0),
        ("Q2", "East", "Hardware", 17.0),
        ("Q2", "East", "Software", 14.0),
        ("Q2", "East", "Services", 10.0),
        ("Q3", "North", "Hardware", 22.0),
        ("Q3", "North", "Software", 16.0),
        ("Q3", "North", "Services", 12.0),
        ("Q3", "South", "Hardware", 15.0),
        ("Q3", "South", "Software", 14.0),
        ("Q3", "South", "Services", 9.0),
        ("Q3", "East", "Hardware", 19.0),
        ("Q3", "East", "Software", 17.0),
        ("Q3", "East", "Services", 11.0),
    ];
    let batch = record_batch(
        vec![
            Field::new("quarter", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.2).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float32Array::from(
                rows.iter().map(|row| row.3).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn temporal_month_spine_df(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let months = (1..=12).collect::<Vec<i32>>();
    let quarters = months
        .iter()
        .map(|month| ((month - 1) / 3) + 1)
        .collect::<Vec<_>>();
    let years = vec![2024; months.len()];
    let values = vec![
        14.0, 0.0, 18.0, 22.0, 0.0, 26.0, 20.0, 24.0, 0.0, 28.0, 0.0, 31.0,
    ];
    let batch = record_batch(
        vec![
            Field::new("year", DataType::Int32, false),
            Field::new("quarter", DataType::Int32, false),
            Field::new("month", DataType::Int32, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(Int32Array::from(years)) as ArrayRef,
            Arc::new(Int32Array::from(quarters)) as ArrayRef,
            Arc::new(Int32Array::from(months)) as ArrayRef,
            Arc::new(Float32Array::from(values)) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn temporal_numeric_label_df(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let batch = record_batch(
        vec![
            Field::new("year", DataType::Int32, false),
            Field::new("quarter", DataType::Int32, false),
            Field::new("month", DataType::Int32, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(Int32Array::from(vec![2024, 2024, 2024, 2025, 2025, 2025])) as ArrayRef,
            Arc::new(Int32Array::from(vec![1, 1, 2, 1, 2, 2])) as ArrayRef,
            Arc::new(Int32Array::from(vec![1, 2, 4, 1, 4, 5])) as ArrayRef,
            Arc::new(Float32Array::from(vec![10.0, 16.0, 24.0, 12.0, 20.0, 28.0])) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn temporal_fact_df(
    ctx: &SessionContext,
    rows: &[(i64, Option<&str>, f64)],
) -> datafusion::dataframe::DataFrame {
    let batch = record_batch(
        vec![
            Field::new(
                "timestamp",
                DataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Millisecond, None),
                false,
            ),
            Field::new("segment", DataType::Utf8, true),
            Field::new("value", DataType::Float64, false),
        ],
        vec![
            Arc::new(TimestampMillisecondArray::from(
                rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|row| row.2).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn facet_nested_df(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let batch = record_batch(
        vec![
            Field::new("market", DataType::Utf8, false),
            Field::new("cyl", DataType::Utf8, false),
            Field::new("make", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "North", "North", "North", "South", "South", "South",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec!["4", "4", "6", "4", "6", "6"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["F", "T", "A", "T", "A", "V"])) as ArrayRef,
            Arc::new(Float32Array::from(vec![34.0, 31.0, 22.0, 29.0, 24.0, 27.0])) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn nested_heatmap_df(
    ctx: &SessionContext,
    rows: &[(&str, &str, &str, &str, f32)],
) -> datafusion::dataframe::DataFrame {
    let batch = record_batch(
        vec![
            Field::new("x_group", DataType::Utf8, false),
            Field::new("x_member", DataType::Utf8, false),
            Field::new("y_group", DataType::Utf8, false),
            Field::new("y_member", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.2).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.3).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float32Array::from(
                rows.iter().map(|row| row.4).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

#[tokio::test]
async fn test_nested_position_grouped_bar_shared_slots_hidden_leaf_axis() {
    let ctx = SessionContext::new();

    let plot = Plot::<Cartesian>::new()
        .data(grouped_bar_df(&ctx))
        .legend("fill", |legend| legend.title("Team"))
        .mark(
            Rect::new()
                .x_with(nested_x("quarter", "team"), |x| {
                    x.axis(|a| a.title("Quarter").grid(false))
                        .level(0, |l| l.padding_inner(0.45).padding_outer(0.15))
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .padding_inner(0.08)
                                .axis(|a| a.visible(false))
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 60.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill_with(col("team"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "grouped_bar_shared_slots_hidden_leaf_axis",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_grouped_bar_shared_slots() {
    let ctx = SessionContext::new();

    let plot = Plot::<Cartesian>::new()
        .data(grouped_bar_df(&ctx))
        .legend("fill", |legend| legend.title("Team"))
        .mark(
            Rect::new()
                .x_with(nested_x("quarter", "team"), |x| {
                    x.axis(|a| a.title("Quarter / Team").grid(false))
                        .level(0, |l| l.padding_inner(0.45).padding_outer(0.15))
                        .level(1, |l| l.nest_scope(NestScope::Shared).padding_inner(0.08))
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 60.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill_with(col("team"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "grouped_bar_shared_slots",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_grouped_stacked_bar() {
    let ctx = SessionContext::new();

    let plot = Plot::<Cartesian>::new()
        .data(grouped_stacked_bar_df(&ctx))
        .legend("fill", |legend| legend.title("Segment"))
        .mark(
            Rect::new().transform(
                Stack::new(col("value"))
                    .group_by([col("quarter"), col("team")])
                    .sort_by_exprs([col("segment")])
                    .name("segment_stack"),
                |mark, stack| {
                    mark.x_with(nested_x("quarter", "team"), |x| {
                        x.axis(|a| a.title("Quarter / Team").grid(false))
                            .level(0, |l| l.padding_inner(0.45).padding_outer(0.15))
                            .level(1, |l| l.nest_scope(NestScope::Shared).padding_inner(0.08))
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y_with(stack.start(), |y| {
                        y.scale(|s| s.domain((0.0, 55.0)))
                            .axis(|a| a.title("Value").grid(true))
                    })
                    .y2(stack.end())
                    .fill_with(col("segment"), |fill| fill)
                    .stroke("#ffffff")
                    .stroke_width(1.0)
                },
            ),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile grouped stacked bar");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "grouped_stacked_bar",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_temporal_nested_manual_complete_month_spine() {
    let ctx = SessionContext::new();

    let plot = Plot::<Cartesian>::new()
        .data(temporal_month_spine_df(&ctx))
        .mark(
            Rect::new()
                .x_with(nested(["year", "quarter", "month"]), |x| {
                    x.axis(|a| a.title("Month grouped by quarter and year").grid(false))
                        .level(0, |l| {
                            l.label_with(time::year_label(col("year")))
                                .padding_inner(0.28)
                                .padding_outer(0.12)
                        })
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .label_with(time::quarter_label(col("quarter")))
                                .padding_inner(0.16)
                                .padding_outer(0.04)
                        })
                        .level(2, |l| {
                            l.label_with(time::month_abbrev_from_number(col("month")))
                                .padding_inner(0.04)
                                .axis(|a| a.label_angle(-90.0))
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 34.0)))
                        .axis(|a| a.title("Completed value").grid(true))
                })
                .y2(col("value"))
                .fill("#4c78a8")
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile temporal nested bar");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "temporal_nested_manual_complete_month_spine",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_temporal_nested_manual_numeric_keys_display_labels() {
    let ctx = SessionContext::new();

    let plot = Plot::<Cartesian>::new()
        .data(temporal_numeric_label_df(&ctx))
        .mark(
            Rect::new()
                .x_with(nested(["year", "quarter", "month"]), |x| {
                    x.axis(|a| a.title("Numeric keys with display labels").grid(false))
                        .level(0, |l| {
                            l.label_with(time::year_label(col("year")))
                                .padding_inner(0.34)
                                .padding_outer(0.14)
                        })
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .label_with(time::quarter_label(col("quarter")))
                                .padding_inner(0.2)
                                .padding_outer(0.04)
                        })
                        .level(2, |l| {
                            l.label_with(time::month_name_from_number(col("month")))
                                .padding_inner(0.08)
                                .axis(|a| a.label_angle(-90.0))
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 30.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill("#7aa6c2")
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile temporal numeric labels");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "temporal_nested_manual_numeric_keys_display_labels",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_temporal_nested_months_timefill_default_extent() {
    let ctx = SessionContext::new();
    let df = temporal_fact_df(
        &ctx,
        &[
            (1_704_412_800_000, None, 14.0),
            (1_705_708_800_000, None, 5.0),
            (1_709_337_600_000, None, 18.0),
            (1_717_459_200_000, None, 26.0),
        ],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new().transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month()
                .name("period"),
            |mark, period| {
                mark.transform(
                    Aggregate::new()
                        .group_by(period.keys())
                        .sum("total", col("value")),
                    |mark, aggregate| {
                        mark.transform(
                            TimeFill::new(aggregate.output("total"))
                                .levels(period.levels())
                                .fill_value(lit(0.0)),
                            |mark, filled| {
                                mark.x_with(period.nested(), |x| {
                                    x.axis(|a| a.title("Month grouped by quarter").grid(false))
                                        .level(0, |l| l.padding_inner(0.3).padding_outer(0.12))
                                        .level(1, |l| {
                                            l.nest_scope(NestScope::Shared)
                                                .padding_inner(0.18)
                                                .padding_outer(0.04)
                                        })
                                        .level(2, |l| {
                                            l.padding_inner(0.04).axis(|a| a.label_angle(-90.0))
                                        })
                                })
                                .x2_with(col(":x"), |x| x.band(1.0))
                                .y_with(lit(0.0), |y| {
                                    y.scale(|s| s.domain((0.0, 30.0)))
                                        .axis(|a| a.title("Total").grid(true))
                                })
                                .y2(filled.value())
                                .fill("#4c78a8")
                                .stroke("#ffffff")
                                .stroke_width(1.0)
                            },
                        )
                    },
                )
            },
        ),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile temporal timefill default extent");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "temporal_nested_months_timefill_default_extent",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_temporal_nested_months_timefill_explicit_extent() {
    let ctx = SessionContext::new();
    let df = temporal_fact_df(
        &ctx,
        &[
            (1_707_868_800_000, None, 16.0),
            (1_715_126_400_000, None, 22.0),
        ],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new().transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month()
                .name("period"),
            |mark, period| {
                mark.transform(
                    Aggregate::new()
                        .group_by(period.keys())
                        .sum("total", col("value")),
                    |mark, aggregate| {
                        mark.transform(
                            TimeFill::new(aggregate.output("total"))
                                .levels(period.levels())
                                .extent(
                                    [lit(2024_i32), lit(1_i32), lit(1_i32)],
                                    [lit(2024_i32), lit(4_i32), lit(12_i32)],
                                )
                                .fill_value(lit(0.0)),
                            |mark, filled| {
                                mark.x_with(period.nested(), |x| {
                                    x.axis(|a| {
                                        a.title("Explicit full-year month extent").grid(false)
                                    })
                                    .level(0, |l| l.padding_inner(0.26).padding_outer(0.1))
                                    .level(1, |l| {
                                        l.nest_scope(NestScope::Shared)
                                            .padding_inner(0.14)
                                            .padding_outer(0.04)
                                    })
                                    .level(2, |l| {
                                        l.padding_inner(0.02).axis(|a| a.label_angle(-90.0))
                                    })
                                })
                                .x2_with(col(":x"), |x| x.band(1.0))
                                .y_with(lit(0.0), |y| {
                                    y.scale(|s| s.domain((0.0, 25.0)))
                                        .axis(|a| a.title("Total").grid(true))
                                })
                                .y2(filled.value())
                                .fill("#7aa6c2")
                                .stroke("#ffffff")
                                .stroke_width(1.0)
                            },
                        )
                    },
                )
            },
        ),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile temporal timefill explicit extent");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "temporal_nested_months_timefill_explicit_extent",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_temporal_nested_grouped_stacked_months() {
    let ctx = SessionContext::new();
    let df = temporal_fact_df(
        &ctx,
        &[
            (1_704_412_800_000, Some("Hardware"), 10.0),
            (1_705_708_800_000, Some("Hardware"), 5.0),
            (1_709_337_600_000, Some("Hardware"), 18.0),
            (1_714_192_800_000, Some("Hardware"), 11.0),
            (1_704_585_600_000, Some("Services"), 8.0),
            (1_709_769_600_000, Some("Services"), 12.0),
            (1_715_126_400_000, Some("Services"), 15.0),
            (1_718_668_800_000, Some("Services"), 9.0),
        ],
    );

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| legend.title("Segment"))
        .mark(
            Rect::new().transform(
                TimeLevels::new(col("timestamp"))
                    .year()
                    .quarter()
                    .month()
                    .name("period"),
                |mark, period| {
                    mark.transform(
                        Aggregate::new()
                            .group_by(period.keys_with([col("segment")]))
                            .sum("total", col("value")),
                        |mark, aggregate| {
                            mark.transform(
                                TimeFill::new(aggregate.output("total"))
                                    .levels(period.levels())
                                    .group_by([col("segment")])
                                    .fill_value(lit(0.0)),
                                |mark, filled| {
                                    mark.transform(
                                        Stack::new(filled.value())
                                            .group_by(period.keys())
                                            .sort_by_exprs([col("segment")])
                                            .name("segment_stack"),
                                        |mark, stack| {
                                            mark.x_with(period.nested(), |x| {
                                                x.axis(|a| {
                                                    a.title("Monthly totals by segment").grid(false)
                                                })
                                                .level(0, |l| {
                                                    l.padding_inner(0.28).padding_outer(0.12)
                                                })
                                                .level(1, |l| {
                                                    l.nest_scope(NestScope::Shared)
                                                        .padding_inner(0.16)
                                                        .padding_outer(0.04)
                                                })
                                                .level(2, |l| {
                                                    l.padding_inner(0.04)
                                                        .axis(|a| a.label_angle(-90.0))
                                                })
                                            })
                                            .x2_with(col(":x"), |x| x.band(1.0))
                                            .y_with(stack.start(), |y| {
                                                y.scale(|s| s.domain((0.0, 25.0)))
                                                    .axis(|a| a.title("Stacked total").grid(true))
                                            })
                                            .y2(stack.end())
                                            .fill_with(col("segment"), |fill| fill)
                                            .stroke("#ffffff")
                                            .stroke_width(1.0)
                                        },
                                    )
                                },
                            )
                        },
                    )
                },
            ),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile temporal grouped stacked months");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "temporal_nested_grouped_stacked_months",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_temporal_nested_quarters_across_years() {
    let ctx = SessionContext::new();
    let df = temporal_fact_df(
        &ctx,
        &[
            (1_672_531_200_000, None, 12.0),
            (1_688_169_600_000, None, 20.0),
            (1_712_880_000_000, None, 24.0),
            (1_728_345_600_000, None, 30.0),
        ],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new().transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .name("period"),
            |mark, period| {
                mark.transform(
                    Aggregate::new()
                        .group_by(period.keys())
                        .sum("total", col("value")),
                    |mark, aggregate| {
                        mark.transform(
                            TimeFill::new(aggregate.output("total"))
                                .levels(period.levels())
                                .fill_value(lit(0.0)),
                            |mark, filled| {
                                mark.x_with(period.nested(), |x| {
                                    x.axis(|a| a.title("Quarter grouped by year").grid(false))
                                        .level(0, |l| l.padding_inner(0.32).padding_outer(0.14))
                                        .level(1, |l| {
                                            l.nest_scope(NestScope::Shared).padding_inner(0.08)
                                        })
                                })
                                .x2_with(col(":x"), |x| x.band(1.0))
                                .y_with(lit(0.0), |y| {
                                    y.scale(|s| s.domain((0.0, 34.0)))
                                        .axis(|a| a.title("Quarter total").grid(true))
                                })
                                .y2(filled.value())
                                .fill("#4c78a8")
                                .stroke("#ffffff")
                                .stroke_width(1.0)
                            },
                        )
                    },
                )
            },
        ),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile temporal quarters across years");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "temporal_nested_quarters_across_years",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_temporal_nested_faceted_shared_months() {
    let ctx = SessionContext::new();
    let df = temporal_fact_df(
        &ctx,
        &[
            (1_704_412_800_000, Some("North"), 18.0),
            (1_709_337_600_000, Some("North"), 24.0),
            (1_717_459_200_000, Some("North"), 28.0),
            (1_704_585_600_000, Some("South"), 12.0),
            (1_712_880_000_000, Some("South"), 21.0),
            (1_718_668_800_000, Some("South"), 26.0),
        ],
    );

    let plot = Plot::<FacetColumn>::new().data(df).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Rect::new().transform(
                    TimeLevels::new(col("timestamp"))
                        .year()
                        .quarter()
                        .month()
                        .name("period"),
                    |mark, period| {
                        mark.transform(
                            Aggregate::new()
                                .group_by(period.keys_with([col("segment")]))
                                .sum("total", col("value")),
                            |mark, aggregate| {
                                mark.transform(
                                    TimeFill::new(aggregate.output("total"))
                                        .levels(period.levels())
                                        .group_by([col("segment")])
                                        .extent(
                                            [lit(2024_i32), lit(1_i32), lit(1_i32)],
                                            [lit(2024_i32), lit(2_i32), lit(6_i32)],
                                        )
                                        .fill_value(lit(0.0)),
                                    |mark, filled| {
                                        mark.x_with(period.nested(), |x| {
                                            x.axis(|a| {
                                                a.title("Month grouped by quarter").grid(false)
                                            })
                                            .level(0, |l| l.padding_inner(0.28).padding_outer(0.12))
                                            .level(1, |l| {
                                                l.nest_scope(NestScope::Shared)
                                                    .padding_inner(0.16)
                                                    .padding_outer(0.04)
                                            })
                                            .level(
                                                2,
                                                |l| {
                                                    l.padding_inner(0.04)
                                                        .axis(|a| a.label_angle(-90.0))
                                                },
                                            )
                                        })
                                        .x2_with(col(":x"), |x| x.band(1.0))
                                        .y_with(lit(0.0), |y| {
                                            y.scale(|s| s.domain((0.0, 30.0)))
                                                .axis(|a| a.title("Total").grid(true))
                                        })
                                        .y2(filled.value())
                                        .fill("#7aa6c2")
                                        .stroke("#ffffff")
                                        .stroke_width(1.0)
                                    },
                                )
                            },
                        )
                    },
                ),
            ),
        )
        .column(col("segment")),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile temporal faceted shared months");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "temporal_nested_faceted_shared_months",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_temporal_nested_heatmap_month_day() {
    let ctx = SessionContext::new();
    let df = temporal_fact_df(
        &ctx,
        &[
            (1_704_067_200_000, Some("Alpha"), 0.20),
            (1_704_240_000_000, Some("Alpha"), 0.75),
            (1_704_326_400_000, Some("Beta"), 0.45),
            (1_704_412_800_000, Some("Beta"), 0.90),
            (1_704_585_600_000, Some("Alpha"), 0.35),
            (1_704_672_000_000, Some("Beta"), 0.65),
        ],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new().transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .day_of_month()
                .name("period"),
            |mark, period| {
                mark.transform(
                    Aggregate::new()
                        .group_by(period.keys_with([col("segment")]))
                        .sum("total", col("value")),
                    |mark, aggregate| {
                        mark.transform(
                            TimeFill::new(aggregate.output("total"))
                                .levels(period.levels())
                                .group_by([col("segment")])
                                .extent(
                                    [lit(2024_i32), lit(1_i32), lit(1_i32)],
                                    [lit(2024_i32), lit(1_i32), lit(8_i32)],
                                )
                                .fill_value(lit(0.0)),
                            |mark, filled| {
                                mark.x_with(period.nested(), |x| {
                                    x.axis(|a| a.title("Day grouped by month").grid(false))
                                        .level(0, |l| l.padding_inner(0.0).padding_outer(0.0))
                                        .level(1, |l| {
                                            l.padding_inner(0.0)
                                                .padding_outer(0.0)
                                                .nest_scope(NestScope::Shared)
                                        })
                                        .level(2, |l| {
                                            l.padding_inner(0.0)
                                                .padding_outer(0.0)
                                                .axis(|a| a.label_angle(-90.0))
                                        })
                                })
                                .x2_with(col(":x"), |x| x.band(1.0))
                                .y_with(col("segment"), |y| {
                                    y.axis(|a| a.title("Series").grid(false))
                                })
                                .y2_with(col(":y"), |y| y.band(1.0))
                                .fill_with(filled.value(), |fill| {
                                    fill.scale_with::<Linear>(|s| {
                                        s.domain((0.0, 1.0)).range_colors(vec![
                                            Srgba::new(0.97, 0.98, 1.0, 1.0),
                                            Srgba::new(0.03, 0.19, 0.42, 1.0),
                                        ])
                                    })
                                })
                                .stroke("#ffffff")
                                .stroke_width(0.5)
                            },
                        )
                    },
                )
            },
        ),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile temporal month-day heatmap");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "temporal_nested_heatmap_month_day",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_source_column_grouped_bar() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("quarter", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "Q1", "Q1", "Q1", "Q2", "Q2", "Q3", "Q3", "Q3",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "North", "South", "East", "North", "East", "North", "South", "East",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                42.0, 30.0, 34.0, 47.0, 38.0, 51.0, 39.0, 44.0,
            ])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| legend.title("Team"))
        .mark(
            Rect::new()
                .x_with(nested(["quarter", "team"]), |x| {
                    x.axis(|a| a.title("Source columns").grid(false))
                        .level(0, |l| l.padding_inner(0.45).padding_outer(0.15))
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .padding_inner(0.08)
                                .axis(|a| a.visible(false))
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 60.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill_with(col("team"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "source_column_grouped_bar",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_explicit_level_ordering() {
    let ctx = SessionContext::new();

    let plot = Plot::<Cartesian>::new()
        .data(grouped_bar_df(&ctx))
        .legend("fill", |legend| legend.title("Team"))
        .mark(
            Rect::new()
                .x_with(nested_x("quarter", "team"), |x| {
                    x.axis(|a| {
                        a.title("Team grouped by quarter")
                            .grid(false)
                            .label_angle(-90.0)
                    })
                    .level(0, |l| {
                        l.domain_values(vec![lit("Q3"), lit("Q1"), lit("Q2")])
                            .padding_inner(0.45)
                            .padding_outer(0.15)
                    })
                    .level(1, |l| {
                        l.nest_scope(NestScope::Shared)
                            .domain_values(vec![lit("South"), lit("North"), lit("East")])
                            .padding_inner(0.08)
                    })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 60.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill_with(col("team"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "explicit_level_ordering",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_symbol_centers() {
    let ctx = SessionContext::new();

    let plot = Plot::<Cartesian>::new()
        .data(grouped_bar_df(&ctx))
        .legend("fill", |legend| legend.title("Team"))
        .mark(
            Symbol::new()
                .x_with(nested_x("quarter", "team"), |x| {
                    x.axis(|a| a.title("Team grouped by quarter").grid(false))
                        .level(0, |l| l.padding_inner(0.42).padding_outer(0.14))
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .padding_inner(0.08)
                                .axis(|a| a.label_angle(-90.0))
                        })
                        .band(0.5)
                })
                .y_with(col("value"), |y| {
                    y.scale(|s| s.domain((0.0, 60.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .fill_with(col("team"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(110.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "nested_position", "symbol_centers").await;
}

#[tokio::test]
async fn test_nested_position_bokeh_style_variable_parent_width_axis() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("cylinders", DataType::Utf8, false),
            Field::new("manufacturer", DataType::Utf8, false),
            Field::new("mpg", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "3", "4", "4", "4", "4", "5", "5", "6", "6", "6", "8", "8", "8",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "mazda", "amc", "buick", "ford", "toyota", "audi", "mercedes", "amc", "buick",
                "volvo", "amc", "dodge", "ford",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                20.4, 25.7, 28.1, 29.3, 30.4, 28.0, 25.2, 18.6, 20.5, 23.8, 15.0, 14.6, 15.2,
            ])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(nested_x("cylinders", "manufacturer"), |x| {
                x.axis(|a| {
                    a.title("Manufacturer grouped by cylinders")
                        .grid(false)
                        .label_angle(-90.0)
                })
                .level(0, |l| l.padding_inner(0.35).padding_outer(0.2))
                .level(1, |l| l.padding_inner(0.06))
            })
            .x2_with(col(":x"), |x| x.band(1.0))
            .y_with(lit(0.0), |y| {
                y.scale(|s| s.domain((0.0, 36.0)))
                    .axis(|a| a.title("Mean MPG").grid(true))
            })
            .y2(col("mpg"))
            .fill_with(col("cylinders"), |fill| fill)
            .stroke("#ffffff")
            .stroke_width(1.0),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "bokeh_style_variable_parent_width_axis",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_three_level_category_bars() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("category", DataType::Utf8, false),
            Field::new("item", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "North", "North", "North", "South", "South", "South", "South",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Fruit", "Fruit", "Grain", "Fruit", "Fruit", "Grain", "Grain",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec!["A", "B", "O", "A", "C", "R", "W"])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                34.0, 28.0, 22.0, 30.0, 26.0, 35.0, 31.0,
            ])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");
    let nested = nested(["region", "category", "item"]);

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(nested, |x| {
                x.axis(|a| a.title("Item grouped by category and region").grid(false))
                    .level(0, |l| l.padding_inner(0.34).padding_outer(0.16))
                    .level(1, |l| l.padding_inner(0.22).padding_outer(0.04))
                    .level(2, |l| l.padding_inner(0.08))
            })
            .x2_with(col(":x"), |x| x.band(1.0))
            .y_with(lit(0.0), |y| {
                y.scale(|s| s.domain((0.0, 40.0)))
                    .axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill_with(col("category"), |fill| fill)
            .stroke("#ffffff")
            .stroke_width(1.0),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "nested_three_level_category_bars",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_category_axis_three_level() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("category", DataType::Utf8, false),
            Field::new("item", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "North", "North", "North", "South", "South", "South",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Hardware", "Hardware", "Software", "Hardware", "Software", "Software",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Drills", "Saws", "Apps", "Saws", "Apps", "Cloud",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![31.0, 24.0, 28.0, 22.0, 35.0, 39.0])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");
    let nested = nested(["region", "category", "item"]);

    let plot = Plot::<Cartesian>::new()
        .canvas_size(680.0, 420.0)
        .data(df)
        .mark(
            Rect::new()
                .x_with(nested, |x| {
                    x.axis(|a| a.title("Item grouped by category and region").grid(false))
                        .level(0, |l| {
                            l.axis(|a| a.title("Region"))
                                .padding_inner(0.35)
                                .padding_outer(0.14)
                        })
                        .level(1, |l| {
                            l.axis(|a| a.title("Category"))
                                .padding_inner(0.22)
                                .padding_outer(0.04)
                        })
                        .level(2, |l| {
                            l.axis(|a| a.title("Item").label_angle(-25.0))
                                .padding_inner(0.08)
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 45.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill_with(col("category"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "nested_category_axis_three_level",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_parent_span_overlay() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("item", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "West", "West", "West", "East", "East", "Central", "Central",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec!["A", "B", "C", "A", "B", "A", "B"])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                28.0, 35.0, 32.0, 41.0, 38.0, 31.0, 36.0,
            ])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Rect::new()
                .x_with(nested_x("region", "item"), |x| {
                    x.axis(|a| a.title("Item grouped by region").grid(false))
                        .level(0, |l| l.padding_inner(0.38).padding_outer(0.18))
                        .level(1, |l| l.padding_inner(0.08))
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 50.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill("#7aa6c2")
                .stroke("#ffffff")
                .stroke_width(1.0),
        )
        .mark(
            Rule::new()
                .x_with(nested_x("region", "item"), |x| {
                    x.level(0, |l| l.padding_inner(0.38).padding_outer(0.18))
                        .level(1, |l| l.padding_inner(0.08))
                        .level_band(0, 0.0)
                })
                .x2_with(col(":x"), |x| x.level_band(0, 1.0))
                .y(lit(47.0))
                .y2(lit(47.0))
                .stroke("#111827")
                .stroke_width(2.0)
                .opacity(0.75),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "parent_span_overlay",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_heatmap_zero_padding_both_axes() {
    let ctx = SessionContext::new();
    let mut x_group = Vec::new();
    let mut x_member = Vec::new();
    let mut y_group = Vec::new();
    let mut y_member = Vec::new();
    let mut value = Vec::new();
    for (gx, members_x) in [("A", ["a1", "a2"]), ("B", ["b1", "b2"])] {
        for mx in members_x {
            for (gy, members_y) in [("North", ["n1", "n2"]), ("South", ["s1", "s2"])] {
                for my in members_y {
                    x_group.push(gx);
                    x_member.push(mx);
                    y_group.push(gy);
                    y_member.push(my);
                    value.push(match (gx, mx, gy, my) {
                        ("A", "a1", "North", "n1") => 0.15,
                        ("A", "a2", "South", "s2") => 0.95,
                        ("B", "b1", "North", "n2") => 0.70,
                        ("B", "b2", "South", "s1") => 0.45,
                        _ => 0.30,
                    });
                }
            }
        }
    }
    let batch = record_batch(
        vec![
            Field::new("x_group", DataType::Utf8, false),
            Field::new("x_member", DataType::Utf8, false),
            Field::new("y_group", DataType::Utf8, false),
            Field::new("y_member", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(x_group)) as ArrayRef,
            Arc::new(StringArray::from(x_member)) as ArrayRef,
            Arc::new(StringArray::from(y_group)) as ArrayRef,
            Arc::new(StringArray::from(y_member)) as ArrayRef,
            Arc::new(Float32Array::from(value)) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");

    let x_expr = nested_x("x_group", "x_member");
    let y_expr = nested_x("y_group", "y_member");
    let plot = Plot::<Cartesian>::new().data(df).mark(
        Rect::new()
            .x_with(x_expr, |x| {
                x.axis(|a| a.title("Nested X").grid(false))
                    .level(0, |l| l.padding_inner(0.0).padding_outer(0.0))
                    .level(1, |l| l.padding_inner(0.0).padding_outer(0.0))
            })
            .x2_with(col(":x"), |x| x.band(1.0))
            .y_with(y_expr, |y| {
                y.axis(|a| a.title("Nested Y").grid(false))
                    .level(0, |l| l.padding_inner(0.0).padding_outer(0.0))
                    .level(1, |l| l.padding_inner(0.0).padding_outer(0.0))
            })
            .y2_with(col(":y"), |y| y.band(1.0))
            .fill_with(col("value"), |fill| {
                fill.scale_with::<Linear>(|s| {
                    s.domain((0.0, 1.0)).range_colors(vec![
                        Srgba::new(0.97, 0.98, 1.0, 1.0),
                        Srgba::new(0.03, 0.19, 0.42, 1.0),
                    ])
                })
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "heatmap_zero_padding_both_axes",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_heatmap_shared_leaf_slots() {
    let ctx = SessionContext::new();
    let rows = [
        ("A", "a1", "North", "n1", 0.15),
        ("A", "a2", "North", "n2", 0.65),
        ("B", "b1", "South", "s1", 0.45),
        ("B", "b2", "South", "s2", 0.90),
        ("A", "a1", "South", "s2", 0.30),
        ("B", "b1", "North", "n1", 0.75),
    ];
    let df = nested_heatmap_df(&ctx, &rows);

    let plot = Plot::<Cartesian>::new()
        .canvas_size(560.0, 460.0)
        .data(df)
        .mark(
            Rect::new()
                .x_with(nested_x("x_group", "x_member"), |x| {
                    x.axis(|a| a.title("Shared nested X").grid(false))
                        .level(0, |l| l.padding_inner(0.08).padding_outer(0.04))
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .padding_inner(0.0)
                                .padding_outer(0.0)
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(nested_x("y_group", "y_member"), |y| {
                    y.axis(|a| a.title("Shared nested Y").grid(false))
                        .level(0, |l| l.padding_inner(0.08).padding_outer(0.04))
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .padding_inner(0.0)
                                .padding_outer(0.0)
                        })
                })
                .y2_with(col(":y"), |y| y.band(1.0))
                .fill_with(col("value"), |fill| {
                    fill.scale_with::<Linear>(|s| {
                        s.domain((0.0, 1.0)).range_colors(vec![
                            Srgba::new(0.97, 0.98, 1.0, 1.0),
                            Srgba::new(0.03, 0.19, 0.42, 1.0),
                        ])
                    })
                })
                .stroke("#ffffff")
                .stroke_width(0.5),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "nested_axis_heatmap_shared_leaf_slots",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_heatmap_no_leaf_axes() {
    let ctx = SessionContext::new();
    let rows = [
        ("A", "a1", "North", "n1", 0.20),
        ("A", "a2", "North", "n2", 0.55),
        ("B", "b1", "South", "s1", 0.70),
        ("B", "b2", "South", "s2", 0.35),
        ("A", "a2", "South", "s1", 0.85),
        ("B", "b1", "North", "n2", 0.45),
    ];
    let df = nested_heatmap_df(&ctx, &rows);

    let plot = Plot::<Cartesian>::new()
        .canvas_size(520.0, 420.0)
        .data(df)
        .mark(
            Rect::new()
                .x_with(nested_x("x_group", "x_member"), |x| {
                    x.axis(|a| a.title("Nested X").grid(false))
                        .level(0, |l| l.padding_inner(0.08).padding_outer(0.04))
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .padding_inner(0.0)
                                .padding_outer(0.0)
                                .axis(|a| a.visible(false))
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(nested_x("y_group", "y_member"), |y| {
                    y.axis(|a| a.title("Nested Y").grid(false))
                        .level(0, |l| l.padding_inner(0.08).padding_outer(0.04))
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .padding_inner(0.0)
                                .padding_outer(0.0)
                                .axis(|a| a.visible(false))
                        })
                })
                .y2_with(col(":y"), |y| y.band(1.0))
                .fill_with(col("value"), |fill| {
                    fill.scale_with::<Linear>(|s| {
                        s.domain((0.0, 1.0)).range_colors(vec![
                            Srgba::new(0.97, 0.98, 1.0, 1.0),
                            Srgba::new(0.03, 0.19, 0.42, 1.0),
                        ])
                    })
                })
                .stroke("#ffffff")
                .stroke_width(0.5),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "nested_axis_heatmap_no_leaf_axes",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_heatmap_parent_gaps() {
    let ctx = SessionContext::new();
    let rows = [
        ("A", "a1", "North", "n1", 0.15),
        ("A", "a1", "North", "n2", 0.35),
        ("A", "a2", "South", "s1", 0.55),
        ("A", "a2", "South", "s2", 0.75),
        ("B", "b1", "North", "n1", 0.25),
        ("B", "b1", "North", "n2", 0.45),
        ("B", "b2", "South", "s1", 0.65),
        ("B", "b2", "South", "s2", 0.85),
    ];
    let df = nested_heatmap_df(&ctx, &rows);

    let plot = Plot::<Cartesian>::new()
        .canvas_size(560.0, 460.0)
        .data(df)
        .mark(
            Rect::new()
                .x_with(nested_x("x_group", "x_member"), |x| {
                    x.axis(|a| a.title("Nested X").grid(false))
                        .level(0, |l| {
                            l.padding_inner(0.0)
                                .padding_outer(0.0)
                                .padding_inner_px(14.0)
                        })
                        .level(1, |l| l.padding_inner(0.0).padding_outer(0.0))
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(nested_x("y_group", "y_member"), |y| {
                    y.axis(|a| a.title("Nested Y").grid(false))
                        .level(0, |l| {
                            l.padding_inner(0.0)
                                .padding_outer(0.0)
                                .padding_inner_px(14.0)
                        })
                        .level(1, |l| l.padding_inner(0.0).padding_outer(0.0))
                })
                .y2_with(col(":y"), |y| y.band(1.0))
                .fill_with(col("value"), |fill| {
                    fill.scale_with::<Linear>(|s| {
                        s.domain((0.0, 1.0)).range_colors(vec![
                            Srgba::new(0.97, 0.98, 1.0, 1.0),
                            Srgba::new(0.03, 0.19, 0.42, 1.0),
                        ])
                    })
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "nested_axis_heatmap_parent_gaps",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_y_lollipop() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("department", DataType::Utf8, false),
            Field::new("role", DataType::Utf8, false),
            Field::new("score", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "Sales",
                "Sales",
                "Sales",
                "Support",
                "Support",
                "Engineering",
                "Engineering",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "AE", "AM", "SE", "Tier 1", "Tier 2", "Backend", "Frontend",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                74.0, 68.0, 81.0, 62.0, 71.0, 88.0, 84.0,
            ])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");
    let nested_y = nested_x("department", "role");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Rule::new()
                .x_with(lit(0.0), |x| {
                    x.scale(|s| s.domain((0.0, 100.0)))
                        .axis(|a| a.title("Score").grid(true))
                })
                .x2(col("score"))
                .y_with(nested_y.clone(), |y| {
                    y.axis(|a| a.title("Role grouped by department").grid(false))
                        .level(0, |l| l.padding_inner(0.35).padding_outer(0.15))
                        .level(1, |l| l.padding_inner(0.1))
                        .band(0.5)
                })
                .y2_with(col(":y"), |y| y.band(0.5))
                .stroke("#64748b")
                .stroke_width(1.6),
        )
        .mark(
            Symbol::new()
                .x(col("score"))
                .y(nested_y)
                .fill("#f59e0b")
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(85.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "nested_position", "y_lollipop").await;
}

#[tokio::test]
async fn test_nested_position_y_axis_two_level() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("score", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "North", "North", "South", "South", "South", "West", "West",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Alpha", "Beta", "Alpha", "Beta", "Gamma", "Alpha", "Gamma",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                34.0, 42.0, 28.0, 39.0, 31.0, 45.0, 37.0,
            ])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");

    let plot = Plot::<Cartesian>::new()
        .canvas_size(560.0, 430.0)
        .data(df)
        .mark(
            Rect::new()
                .x_with(lit(0.0), |x| {
                    x.scale(|s| s.domain((0.0, 50.0)))
                        .axis(|a| a.title("Score").grid(true))
                })
                .x2(col("score"))
                .y_with(nested_x("division", "team"), |y| {
                    y.axis(|a| a.title("Team grouped by division").grid(false))
                        .level(0, |l| {
                            l.axis(|a| a.title("Division"))
                                .padding_inner(0.36)
                                .padding_outer(0.12)
                        })
                        .level(1, |l| {
                            l.axis(|a| a.title("Team"))
                                .padding_inner(0.08)
                                .padding_outer(0.0)
                        })
                })
                .y2_with(col(":y"), |y| y.band(1.0))
                .fill_with(col("division"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "nested_y_axis_two_level",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_axis_long_labels_measurement() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("portfolio", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "North America enterprise",
                "North America enterprise",
                "North America enterprise",
                "International growth markets",
                "International growth markets",
                "International growth markets",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Cloud infrastructure",
                "Analytics platform",
                "Customer operations",
                "Cloud infrastructure",
                "Analytics platform",
                "Customer operations",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![41.0, 35.0, 29.0, 37.0, 43.0, 31.0])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");

    let plot = Plot::<Cartesian>::new()
        .canvas_size(760.0, 460.0)
        .data(df)
        .mark(
            Rect::new()
                .x_with(nested_x("division", "portfolio"), |x| {
                    x.axis(|a| a.title("Portfolio grouped by division").grid(false))
                        .level(0, |l| {
                            l.axis(|a| a.title("Division").show_title(true))
                                .padding_inner(0.42)
                                .padding_outer(0.14)
                        })
                        .level(1, |l| {
                            l.axis(|a| a.title("Portfolio").label_angle(-90.0))
                                .nest_scope(NestScope::Shared)
                                .padding_inner(0.08)
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 50.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill_with(col("division"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "nested_axis_long_labels_measurement",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_facet_shared_whole_path_slots() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("market", DataType::Utf8, false),
            Field::new("quarter", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "North", "North", "North", "South", "South", "South", "South",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Q1", "Q1", "Q2", "Q1", "Q2", "Q2", "Q3",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "East", "North", "East", "South", "East", "South", "North",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                32.0, 45.0, 38.0, 28.0, 42.0, 35.0, 48.0,
            ])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");

    let plot = Plot::<FacetColumn>::new().data(df).mark(
        Subplot::new(
            Plot::<Cartesian>::new()
                .legend("fill", |legend| legend.title("Team"))
                .mark(
                    Rect::new()
                        .x_with(nested_x("quarter", "team"), |x| {
                            x.with_domain_scope(CoordinationScope::Shared)
                                .axis(|a| a.title("Quarter").grid(false))
                                .level(0, |l| l.padding_inner(0.42).padding_outer(0.12))
                                .level(1, |l| {
                                    l.nest_scope(NestScope::Shared)
                                        .padding_inner(0.08)
                                        .axis(|a| a.visible(false))
                                })
                        })
                        .x2_with(col(":x"), |x| x.band(1.0))
                        .y_with(lit(0.0), |y| {
                            y.with_domain_scope(CoordinationScope::Shared)
                                .scale(|s| s.domain((0.0, 55.0)))
                                .axis(|a| a.title("Value").grid(true))
                        })
                        .y2(col("value"))
                        .fill_with(col("team"), |fill| fill)
                        .stroke("#ffffff")
                        .stroke_width(1.0),
                ),
        )
        .column(col("market")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "facet_shared_whole_path_slots",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_facet_shared_parent_free_leaf() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new().data(facet_nested_df(&ctx)).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .x_with(nested_x("cyl", "make"), |x| {
                        x.axis(|a| a.title("Make grouped by cylinders").grid(false))
                            .level(0, |l| {
                                l.domain_scope(CoordinationScope::Shared)
                                    .padding_inner(0.38)
                                    .padding_outer(0.14)
                            })
                            .level(1, |l| {
                                l.domain_scope(CoordinationScope::Shared)
                                    .nest_scope(NestScope::Free)
                                    .padding_inner(0.08)
                            })
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y_with(lit(0.0), |y| {
                        y.with_domain_scope(CoordinationScope::Shared)
                            .scale(|s| s.domain((0.0, 40.0)))
                            .axis(|a| a.title("Value").grid(true))
                    })
                    .y2(col("value"))
                    .fill_with(col("make"), |fill| fill)
                    .stroke("#ffffff")
                    .stroke_width(1.0),
            ),
        )
        .column(col("market")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "facet_nested_position_shared_parent_free_leaf",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_facet_shared_leaf_slots() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new().data(facet_nested_df(&ctx)).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .x_with(nested_x("cyl", "make"), |x| {
                        x.axis(|a| a.title("Make grouped by cylinders").grid(false))
                            .level(0, |l| {
                                l.domain_scope(CoordinationScope::Shared)
                                    .padding_inner(0.38)
                                    .padding_outer(0.14)
                            })
                            .level(1, |l| {
                                l.domain_scope(CoordinationScope::Shared)
                                    .nest_scope(NestScope::Shared)
                                    .padding_inner(0.08)
                            })
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y_with(lit(0.0), |y| {
                        y.with_domain_scope(CoordinationScope::Shared)
                            .scale(|s| s.domain((0.0, 40.0)))
                            .axis(|a| a.title("Value").grid(true))
                    })
                    .y2(col("value"))
                    .fill_with(col("make"), |fill| fill)
                    .stroke("#ffffff")
                    .stroke_width(1.0),
            ),
        )
        .column(col("market")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "facet_nested_position_shared_leaf_slots",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_facet_free_domains() {
    let ctx = SessionContext::new();
    let plot = Plot::<FacetColumn>::new().data(facet_nested_df(&ctx)).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .x_with(nested_x("cyl", "make"), |x| {
                        x.axis(|a| a.title("Local make groups").grid(false))
                            .level(0, |l| {
                                l.domain_scope(CoordinationScope::Free)
                                    .padding_inner(0.38)
                                    .padding_outer(0.14)
                            })
                            .level(1, |l| {
                                l.domain_scope(CoordinationScope::Free)
                                    .nest_scope(NestScope::Free)
                                    .padding_inner(0.08)
                            })
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y_with(lit(0.0), |y| {
                        y.with_domain_scope(CoordinationScope::Shared)
                            .scale(|s| s.domain((0.0, 40.0)))
                            .axis(|a| a.title("Value").grid(true))
                    })
                    .y2(col("value"))
                    .fill_with(col("make"), |fill| fill)
                    .stroke("#ffffff")
                    .stroke_width(1.0),
            ),
        )
        .column(col("market")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "facet_nested_position_free_domains",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_facet_heatmap_shared_x_free_y() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("market", DataType::Utf8, false),
            Field::new("x_group", DataType::Utf8, false),
            Field::new("x_member", DataType::Utf8, false),
            Field::new("y_group", DataType::Utf8, false),
            Field::new("y_member", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "North", "North", "North", "North", "South", "South", "South", "South",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "A", "A", "B", "B", "A", "A", "B", "B",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "a1", "a2", "b1", "b1", "a2", "a2", "b1", "b2",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "P", "P", "R", "R", "Q", "Q", "Q", "S",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "p1", "p2", "r1", "r2", "q1", "q2", "q1", "s1",
            ])) as ArrayRef,
            Arc::new(Float32Array::from(vec![
                0.20, 0.65, 0.35, 0.85, 0.55, 0.30, 0.75, 0.45,
            ])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");

    let plot = Plot::<FacetColumn>::new().data(df).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .x_with(nested_x("x_group", "x_member"), |x| {
                        x.axis(|a| a.title("Shared nested X").grid(false))
                            .level(0, |l| {
                                l.domain_scope(CoordinationScope::Shared)
                                    .padding_inner(0.08)
                                    .padding_outer(0.04)
                            })
                            .level(1, |l| {
                                l.domain_scope(CoordinationScope::Shared)
                                    .nest_scope(NestScope::Shared)
                                    .padding_inner(0.0)
                            })
                    })
                    .x2_with(col(":x"), |x| x.band(1.0))
                    .y_with(nested_x("y_group", "y_member"), |y| {
                        y.axis(|a| a.title("Local nested Y").grid(false))
                            .level(0, |l| {
                                l.domain_scope(CoordinationScope::Free)
                                    .padding_inner(0.08)
                                    .padding_outer(0.04)
                            })
                            .level(1, |l| {
                                l.domain_scope(CoordinationScope::Free)
                                    .nest_scope(NestScope::Free)
                                    .padding_inner(0.0)
                            })
                    })
                    .y2_with(col(":y"), |y| y.band(1.0))
                    .fill_with(col("value"), |fill| {
                        fill.scale_with::<Linear>(|s| {
                            s.domain((0.0, 1.0)).range_colors(vec![
                                Srgba::new(0.96, 0.98, 1.0, 1.0),
                                Srgba::new(0.05, 0.24, 0.45, 1.0),
                            ])
                        })
                    }),
            ),
        )
        .column(col("market")),
    );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "facet_nested_position_heatmap_shared_x_free_y",
    )
    .await;
}

#[tokio::test]
async fn test_nested_position_repeat_axis_titles() {
    let ctx = SessionContext::new();
    let batch = record_batch(
        vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "North", "North", "North", "South", "South", "South",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec!["Bk", "Gm", "Ty", "Bk", "Gm", "Ty"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["Aa", "Bb", "Cc", "Aa", "Bb", "Cc"])) as ArrayRef,
            Arc::new(Float32Array::from(vec![22.0, 31.0, 26.0, 28.0, 24.0, 34.0])) as ArrayRef,
        ],
    );
    let df = ctx.read_batch(batch).expect("dataframe");
    let nested = nested(["region".to_string(), repeat::column_name()]);
    let cell = Plot::<Cartesian>::new().mark(
        Rect::new()
            .x_with(nested, |x| {
                x.axis(|a| a.title("Category grouped by region").grid(false))
                    .level(0, |l| l.axis(|a| a.title("Region")).padding_inner(0.34))
                    .level(1, |l| {
                        l.axis(|a| a.title(repeat::column_title()))
                            .padding_inner(0.08)
                    })
            })
            .x2_with(col(":x"), |x| x.band(1.0))
            .y_with(lit(0.0), |y| {
                y.scale(|s| s.domain((0.0, 40.0)))
                    .axis(|a| a.title("Value").grid(true))
            })
            .y2(col("value"))
            .fill("#4c78a8")
            .stroke("#ffffff")
            .stroke_width(1.0),
    );
    let plot = Plot::<RepeatColumns>::new()
        .data(df)
        .plot_size(260.0, 165.0)
        .configure_coord(|c| {
            c.columns(vec![
                RepeatVariable::field("product").title("Product"),
                RepeatVariable::field("team").title("Team"),
            ])
            .cell(cell)
        });

    let compiled = plot.compile(&ctx).await.expect("compile repeat columns");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "nested_position",
        "repeat_nested_position_axis_titles",
    )
    .await;
}
