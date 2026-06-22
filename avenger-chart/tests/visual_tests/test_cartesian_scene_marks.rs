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

fn text_leader_geometry_data() -> DataFrame {
    let x = Float64Array::from(vec![0.6, 1.4, 2.2, 3.0, 0.8, 1.6, 2.4, 3.2, 0.9, 2.6, 3.6]);
    let y = Float64Array::from(vec![
        2.35, 2.35, 2.35, 2.35, 1.5, 1.5, 1.5, 1.5, 0.5, 0.5, 2.35,
    ]);
    let label = StringArray::from(vec![
        "right", "left", "above", "below", "elbow", "curve", "rotated", "inside", "min", "off",
        "hidden",
    ]);
    let dx = Float64Array::from(vec![
        70.0, -70.0, 0.0, 0.0, 65.0, -65.0, 60.0, 10.0, 42.0, 75.0, -80.0,
    ]);
    let dy = Float64Array::from(vec![
        0.0, 0.0, -34.0, 42.0, -42.0, 42.0, 38.0, 0.0, 0.0, 0.0, 0.0,
    ]);
    let shape = StringArray::from(vec![
        "straight", "straight", "straight", "straight", "elbow", "curved", "curved", "straight",
        "straight", "straight", "straight",
    ]);
    let angle = Float64Array::from(vec![
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -25.0, 0.0, 0.0, 0.0, 0.0,
    ]);
    let leader = BooleanArray::from(vec![
        true, true, true, true, true, true, true, true, true, false, true,
    ]);
    let defined = BooleanArray::from(vec![
        true, true, true, true, true, true, true, true, true, true, false,
    ]);
    let label_padding =
        Float64Array::from(vec![2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 18.0, 2.0, 2.0, 2.0]);
    let target_radius =
        Float64Array::from(vec![0.0, 0.0, 0.0, 0.0, 0.0, 6.0, 5.0, 0.0, 0.0, 0.0, 0.0]);
    let min_length =
        Float64Array::from(vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 90.0, 1.0, 1.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("dx", DataType::Float64, false),
        Field::new("dy", DataType::Float64, false),
        Field::new("shape", DataType::Utf8, false),
        Field::new("angle", DataType::Float64, false),
        Field::new("leader", DataType::Boolean, false),
        Field::new("defined", DataType::Boolean, false),
        Field::new("label_padding", DataType::Float64, false),
        Field::new("target_radius", DataType::Float64, false),
        Field::new("min_length", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(label),
            Arc::new(dx),
            Arc::new(dy),
            Arc::new(shape),
            Arc::new(angle),
            Arc::new(leader),
            Arc::new(defined),
            Arc::new(label_padding),
            Arc::new(target_radius),
            Arc::new(min_length),
        ],
    )
    .expect("text leader geometry batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("text leader geometry dataframe")
}

fn text_leader_arrowhead_data() -> DataFrame {
    let x = Float64Array::from(vec![0.7, 2.2, 3.7, 5.2, 0.7, 2.2, 3.7, 5.2]);
    let y = Float64Array::from(vec![1.8, 1.8, 1.8, 1.8, 0.7, 0.7, 0.7, 0.7]);
    let label = StringArray::from(vec!["", "", "", "", "", "", "", ""]);
    let dx = Float64Array::from(vec![46.0, 46.0, 46.0, -46.0, 46.0, -46.0, 46.0, -46.0]);
    let dy = Float64Array::from(vec![0.0, 0.0, 0.0, 0.0, -28.0, -28.0, 28.0, 28.0]);
    let arrow = StringArray::from(vec![
        "none", "open", "triangle", "open", "triangle", "triangle", "open", "triangle",
    ]);
    let arrow_length = Float64Array::from(vec![6.0, 6.0, 6.0, 12.0, 5.0, 10.0, 8.0, 0.0]);
    let arrow_width = Float64Array::from(vec![5.0, 5.0, 5.0, 4.0, 14.0, 9.0, 0.0, 10.0]);
    let target_radius = Float64Array::from(vec![0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0]);
    let stroke = StringArray::from(vec![
        "#374151", "#2563eb", "#dc2626", "#7c3aed", "#059669", "#d97706", "#0891b2", "#be123c",
    ]);
    let symbol_size = Float64Array::from(vec![45.0, 45.0, 45.0, 45.0, 45.0, 100.0, 45.0, 45.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("dx", DataType::Float64, false),
        Field::new("dy", DataType::Float64, false),
        Field::new("arrow", DataType::Utf8, false),
        Field::new("arrow_length", DataType::Float64, false),
        Field::new("arrow_width", DataType::Float64, false),
        Field::new("target_radius", DataType::Float64, false),
        Field::new("stroke", DataType::Utf8, false),
        Field::new("symbol_size", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(label),
            Arc::new(dx),
            Arc::new(dy),
            Arc::new(arrow),
            Arc::new(arrow_length),
            Arc::new(arrow_width),
            Arc::new(target_radius),
            Arc::new(stroke),
            Arc::new(symbol_size),
        ],
    )
    .expect("text leader arrowhead batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("text leader arrowhead dataframe")
}

fn text_leader_stroke_style_data() -> DataFrame {
    let x = Float64Array::from(vec![0.7, 1.8, 2.9, 4.0, 0.9, 2.0, 3.1, 4.2]);
    let y = Float64Array::from(vec![1.8, 1.8, 1.8, 1.8, 0.7, 0.7, 0.7, 0.7]);
    let label = StringArray::from(vec![
        "thin", "dash", "dot", "miter", "round", "bevel", "curve", "arrow",
    ]);
    let dx = Float64Array::from(vec![50.0, 50.0, 50.0, -50.0, 50.0, 50.0, -50.0, -50.0]);
    let dy = Float64Array::from(vec![-34.0, -34.0, -34.0, 34.0, -34.0, -34.0, 34.0, 34.0]);
    let shape = StringArray::from(vec![
        "straight", "straight", "straight", "elbow", "elbow", "elbow", "curved", "elbow",
    ]);
    let stroke = StringArray::from(vec![
        "#111827", "#1d4ed8", "#047857", "#b45309", "#7c3aed", "#be123c", "#0891b2", "#4b5563",
    ]);
    let stroke_width = Float64Array::from(vec![0.75, 1.5, 3.0, 2.5, 2.5, 2.5, 2.0, 4.0]);
    let stroke_cap = StringArray::from(vec![
        "butt", "round", "square", "butt", "round", "square", "round", "round",
    ]);
    let stroke_join = StringArray::from(vec![
        "miter", "round", "bevel", "miter", "round", "bevel", "round", "round",
    ]);
    let dash = StringArray::from(vec![
        "solid", "dashed", "dotted", "solid", "dashed", "dotted", "dashdot", "solid",
    ]);
    let arrow = StringArray::from(vec![
        "none", "none", "none", "none", "none", "none", "open", "triangle",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("dx", DataType::Float64, false),
        Field::new("dy", DataType::Float64, false),
        Field::new("shape", DataType::Utf8, false),
        Field::new("stroke", DataType::Utf8, false),
        Field::new("stroke_width", DataType::Float64, false),
        Field::new("stroke_cap", DataType::Utf8, false),
        Field::new("stroke_join", DataType::Utf8, false),
        Field::new("dash", DataType::Utf8, false),
        Field::new("arrow", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(label),
            Arc::new(dx),
            Arc::new(dy),
            Arc::new(shape),
            Arc::new(stroke),
            Arc::new(stroke_width),
            Arc::new(stroke_cap),
            Arc::new(stroke_join),
            Arc::new(dash),
            Arc::new(arrow),
        ],
    )
    .expect("text leader stroke style batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("text leader stroke style dataframe")
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

fn image_alignment_data() -> DataFrame {
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut align = Vec::new();
    let mut baseline = Vec::new();

    let aligns = ["left", "center", "right"];
    let baselines = ["top", "middle", "bottom"];
    for (row, baseline_value) in baselines.iter().enumerate() {
        for (col, align_value) in aligns.iter().enumerate() {
            x.push(col as f64);
            y.push(2.0 - row as f64);
            align.push(*align_value);
            baseline.push(*baseline_value);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("align", DataType::Utf8, false),
        Field::new("baseline", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(x)),
            Arc::new(Float64Array::from(y)),
            Arc::new(StringArray::from(align)),
            Arc::new(StringArray::from(baseline)),
        ],
    )
    .expect("image alignment batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("image alignment dataframe")
}

fn path_transform_data() -> DataFrame {
    let x = Float64Array::from(vec![0.4, 1.2, 2.0, 0.8, 1.7]);
    let y = Float64Array::from(vec![0.55, 1.35, 0.85, 2.15, 2.45]);
    let path = StringArray::from(vec![
        "M 0 -18 L 15 12 L -15 12 Z",
        "M -16 -16 L 16 -16 L 16 16 L -16 16 Z",
        "M -18 0 C -8 -18 8 -18 18 0 C 8 18 -8 18 -18 0 Z",
        "M -18 -10 L 0 -18 L 18 -10 L 10 16 L -10 16 Z",
        "M -16 12 L 0 -16 L 16 12 Z",
    ]);
    let path_transform = StringArray::from(vec![
        "rotate(-20) scale(1.0)",
        "rotate(12) scale(0.85)",
        "rotate(28) scale(1.05)",
        "rotate(-38) scale(0.9)",
        "rotate(42) scale(1.15)",
    ]);
    let group = StringArray::from(vec!["alpha", "beta", "gamma", "alpha", "beta"]);
    let opacity = Float64Array::from(vec![0.95, 0.8, 0.68, 0.78, 0.9]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("path", DataType::Utf8, false),
        Field::new("path_transform", DataType::Utf8, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("opacity", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(path),
            Arc::new(path_transform),
            Arc::new(group),
            Arc::new(opacity),
        ],
    )
    .expect("path transform batch");

    SessionContext::new()
        .read_batch(batch)
        .expect("path transform dataframe")
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
async fn test_cartesian_text_leader_geometry_and_offsets() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Text leader geometry")
        .data(text_leader_geometry_data())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 4.0))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 3.0))).axis(|a| a.title("y"))
                })
                .size(52.0)
                .fill("#ffffff")
                .stroke("#9ca3af")
                .stroke_width(1.0),
        )
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .align("center")
                .baseline("middle")
                .font_size(12.0)
                .color("#111827")
                .angle_with(col("angle"), |c| c.no_scale())
                .leader(ChannelValue::from(col("leader")).no_scale())
                .defined(ChannelValue::from(col("defined")).no_scale())
                .leader_shape(ChannelValue::from(col("shape")).no_scale())
                .leader_label_padding(ChannelValue::from(col("label_padding")).no_scale())
                .leader_target_radius(ChannelValue::from(col("target_radius")).no_scale())
                .leader_min_length(ChannelValue::from(col("min_length")).no_scale())
                .leader_stroke("#374151")
                .leader_stroke_width(1.5)
                .leader_stroke_cap("round")
                .leader_stroke_join("round")
                .leader_offset_x(ChannelValue::from(col("dx")).no_scale())
                .leader_offset_y(ChannelValue::from(col("dy")).no_scale()),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile text leader geometry plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_text_leader_geometry_and_offsets",
    )
    .await;
}

