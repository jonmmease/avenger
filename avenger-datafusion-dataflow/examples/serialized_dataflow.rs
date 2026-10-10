//! Native definition -> protobuf + Arrow IPC -> independent native runtime.
use avenger_datafusion_dataflow::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    datafusion::logical_expr::{col, JoinType, LogicalPlanBuilder},
    DataflowBuilder, Result, Runtime, RuntimeConfig, TableSnapshot,
};
use std::sync::Arc;
fn table(values: &[i64]) -> Result<TableSnapshot> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "amount",
        DataType::Int64,
        false,
    )]));
    TableSnapshot::from_batches(
        schema.clone(),
        vec![RecordBatch::try_new(
            schema,
            vec![Arc::new(Int64Array::from(values.to_vec()))],
        )?],
    )
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let fixed = b.table_snapshot("sales", table(&[10, 20, 30])?)?;
    let selected = b.table_input("selected", fixed.schema().as_arrow().clone().into())?;
    let cutoff = b.scalar_input("cutoff", DataType::Int64)?;
    let rows = b.add_plan(
        "filtered",
        LogicalPlanBuilder::from(fixed.plan_ref())
            .join(
                selected.plan_ref(),
                JoinType::LeftSemi,
                (vec!["amount"], vec!["amount"]),
                None,
            )?
            .filter(col("amount").gt(cutoff.expr_ref()))?
            .build()?,
    )?;
    b.table_output("marks", &rows)?;
    let bytes = b.finish()?.to_bytes()?;
    println!("Artifact: {} bytes", bytes.len());
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let definition = runtime.decode_dataflow(&bytes)?;
    let names = definition.interface().root();
    let prepared = runtime.prepare(&definition).await?;
    let selected = table(&[20, 30])?;
    for cutoff in [0_i64, 25, 0] {
        let inputs = prepared
            .inputs()
            .scalar(&names.scalar_input("cutoff")?, cutoff.into())?
            .table(&names.table_input("selected")?, selected.clone())?
            .finish()?;
        let out = names.table_output("marks")?;
        let result = prepared.query(&[out], &[], &inputs).await?;
        println!(
            "cutoff={cutoff}, hits={}\n{}",
            result.report().cache_hits,
            pretty_format_batches(result.table(&out)?.batches())?
        );
    }
    Ok(())
}
