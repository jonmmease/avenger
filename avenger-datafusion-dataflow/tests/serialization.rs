#[path = "common/scoped.rs"]
mod scoped;
use avenger_datafusion_dataflow::{
    datafusion::{
        common::ScalarValue,
        logical_expr::{col, lit, LogicalPlanBuilder},
    },
    DataflowBuilder, Result, Runtime, RuntimeConfig,
};

#[tokio::test]
async fn fixed_assets_and_scalar_subqueries_round_trip_to_native_dataflow() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let sales = b.table_snapshot("sales", scoped::sales())?;
    let min = b.scalar_input(
        "min",
        avenger_datafusion_dataflow::arrow::datatypes::DataType::Int64,
    )?;
    let table = b.add_plan(
        "filtered",
        LogicalPlanBuilder::from(sales.plan_ref())
            .filter(col("amount").gt(min.expr_ref()))?
            .build()?,
    )?;
    b.table_output("marks", &table)?;
    let scalar = b.add_scalar("answer", min.expr_ref() + lit(1_i64))?;
    b.scalar_output("answer", &scalar)?;
    let original = b.finish()?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let decoded = runtime.decode_dataflow(&original.to_bytes()?)?;
    assert_eq!(decoded.num_nodes(), original.num_nodes());
    let names = decoded.interface().root();
    let prepared = runtime.prepare(&decoded).await?;
    let inputs = prepared
        .inputs()
        .scalar(&names.scalar_input("min")?, 40_i64.into())?
        .finish()?;
    let out = names.table_output("marks")?;
    let scalar = names.scalar_output("answer")?;
    let result = prepared.query(&[out], &[scalar], &inputs).await?;
    assert_eq!(scoped::amounts(result.table(&out)?), vec![50, 80, 90]);
    assert_eq!(result.scalar(&scalar)?, &ScalarValue::Int64(Some(41)));
    assert!(prepared.inputs().scalar(&min, 1_i64.into()).is_err());
    assert!(runtime.decode_dataflow(b"invalid").is_err());
    Ok(())
}

#[tokio::test]
async fn nested_scopes_round_trip_with_rebound_handles() -> Result<()> {
    let mut b = DataflowBuilder::new();
    scoped::build(&mut b)?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let decoded = runtime.decode_dataflow(&b.finish()?.to_bytes()?)?;
    let root = decoded.interface().root();
    let regions = root.scope("regions")?;
    let years = regions.scope("years")?;
    let prepared = runtime.prepare(&decoded).await?;
    let inputs = prepared
        .inputs()
        .table(&root.table_input("sales")?, scoped::sales())?
        .scalar(&root.scalar_input("multiplier")?, 1_i64.into())?
        .scope_defaults(regions.handle().unwrap(), |b| {
            b.scalar(&regions.scalar_input("limit")?, 10_i64.into())
        })?
        .scope_defaults(years.handle().unwrap(), |b| {
            b.scalar(&years.scalar_input("fraction")?, 2_i64.into())?
                .table(
                    &years.table_input("selected")?,
                    scoped::products(&["A", "B"]),
                )
        })?
        .finish()?;
    let output = years.scalar_output("maximum")?;
    let result = prepared.query(&[], &[output], &inputs).await?;
    assert_eq!(result.scope(regions.handle().unwrap())?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn registered_volatile_udf_requires_registry_and_matching_version_at_decode() -> Result<()> {
    use avenger_datafusion_dataflow::{
        datafusion::{
            arrow::datatypes::DataType,
            execution::context::SessionContext,
            logical_expr::{create_udf, ColumnarValue, Volatility},
        },
        SemanticConfig,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let udf = create_udf(
        "draw",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |_| {
            Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(
                count.fetch_add(1, Ordering::SeqCst) as i64,
            ))))
        }),
    );
    let versions = std::collections::BTreeMap::from([("scalar:draw".into(), "1".into())]);
    let mut b = DataflowBuilder::with_semantics(SemanticConfig {
        function_versions: versions.clone(),
        ..SemanticConfig::default()
    });
    let draw = b.add_scalar("draw", udf.call(vec![]))?;
    b.scalar_output("draw", &draw)?;
    let graph = b.finish()?;
    let bytes = graph.to_bytes()?;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(Runtime::new(RuntimeConfig::default())?
        .decode_dataflow(&bytes)
        .is_err());
    let config = RuntimeConfig {
        function_versions: versions,
        ..RuntimeConfig::default()
    };
    assert!(Runtime::new(config.clone())?
        .decode_dataflow(&bytes)
        .is_err());
    let context = SessionContext::new();
    context.register_udf(udf);
    let runtime = Runtime::with_session_state(context.state(), config)?;
    let decoded = runtime.decode_dataflow(&bytes)?;
    let decoded = runtime.decode_dataflow(&decoded.to_bytes()?)?;
    let prepared = runtime.prepare(&decoded).await?;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let output = decoded.interface().root().scalar_output("draw")?;
    for expected in [0, 1] {
        let result = prepared
            .query(&[], &[output], &prepared.inputs().finish()?)
            .await?;
        assert_eq!(result.scalar(&output)?, &ScalarValue::Int64(Some(expected)));
        assert_eq!(result.report().cache_hits, 0);
    }
    assert_eq!(runtime.cache_stats().entries, 0);
    Ok(())
}