#[tokio::test]
async fn test_cartesian_text_leader_arrowheads() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Text leader arrowheads")
        .data(text_leader_arrowhead_data())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 6.0))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 2.3))).axis(|a| a.title("y"))
                })
                .size_with(col("symbol_size"), |c| c.no_scale())
                .fill("#ffffff")
                .stroke("#9ca3af")
                .stroke_width(1.0),
        )
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .align("center")
                .baseline("middle")
                .font_size(12.0)
                .color("#111827")
                .leader(true)
                .leader_shape("straight")
                .leader_arrow(ChannelValue::from(col("arrow")).no_scale())
                .leader_arrow_length(ChannelValue::from(col("arrow_length")).no_scale())
                .leader_arrow_width(ChannelValue::from(col("arrow_width")).no_scale())
                .leader_target_radius(ChannelValue::from(col("target_radius")).no_scale())
                .leader_stroke(ChannelValue::from(col("stroke")).no_scale())
                .leader_stroke_width(1.6)
                .leader_stroke_cap("round")
                .leader_stroke_join("round")
                .leader_offset_x(ChannelValue::from(col("dx")).no_scale())
                .leader_offset_y(ChannelValue::from(col("dy")).no_scale()),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile text leader arrowhead plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_text_leader_arrowheads",
    )
    .await;
}

