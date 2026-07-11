use crate::visual_tests::helpers::assert_visual_match_with_canvas_config_and_sidecars;
use avenger_chart::plot::CompiledPlot;
use avenger_chart::prelude::*;
use avenger_wgpu::canvas::CanvasConfig;
use datafusion::arrow::array::{ArrayRef, Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::sync::Arc;

const CATEGORY: &str = "typst_math";

fn dataframe(
    ctx: &SessionContext,
    x: Vec<f64>,
    y: Vec<f64>,
    series: Vec<String>,
    order: Vec<f64>,
) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("order", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(x)) as ArrayRef,
            Arc::new(Float64Array::from(y)) as ArrayRef,
            Arc::new(StringArray::from(series)) as ArrayRef,
            Arc::new(Float64Array::from(order)) as ArrayRef,
        ],
    )
    .expect("typst math visual test data");

    ctx.read_batch(batch)
        .expect("typst math visual test dataframe")
}

fn bessel_like(order: usize, x: f64) -> f64 {
    let phase = order as f64 * 0.72;
    let amplitude = 1.0 - order as f64 * 0.14;
    amplitude * (x - phase).cos() / (1.0 + 0.11 * x * x)
}

fn bessel_family_data(ctx: &SessionContext) -> DataFrame {
    let labels = ["$J_0(x)$", "$J_1(x)$", "$J_2(x)$"];
    let mut x_values = Vec::new();
    let mut y_values = Vec::new();
    let mut series = Vec::new();
    let mut order_values = Vec::new();

    for (series_order, label) in labels.iter().enumerate() {
        for i in 0..=80 {
            let x = i as f64 * 0.15;
            x_values.push(x);
            y_values.push(bessel_like(series_order, x));
            series.push((*label).to_string());
            order_values.push(x);
        }
    }

    dataframe(ctx, x_values, y_values, series, order_values)
}

fn damped_oscillator_data(ctx: &SessionContext) -> DataFrame {
    let series_specs = [
        ("$omega = 1$", 1.0),
        ("$omega = 2$", 2.0),
        ("$omega = 3$", 3.0),
    ];
    let mut x_values = Vec::new();
    let mut y_values = Vec::new();
    let mut series = Vec::new();
    let mut order_values = Vec::new();

    for (label, omega) in series_specs {
        for i in 0..=100 {
            let t = i as f64 * 0.08;
            x_values.push(t);
            y_values.push((-0.28 * t).exp() * (omega * t).cos());
            series.push(label.to_string());
            order_values.push(t);
        }
    }

    dataframe(ctx, x_values, y_values, series, order_values)
}

fn root_fraction_data(ctx: &SessionContext) -> DataFrame {
    let mut x_values = Vec::new();
    let mut y_values = Vec::new();
    let mut series = Vec::new();
    let mut order_values = Vec::new();

    for i in 0..=100 {
        let x = i as f64 * 0.04;
        x_values.push(x);
        y_values.push(x.sqrt() / (1.0 + x * x));
        series.push("root fraction".to_string());
        order_values.push(x);
    }

    dataframe(ctx, x_values, y_values, series, order_values)
}

fn label_dataframe(ctx: &SessionContext, rows: Vec<(f64, f64, &str)>) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut label = Vec::new();
    for (x_value, y_value, label_value) in rows {
        x.push(x_value);
        y.push(y_value);
        label.push(label_value);
    }

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(x)) as ArrayRef,
            Arc::new(Float64Array::from(y)) as ArrayRef,
            Arc::new(StringArray::from(label)) as ArrayRef,
        ],
    )
    .expect("typst math label batch");

    ctx.read_batch(batch).expect("typst math label dataframe")
}

fn occlusion_rect_dataframe(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x0", DataType::Float64, false),
        Field::new("x1", DataType::Float64, false),
        Field::new("y0", DataType::Float64, false),
        Field::new("y1", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![0.43])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.64])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.43])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.60])) as ArrayRef,
        ],
    )
    .expect("typst math occlusion rect batch");

    ctx.read_batch(batch)
        .expect("typst math occlusion rect dataframe")
}

fn static_text_markup_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![2.4, 3.6, 2.9])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "#overline[baseline]",
                "#underline[checked]",
                "#strike[old] + current",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "#overline[mean]",
                "#underline[underlined]",
                "H#sub[2]O #super[\\*]",
            ])) as ArrayRef,
        ],
    )
    .expect("typst static text markup batch");

    ctx.read_batch(batch)
        .expect("typst static text markup dataframe")
}

