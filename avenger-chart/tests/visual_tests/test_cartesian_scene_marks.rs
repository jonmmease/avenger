use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use avenger_chart_core::ChannelValue;
use datafusion::arrow::array::{BooleanArray, Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn rule_reference_data() -> DataFrame {
    let x = Float64Array::from(vec![0.5, 1.0, 1.5, 2.0, 0.4, 2.6]);
    let y = Float64Array::from(vec![0.5, 2.3, 1.0, 3.6, 3.0, 0.8]);
    let x2 = Float64Array::from(vec![2.8, 2.5, 0.8, 2.0, 2.7, 2.6]);
    let y2 = Float64Array::from(vec![2.8, 0.9, 3.8, 0.3, 3.0, 4.1]);
    let group = StringArray::from(vec![
        "diagonal",
        "diagonal",
        "reverse",
        "vertical",
        "horizontal",
        "vertical",
    ]);
    let width = Float64Array::from(vec![1.0, 2.5, 4.0, 5.5, 3.0, 6.0]);
    let opacity = Float64Array::from(vec![0.25, 0.45, 0.65, 0.85, 0.55, 1.0]);
    let cap = StringArray::from(vec!["butt", "round", "square", "round", "butt", "square"]);
    let dash = StringArray::from(vec![
        "solid", "dashed", "dotted", "solid", "dashdot", "dashed",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("x2", DataType::Float64, false),
        Field::new("y2", DataType::Float64, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("width", DataType::Float64, false),
        Field::new("opacity", DataType::Float64, false),
        Field::new("cap", DataType::Utf8, false),
        Field::new("dash", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(x2),
            Arc::new(y2),
            Arc::new(group),
            Arc::new(width),
            Arc::new(opacity),
            Arc::new(cap),
            Arc::new(dash),
        ],
    )
    .expect("rule reference batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("rule reference dataframe")
}

fn text_style_data() -> DataFrame {
    let x = Float64Array::from(vec![0.5, 1.5, 2.5, 0.5, 1.5, 2.5, 0.5, 1.5, 2.5]);
    let y = Float64Array::from(vec![2.5, 2.5, 2.5, 1.5, 1.5, 1.5, 0.5, 0.5, 0.5]);
    let label = StringArray::from(vec![
        "left/top",
        "center/top",
        "right/top",
        "left/mid",
        "center/mid",
        "right/mid",
        "left/bottom",
        "center/bottom",
        "right/bottom",
    ]);
    let align = StringArray::from(vec![
        "left", "center", "right", "left", "center", "right", "left", "center", "right",
    ]);
    let baseline = StringArray::from(vec![
        "top", "top", "top", "middle", "middle", "middle", "bottom", "bottom", "bottom",
    ]);
    let angle = Float64Array::from(vec![0.0, -20.0, 0.0, 18.0, 0.0, -18.0, 0.0, 20.0, 0.0]);
    let font_size = Float64Array::from(vec![12.0, 13.0, 12.0, 14.0, 18.0, 14.0, 12.0, 13.0, 12.0]);
    let weight = StringArray::from(vec![
        "normal", "bold", "normal", "bold", "normal", "bold", "normal", "bold", "normal",
    ]);
    let style = StringArray::from(vec![
        "normal", "normal", "italic", "normal", "italic", "normal", "italic", "normal", "normal",
    ]);
    let color_key = StringArray::from(vec![
        "cool", "warm", "cool", "warm", "accent", "warm", "cool", "warm", "cool",
    ]);
    let opacity = Float64Array::from(vec![0.45, 0.7, 1.0, 0.7, 1.0, 0.7, 1.0, 0.7, 0.45]);
    let limit = Float64Array::from(vec![70.0, 80.0, 70.0, 66.0, 90.0, 66.0, 70.0, 80.0, 70.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("align", DataType::Utf8, false),
        Field::new("baseline", DataType::Utf8, false),
        Field::new("angle", DataType::Float64, false),
        Field::new("font_size", DataType::Float64, false),
        Field::new("weight", DataType::Utf8, false),
        Field::new("style", DataType::Utf8, false),
        Field::new("color_key", DataType::Utf8, false),
        Field::new("opacity", DataType::Float64, false),
        Field::new("limit", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(label),
            Arc::new(align),
            Arc::new(baseline),
            Arc::new(angle),
            Arc::new(font_size),
            Arc::new(weight),
            Arc::new(style),
            Arc::new(color_key),
            Arc::new(opacity),
            Arc::new(limit),
        ],
    )
    .expect("text style batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("text style dataframe")
}

fn area_orientation_data() -> DataFrame {
    let t = Float64Array::from(vec![5.0, 0.0, 1.0, 2.0, 3.0, 4.0, 6.0]);
    let vertical_y = Float64Array::from(vec![4.9, 1.0, 2.0, 4.4, 3.6, 5.2, 3.0]);
    let horizontal_x = Float64Array::from(vec![4.8, 0.8, 1.7, 3.5, 2.8, 4.2, 2.2]);
    let vertical_defined = BooleanArray::from(vec![true, true, true, true, false, true, true]);
    let horizontal_defined = BooleanArray::from(vec![true, true, true, false, true, true, true]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("t", DataType::Float64, false),
        Field::new("vertical_y", DataType::Float64, false),
        Field::new("horizontal_x", DataType::Float64, false),
        Field::new("vertical_defined", DataType::Boolean, false),
        Field::new("horizontal_defined", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(t),
            Arc::new(vertical_y),
            Arc::new(horizontal_x),
            Arc::new(vertical_defined),
            Arc::new(horizontal_defined),
        ],
    )
    .expect("area orientation batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("area orientation dataframe")
}

fn trail_style_data() -> DataFrame {
    let mut series = Vec::new();
    let mut t = Vec::new();
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut size = Vec::new();
    let mut opacity = Vec::new();
    let mut defined = Vec::new();

    let specs = [
        ("fast stream", 0.0, 0.95, 0.0),
        ("slow stream", 0.55, 0.58, 1.15),
    ];

    for (name, phase, alpha, y_offset) in specs {
        for i in (0..9).rev() {
            let t_value = i as f64;
            series.push(name);
            t.push(t_value);
            x.push(t_value);
            y.push(y_offset + 2.8 + (t_value * 0.75 + phase).sin() * 1.15);
            size.push(3.0 + t_value * 1.45);
            opacity.push(alpha);
            defined.push(!(name == "slow stream" && i == 4));
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("series", DataType::Utf8, false),
        Field::new("t", DataType::Float64, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("size", DataType::Float64, false),
        Field::new("opacity", DataType::Float64, false),
        Field::new("defined", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(series)),
            Arc::new(Float64Array::from(t)),
            Arc::new(Float64Array::from(x)),
            Arc::new(Float64Array::from(y)),
            Arc::new(Float64Array::from(size)),
            Arc::new(Float64Array::from(opacity)),
            Arc::new(BooleanArray::from(defined)),
        ],
    )
    .expect("trail style batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("trail style dataframe")
}

#[tokio::test]
async fn test_cartesian_rule_reference_grid() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Rule reference grid")
        .data(rule_reference_data())
        .mark(
            Rule::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 3.0))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 4.5))).axis(|a| a.title("y"))
                })
                .x2(col("x2"))
                .y2(col("y2"))
                .stroke_with(col("group"), |c| c.legend(|l| l.title("rule")))
                .stroke_width_with(col("width"), |c| c.no_scale())
                .stroke_dash_with(col("dash"), |c| c.no_scale())
                .stroke_cap(ChannelValue::from(col("cap")).no_scale())
                .opacity_with(col("opacity"), |c| c.no_scale()),
        )
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(70.0)
                .fill("#ffffff")
                .stroke("#111827")
                .stroke_width(1.0)
                .opacity(0.8),
        );

    let compiled = plot.compile(&ctx).await.expect("compile rule plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_rule_reference_grid",
    )
    .await;
}