#[tokio::test]
async fn test_cartesian_text_leader_stroke_styles() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Text leader stroke styles")
        .data(text_leader_stroke_style_data())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 4.8))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 2.3))).axis(|a| a.title("y"))
                })
                .size(45.0)
                .fill("#ffffff")
                .stroke("#d1d5db")
                .stroke_width(1.0),
        )
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .align("center")
                .baseline("middle")
                .font_size(12.0)
                .color("#374151")
                .leader(true)
                .leader_shape(ChannelValue::from(col("shape")).no_scale())
                .leader_arrow(ChannelValue::from(col("arrow")).no_scale())
                .leader_stroke(ChannelValue::from(col("stroke")).no_scale())
                .leader_stroke_width(ChannelValue::from(col("stroke_width")).no_scale())
                .leader_stroke_cap(ChannelValue::from(col("stroke_cap")).no_scale())
                .leader_stroke_join(ChannelValue::from(col("stroke_join")).no_scale())
                .leader_stroke_dash(ChannelValue::from(col("dash")).no_scale())
                .leader_arrow_length(8.0)
                .leader_arrow_width(7.0)
                .leader_offset_x(ChannelValue::from(col("dx")).no_scale())
                .leader_offset_y(ChannelValue::from(col("dy")).no_scale()),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile text leader stroke style plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_text_leader_stroke_styles",
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

