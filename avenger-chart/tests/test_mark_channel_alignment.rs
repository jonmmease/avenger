use avenger_chart::prelude::*;
use avenger_common::{
    types::ColorOrGradient,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scenegraph::marks::{
    mark::SceneMark, rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::SessionContext,
};
use std::sync::Arc;

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

fn collect_rules<'a>(mark: &'a SceneMark, rules: &mut Vec<&'a SceneRuleMark>) {
    match mark {
        SceneMark::Rule(rule) => rules.push(rule),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_rules(mark, rules);
            }
        }
        _ => {}
    }
}

fn collect_text<'a>(mark: &'a SceneMark, text: &mut Vec<&'a SceneTextMark>) {
    match mark {
        SceneMark::Text(text_mark) => text.push(text_mark),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_text(mark, text);
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
async fn rule_and_text_channel_descriptors_expose_scene_mark_channels() {
    let ctx = SessionContext::new();

    let rule_plot = Plot::<Cartesian>::new().mark(
        Rule::new()
            .x(col("x"))
            .y(col("y"))
            .x2(col("x2"))
            .y2(col("y2"))
            .stroke_cap(col("cap"))
            .opacity(col("opacity")),
    );
    let rule = rule_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let rule_channels = rule.supported_channels();
    assert!(channel(&rule_channels, "stroke_cap").allow_column_ref);
    assert!(channel(&rule_channels, "opacity").allow_column_ref);

    let text_plot = Plot::<Cartesian>::new().mark(
        Text::new()
            .x(col("x"))
            .y(col("y"))
            .text(col("label"))
            .align(col("align"))
            .baseline(col("baseline")),
    );
    let text = text_plot.compile(&ctx).await.unwrap().marks()[0].clone();
    let text_channels = text.supported_channels();
    assert!(channel(&text_channels, "text").allow_column_ref);
    assert!(channel(&text_channels, "font_size").allow_column_ref);
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

#[tokio::test]
async fn rule_opacity_is_folded_into_stroke_alpha() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new().mark(
        Rule::new()
            .x(0.0)
            .y(0.0)
            .x2(1.0)
            .y2(1.0)
            .stroke("#ff0000")
            .opacity(0.25),
    );

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut rules = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_rules(mark, &mut rules);
    }

    let rule = rules.first().expect("rule mark");
    assert_eq!(alpha(&rule.stroke), 0.25);
}

#[tokio::test]
async fn text_column_values_render_as_strings_without_scale() {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("label", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![0.0, 1.0])),
            Arc::new(Float64Array::from(vec![0.0, 1.0])),
            Arc::new(StringArray::from(vec!["left", "right"])),
        ],
    )
    .unwrap();
    let df = ctx.read_batch(batch).unwrap();
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(Text::new().x(col("x")).y(col("y")).text(col("label")));

    let evaluated = plot
        .compile(&ctx)
        .await
        .unwrap()
        .evaluate(&ctx, None)
        .await
        .unwrap();

    let mut text_marks = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_text(mark, &mut text_marks);
    }

    let text = text_marks.first().expect("text mark");
    assert_eq!(
        text.text.as_vec(2, None),
        vec!["left".to_string(), "right".to_string()]
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
