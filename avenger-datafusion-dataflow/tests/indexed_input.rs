use avenger_datafusion_dataflow::{
    arrow::{
        array::UInt64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datafusion::{
        logical_expr::{col, lit, LogicalPlanBuilder},
        prelude::{SessionConfig, SessionContext},
    },
    DataflowBuilder, Result, Runtime, RuntimeConfig, TableSnapshot, TableStore,
};
use std::sync::Arc;

fn batch(values: Vec<u64>) -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![Field::new("x", DataType::UInt64, false)])),
        vec![Arc::new(UInt64Array::from(values))],
    )
    .unwrap()
}

fn values(table: &TableSnapshot, name: &str) -> Vec<u64> {
    table
        .batch_iter()
        .flat_map(|b| {
            b.column_by_name(name)
                .unwrap()
                .as_any()
                .downcast_ref::<UInt64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}

#[tokio::test]
async fn indexed_inputs_preserve_order_across_appends_filters_and_serialization() -> Result<()> {
    let first = batch((0..20_000).rev().collect());
    let store = TableStore::new(TableSnapshot::from_batches(first.schema(), vec![first])?);
    let original = store.snapshot();
    let mut b = DataflowBuilder::new();
    let input = b.table_input("source", original.schema().clone())?;
    assert!(input.plan_ref_with_row_index("x").is_err());
    assert!(input.plan_ref_with_row_index("").is_err());
    let rows = b.add_plan(
        "indexed",
        LogicalPlanBuilder::from(input.plan_ref_with_row_index("ordinal")?)
            .filter(col("x").lt(lit(5u64)))?
            .sort(vec![col("ordinal").sort(true, true)])?
            .build()?,
    )?;
    b.table_output("rows", &rows)?;
    let plain = b.add_plan("plain", input.plan_ref())?;
    b.table_output("plain", &plain)?;
    let graph = b.finish()?;
    for partitions in [1, 4] {
        let runtime = Runtime::with_session_state(
            SessionContext::new_with_config(
                SessionConfig::new()
                    .with_target_partitions(partitions)
                    .with_batch_size(128),
            )
            .state(),
            RuntimeConfig::default(),
        )?;
        let decoded = runtime.decode_dataflow(&graph.to_bytes()?)?;
        let root = decoded.interface().root();
        let input = root.table_input("source")?;
        let output = root.table_output("rows")?;
        let plain = root.table_output("plain")?;
        let flow = runtime.prepare(&decoded).await?;
        let old_inputs = flow.inputs().table(&input, original.clone())?.finish()?;
        let old = flow.query(&[output, plain], &[], &old_inputs).await?;
        assert_eq!(values(old.table(&output)?, "x"), vec![4, 3, 2, 1, 0]);
        assert_eq!(
            values(old.table(&output)?, "ordinal"),
            vec![19995, 19996, 19997, 19998, 19999]
        );
        assert_eq!(old.table(&plain)?.schema(), original.schema());
        let appended = store.append_batches(vec![batch(vec![]), batch(vec![3, 1])])?;
        let new_inputs = old_inputs.edit().table(&input, appended)?.finish()?;
        let new = flow.query(&[output], &[], &new_inputs).await?;
        let indices = values(new.table(&output)?, "ordinal");
        assert_eq!(
            &indices[indices.len() - 2..],
            &[
                store.snapshot().num_rows() as u64 - 2,
                store.snapshot().num_rows() as u64 - 1
            ]
        );
        let again = flow.query(&[output], &[], &old_inputs).await?;
        assert_eq!(
            values(again.table(&output)?, "ordinal"),
            values(old.table(&output)?, "ordinal")
        );
        let empty = flow
            .inputs()
            .table(&input, TableSnapshot::empty(original.schema().clone()))?
            .finish()?;
        assert_eq!(
            flow.query(&[output], &[], &empty)
                .await?
                .table(&output)?
                .num_rows(),
            0
        );
    }
    Ok(())
}

#[tokio::test]
async fn legacy_artifacts_still_decode() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let rows = b.table_snapshot(
        "source",
        TableSnapshot::from_batches(batch(vec![1]).schema(), vec![batch(vec![1])])?,
    )?;
    b.table_output("rows", &rows)?;
    let mut artifact = b.finish()?.to_proto()?;
    artifact.version = 1;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let graph = runtime.decode_dataflow_proto(artifact)?;
    let flow = runtime.prepare(&graph).await?;
    let output = flow.interface().root().table_output("rows")?;
    let result = flow.query(&[output], &[], &flow.inputs().finish()?).await?;
    assert_eq!(values(result.table(&output)?, "x"), vec![1]);
    Ok(())
}

#[test]
fn decoding_rejects_inconsistent_indexed_read_schemas() -> Result<()> {
    use avenger_datafusion_dataflow::protobuf::{
        self as wire, datafusion::logical_plan_node::LogicalPlanType,
    };
    use prost::Message;
    let mut builder = DataflowBuilder::new();
    let input = builder.table_input("source", batch(vec![1]).schema())?;
    let rows = builder.add_plan("indexed", input.plan_ref_with_row_index("ordinal")?)?;
    builder.table_output("rows", &rows)?;
    let original = builder.finish()?.to_proto()?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    for index in [None, Some("x"), Some("other"), Some("")] {
        let mut artifact = original.clone();
        let Some(wire::node::Program::Plan(plan)) = &mut artifact.nodes[0].program else {
            panic!()
        };
        let Some(LogicalPlanType::Extension(extension)) = &mut plan.logical_plan_type else {
            panic!()
        };
        let mut read = wire::Read::decode(extension.node.as_slice()).unwrap();
        read.row_index = index.map(str::to_owned);
        extension.node = read.encode_to_vec();
        assert!(runtime.decode_dataflow_proto(artifact).is_err());
    }
    Ok(())
}
