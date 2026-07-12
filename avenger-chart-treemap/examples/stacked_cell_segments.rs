use std::sync::Arc;

use arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::plot::Chart;
use avenger_chart_core::ChannelValue;
use avenger_chart_treemap::{TreeRect, Treemap};
use datafusion::{functions_aggregate::expr_fn::sum, logical_expr::col, prelude::SessionContext};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let data = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
            Field::new("v0", DataType::Float64, false),
            Field::new("v1", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "North America",
                "North America",
                "North America",
                "International",
                "International",
            ])),
            Arc::new(StringArray::from(vec![
                "Platform", "Platform", "Services", "Platform", "Services",
            ])),
            Arc::new(StringArray::from(vec![
                "Enterprise",
                "Consumer",
                "Enterprise",
                "Enterprise",
                "Consumer",
            ])),
            Arc::new(Float64Array::from(vec![30.0, 12.0, 28.0, 35.0, 25.0])),
            Arc::new(Float64Array::from(vec![0.0, 0.7142857, 0.0, 0.0, 0.0])),
            Arc::new(Float64Array::from(vec![0.7142857, 1.0, 1.0, 1.0, 1.0])),
        ],
    )?;
    let df = ctx.read_batch(data)?;

    let evaluated = Chart::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales"))),
    )
    .data(df)
    .plot_size(640.0, 360.0)
    .mark(
        TreeRect::new()
            .id("segments")
            .fill(col("segment"))
            .v(ChannelValue::from(col("v0")).no_scale())
            .v2(ChannelValue::from(col("v1")).no_scale())
            .stroke("#ffffff"),
    )
    .compile(&ctx)
    .await?
    .evaluate(&ctx, None)
    .await?;

    println!(
        "stacked-cell treemap evaluated with {} top-level scene marks",
        evaluated.scene_graph.marks.len()
    );
    Ok(())
}
