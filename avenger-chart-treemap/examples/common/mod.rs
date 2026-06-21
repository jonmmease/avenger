use std::sync::Arc;

use arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::plot::Plot;
use avenger_chart_treemap::Treemap;
use datafusion::prelude::SessionContext;

pub fn deep_data() -> RecordBatch {
    RecordBatch::try_new(
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
                "Enterprise",
                "Enterprise",
                "Consumer",
                "Consumer",
            ])),
            Arc::new(StringArray::from(vec![
                "North America",
                "North America",
                "North America",
                "International",
                "International",
                "International",
                "North America",
                "International",
            ])),
            Arc::new(StringArray::from(vec![
                "Platform",
                "Services",
                "Operations",
                "Platform",
                "Services",
                "Operations",
                "Retail",
                "Retail",
            ])),
            Arc::new(StringArray::from(vec![
                "Core",
                "Support",
                "Automation",
                "Core",
                "Support",
                "Automation",
                "Storefront",
                "Marketplace",
            ])),
            Arc::new(Float64Array::from(vec![
                42.0, 28.0, 18.0, 35.0, 25.0, 20.0, 30.0, 22.0,
            ])),
        ],
    )
    .expect("treemap example data")
}

pub async fn evaluate_and_print(
    ctx: &SessionContext,
    plot: Plot<Treemap>,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let evaluated = plot.compile(ctx).await?.evaluate(ctx, None).await?;
    println!(
        "{name} evaluated with {} top-level scene marks",
        evaluated.scene_graph.marks.len()
    );
    Ok(())
}
