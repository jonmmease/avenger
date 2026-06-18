use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::functions::expr_fn::concat;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

fn kde_density_data(ctx: &SessionContext) -> DataFrame {
    let values = vec![
        -2.65, -2.35, -2.1, -1.95, -1.7, -1.55, -1.35, -1.05, -0.8, -0.55, 0.72, 0.9, 1.05, 1.18,
        1.34, 1.48, 1.62, 1.78, 1.96, 2.14, 2.35, 2.62, 2.94, 3.22,
    ];
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Float64,
            false,
        )])),
        vec![Arc::new(Float64Array::from(values)) as _],
    )
    .expect("kde density batch");
    ctx.read_batch(batch).expect("kde density dataframe")
}

fn grouped_density_data(ctx: &SessionContext) -> DataFrame {
    let series = vec![
        "North", "North", "North", "North", "North", "North", "North", "North", "North", "North",
        "North", "North", "South", "South", "South", "South", "South", "South", "South", "South",
        "South", "South", "South", "South",
    ];
    let values = vec![
        -2.4, -2.15, -1.9, -1.72, -1.5, -1.25, -0.92, -0.65, -0.28, 0.1, 0.35, 0.62, 0.2, 0.54,
        0.84, 1.08, 1.28, 1.48, 1.68, 1.95, 2.24, 2.55, 2.88, 3.22,
    ];
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("series", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(series)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("grouped kde batch");
    ctx.read_batch(batch).expect("grouped kde dataframe")
}

fn grouped_violin_data(ctx: &SessionContext) -> DataFrame {
    let mut groups = Vec::<String>::new();
    let mut values = Vec::<f64>::new();

    let mut add_cluster = |group: &str, center: f64, spread: f64, count: usize, phase: f64| {
        for i in 0..count {
            let t = i as f64;
            let jitter =
                ((t * 1.618 + phase).sin() * 0.55 + (t * 0.73 + phase).cos() * 0.25) * spread;
            groups.push(group.to_string());
            values.push(center + jitter);
        }
    };

    add_cluster("Alpha", -1.05, 0.72, 18, 0.1);
    add_cluster("Alpha", 1.08, 0.52, 18, 1.3);
    add_cluster("Beta", 0.2, 0.62, 44, 2.1);
    add_cluster("Gamma", 0.95, 1.35, 30, 3.4);

    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("group", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(groups)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("grouped violin batch");
    ctx.read_batch(batch).expect("grouped violin dataframe")
}

fn nested_violin_data(ctx: &SessionContext) -> DataFrame {
    let mut markets = Vec::<String>::new();
    let mut divisions = Vec::<String>::new();
    let mut teams = Vec::<String>::new();
    let mut values = Vec::<f64>::new();

    let mut add_cluster =
        |market: &str, division: &str, team: &str, center: f64, spread: f64, count: usize| {
            let phase = (market.len() + division.len() * 3 + team.len() * 7) as f64 * 0.19;
            for i in 0..count {
                let t = i as f64;
                let wave = (t * 1.37 + phase).sin() * 0.62 + (t * 0.61 + phase * 0.7).cos() * 0.28;
                markets.push(market.to_string());
                divisions.push(division.to_string());
                teams.push(team.to_string());
                values.push(center + wave * spread);
            }
        };

    add_cluster("North", "Platform", "Alpha", -0.95, 0.64, 18);
    add_cluster("North", "Platform", "Beta", 0.48, 0.72, 38);
    add_cluster("North", "Experience", "Alpha", 1.05, 0.52, 24);
    add_cluster("North", "Experience", "Gamma", 1.72, 0.48, 12);
    add_cluster("South", "Platform", "Alpha", -0.62, 0.56, 13);
    add_cluster("South", "Platform", "Gamma", 0.08, 0.62, 28);
    add_cluster("South", "Experience", "Beta", 1.18, 0.74, 44);
    add_cluster("South", "Experience", "Gamma", 1.92, 0.58, 20);

    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("market", DataType::Utf8, false),
            Field::new("division", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(markets)) as _,
            Arc::new(StringArray::from(divisions)) as _,
            Arc::new(StringArray::from(teams)) as _,
            Arc::new(Float64Array::from(values)) as _,
        ],
    )
    .expect("nested violin batch");
    ctx.read_batch(batch).expect("nested violin dataframe")
}

