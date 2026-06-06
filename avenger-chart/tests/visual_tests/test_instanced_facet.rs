use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn instanced_facet_data(ctx: &SessionContext) -> DataFrame {
    let mut groups = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();

    for group_index in 0..6 {
        let group = format!("Group {}", group_index + 1);
        let x_offset = (group_index % 3) as f64 * 0.35;
        let y_offset = (group_index / 3) as f64 * 0.45;
        for point_index in 0..120 {
            let t = point_index as f64;
            groups.push(group.clone());
            xs.push(((t * 0.618_033_988_75 + x_offset).fract()) * 10.0);
            ys.push(((t * 0.414_213_562_37 + y_offset).fract()) * 10.0);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("group_name", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(groups)),
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
        ],
    )
    .expect("instanced facet test data");

    ctx.read_batch(batch).expect("read instanced facet data")
}

#[tokio::test]
async fn facet_wrap_instanced_symbols_interleaved_with_chrome() {
    let ctx = SessionContext::new();
    let leaf = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .share_domain()
                    .axis(|a| a.title("x").grid(true))
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .share_domain()
                    .axis(|a| a.title("y").grid(true))
            })
            .fill_with(col("group_name"), |c| c.legend(|l| l.title("Group")))
            .stroke("#ffffff")
            .stroke_width(0.4)
            .opacity(0.82)
            .size(28.0),
    );

    let plot = Plot::<FacetWrap>::new()
        .canvas_size(980.0, 640.0)
        .data(instanced_facet_data(&ctx))
        .mark(Subplot::new(leaf).wrap_with(col("group_name"), |c| {
            c.columns(3).guide(|g| g.title("Instanced facet"))
        }));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "instanced_facet",
        "facet_wrap_instanced_symbols_interleaved_with_chrome",
    )
    .await;
}