fn emoji_text_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![2.2, 3.4, 2.8])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Face #emoji.face",
                "Rocket 🚀",
                "Trend #emoji.chart.up",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Happy #emoji.face",
                "Launch 🚀",
                "Growth #emoji.chart.up",
            ])) as ArrayRef,
        ],
    )
    .expect("typst emoji text batch");

    ctx.read_batch(batch).expect("typst emoji text dataframe")
}

fn math_font_weight_data(ctx: &SessionContext) -> DataFrame {
    let weights = [
        ("300 Light", "300", 2.8),
        ("500 Medium", "500", 1.8),
        ("700 Bold", "700", 0.8),
    ];
    let label_template = " Lato text + $R^2 = alpha x^2 + beta$ + $sqrt(x)/(1+x^2)$";

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("weight", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![0.05; weights.len()])) as ArrayRef,
            Arc::new(Float64Array::from(
                weights.iter().map(|(_, _, y)| *y).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                weights
                    .iter()
                    .map(|(_, weight, _)| *weight)
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                weights
                    .iter()
                    .map(|(name, _, _)| format!("{name}:{label_template}"))
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )
    .expect("typst math font weight batch");

    ctx.read_batch(batch)
        .expect("typst math font weight dataframe")
}

async fn assert_typst_math_wgpu(compiled: CompiledPlot, ctx: &SessionContext, baseline_name: &str) {
    assert_visual_match_with_canvas_config_and_sidecars(
        Arc::new(compiled),
        ctx,
        None,
        CATEGORY,
        baseline_name,
        0.9999,
        CanvasConfig::default(),
    )
    .await;
}

async fn assert_typst_math_wgpu_with_params(
    compiled: CompiledPlot,
    ctx: &SessionContext,
    baseline_name: &str,
    params: IndexMap<String, ScalarValue>,
) {
    assert_visual_match_with_canvas_config_and_sidecars(
        Arc::new(compiled),
        ctx,
        Some(params),
        CATEGORY,
        baseline_name,
        0.9999,
        CanvasConfig::default(),
    )
    .await;
}

fn param_markup_visual_params() -> IndexMap<String, ScalarValue> {
    let mut params = IndexMap::new();
    params.insert(
        "title_word".to_string(),
        ScalarValue::Utf8(Some("model fit".to_string())),
    );
    params.insert("r_sq".to_string(), ScalarValue::Float64(Some(0.94)));
    params.insert(
        "subtitle_color".to_string(),
        ScalarValue::Utf8(Some("red".to_string())),
    );
    params.insert(
        "subtitle_word".to_string(),
        ScalarValue::Utf8(Some("checked inputs".to_string())),
    );
    params.insert(
        "x_unit".to_string(),
        ScalarValue::Utf8(Some("seconds".to_string())),
    );
    params.insert(
        "y_unit".to_string(),
        ScalarValue::Utf8(Some("amplitude".to_string())),
    );
    params.insert(
        "legend_word".to_string(),
        ScalarValue::Utf8(Some("family".to_string())),
    );
    params.insert(
        "note_word".to_string(),
        ScalarValue::Utf8(Some("forecast".to_string())),
    );
    params.insert("slope".to_string(), ScalarValue::Float64(Some(0.17)));
    params.insert("intercept".to_string(), ScalarValue::Float64(Some(0.42)));
    params
}