#[tokio::test]
async fn test_cartesian_image_alignment_grid() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Image alignment grid")
        .data(image_alignment_data())
        .mark(
            Image::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((-0.45, 2.45))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((-0.45, 2.45))).axis(|a| a.title("y"))
                })
                .image(TINY_PNG_DATA_URI)
                .width(ChannelValue::from(lit(36.0)).no_scale())
                .height(ChannelValue::from(lit(28.0)).no_scale())
                .align(ChannelValue::from(col("align")).no_scale())
                .baseline(ChannelValue::from(col("baseline")).no_scale())
                .aspect(false)
                .smooth(false),
        )
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(42.0)
                .fill("#111827")
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile image plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_image_alignment_grid",
    )
    .await;
}

#[tokio::test]
async fn test_cartesian_path_svg_transform_styles() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Path SVG transforms")
        .data(path_transform_data())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 2.4))).axis(|a| a.title("x"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.2, 2.8))).axis(|a| a.title("y"))
                })
                .size(50.0)
                .fill("#ffffff")
                .stroke("#111827")
                .stroke_width(0.9)
                .opacity(0.65),
        )
        .mark(
            PathMark::new()
                .x(col("x"))
                .y(col("y"))
                .path_with(col("path"), |c| c.no_scale())
                .path_transform_with(col("path_transform"), |c| c.no_scale())
                .fill_with(col("group"), |c| c.legend(|l| l.visible(false)))
                .stroke("#111827")
                .stroke_width(1.6)
                .stroke_join("round")
                .opacity_with(col("opacity"), |c| c.no_scale()),
        );

    let compiled = plot.compile(&ctx).await.expect("compile path plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "cartesian_scene_marks",
        "cartesian_path_svg_transform_styles",
    )
    .await;
}

const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";