#[tokio::test]
async fn test_cartesian_text_label_styles() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Text label styles")
        .data(text_style_data())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 3.0))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 3.0))).axis(|a| a.title("y"))
                })
                .size(45.0)
                .fill("#ffffff")
                .stroke("#9ca3af")
                .stroke_width(1.0),
        )
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .align(ChannelValue::from(col("align")).no_scale())
                .baseline(ChannelValue::from(col("baseline")).no_scale())
                .angle_with(col("angle"), |c| c.no_scale())
                .font_size_with(col("font_size"), |c| c.no_scale())
                .font_weight(ChannelValue::from(col("weight")).no_scale())
                .font_style(ChannelValue::from(col("style")).no_scale())
                .limit(ChannelValue::from(col("limit")).no_scale())
                .color_with(col("color_key"), |c| c.legend(|l| l.visible(false)))
                .opacity_with(col("opacity"), |c| c.no_scale()),
        );

    let compiled = plot.compile(&ctx).await.expect("compile text plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_text_label_styles",
    )
    .await;
}

#[tokio::test]
async fn test_cartesian_area_vertical_horizontal_styles() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Area orientation and styles")
        .data(area_orientation_data())
        .mark(
            Area::new()
                .x_with(col("t"), |c| {
                    c.scale(|s| s.domain((0.0, 6.0))).axis(|a| a.title("x"))
                })
                .y_with(col("vertical_y"), |c| {
                    c.scale(|s| s.domain((0.0, 6.0))).axis(|a| a.title("y"))
                })
                .y2(0.0)
                .fill("#60a5fa")
                .stroke("#1d4ed8")
                .stroke_width(2.0)
                .stroke_join("round")
                .opacity(0.62)
                .defined(ChannelValue::from(col("vertical_defined")).no_scale())
                .order(ChannelValue::from(col("t")).no_scale()),
        )
        .mark(
            Area::new()
                .orientation("horizontal")
                .x(col("horizontal_x"))
                .x2(0.0)
                .y(col("t"))
                .fill("#f97316")
                .stroke("#9a3412")
                .stroke_width(1.5)
                .stroke_dash("dashed")
                .opacity(0.44)
                .defined(ChannelValue::from(col("horizontal_defined")).no_scale())
                .order(ChannelValue::from(col("t")).no_scale()),
        )
        .mark(
            Rule::new()
                .x(0.0)
                .x2(6.0)
                .y(0.0)
                .y2(0.0)
                .stroke("#374151")
                .stroke_width(1.0)
                .opacity(0.35),
        );

    let compiled = plot.compile(&ctx).await.expect("compile area plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_area_vertical_horizontal_styles",
    )
    .await;
}

#[tokio::test]
async fn test_cartesian_trail_size_and_opacity() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Trail size and opacity")
        .data(trail_style_data())
        .mark(
            Trail::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 8.0))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.5, 5.2))).axis(|a| a.title("y"))
                })
                .size_with(col("size"), |c| c.no_scale())
                .stroke_with(col("series"), |c| c.legend(|l| l.title("series")))
                .opacity_with(col("opacity"), |c| c.no_scale())
                .defined(ChannelValue::from(col("defined")).no_scale())
                .order(ChannelValue::from(col("t")).no_scale()),
        )
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(36.0)
                .fill("#ffffff")
                .stroke("#111827")
                .stroke_width(0.75)
                .opacity(0.5),
        );

    let compiled = plot.compile(&ctx).await.expect("compile trail plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_trail_size_and_opacity",
    )
    .await;
}