#[tokio::test]
async fn param_driven_typst_markup() {
    let ctx = SessionContext::new();
    let line_df = damped_oscillator_data(&ctx);
    let annotation_df = label_dataframe(
        &ctx,
        vec![(0.15, 0.66, "Note #note_word: $#slope x + #intercept$")],
    );

    let plot = Chart::<Cartesian>::new()
        .configure_title("Param title #title_word: $R^2 = #r_sq$", |t| t.typst())
        .configure_subtitle(
            "Subtitle #underline(stroke: subtitle_color)[#subtitle_word]",
            |s| s.typst(),
        )
        .data(line_df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 8.0)))
                        .axis(|axis| axis.title("Time $t$ in #x_unit").typst().grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((-1.05, 1.05)))
                        .axis(|axis| axis.title("Response #y_unit $y(t)$").typst().grid(true))
                })
                .stroke_with(col("series"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|legend| legend.title("Legend #legend_word").typst())
                })
                .stroke_width(2.0)
                .order(col("order")),
        )
        .mark(
            Text::new()
                .data(annotation_df)
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .typst()
                .align("left")
                .baseline("middle")
                .font_size(13.0)
                .color("#111827"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile param-driven Typst markup plot");
    assert_typst_math_wgpu_with_params(
        compiled,
        &ctx,
        "param_driven_typst_markup",
        param_markup_visual_params(),
    )
    .await;
}

#[tokio::test]
async fn mixed_text_math_font_weights() {
    let ctx = SessionContext::new();
    let df = math_font_weight_data(&ctx);

    let plot = Chart::<Cartesian>::new()
        .title("Mixed Lato + Lete Sans Math weights")
        .subtitle("Math selects the nearest bundled Lete Sans Math face")
        .canvas_size(760.0, 300.0)
        .mark(
            Text::new()
                .data(df)
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.visible(false))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 3.3)))
                        .axis(|axis| axis.visible(false))
                })
                .text(col("label"))
                .typst()
                .align("left")
                .baseline("middle")
                .font_size(20.0)
                .font_weight(ChannelValue::from(col("weight")).no_scale())
                .color("#111827"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile mixed text math font weights plot");
    assert_typst_math_wgpu(compiled, &ctx, "mixed_text_math_font_weights").await;
}

#[tokio::test]
async fn static_text_markup_showcase() {
    let ctx = SessionContext::new();
    let df = static_text_markup_data(&ctx);

    let plot = Chart::<Cartesian>::new()
        .configure_title("Typst text #underline[markup]", |t| t.typst())
        .configure_subtitle(
            "Decorations: #strike[removed], #overline[mean], H#sub[2]O",
            |s| s.typst(),
        )
        .data(df.clone())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.5, 3.5)))
                        .axis(|axis| axis.title("Index #super[\\*]").typst().grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 4.2)))
                        .axis(|axis| axis.title("#overline[value]").typst().grid(true))
                })
                .fill_with(col("group"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|legend| legend.title("Legend #underline[group]").typst())
                })
                .stroke("#111827")
                .stroke_width(1.0)
                .size(180.0),
        )
        .mark(
            Text::new()
                .data(df)
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .typst()
                .align("center")
                .baseline("bottom")
                .font_size(13.0)
                .color("#111827"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile static Typst text markup plot");
    assert_typst_math_wgpu(compiled, &ctx, "static_text_markup_showcase").await;
}

#[tokio::test]
async fn emoji_text_showcase() {
    let ctx = SessionContext::new();
    let df = emoji_text_data(&ctx);

    let plot = Chart::<Cartesian>::new()
        .configure_title("Emoji text #emoji.face + literal 🚀", |t| t.typst())
        .configure_subtitle("Aliases and system color emoji #emoji.chart.up", |s| {
            s.typst()
        })
        .data(df.clone())
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.5, 3.5)))
                        .axis(|axis| axis.title("Step #emoji.rocket").typst().grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 4.2)))
                        .axis(|axis| axis.title("Score #emoji.chart.up").typst().grid(true))
                })
                .fill_with(col("group"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|legend| legend.title("Legend #emoji.face").typst())
                })
                .stroke("#111827")
                .stroke_width(1.0)
                .size(180.0),
        )
        .mark(
            Text::new()
                .data(df)
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .typst()
                .align("center")
                .baseline("bottom")
                .font_size(13.0)
                .color("#111827"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile emoji text plot");
    assert_typst_math_wgpu(compiled, &ctx, "emoji_text_showcase").await;
}

#[tokio::test]
async fn bessel_family_legend() {
    let ctx = SessionContext::new();
    let df = bessel_family_data(&ctx);

    let plot = Chart::<Cartesian>::new()
        .configure_title("Bessel functions $J_n(x)$", |t| t.typst())
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|axis| axis.title("$x$").typst().grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((-1.05, 1.05)))
                        .axis(|axis| axis.title("$J_n(x)$").typst().grid(true))
                })
                .stroke_with(col("series"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|legend| legend.title("Order $n$").typst())
                })
                .stroke_width(2.0)
                .order(col("order")),
        );

    let compiled = plot.compile(&ctx).await.expect("compile Bessel plot");
    assert_typst_math_wgpu(compiled, &ctx, "bessel_family_legend").await;
}

