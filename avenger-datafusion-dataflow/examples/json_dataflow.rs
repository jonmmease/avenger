//! Run with --features json. Load once, submit independent requests, fetch materialized panels.
use avenger_datafusion_dataflow::{
    json::{AssetBindings, DataflowSpec, FileSourceResolver, QueryRequest},
    Runtime, RuntimeConfig,
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec: DataflowSpec =
        serde_json::from_str(include_str!("../tests/fixtures/facets.dataflow.json"))?;
    let mut request: QueryRequest =
        serde_json::from_str(include_str!("../tests/fixtures/facets.query.json"))?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let flow = runtime
        .load_spec(&spec, &FileSourceResolver::new("."))
        .await?;
    let root = flow.interface().root();
    let regions = root.scope("regions")?;
    let years = regions.scope("years")?;
    let prepared = runtime.prepare(&flow).await?;
    for minimum in [80, 150] {
        request
            .bindings
            .scalars
            .insert("minimum".into(), minimum.into());
        let result = prepared
            .query_request(&request, &AssetBindings::new())
            .await?;
        println!(
            "minimum={minimum}, total={}, hits={}, plans={}",
            result.scalar(&root.scalar_output("total")?)?,
            result.report().cache_hits,
            result.report().physical_plans
        );
        for (region, panel) in result.scope(regions.handle().unwrap())?.iter() {
            for (year, panel) in panel.scope(years.handle().unwrap())?.iter() {
                println!(
                    "  {:?}/{:?}: threshold={}, rows={}",
                    region.values(),
                    year.values(),
                    panel.scalar(&years.scalar_output("threshold")?)?,
                    panel.table(&years.table_output("marks")?)?.num_rows()
                );
            }
        }
    }
    let spec: DataflowSpec =
        serde_json::from_str(include_str!("../tests/fixtures/expressions.dataflow.json"))?;
    let request: QueryRequest =
        serde_json::from_str(include_str!("../tests/fixtures/expressions.query.json"))?;
    let flow = runtime
        .load_spec(&spec, &FileSourceResolver::new("."))
        .await?;
    let flow = runtime.decode_dataflow(&flow.to_bytes()?)?;
    let root = flow.interface().root();
    for input in root.inputs() {
        println!("expression example input: {}", input.name());
    }
    let prepared = runtime.prepare(&flow).await?;
    let result = prepared
        .query_request(&request, &AssetBindings::new())
        .await?;
    let rows = result.table(&root.table_output("rows")?)?;
    assert_eq!(rows.num_rows(), 2);
    println!(
        "JSON expressions after protobuf round trip:\n{}",
        avenger_datafusion_dataflow::arrow::util::pretty::pretty_format_batches(rows.batches())?
    );
    Ok(())
}
