use avenger_chart::{Chart, RenderOptions};
use avenger_datafusion_dataflow::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        record_batch::RecordBatch,
    },
    datafusion::prelude::SessionContext,
    Runtime, RuntimeConfig, TableSnapshot, TableStore,
};
use avenger_vegalite_compiler::{compile_vegalite_with_input, spec::UnitSpec, ChartDefinition};
use std::sync::Arc;

fn batch(categories: Vec<&str>, amounts: Vec<f64>) -> anyhow::Result<RecordBatch> {
    Ok(RecordBatch::try_from_iter([
        (
            "category",
            Arc::new(StringArray::from(categories)) as ArrayRef,
        ),
        ("amount", Arc::new(Float64Array::from(amounts)) as ArrayRef),
    ])?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let spec = UnitSpec::from_json(
        r#"{
        "data": {"name": "sales"},
        "mark": "bar",
        "encoding": {
            "x": {"field": "category", "type": "nominal", "sort": null},
            "y": {"field": "amount", "aggregate": "sum", "type": "quantitative"}
        }
    }"#,
    )?;
    let initial = batch(vec!["B", "A"], vec![2., 4.])?;
    let store = TableStore::new(TableSnapshot::from_batches(
        initial.schema(),
        vec![initial],
    )?);
    let first_snapshot = store.snapshot();
    let definition = compile_vegalite_with_input(&spec, first_snapshot.schema().clone())?;
    let codec = Arc::new(avenger_transform::TransformExtensionCodec::default());
    let bytes = definition.to_bytes_with_codec(codec.clone())?;
    let runtime = Runtime::with_session_state_and_codec(
        SessionContext::new().state(),
        RuntimeConfig {
            function_versions: avenger_transform::function_versions(),
            ..Default::default()
        },
        codec,
    )?;
    let restored = ChartDefinition::from_bytes(&bytes, &runtime)?;
    let sales = restored
        .dataflow()
        .interface()
        .root()
        .table_input("sales")?;
    let chart = Chart::prepare(restored, Default::default()).await?;
    let first_inputs = chart
        .inputs()?
        .table(&sales, first_snapshot.clone())?
        .finish()?;
    let first = chart
        .render(RenderOptions::default().inputs(first_inputs.clone()))
        .await?;

    let second_snapshot = store.append_batch(batch(vec!["A", "C"], vec![3., 5.])?)?;
    let second_inputs = first_inputs
        .edit()
        .table(&sales, second_snapshot.clone())?
        .finish()?;
    let second = chart
        .render(RenderOptions::default().inputs(second_inputs))
        .await?;
    assert_eq!(first.inputs().table_value(&sales)?.num_rows(), 2);
    assert_eq!(second.inputs().table_value(&sales)?.num_rows(), 4);
    println!(
        "Restored a {}-byte definition; rendered {} then {} source rows",
        bytes.len(),
        first_snapshot.num_rows(),
        second_snapshot.num_rows()
    );

    #[cfg(feature = "svg")]
    {
        let directory = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/streaming-bars".into());
        std::fs::create_dir_all(&directory)?;
        std::fs::write(format!("{directory}/first.svg"), first.to_svg()?)?;
        std::fs::write(format!("{directory}/second.svg"), second.to_svg()?)?;
        println!("Wrote {directory}/first.svg and {directory}/second.svg");
    }
    Ok(())
}
