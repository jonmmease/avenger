use std::sync::Arc;

use avenger_chart::pixel_frame::{
    PixelFrameRectPositionChannels, PixelFrameRulePositionChannels,
    PixelFrameSymbolPositionChannels, PixelFrameTextPositionChannels,
};
use avenger_chart::prelude::*;
use avenger_scenegraph::marks::{
    mark::SceneMark, rect::SceneRectMark, rule::SceneRuleMark, symbol::SceneSymbolMark,
    text::SceneTextMark,
};
use datafusion::{
    arrow::{
        array::{BooleanArray, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema},
    },
    logical_expr::{col, lit},
    prelude::SessionContext,
};

fn pixel_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("x_alt", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("flag", DataType::Boolean, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("size", DataType::Float32, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float32Array::from(vec![10.0, 40.0])),
            Arc::new(Float32Array::from(vec![20.0, 50.0])),
            Arc::new(Float32Array::from(vec![12.0, 30.0])),
            Arc::new(BooleanArray::from(vec![true, false])),
            Arc::new(StringArray::from(vec!["a", "b"])),
            Arc::new(Float32Array::from(vec![4.0, 16.0])),
            Arc::new(StringArray::from(vec!["A", "B"])),
        ],
    )
    .unwrap();
    ctx.read_batch(batch).unwrap()
}

fn visit_marks<'a>(marks: &'a [SceneMark], output: &mut Vec<&'a SceneMark>) {
    for mark in marks {
        output.push(mark);
        if let SceneMark::Group(group) = mark {
            visit_marks(&group.marks, output);
        }
    }
}

#[tokio::test]
async fn pixel_frame_marks_use_raw_positions_and_scaled_visual_channels() {
    let ctx = SessionContext::new();
    let chart = Chart::<PixelFrame>::new()
        .data(pixel_data(&ctx))
        .plot_size(80.0, 50.0)
        .mark(
            Rect::<PixelFrame>::new()
                .x(col("x"))
                .x2(col("x") + lit(6.0_f32))
                .y(col("y"))
                .y2(col("y") + lit(5.0_f32))
                .fill("#0072B2"),
        )
        .mark(
            Rule::<PixelFrame>::new()
                .x(col("x"))
                .x2(col("x") + lit(8.0_f32))
                .y(col("y"))
                .y2(col("y") + lit(3.0_f32)),
        )
        .mark(
            Symbol::<PixelFrame>::new()
                .x_with(col("x"), |position| {
                    position.when_scaled(col("flag"), col("x_alt"))
                })
                .y(col("y"))
                .fill(col("category"))
                .size(col("size")),
        )
        .mark(
            Text::<PixelFrame>::new()
                .x(col("x"))
                .y(col("y"))
                .text(col("label")),
        );

    let compiled = chart.compile(&ctx).await.unwrap();
    let bytes = bincode::serialize(&compiled).unwrap();
    let decoded: avenger_chart::plot::CompiledPlot = bincode::deserialize(&bytes).unwrap();
    let direct = compiled.evaluate(&ctx, None).await.unwrap();
    let round_tripped = decoded.evaluate(&ctx, None).await.unwrap();
    assert_eq!(
        bincode::serialize(&direct.scene_graph).unwrap(),
        bincode::serialize(&round_tripped.scene_graph).unwrap()
    );

    let mut marks = Vec::new();
    visit_marks(&direct.scene_graph.marks, &mut marks);
    let rect: &SceneRectMark = marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Rect(rect) if rect.len == 2 => Some(rect),
            _ => None,
        })
        .unwrap();
    let rule: &SceneRuleMark = marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Rule(rule) if rule.len == 2 => Some(rule),
            _ => None,
        })
        .unwrap();
    let symbol: &SceneSymbolMark = marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Symbol(symbol) if symbol.len == 2 => Some(symbol),
            _ => None,
        })
        .unwrap();
    let text: &SceneTextMark = marks
        .iter()
        .find_map(|mark| match mark {
            SceneMark::Text(text) if text.len == 2 => Some(text.as_ref()),
            _ => None,
        })
        .unwrap();

    assert_eq!(rect.x.as_vec(2, None), vec![10.0, 40.0]);
    assert_eq!(rect.x2.as_ref().unwrap().as_vec(2, None), vec![16.0, 46.0]);
    assert_eq!(rule.x.as_vec(2, None), vec![10.0, 40.0]);
    assert_eq!(rule.x2.as_vec(2, None), vec![18.0, 48.0]);
    assert_eq!(symbol.x.as_vec(2, None), vec![20.0, 40.0]);
    assert_eq!(symbol.y.as_vec(2, None), vec![12.0, 30.0]);
    assert_eq!(text.x.as_vec(2, None), vec![10.0, 40.0]);
    assert_eq!(text.y.as_vec(2, None), vec![12.0, 30.0]);

    let fills = symbol.fill.as_vec(2, None);
    let sizes = symbol.size.as_vec(2, None);
    assert_ne!(fills[0], fills[1], "fill must retain its ordinal scale");
    assert_ne!(sizes[0], sizes[1], "size must retain its numeric scale");
    assert_ne!(sizes, vec![4.0, 16.0], "size values should be scaled");
}