#[tokio::test]
async fn configured_function_codec_payloads_and_hook_errors_are_preserved() -> Result<()> {
    use avenger_datafusion_dataflow::{
        datafusion::{
            arrow::datatypes::{DataType, Schema},
            catalog::TableProvider,
            common::{DataFusionError, TableReference},
            execution::{context::SessionContext, TaskContext},
            logical_expr::{
                create_udf, ColumnarValue, Extension, LogicalPlan, ScalarUDF, Volatility,
            },
        },
        LogicalExtensionCodec, SemanticConfig,
    };
    use std::sync::Arc;
    fn function(value: i64) -> ScalarUDF {
        create_udf(
            "configured",
            vec![],
            DataType::Int64,
            Volatility::Immutable,
            Arc::new(move |_| Ok(ColumnarValue::Scalar(value.into()))),
        )
    }
    #[derive(Debug)]
    struct Codec {
        value: i64,
        fail: bool,
    }
    impl LogicalExtensionCodec for Codec {
        fn try_encode(&self, _: &Extension, _: &mut Vec<u8>) -> datafusion::common::Result<()> {
            unreachable!()
        }
        fn try_decode(
            &self,
            _: &[u8],
            _: &[LogicalPlan],
            _: &TaskContext,
        ) -> datafusion::common::Result<Extension> {
            unreachable!()
        }
        fn try_encode_table_provider(
            &self,
            _: &TableReference,
            _: Arc<dyn TableProvider>,
            _: &mut Vec<u8>,
        ) -> datafusion::common::Result<()> {
            unreachable!()
        }
        fn try_decode_table_provider(
            &self,
            _: &[u8],
            _: &TableReference,
            _: Arc<Schema>,
            _: &TaskContext,
        ) -> datafusion::common::Result<Arc<dyn TableProvider>> {
            unreachable!()
        }
        fn try_encode_udf(
            &self,
            _: &ScalarUDF,
            buf: &mut Vec<u8>,
        ) -> datafusion::common::Result<()> {
            if self.fail {
                return Err(DataFusionError::Plan("configuration export failed".into()));
            }
            buf.extend_from_slice(&self.value.to_le_bytes());
            Ok(())
        }
        fn try_decode_udf(
            &self,
            name: &str,
            buf: &[u8],
        ) -> datafusion::common::Result<Arc<ScalarUDF>> {
            if name != "configured" {
                return Err(DataFusionError::Plan("unknown configured function".into()));
            }
            let bytes = buf
                .try_into()
                .map_err(|_| DataFusionError::Plan("invalid configured payload".into()))?;
            Ok(Arc::new(function(i64::from_le_bytes(bytes))))
        }
    }
    let versions = std::collections::BTreeMap::from([("scalar:configured".into(), "1".into())]);
    let mut b = DataflowBuilder::with_semantics(SemanticConfig {
        function_versions: versions.clone(),
        ..SemanticConfig::default()
    });
    let node = b.add_scalar("configured", function(42).call(vec![]))?;
    b.scalar_output("out", &node)?;
    let flow = b.finish()?;
    assert!(flow
        .to_bytes_with_codec(Arc::new(Codec {
            value: 42,
            fail: true
        }))
        .unwrap_err()
        .to_string()
        .contains("configuration export failed"));
    let codec = Arc::new(Codec {
        value: 42,
        fail: false,
    });
    let bytes = flow.to_bytes_with_codec(codec.clone())?;
    let config = RuntimeConfig {
        function_versions: versions,
        ..RuntimeConfig::default()
    };
    assert!(Runtime::new(config.clone())?
        .decode_dataflow(&bytes)
        .is_err());
    let runtime =
        Runtime::with_session_state_and_codec(SessionContext::new().state(), config, codec)?;
    let flow = runtime.decode_dataflow(&bytes)?;
    let out = flow.interface().root().scalar_output("out")?;
    let p = runtime.prepare(&flow).await?;
    assert_eq!(
        p.query(&[], &[out], &p.inputs().finish()?)
            .await?
            .scalar(&out)?,
        &ScalarValue::Int64(Some(42))
    );
    Ok(())
}

