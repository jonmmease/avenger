use avenger_chart::prelude::*;
use avenger_chart_core::ChannelValue;
use datafusion::logical_expr::lit;
use datafusion::prelude::*;

const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                (0.0, 0.4, 0.0, 0.2, 0.95, 'alpha', true, 'A'),
                (1.0, 1.2, 0.0, 0.8, 0.80, 'beta', true, 'B'),
                (2.0, 0.7, 0.0, 1.4, 0.65, 'gamma', false, 'C'),
                (3.0, 1.6, 0.0, 2.0, 0.90, 'alpha', true, 'D')
            ) AS t(x, y, base, x2, opacity, group_name, defined, label)",
        )
        .await?;

    let plot = Chart::<Cartesian>::new()
        .title("Cartesian scene mark coverage")
        .data(df)
        .mark(
            Area::new()
                .x(col("x"))
                .y(col("y"))
                .y2(col("base"))
                .fill("#93c5fd")
                .stroke("#2563eb")
                .opacity(0.35)
                .defined(ChannelValue::from(col("defined")).no_scale())
                .order(ChannelValue::from(col("x")).no_scale()),
        )
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#1d4ed8")
                .stroke_width(2.0)
                .defined(ChannelValue::from(col("defined")).no_scale())
                .order(ChannelValue::from(col("x")).no_scale()),
        )
        .mark(
            Trail::new()
                .x(col("x"))
                .y(col("y") + lit(0.45))
                .size_with(col("x") * lit(3.0) + lit(4.0), |c| c.no_scale())
                .stroke("#f97316")
                .opacity(0.55)
                .order(ChannelValue::from(col("x")).no_scale()),
        )
        .mark(
            Rect::new()
                .x(col("x") - lit(0.14))
                .x2(col("x") + lit(0.14))
                .y(lit(0.0))
                .y2(col("y") * lit(0.45))
                .fill("#fde68a")
                .stroke("#92400e")
                .corner_radius(4.0)
                .opacity(0.65),
        )
        .mark(
            Rule::new()
                .x(0.0)
                .x2(3.0)
                .y(1.0)
                .y2(1.0)
                .stroke("#374151")
                .stroke_dash("dashed")
                .opacity(0.45),
        )
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(90.0)
                .fill_with(col("group_name"), |c| c.legend(|l| l.visible(false)))
                .stroke("#111827")
                .stroke_width(1.0),
        )
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y") + lit(0.2))
                .text(col("label"))
                .align("center")
                .baseline("bottom")
                .font_size(11.0)
                .color("#111827"),
        )
        .mark(
            Image::new()
                .x(2.7)
                .y(1.9)
                .image(TINY_PNG_DATA_URI)
                .width(28.0)
                .height(22.0)
                .align("center")
                .baseline("middle")
                .smooth(false),
        )
        .mark(
            PathMark::new()
                .x(0.35)
                .y(1.85)
                .path("M 0 -14 L 12 10 L -12 10 Z")
                .path_transform("rotate(-20)")
                .fill("#22c55e")
                .stroke("#14532d")
                .stroke_width(1.4)
                .opacity(0.8),
        );

    let compiled = plot.compile(&ctx).await?;
    let evaluated = compiled.evaluate(&ctx, None).await?;
    println!(
        "scene: {} x {}, root marks={}",
        evaluated.scene_graph.width,
        evaluated.scene_graph.height,
        evaluated.scene_graph.marks.len()
    );

    Ok(())
}
