use std::sync::Arc;

use arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::plot::Chart;
use avenger_chart_core::ChannelValue;
use avenger_chart_treemap::{TreeRect, Treemap};
use datafusion::{
    functions_aggregate::expr_fn::sum,
    logical_expr::{col, lit},
    prelude::SessionContext,
};

fn sales_data() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
            Field::new("highlight", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "North America",
                "North America",
                "International",
                "International",
            ])),
            Arc::new(StringArray::from(vec![
                "Platform", "Services", "Platform", "Services",
            ])),
            Arc::new(Float64Array::from(vec![42.0, 28.0, 35.0, 25.0])),
            Arc::new(StringArray::from(vec![
                "#d62728", "#d62728", "#d62728", "#d62728",
            ])),
        ],
    )
    .expect("sales data")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let base = ctx.read_batch(sales_data())?;
    let selected = ctx
        .read_batch(sales_data())?
        .filter(col("division").eq(lit("International")))?;

    let evaluated = Chart::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales"))),
    )
    .data(base)
    .plot_size(640.0, 360.0)
    .mark(TreeRect::new().id("base").fill("#d8dde3").stroke("#ffffff"))
    .mark(
        TreeRect::new()
            .id("selected")
            .data(selected)
            .fill(ChannelValue::from(col("highlight")).no_scale())
            .opacity(0.85)
            .stroke("#ffffff"),
    )
    .compile(&ctx)
    .await?
    .evaluate(&ctx, None)
    .await?;

    println!(
        "overlay treemap evaluated with {} top-level scene marks",
        evaluated.scene_graph.marks.len()
    );
    Ok(())
}