#[tokio::test]
async fn damped_oscillator_labels() {
    let ctx = SessionContext::new();
    let df = damped_oscillator_data(&ctx);

    let plot = Chart::<Cartesian>::new()
        .configure_title("Damped oscillator $x(t) = A r^t$", |t| t.typst())
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 8.0)))
                        .axis(|axis| axis.title("$t$").typst().grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((-1.05, 1.05)))
                        .axis(|axis| axis.title("$x(t)$").typst().grid(true))
                })
                .stroke_with(col("series"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|legend| legend.title("Frequency $omega$").typst())
                })
                .stroke_width(2.0)
                .order(col("order")),
        );

    let compiled = plot.compile(&ctx).await.expect("compile oscillator plot");
    assert_typst_math_wgpu(compiled, &ctx, "damped_oscillator_labels").await;
}

#[tokio::test]
async fn root_fraction_title() {
    let ctx = SessionContext::new();
    let df = root_fraction_data(&ctx);

    let plot = Chart::<Cartesian>::new()
        .configure_title("Root fraction $y = sqrt(x) / (1 + x^2)$", |t| t.typst())
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 4.0)))
                        .axis(|axis| axis.title("$x$").typst().grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 0.75)))
                        .axis(|axis| axis.title("$sqrt(x) / (1 + x^2)$").typst().grid(true))
                })
                .stroke("#0072b2")
                .stroke_width(2.5)
                .order(col("order")),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile root fraction plot");
    assert_typst_math_wgpu(compiled, &ctx, "root_fraction_title").await;
}

#[tokio::test]
async fn bessel_equation_annotation() {
    let ctx = SessionContext::new();
    let line_df = bessel_family_data(&ctx);
    let annotation_df = label_dataframe(
        &ctx,
        vec![(
            5.45,
            0.78,
            "Bessel equation $x^2 y + x y + (x^2 - n^2)y = 0$",
        )],
    );

    let plot = Chart::<Cartesian>::new()
        .configure_title("Annotated Bessel-like curve $J_n(x)$", |t| t.typst())
        .data(line_df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|axis| axis.title("$x$").typst().grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((-1.05, 1.05)))
                        .axis(|axis| axis.title("$J_n(x)$").typst().grid(true))
                })
                .stroke_with(col("series"), |c| c.legend(|legend| legend.visible(false)))
                .stroke_width(2.0)
                .order(col("order")),
        )
        .mark(
            Text::new()
                .data(annotation_df)
                .x(col("x"))
                .y(col("y"))
                .text(col("label"))
                .typst()
                .align("center")
                .baseline("middle")
                .font_size(13.0)
                .color("#111827"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile Bessel annotation plot");
    assert_typst_math_wgpu(compiled, &ctx, "bessel_equation_annotation").await;
}

#[tokio::test]
async fn escaped_dollar_plain_text() {
    let ctx = SessionContext::new();
    let df = label_dataframe(&ctx, vec![(0.52, 0.52, "Price \\$7, score $R^2 = 0.94$")]);

    let plot = Chart::<Cartesian>::new()
        .configure_title("Cost is \\$5, score is $R^2$", |t| t.typst())
        .mark(
            Text::new()
                .data(df)
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.title("$x$").typst())
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.title("$y$").typst())
                })
                .text(col("label"))
                .typst()
                .align("center")
                .baseline("middle")
                .font_size(17.0)
                .color("#111827"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile escaped dollar plot");
    assert_typst_math_wgpu(compiled, &ctx, "escaped_dollar_plain_text").await;
}

#[tokio::test]
async fn mark_occlusion_math_label() {
    let ctx = SessionContext::new();
    let label_df = label_dataframe(&ctx, vec![(0.50, 0.52, "peak $x_i^2$")]);
    let rect_df = occlusion_rect_dataframe(&ctx);

    let plot = Chart::<Cartesian>::new()
        .configure_title("Math z-order $x_i^2$", |t| t.typst())
        .mark(
            Text::new()
                .data(label_df)
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.title("$x$").typst())
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.title("$y$").typst())
                })
                .text(col("label"))
                .typst()
                .align("center")
                .baseline("middle")
                .font_size(28.0)
                .color("#111827"),
        )
        .mark(
            Rect::new()
                .data(rect_df)
                .x(col("x0"))
                .x2(col("x1"))
                .y(col("y0"))
                .y2(col("y1"))
                .fill("#d62728")
                .opacity(0.72)
                .stroke("#7f1d1d")
                .stroke_width(1.0),
        );

    let compiled = plot.compile(&ctx).await.expect("compile math z-order plot");
    assert_typst_math_wgpu(compiled, &ctx, "mark_occlusion_math_label").await;
}
