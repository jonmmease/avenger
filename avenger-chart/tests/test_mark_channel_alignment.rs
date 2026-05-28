use avenger_chart::prelude::*;
use avenger_common::{
    types::ColorOrGradient,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scenegraph::marks::{mark::SceneMark, symbol::SceneSymbolMark};
use datafusion::prelude::SessionContext;

fn channel<'a>(
    channels: &'a [avenger_chart_core::ChannelDescriptor],
    name: &str,
) -> &'a avenger_chart_core::ChannelDescriptor {
    channels
        .iter()
        .find(|channel| channel.name == name)
        .unwrap_or_else(|| panic!("missing channel descriptor {name}"))
}

fn collect_symbols<'a>(mark: &'a SceneMark, symbols: &mut Vec<&'a SceneSymbolMark>) {
    match mark {
        SceneMark::Symbol(symbol) => symbols.push(symbol),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_symbols(mark, symbols);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn cartesian_mark_channel_descriptors_align_with_existing_renderers() {
    let ctx = SessionContext::new();

    let symbol_plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .opacity(col("opacity"))
            .stroke_width(2.0),
    );
    let symbol = symbol_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let symbol_channels = symbol.supported_channels();
    assert!(!channel(&symbol_channels, "stroke_width").allow_column_ref);
    assert!(channel(&symbol_channels, "opacity").allow_column_ref);

    let line_plot =
        Plot::<Cartesian>::new().mark(Line::new().x(col("x")).y(col("y")).opacity(col("opacity")));
    let line = line_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let line_channels = line.supported_channels();
    assert!(channel(&line_channels, "opacity").allow_column_ref);
    assert!(
        line_channels
            .iter()
            .all(|channel| channel.name != "stroke_opacity")
    );

    let rect_plot = Plot::<Cartesian>::new().mark(
        Rect::new()
            .x(col("x"))
            .x2(col("x2"))
            .y(col("y"))
            .y2(col("y2"))
            .corner_radius(col("corner_radius")),
    );
    let rect = rect_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let rect_channels = rect.supported_channels();
    assert!(channel(&rect_channels, "corner_radius").allow_column_ref);
}

#[tokio::test]
async fn symbol_opacity_is_folded_into_fill_and_stroke_alpha() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(0.0)
            .y(0.0)
            .fill("#ff0000")
            .stroke("#0000ff")
            .opacity(0.25),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut symbols = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_symbols(mark, &mut symbols);
    }

    let symbol = symbols.first().expect("symbol mark");
    assert_eq!(
        alpha(&symbol.fill),
        0.25,
        "fill alpha should include opacity"
    );
    assert_eq!(
        alpha(&symbol.stroke),
        0.25,
        "stroke alpha should include opacity"
    );
}

fn alpha(colors: &ScalarOrArray<ColorOrGradient>) -> f32 {
    match colors.value() {
        ScalarOrArrayValue::Scalar(ColorOrGradient::Color(color)) => color[3],
        ScalarOrArrayValue::Array(colors) => match colors.first().expect("first color") {
            ColorOrGradient::Color(color) => color[3],
            ColorOrGradient::GradientIndex(_) => panic!("expected concrete color"),
        },
        ScalarOrArrayValue::Scalar(ColorOrGradient::GradientIndex(_)) => {
            panic!("expected concrete color")
        }
    }
}
