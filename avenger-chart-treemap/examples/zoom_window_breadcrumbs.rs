use std::sync::Arc;

use arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::plot::Plot;
use avenger_chart_treemap::{TreeRect, Treemap, TreemapGuide};
use datafusion::{
    functions_aggregate::expr_fn::sum,
    logical_expr::{col, lit},
    prelude::SessionContext,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let data = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("region", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec![
                "Enterprise",
                "Enterprise",
                "Enterprise",
                "Enterprise",
                "Consumer",
            ])),
            Arc::new(StringArray::from(vec![
                "North America",
                "North America",
                "International",
                "International",
                "North America",
            ])),
            Arc::new(StringArray::from(vec![
                "Platform", "Services", "Platform", "Services", "Retail",
            ])),
            Arc::new(StringArray::from(vec![
                "Core",
                "Support",
                "Core",
                "Support",
                "Storefront",
            ])),
            Arc::new(Float64Array::from(vec![42.0, 28.0, 35.0, 25.0, 30.0])),
        ],
    )?;
    let df = ctx.read_batch(data.clone())?;
    let mark_df = ctx
        .read_batch(data)?
        .filter(col("division").eq(lit("Enterprise")))?;

    let evaluated = Plot::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .root_path_id("division=Enterprise")
            .display_levels(2),
    )
    .data(df)
    .plot_size(640.0, 360.0)
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(true),
    )
    .mark(
        TreeRect::new()
            .data(mark_df)
            .fill(col("region"))
            .stroke("#ffffff"),
    )
    .compile(&ctx)
    .await?
    .evaluate(&ctx, None)
    .await?;

    println!(
        "zoom-window treemap evaluated with {} top-level scene marks",
        evaluated.scene_graph.marks.len()
    );
    Ok(())
}
