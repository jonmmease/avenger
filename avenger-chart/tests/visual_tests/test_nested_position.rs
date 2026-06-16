use std::sync::Arc;

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array, StringArray, StructArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{SessionContext, named_struct},
};
use palette::Srgba;

fn record_batch(fields: Vec<Field>, columns: Vec<ArrayRef>) -> RecordBatch {
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).expect("record batch")
}

fn nested_x(outer: &str, inner: &str) -> datafusion::logical_expr::Expr {
    named_struct(vec![lit("group"), col(outer), lit("member"), col(inner)])
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
async fn test_nested_position_struct_column_grouped_bar() {
    let ctx = SessionContext::new();
    let group_field = Arc::new(Field::new("quarter", DataType::Utf8, true));
    let member_field = Arc::new(Field::new("team", DataType::Utf8, true));
    let nested = Arc::new(StructArray::from(vec![
        (
            group_field.clone(),
            Arc::new(StringArray::from(vec![
                "Q1", "Q1", "Q1", "Q2", "Q2", "Q3", "Q3", "Q3",
            ])) as ArrayRef,
        ),
        (
            member_field.clone(),
            Arc::new(StringArray::from(vec![
                "North", "South", "East", "North", "East", "North", "South", "East",
            ])) as ArrayRef,
        ),
    ])) as ArrayRef;
    let batch = record_batch(
        vec![
            Field::new(
                "nested",
                DataType::Struct(
                    vec![group_field.as_ref().clone(), member_field.as_ref().clone()].into(),
                ),
                false,
            ),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float32, false),
        ],
        vec![
            nested,
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
                .x_with(col("nested"), |x| {
                    x.axis(|a| a.title("Struct column").grid(false))
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
        "struct_column_grouped_bar",
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
                            .label_angle(-25.0)
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
                                .axis(|a| a.label_angle(-30.0))
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
                        .label_angle(-45.0)
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
    let nested = named_struct(vec![
        lit("region"),
        col("region"),
        lit("category"),
        col("category"),
        lit("item"),
        col("item"),
    ]);

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
