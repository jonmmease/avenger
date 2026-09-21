//! Manually aggregate a table, bind the result, and export two parameter snapshots.
use avenger_chart::{Chart, ChartOptions, RenderOptions};
use avenger_chart_definition::{
    dataflow::{
        arrow::{
            array::{Float64Array, StringArray},
            record_batch::RecordBatch,
        },
        datafusion::{
            common::ScalarValue,
            functions_aggregate::expr_fn::sum,
            logical_expr::{col, LogicalPlanBuilder},
        },
        *,
    },
    *,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let batch = RecordBatch::try_from_iter(vec![
        (
            "category",
            Arc::new(StringArray::from(vec!["A", "A", "B", "B", "C", "C"])) as _,
        ),
        (
            "amount",
            Arc::new(Float64Array::from(vec![12., 18., 25., 15., 10., 12.])) as _,
        ),
    ])?;
    let mut flow = DataflowBuilder::new();
    let source = flow.table_snapshot(
        "sales",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let factor = flow.scalar_input("factor", dataflow::arrow::datatypes::DataType::Float64)?;
    let totals = flow.add_plan(
        "totals",
        LogicalPlanBuilder::from(source.plan_ref())
            .aggregate(
                vec![col("category")],
                vec![sum(col("amount") * factor.expr_ref()).alias("total")],
            )?
            .sort(vec![col("category").sort(true, false)])?
            .build()?,
    )?;
    let bars = flow.table_output("bars", &totals)?;
    let mut definition = ChartDefinition::builder(flow.finish()?);
    definition.title("Sales by category");
    definition.parameter("factor", &factor, ScalarValue::Float64(Some(1.)))?;
    definition.plot("sales", |plot| {
        plot.content_size(400., 240.);
        let x = plot.scale(
            "x",
            Scale::band(Domain::column(&bars, "category"), Range::PlotWidth).padding_inner(0.15),
        )?;
        let y = plot.scale(
            "y",
            Scale::linear(Domain::numeric(0., 100.), Range::PlotHeightReversed),
        )?;
        plot.rect(
            "bars",
            &bars,
            RectEncoding::new()
                .x(x.field("category"))
                .width(x.bandwidth())
                .y(y.field("total"))
                .y2(y.constant(0.)),
        )?;
        plot.axis(Axis::bottom(&x).title("Category"))?;
        plot.axis(Axis::left(&y).title("Sales"))?;
        Ok(())
    })?;
    let definition = definition.finish()?;
    let bytes = definition.to_bytes()?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let decoded = ChartDefinition::from_bytes(&bytes, &runtime)?;
    let chart = Chart::prepare(
        decoded,
        ChartOptions {
            dataflow: Some(runtime),
            text_engine: Some(d3_text_engine()),
        },
    )
    .await?;
    let first = chart.render(RenderOptions::default()).await?;
    std::fs::write("chart-bars.svg", first.to_svg()?)?;
    std::fs::write("chart-bars.pdf", first.to_pdf()?)?;
    std::fs::write("chart-bars.png", first.to_png(2.).await?)?;
    let second = chart
        .render(RenderOptions::default().parameter("factor", ScalarValue::Float64(Some(2.))))
        .await?;
    std::fs::write("chart-bars-doubled.svg", second.to_svg()?)?;
    println!(
        "Wrote chart-bars.svg, .pdf, .png, and chart-bars-doubled.svg.\n{:?}",
        second.report()
    );
    Ok(())
}

fn d3_text_engine() -> avenger_text::TextEngine {
    let mut registry = avenger_text::NumberFormatRegistry::default();
    registry.register(
        "d3",
        std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider),
    );
    avenger_text::default_text_engine().with_number_formatting(
        avenger_text::NumberFormatConfig::new("d3"),
        std::sync::Arc::new(registry),
    )
}
