use crate::visual_tests::helpers::assert_visual_match_with_canvas_config_and_sidecars;
use avenger_chart::plot::CompiledPlot;
use avenger_chart::prelude::*;
use avenger_wgpu::canvas::CanvasConfig;
use datafusion::arrow::array::{ArrayRef, Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
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

#[tokio::test]
async fn bessel_family_legend() {
    let ctx = SessionContext::new();
    let df = bessel_family_data(&ctx);

    let plot = Plot::<Cartesian>::new()
        .title("Bessel functions $J_n(x)$")
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|axis| axis.title("$x$").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((-1.05, 1.05)))
                        .axis(|axis| axis.title("$J_n(x)$").grid(true))
                })
                .stroke_with(col("series"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|legend| legend.title("Order $n$"))
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

    let plot = Plot::<Cartesian>::new()
        .title("Damped oscillator $x(t) = A r^t$")
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 8.0)))
                        .axis(|axis| axis.title("$t$").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((-1.05, 1.05)))
                        .axis(|axis| axis.title("$x(t)$").grid(true))
                })
                .stroke_with(col("series"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|legend| legend.title("Frequency $omega$"))
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

    let plot = Plot::<Cartesian>::new()
        .title("Root fraction $y = sqrt(x) / (1 + x^2)$")
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 4.0)))
                        .axis(|axis| axis.title("$x$").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 0.75)))
                        .axis(|axis| axis.title("$sqrt(x) / (1 + x^2)$").grid(true))
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

    let plot = Plot::<Cartesian>::new()
        .title("Annotated Bessel-like curve $J_n(x)$")
        .data(line_df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|axis| axis.title("$x$").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((-1.05, 1.05)))
                        .axis(|axis| axis.title("$J_n(x)$").grid(true))
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

    let plot = Plot::<Cartesian>::new()
        .title("Cost is \\$5, score is $R^2$")
        .mark(
            Text::new()
                .data(df)
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.title("$x$"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.title("$y$"))
                })
                .text(col("label"))
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

    let plot = Plot::<Cartesian>::new()
        .title("Math z-order $x_i^2$")
        .mark(
            Text::new()
                .data(label_df)
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.title("$x$"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 1.0)))
                        .axis(|axis| axis.title("$y$"))
                })
                .text(col("label"))
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