#[tokio::test]
async fn dictionary_multibatch_empty_assets_and_typed_nulls_round_trip() -> Result<()> {
    use avenger_datafusion_dataflow::{
        arrow::{
            array::StringDictionaryBuilder,
            datatypes::{Field, Int8Type, Schema},
            record_batch::RecordBatch,
        },
        datafusion::logical_expr::lit,
        TableSnapshot, TableStore,
    };
    use std::sync::Arc;
    let mut dictionary = StringDictionaryBuilder::<Int8Type>::new();
    dictionary.append("east")?;
    dictionary.append_null();
    dictionary.append("west")?;
    let array = Arc::new(dictionary.finish());
    use avenger_datafusion_dataflow::arrow::array::Array;
    let schema = Arc::new(Schema::new(vec![Field::new(
        "region",
        array.data_type().clone(),
        true,
    )]));
    let batch = RecordBatch::try_new(schema.clone(), vec![array])?;
    let store = TableStore::new(TableSnapshot::from_batches(
        schema.clone(),
        vec![batch.slice(0, 1)],
    )?);
    let asset = store.append_batch(batch.slice(1, 2))?;
    let mut builder = DataflowBuilder::new();
    let first = builder.table_snapshot("first", asset.clone())?;
    let second = builder.table_snapshot("second", asset)?;
    let empty = builder.table_snapshot("empty", TableSnapshot::empty(schema))?;
    let null = builder.add_scalar("null", lit(ScalarValue::Int64(None)))?;
    builder.table_output("first", &first)?;
    builder.table_output("second", &second)?;
    builder.table_output("empty", &empty)?;
    builder.scalar_output("null", &null)?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let decoded = runtime.decode_dataflow(&builder.finish()?.to_bytes()?)?;
    let names = decoded.interface().root();
    let prepared = runtime.prepare(&decoded).await?;
    let first = names.table_output("first")?;
    let second = names.table_output("second")?;
    let empty = names.table_output("empty")?;
    let null = names.scalar_output("null")?;
    let result = prepared
        .query(
            &[first, second, empty],
            &[null],
            &prepared.inputs().finish()?,
        )
        .await?;
    assert_eq!(result.table(&first)?.id(), result.table(&second)?.id());
    assert_eq!(result.table(&first)?.batches().len(), 2);
    assert_eq!(result.table(&first)?.num_rows(), 3);
    assert_eq!(result.table(&empty)?.num_rows(), 0);
    assert_eq!(result.scalar(&null)?, &ScalarValue::Int64(None));
    prepared.clear_results();
    assert_eq!(
        prepared
            .query(&[first], &[], &prepared.inputs().finish()?)
            .await?
            .table(&first)?
            .num_rows(),
        3
    );
    Ok(())
}