#[tokio::test]
async fn kde_density_area() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("KDE density")
        .subtitle("Bimodal area density from an eager transform")
        .canvas_size(680.0, 420.0)
        .data(kde_density_data(&ctx))
        .mark(
            Area::new().transform(
                Kde::new(col("value"))
                    .bandwidth(0.38)
                    .steps(160)
                    .extent(-3.2, 3.8)
                    .as_fields("sample", "density"),
                |mark, kde| {
                    mark.x_with(kde.value(), |c| c.axis(|a| a.title("Sample")))
                        .y_with(kde.density(), |c| c.axis(|a| a.title("Density")))
                        .y2_with(lit(0.0), |c| c.with_scale_name("y"))
                        .fill("#2f80ed")
                        .stroke("#174ea6")
                        .stroke_width(1.5)
                        .opacity(0.58)
                        .order(ChannelValue::from(kde.value()).no_scale())
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile kde area");
    assert_visual_match_default(&compiled, &ctx, None, "transform_kde", "kde_density_area").await;
}

#[tokio::test]
async fn kde_grouped_density_lines() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Grouped KDE density")
        .subtitle("Shared sample grid with a categorical color legend")
        .canvas_size(720.0, 440.0)
        .data(grouped_density_data(&ctx))
        .mark(
            Line::new().transform(
                Kde::new(col("value"))
                    .group_by([col("series")])
                    .bandwidth(0.42)
                    .steps(150)
                    .resolve(KdeResolve::Shared)
                    .as_fields("sample", "density"),
                |mark, kde| {
                    mark.x_with(kde.value(), |c| c.axis(|a| a.title("Sample")))
                        .y_with(kde.density(), |c| c.axis(|a| a.title("Density")))
                        .stroke_with(col("series"), |c| c.legend(|l| l.title("Series")))
                        .stroke_width(3.0)
                        .opacity(0.86)
                        .order(ChannelValue::from(kde.value()).no_scale())
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile grouped kde");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_kde",
        "kde_grouped_density_lines",
    )
    .await;
}

#[tokio::test]
async fn kde_low_level_grouped_violin_dynamic_band() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Low-level violin from KDE + area")
        .subtitle("Common max-density normalization drives row-wise band width")
        .canvas_size(720.0, 440.0)
        .data(grouped_violin_data(&ctx))
        .mark(
            Area::new().transform(
                Kde::new(col("value"))
                    .group_by([col("group")])
                    .bandwidth(0.34)
                    .steps(120)
                    .extent(-2.8, 3.2)
                    .resolve(KdeResolve::Shared)
                    .as_fields("sample", "density"),
                |mark, kde| {
                    mark.transform_no_output(
                        JoinAggregate::new().max("__global_max_density", kde.density()),
                        |mark| {
                            mark.transform_no_output(
                                Calculate::new()
                                    .expr(
                                        "__half_width",
                                        lit(0.39) * col("density") / col("__global_max_density"),
                                    )
                                    .expr("__band_start", lit(0.5) - col("__half_width"))
                                    .expr("__band_end", lit(0.5) + col("__half_width")),
                                |mark| {
                                    mark.orientation("horizontal")
                                        .details(["group"])
                                        .x_with(col("group"), |c| {
                                            c.scale_with::<Band>(|s| s.padding_inner(0.18))
                                                .band(col("__band_start"))
                                                .axis(|a| a.title("Group"))
                                        })
                                        .x2_with(col("group"), |c| {
                                            c.with_scale_name("x").band(col("__band_end"))
                                        })
                                        .y_with(kde.value(), |c| c.axis(|a| a.title("Value")))
                                        .y2_with(kde.value(), |c| c.with_scale_name("y"))
                                        .fill_with(col("group"), |c| c.legend(|l| l.title("Group")))
                                        .stroke("#172033")
                                        .stroke_width(1.0)
                                        .opacity(0.72)
                                        .order(ChannelValue::from(kde.value()).no_scale())
                                },
                            )
                        },
                    )
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile low-level violin");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_kde",
        "kde_low_level_grouped_violin_dynamic_band",
    )
    .await;
}

#[tokio::test]
async fn kde_low_level_nested_violin_counts_shared_max() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Nested violin from KDE + area")
        .subtitle("Smoothed counts are normalized by one max across all nested groups")
        .canvas_size(800.0, 460.0)
        .data(nested_violin_data(&ctx))
        .mark(
            Area::new().transform(
                Kde::new(col("value"))
                    .group_by([col("division"), col("team")])
                    .counts(true)
                    .bandwidth(0.3)
                    .steps(120)
                    .extent(-2.4, 3.0)
                    .resolve(KdeResolve::Shared)
                    .as_fields("sample", "density"),
                |mark, kde| {
                    mark.transform_no_output(
                        JoinAggregate::new().max("__global_max_density", kde.density()),
                        |mark| {
                            mark.transform_no_output(
                                Calculate::new()
                                    .expr(
                                        "__half_width",
                                        lit(0.42) * col("density") / col("__global_max_density"),
                                    )
                                    .expr(
                                        "__leaf_key",
                                        concat(vec![col("division"), lit(" / "), col("team")]),
                                    )
                                    .expr("__band_start", lit(0.5) - col("__half_width"))
                                    .expr("__band_end", lit(0.5) + col("__half_width")),
                                |mark| {
                                    mark.orientation("horizontal")
                                        .details(["division", "team"])
                                        .x_with(nested(["division", "team"]), |x| {
                                            x.axis(|a| {
                                                a.title("Team grouped by division").grid(false)
                                            })
                                            .level(0, |l| l.padding_inner(0.34).padding_outer(0.12))
                                            .level(1, |l| {
                                                l.nest_scope(NestScope::Shared).padding_inner(0.1)
                                            })
                                            .band(col("__band_start"))
                                        })
                                        .x2_with(col(":x"), |x| x.band(col("__band_end")))
                                        .y_with(kde.value(), |y| {
                                            y.scale(|s| s.domain((-2.4, 3.0)))
                                                .axis(|a| a.title("Value").grid(true))
                                        })
                                        .y2_with(kde.value(), |y| y.with_scale_name("y"))
                                        .fill_with(col("__leaf_key"), |fill| {
                                            fill.legend(|l| l.title("Nested group"))
                                        })
                                        .stroke("#1f2937")
                                        .stroke_width(0.9)
                                        .opacity(0.72)
                                        .order(ChannelValue::from(kde.value()).no_scale())
                                },
                            )
                        },
                    )
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile nested violin");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_kde",
        "kde_low_level_nested_violin_counts_shared_max",
    )
    .await;
}

#[tokio::test]
async fn kde_low_level_nested_violin_facet_counts_shared_max() {
    let ctx = SessionContext::new();
    let leaf = Plot::<Cartesian>::new().mark(
        Area::new().transform_shared(
            Kde::new(col("value"))
                .group_by([col("division"), col("team")])
                .counts(true)
                .bandwidth(0.3)
                .steps(120)
                .extent(-2.4, 3.0)
                .resolve(KdeResolve::Shared)
                .as_fields("sample", "density"),
            |mark, kde| {
                mark.transform_shared_no_output(
                    JoinAggregate::new().max("__global_max_density", kde.density()),
                    |mark| {
                        mark.transform_shared_no_output(
                            Calculate::new()
                                .expr(
                                    "__half_width",
                                    lit(0.42) * col("density") / col("__global_max_density"),
                                )
                                .expr(
                                    "__leaf_key",
                                    concat(vec![col("division"), lit(" / "), col("team")]),
                                )
                                .expr("__band_start", lit(0.5) - col("__half_width"))
                                .expr("__band_end", lit(0.5) + col("__half_width")),
                            |mark| {
                                mark.orientation("horizontal")
                                    .details(["division", "team"])
                                    .x_with(nested(["division", "team"]), |x| {
                                        x.axis(|a| a.title("Team grouped by division").grid(false))
                                            .level(0, |l| {
                                                l.domain_scope(CoordinationScope::Shared)
                                                    .padding_inner(0.34)
                                                    .padding_outer(0.12)
                                            })
                                            .level(1, |l| {
                                                l.domain_scope(CoordinationScope::Shared)
                                                    .nest_scope(NestScope::Shared)
                                                    .padding_inner(0.1)
                                            })
                                            .band(col("__band_start"))
                                    })
                                    .x2_with(col(":x"), |x| x.band(col("__band_end")))
                                    .y_with(kde.value(), |y| {
                                        y.with_domain_scope(CoordinationScope::Shared)
                                            .scale(|s| s.domain((-2.4, 3.0)))
                                            .axis(|a| a.title("Value").grid(true))
                                    })
                                    .y2_with(kde.value(), |y| y.with_scale_name("y"))
                                    .fill_with(col("__leaf_key"), |fill| {
                                        fill.with_domain_scope(CoordinationScope::Shared)
                                            .legend(|l| l.title("Nested group"))
                                    })
                                    .stroke("#1f2937")
                                    .stroke_width(0.9)
                                    .opacity(0.72)
                                    .order(ChannelValue::from(kde.value()).no_scale())
                            },
                        )
                    },
                )
            },
        ),
    );

    let plot = Plot::<FacetColumn>::new()
        .title("Faceted nested violin from KDE + area")
        .subtitle("KDE and max-density normalization run at shared facet scope")
        .canvas_size(920.0, 440.0)
        .data(nested_violin_data(&ctx))
        .mark(Subplot::new(leaf).column(col("market")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile faceted nested violin");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_kde",
        "kde_low_level_nested_violin_facet_counts_shared_max",
    )
    .await;
}

#[tokio::test]
async fn kde_low_level_nested_violin_details_partition_coarse_fill() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Nested violin detail partitioning")
        .subtitle("Fill is coarser than the detail fields that split each violin")
        .canvas_size(800.0, 460.0)
        .data(nested_violin_data(&ctx))
        .mark(
            Area::new().transform(
                Kde::new(col("value"))
                    .group_by([col("division"), col("team")])
                    .counts(true)
                    .bandwidth(0.3)
                    .steps(120)
                    .extent(-2.4, 3.0)
                    .resolve(KdeResolve::Shared)
                    .as_fields("sample", "density"),
                |mark, kde| {
                    mark.transform_no_output(
                        JoinAggregate::new().max("__global_max_density", kde.density()),
                        |mark| {
                            mark.transform_no_output(
                                Calculate::new()
                                    .expr(
                                        "__half_width",
                                        lit(0.42) * col("density") / col("__global_max_density"),
                                    )
                                    .expr("__band_start", lit(0.5) - col("__half_width"))
                                    .expr("__band_end", lit(0.5) + col("__half_width")),
                                |mark| {
                                    mark.orientation("horizontal")
                                        .details(["division", "team"])
                                        .x_with(nested(["division", "team"]), |x| {
                                            x.axis(|a| {
                                                a.title("Team grouped by division").grid(false)
                                            })
                                            .level(0, |l| l.padding_inner(0.34).padding_outer(0.12))
                                            .level(1, |l| {
                                                l.nest_scope(NestScope::Shared).padding_inner(0.1)
                                            })
                                            .band(col("__band_start"))
                                        })
                                        .x2_with(col(":x"), |x| x.band(col("__band_end")))
                                        .y_with(kde.value(), |y| {
                                            y.scale(|s| s.domain((-2.4, 3.0)))
                                                .axis(|a| a.title("Value").grid(true))
                                        })
                                        .y2_with(kde.value(), |y| y.with_scale_name("y"))
                                        .fill_with(col("division"), |fill| {
                                            fill.legend(|l| l.title("Division"))
                                        })
                                        .stroke("#1f2937")
                                        .stroke_width(0.9)
                                        .opacity(0.72)
                                        .order(ChannelValue::from(kde.value()).no_scale())
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
        .expect("compile nested violin detail partition");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_kde",
        "kde_low_level_nested_violin_details_partition_coarse_fill",
    )
    .await;
}
