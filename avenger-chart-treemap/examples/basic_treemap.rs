use std::sync::Arc;

use arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::plot::Plot;
use avenger_chart_treemap::{TreeRect, Treemap};
use datafusion::{functions_aggregate::expr_fn::sum, logical_expr::col, prelude::SessionContext};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let data = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
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
        ],
    )?;
    let df = ctx.read_batch(data)?;

    let evaluated = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "product"])
            .value(sum(col("sales"))),
    )
    .data(df)
    .plot_size(640.0, 360.0)
    .mark(TreeRect::new().fill(col("division")).stroke("#ffffff"))
    .compile(&ctx)
    .await?
    .evaluate(&ctx, None)
    .await?;

    println!(
        "basic treemap evaluated with {} top-level scene marks",
        evaluated.scene_graph.marks.len()
    );
    Ok(())
}
