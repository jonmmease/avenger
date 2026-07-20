use std::sync::Arc;

use avenger_chart::plot::Chart;
use avenger_chart_cartesian::Cartesian;
use avenger_chart_external_test::external_compound_mark::ExternalMeanPoint;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{col, SessionContext},
};

fn grouped_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["A", "A", "B", "B"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![1.0, 3.0, 4.0, 6.0])) as ArrayRef,
        ],
    )
    .expect("batch");
    ctx.read_batch(batch).expect("dataframe")
}

fn collect_symbol_and_category_tick_x(
    marks: &[SceneMark],
    origin_x: f32,
    symbol_x: &mut Vec<f32>,
    tick_x: &mut Vec<f32>,
) {
    for mark in marks {
        match mark {
            SceneMark::Symbol(symbol) => {
                symbol_x.extend(symbol.x_iter().map(|x| origin_x + x));
            }
            SceneMark::Text(text) => {
                tick_x.extend(
                    text.text_iter()
                        .zip(text.x_iter())
                        .filter(|(label, _)| label.as_str() == "A" || label.as_str() == "B")
                        .map(|(_, x)| origin_x + *x),
                );
            }
            SceneMark::Group(group) => collect_symbol_and_category_tick_x(
                &group.marks,
                origin_x + group.origin[0],
                symbol_x,
                tick_x,
            ),
            _ => {}
        }
    }
}

#[tokio::test]
async fn external_compound_mark_compiles_and_supplies_scale_hint() {
    let ctx = SessionContext::new();
    let data = grouped_data(&ctx);
    let compiled = Chart::<Cartesian>::new()
        .data(data.clone())
        .mark(ExternalMeanPoint::new(col("category"), col("value")))
        .compile(&ctx)
        .await
        .expect("compile external compound mark");

    let scales = compiled
        .build_scales_for_dataframe(&data, 300.0, 200.0, &ctx, compiled.get_default_params())
        .await
        .expect("build scales");
    let x_scale = scales.get("x").expect("x scale");
    assert_eq!(x_scale.configured().scale_impl.scale_type(), "band");

    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate external compound mark");
    let mut symbol_x = Vec::new();
    let mut tick_x = Vec::new();
    collect_symbol_and_category_tick_x(
        &evaluated.scene_graph.marks,
        evaluated.scene_graph.origin[0],
        &mut symbol_x,
        &mut tick_x,
    );
    symbol_x.sort_by(f32::total_cmp);
    tick_x.sort_by(f32::total_cmp);
    assert_eq!(symbol_x.len(), 2);
    assert_eq!(tick_x.len(), 2);
    for (symbol, tick) in symbol_x.iter().zip(tick_x) {
        assert!(
            (*symbol - tick).abs() < 0.01,
            "symbol x={symbol} must align with category tick x={tick}"
        );
    }
}
