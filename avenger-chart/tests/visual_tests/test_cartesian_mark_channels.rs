use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use avenger_chart_core::ChannelValue;
use datafusion::arrow::array::{BooleanArray, Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn rounded_bar_data() -> DataFrame {
    let category = StringArray::from(vec!["Alpha", "Beta", "Gamma", "Delta"]);
    let value = Float64Array::from(vec![42.0, 68.0, 54.0, 88.0]);
    let corner_radius = Float64Array::from(vec![0.0, 6.0, 12.0, 20.0]);
    let stroke_width = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0]);
    let bar_opacity = Float64Array::from(vec![1.0, 0.78, 0.56, 0.34]);
    let point_opacity = Float64Array::from(vec![0.3, 0.55, 0.8, 1.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
        Field::new("corner_radius", DataType::Float64, false),
        Field::new("stroke_width", DataType::Float64, false),
        Field::new("bar_opacity", DataType::Float64, false),
        Field::new("point_opacity", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(category),
            Arc::new(value),
            Arc::new(corner_radius),
            Arc::new(stroke_width),
            Arc::new(bar_opacity),
            Arc::new(point_opacity),
        ],
    )
    .expect("rounded bar batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("rounded bar dataframe")
}

fn line_style_data() -> DataFrame {
    let mut series = Vec::new();
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut width = Vec::new();
    let mut opacity = Vec::new();
    let mut dash = Vec::new();
    let mut defined = Vec::new();

    let specs = [
        ("thin transparent", 1.5, 0.35, "solid", 10.0),
        ("medium dashed", 4.0, 0.65, "dashed", 24.0),
        ("wide opaque", 7.0, 1.0, "solid", 38.0),
    ];

    for (name, stroke_width, stroke_opacity, stroke_dash, offset) in specs {
        for i in (0..8).rev() {
            series.push(name);
            x.push(i as f64);
            y.push(offset + (i as f64 * 0.85).sin() * 8.0 + i as f64 * 2.25);
            width.push(stroke_width);
            opacity.push(stroke_opacity);
            dash.push(stroke_dash);
            defined.push(!(name == "medium dashed" && i == 4));
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("series", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("stroke_width", DataType::Float64, false),
        Field::new("opacity", DataType::Float64, false),
        Field::new("dash", DataType::Utf8, false),
        Field::new("defined", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(series)),
            Arc::new(Float64Array::from(x)),
            Arc::new(Float64Array::from(y)),
            Arc::new(Float64Array::from(width)),
            Arc::new(Float64Array::from(opacity)),
            Arc::new(StringArray::from(dash)),
            Arc::new(BooleanArray::from(defined)),
        ],
    )
    .expect("line style batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("line style dataframe")
}

#[tokio::test]
async fn test_existing_mark_opacity_and_rect_radius() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Existing mark channel cleanup")
        .data(rounded_bar_data())
        .mark(
            Rect::new()
                .x_with(col("category"), |c| {
                    c.scale_with::<Band>(|s| s).axis(|a| a.title("category"))
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y_with(lit(0.0), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .axis(|a| a.title("value"))
                })
                .y2(col("value"))
                .fill_with(col("category"), |c| c.legend(|l| l.visible(false)))
                .stroke("#111827")
                .stroke_width_with(col("stroke_width"), |c| c.no_scale())
                .corner_radius(ChannelValue::from(col("corner_radius")).no_scale())
                .opacity_with(col("bar_opacity"), |c| c.no_scale()),
        )
        .mark(
            Symbol::new()
                .x_with(col("category"), |c| c.band(0.5).axis(|a| a.visible(false)))
                .y(col("value") + lit(6.0))
                .size(180.0)
                .fill("#111827")
                .stroke("#ffffff")
                .stroke_width(1.5)
                .opacity_with(col("point_opacity"), |c| c.no_scale()),
        );

    let compiled = plot.compile(&ctx).await.expect("compile rounded bars");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_mark_channels",
        "existing_mark_opacity_and_rect_radius",
    )
    .await;
}

#[tokio::test]
async fn test_line_opacity_width_partitioning() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Line style partitioning")
        .data(line_style_data())
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 7.0))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 70.0))).axis(|a| a.title("y"))
                })
                .stroke_with(col("series"), |c| c.legend(|l| l.title("series")))
                .stroke_width_with(col("stroke_width"), |c| c.no_scale())
                .stroke_dash_with(col("dash"), |c| c.no_scale())
                .opacity_with(col("opacity"), |c| c.no_scale())
                .defined(ChannelValue::from(col("defined")).no_scale())
                .order(ChannelValue::from(col("x")).no_scale()),
        );

    let compiled = plot.compile(&ctx).await.expect("compile partitioned line");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_mark_channels",
        "line_opacity_width_partitioning",
    )
    .await;
}
