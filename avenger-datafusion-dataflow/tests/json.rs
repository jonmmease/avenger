#![cfg(feature = "json")]
use avenger_datafusion_dataflow::{
    datafusion::common::ScalarValue, json::*, Result, Runtime, RuntimeConfig,
};

#[tokio::test]
async fn nested_json_requests_and_native_protobuf_round_trip() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let spec: DataflowSpec =
        serde_json::from_str(include_str!("fixtures/facets.dataflow.json")).unwrap();
    let mut request: QueryRequest =
        serde_json::from_str(include_str!("fixtures/facets.query.json")).unwrap();
    let flow = runtime
        .load_spec(&spec, &FileSourceResolver::new("."))
        .await?;
    let root = flow.interface().root();
    let prepared = runtime.prepare(&flow).await?;
    let result = prepared
        .query_request(&request, &AssetBindings::new())
        .await?;
    assert_eq!(
        result.scalar(&root.scalar_output("total")?)?,
        &ScalarValue::Float64(Some(600.0))
    );
    assert_eq!(
        result
            .table(&root.table_output("eligible_sales")?)?
            .num_rows(),
        3
    );
    let regions = root.scope("regions")?;
    let years = regions.scope("years")?;
    let east = result
        .scope(regions.handle().unwrap())?
        .get(&regions.handle().unwrap().key([ScalarValue::from("East")])?)
        .unwrap();
    let east2025 = east
        .scope(years.handle().unwrap())?
        .get(&years.handle().unwrap().key([2025_i64.into()])?)
        .unwrap();
    assert_eq!(east2025.table(&years.table_output("marks")?)?.num_rows(), 0);
    assert_eq!(
        east2025.scalar(&years.scalar_output("threshold")?)?,
        &ScalarValue::Float64(Some(220.0))
    );
    let other = Runtime::new(RuntimeConfig::default())?;
    let decoded = other.decode_dataflow(&flow.to_bytes()?)?;
    let loaded = other.prepare(&decoded).await?;
    assert_eq!(
        loaded
            .query_request(&request, &AssetBindings::new())
            .await?
            .scalar(&decoded.interface().root().scalar_output("total")?)?,
        &ScalarValue::Float64(Some(600.0))
    );
    request.bindings.scalars.clear();
    assert!(prepared
        .query_request(&request, &AssetBindings::new())
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn explicit_schema_defers_reads_and_sources_are_cached() -> Result<()> {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let spec: DataflowSpec =
        serde_json::from_str(include_str!("fixtures/file.dataflow.json")).unwrap();
    let flow = runtime
        .load_spec(&spec, &FileSourceResolver::new(temp.path()))
        .await?;
    let prepared = runtime.prepare(&flow).await?;
    let mut request: QueryRequest = serde_json::from_value(serde_json::json!({"bindings":{"scalars":{"minimum":0}},"outputs":{"tables":[{"output":"rows"}]}})).unwrap();
    assert!(prepared
        .query_request(&request, &AssetBindings::new())
        .await
        .is_err());
    std::fs::write(temp.path().join("sales.csv"), "amount\n10\n20\n").unwrap();
    let first = prepared
        .query_request(&request, &AssetBindings::new())
        .await?;
    assert_eq!(
        first
            .table(&flow.interface().root().table_output("rows")?)?
            .num_rows(),
        2
    );
    std::fs::remove_file(temp.path().join("sales.csv")).unwrap();
    request.bindings.scalars.insert("minimum".into(), 15.into());
    let warm = prepared
        .query_request(&request, &AssetBindings::new())
        .await?;
    assert_eq!(warm.report().source_executions, 0);
    assert_eq!(warm.report().cache_hits, 1);
    assert_eq!(warm.report().physical_plans, 1);
    assert_eq!(
        warm.table(&flow.interface().root().table_output("rows")?)?
            .num_rows(),
        1
    );
    prepared.clear_results();
    assert!(prepared
        .query_request(&request, &AssetBindings::new())
        .await
        .is_err());
    assert!(flow.to_bytes().is_err());
    Ok(())
}

#[tokio::test]
async fn sql_dependencies_ctes_and_definition_validation() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let resolver = FileSourceResolver::new(".");
    for sql in [
        "DROP TABLE sales",
        "SELECT 1; SELECT 2",
        "SELECT * FROM undeclared",
    ] {
        let spec: DataflowSpec = serde_json::from_value(
            serde_json::json!({"version":1,"dialect":"datafusion","tables":{"a":sql}}),
        )
        .unwrap();
        assert!(runtime.load_spec(&spec, &resolver).await.is_err());
    }
    let spec:DataflowSpec=serde_json::from_value(serde_json::json!({"version":1,"dialect":"datafusion","tables":{"a":"SELECT * FROM b", "b":"WITH b AS (SELECT 42 AS x) SELECT * FROM b"},"outputs":{"tables":{"rows":"a"}}})).unwrap();
    let flow = runtime.load_spec(&spec, &resolver).await?;
    let prepared = runtime.prepare(&flow).await?;
    let out = flow.interface().root().table_output("rows")?;
    assert_eq!(
        prepared
            .query(&[out], &[], &prepared.inputs().finish()?)
            .await?
            .table(&out)?
            .num_rows(),
        1
    );
    let cyclic:DataflowSpec=serde_json::from_value(serde_json::json!({"version":1,"dialect":"datafusion","tables":{"a":"SELECT * FROM b", "b":"SELECT * FROM a"}})).unwrap();
    assert!(runtime.load_spec(&cyclic, &resolver).await.is_err());
    assert!(serde_json::from_str::<DataflowSpec>(r#"{"version":1,"dialect":"datafusion","inputs":{"x":{"kind":"scalar","type":"int64","default":1}}}"#).is_err());
    assert!(serde_json::from_str::<DataflowSpec>(
        r#"{"version":1,"dialect":"datafusion","tables":{"a":"SELECT 1","a":"SELECT 2"}}"#
    )
    .is_err());
    assert!(
        serde_json::from_str::<QueryRequest>(r#"{"bindings":{"scalars":{"x":1,"x":2}}}"#).is_err()
    );
    Ok(())
}

#[tokio::test]
async fn inference_is_explicit_and_all_file_adapters_execute() -> Result<()> {
    use async_trait::async_trait;
    use avenger_datafusion_dataflow::{
        arrow::{datatypes::SchemaRef, ipc::writer::FileWriter},
        datafusion::{catalog::TableProvider, parquet::arrow::ArrowWriter},
        TableSnapshot,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[derive(Debug)]
    struct Resolver {
        inner: FileSourceResolver,
        inferences: AtomicUsize,
    }
    #[async_trait]
    impl SourceResolver for Resolver {
        async fn infer_schema(&self, s: &FileSource) -> Result<SchemaRef> {
            self.inferences.fetch_add(1, Ordering::SeqCst);
            self.inner.infer_schema(s).await
        }
        fn file_provider(
            &self,
            s: &FileSource,
            schema: SchemaRef,
        ) -> Result<Arc<dyn TableProvider>> {
            self.inner.file_provider(s, schema)
        }
        fn asset(&self, name: &str) -> Result<TableSnapshot> {
            self.inner.asset(name)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let resolver = Resolver {
        inner: FileSourceResolver::new(dir.path()),
        inferences: AtomicUsize::new(0),
    };
    std::fs::write(dir.path().join("data.tsv"), "value\n1\n2\n").unwrap();
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let request: QueryRequest =
        serde_json::from_value(serde_json::json!({"outputs":{"tables":[{"output":"rows"}]}}))
            .unwrap();
    let mut batch = None;
    for (file, format, options) in [
        ("data.tsv", "csv", serde_json::json!({"delimiter":"\t"})),
        ("data.arrow", "arrow", serde_json::json!({})),
        ("data.parquet", "parquet", serde_json::json!({})),
    ] {
        let spec:DataflowSpec=serde_json::from_value(serde_json::json!({"version":1,"dialect":"datafusion","sources":{"data":{"url":file,"format":format,"options":options}},"outputs":{"tables":{"rows":"data"}}})).unwrap();
        let flow = runtime.load_spec(&spec, &resolver).await?;
        let prepared = runtime.prepare(&flow).await?;
        let result = prepared
            .query_request(&request, &AssetBindings::new())
            .await?;
        let table = result.table(&flow.interface().root().table_output("rows")?)?;
        assert_eq!(table.num_rows(), 2);
        if batch.is_none() {
            let data = table.batches()[0].clone();
            let mut writer = FileWriter::try_new(
                std::fs::File::create(dir.path().join("data.arrow")).unwrap(),
                data.schema().as_ref(),
            )?;
            writer.write(&data)?;
            writer.finish()?;
            let mut writer = ArrowWriter::try_new(
                std::fs::File::create(dir.path().join("data.parquet")).unwrap(),
                data.schema(),
                None,
            )
            .unwrap();
            writer.write(&data).unwrap();
            writer.close().unwrap();
            batch = Some(data);
        }
        prepared
            .query_request(&request, &AssetBindings::new())
            .await?;
    }
    assert_eq!(resolver.inferences.load(Ordering::SeqCst), 3);
    Ok(())
}

#[tokio::test]
async fn json_values_and_assets_preserve_exact_types_and_binding_identity() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let spec:DataflowSpec=serde_json::from_value(serde_json::json!({"version":1,"dialect":"datafusion","inputs":{"rows":{"kind":"table","schema":[{"name":"x","type":"int64","nullable":true}]},"large":{"kind":"scalar","type":"uint64"}},"scalars":{"out":"$large"},"outputs":{"tables":{"rows":"rows"},"scalars":{"large":"out"}}})).unwrap();
    let flow = runtime
        .load_spec(&spec, &FileSourceResolver::new("."))
        .await?;
    let p = runtime.prepare(&flow).await?;
    let root = flow.interface().root();
    let mut request:QueryRequest=serde_json::from_value(serde_json::json!({"bindings":{"scalars":{"large":"18446744073709551615"},"tables":{"rows":{"values":[{"x":"9223372036854775807"},{}]}}},"outputs":{"tables":[{"output":"rows"}],"scalars":[{"output":"large"}]}})).unwrap();
    let result = p.query_request(&request, &AssetBindings::new()).await?;
    assert_eq!(
        result.scalar(&root.scalar_output("large")?)?,
        &ScalarValue::UInt64(Some(u64::MAX))
    );
    let asset = result.table(&root.table_output("rows")?)?.clone();
    let assets = AssetBindings::from([("selection".into(), asset)]);
    request.bindings.tables.insert(
        "rows".into(),
        TableBinding::Asset(AssetSource {
            asset: "selection".into(),
        }),
    );
    p.query_request(&request, &assets).await?;
    assert_eq!(
        p.query_request(&request, &assets)
            .await?
            .report()
            .cache_hits,
        2
    );
    request
        .bindings
        .scalars
        .insert("large".into(), serde_json::json!(-1));
    assert!(p.query_request(&request, &assets).await.is_err());
    Ok(())
}

#[tokio::test]
async fn correlated_subqueries_and_window_functions_survive_artifacts() -> Result<()> {
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let spec:DataflowSpec=serde_json::from_value(serde_json::json!({"version":1,"dialect":"datafusion","sources":{"rows":{"schema":[{"name":"x","type":"int64","nullable":false}],"values":[{"x":1},{"x":2}]}},"tables":{"out":"SELECT a.x, (SELECT MAX(b.x) FROM rows b WHERE b.x = a.x) AS m, row_number() OVER (ORDER BY a.x) AS n FROM rows a WHERE EXISTS (SELECT 1 FROM rows b WHERE b.x = a.x)"},"outputs":{"tables":{"out":"out"}}})).unwrap();
    let flow = runtime
        .load_spec(&spec, &FileSourceResolver::new("."))
        .await?;
    let other = runtime.decode_dataflow(&flow.to_bytes()?)?;
    let prepared = runtime.prepare(&other).await?;
    let output = other.interface().root().table_output("out")?;
    assert_eq!(
        prepared
            .query(&[output], &[], &prepared.inputs().finish()?)
            .await?
            .table(&output)?
            .num_rows(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn sql_expression_bindings_keep_columns_local_and_round_trip() -> Result<()> {
    use avenger_datafusion_dataflow::datafusion::logical_expr::{col, lit};
    let runtime = Runtime::new(Default::default())?;
    let spec: DataflowSpec = serde_json::from_value(serde_json::json!({
        "version": 1, "dialect": "datafusion",
        "inputs": {
            "selection": {"kind":"expr", "type":"boolean"},
            "unused": {"kind":"expr", "type":"boolean"},
            "measure": {"kind":"expr", "type":"int64"}
        },
        "sources": {"data": {"schema": [{"name":"x", "type":"int64", "nullable":false}, {"name":"y", "type":"int64", "nullable":false}], "values": [{"x":1,"y":10},{"x":2,"y":20},{"x":3,"y":30}]}},
        "tables": {
            "left": "SELECT $measure AS value FROM data AS l WHERE $selection",
            "right": "SELECT $measure AS value FROM data AS r WHERE $selection"
        },
        "outputs": {"tables": {"left":"left", "right":"right"}}
    })).unwrap();
    let flow = runtime
        .load_spec(&spec, &FileSourceResolver::new("."))
        .await?;
    let bytes = flow.to_bytes()?;
    for flow in [flow, runtime.decode_dataflow(&bytes)?] {
        let p = runtime.prepare(&flow).await?;
        let names = flow.interface().root();
        let mut request: QueryRequest = serde_json::from_value(serde_json::json!({
            "bindings": {"exprs": {"selection": "x >= 2", "measure":"y + 1", "unused":"missing.field > 0"}},
            "outputs": {"tables":[{"output":"left"},{"output":"right"}]}
        })).unwrap();
        let result = p.query_request(&request, &AssetBindings::new()).await?;
        for name in ["left", "right"] {
            assert_eq!(result.table(&names.table_output(name)?)?.num_rows(), 2);
        }
        let native = p
            .inputs()
            .expr(&names.expr_input("selection")?, col("x").gt_eq(lit(2_i64)))?
            .expr(&names.expr_input("measure")?, col("y") + lit(1_i64))?
            .expr(&names.expr_input("unused")?, col("unresolved"))?
            .finish()?;
        let out = names.table_output("left")?;
        let expected = p.query(&[out], &[], &native).await?;
        assert_eq!(
            result.table(&out)?.batches(),
            expected.table(&out)?.batches()
        );
        request
            .bindings
            .exprs
            .insert("selection".into(), "y >= 30".into());
        assert_eq!(
            p.query_request(&request, &AssetBindings::new())
                .await?
                .table(&out)?
                .num_rows(),
            1
        );
        for bad in [
            "missing > 0",
            "SELECT true",
            "x > 0; SELECT true",
            "$measure > 0",
            "(SELECT true)",
            "now() IS NOT NULL",
            "sum(x) > 0",
            "x",
            "@secret = 1",
        ] {
            request
                .bindings
                .exprs
                .insert("selection".into(), bad.into());
            assert!(
                p.query_request(&request, &AssetBindings::new())
                    .await
                    .is_err(),
                "{bad}"
            );
        }
        request
            .bindings
            .exprs
            .insert("selection".into(), "true".into());
        request
            .bindings
            .exprs
            .insert("unused".into(), "now() IS NOT NULL".into());
        assert!(p
            .query_request(&request, &AssetBindings::new())
            .await
            .is_err());
    }
    assert!(serde_json::from_str::<QueryRequest>(
        r#"{"bindings":{"exprs":{"x":"true","x":"false"}}}"#
    )
    .is_err());
    Ok(())
}

#[tokio::test]
async fn json_expression_defaults_and_nested_overrides_use_native_binding_rules() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let mut spec: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/facets.dataflow.json")).unwrap();
    spec["scopes"]["regions"]["scopes"]["years"]["inputs"]["selection"] =
        serde_json::json!({"kind":"expr", "type":"boolean"});
    spec["scopes"]["regions"]["scopes"]["years"]["outputs"]["tables"]["selected"] =
        serde_json::json!("selected");
    // Match the fixture's local row alias rather than relying on its table names.
    let rows_alias = spec["scopes"]["regions"]["scopes"]["years"]["rows"]
        .as_str()
        .unwrap()
        .to_owned();
    spec["scopes"]["regions"]["scopes"]["years"]["tables"]["selected"] =
        format!("SELECT * FROM {rows_alias} WHERE $selection").into();
    let flow = runtime
        .load_spec(
            &serde_json::from_value(spec).unwrap(),
            &FileSourceResolver::new("."),
        )
        .await?;
    let p = runtime.prepare(&flow).await?;
    let request: QueryRequest = serde_json::from_value(serde_json::json!({
        "bindings": {"scalars":{"minimum":0.0}, "scope_defaults":[{"scope":["regions","years"],"exprs":{"selection":"true"}}],
            "overrides":[{"path":[{"scope":"regions","key":["East"]},{"scope":"years","key":[2025]}],"exprs":{"selection":"false"}}]},
        "outputs":{"tables":[{"scope":["regions","years"],"output":"selected"}]}
    })).unwrap();
    let result = p.query_request(&request, &AssetBindings::new()).await?;
    let regions = flow.interface().root().scope("regions")?;
    let years = regions.scope("years")?;
    let east = result
        .scope(regions.handle().unwrap())?
        .get(&regions.handle().unwrap().key(["East".into()])?)
        .unwrap();
    let panel = east
        .scope(years.handle().unwrap())?
        .get(&years.handle().unwrap().key([2025_i64.into()])?)
        .unwrap();
    assert_eq!(panel.table(&years.table_output("selected")?)?.num_rows(), 0);
    Ok(())
}

#[tokio::test]
async fn json_standalone_expression_inputs_validate_the_actual_empty_context() -> Result<()> {
    let runtime = Runtime::new(Default::default())?;
    let spec: DataflowSpec = serde_json::from_value(serde_json::json!({
        "version": 1, "dialect": "datafusion",
        "inputs": {"predicate": {"kind":"expr", "type":"boolean"}},
        "scalars": {"value":"$predicate"},
        "outputs": {"scalars":{"value":"value"}}
    }))
    .unwrap();
    let flow = runtime
        .load_spec(&spec, &FileSourceResolver::new("."))
        .await?;
    let p = runtime.prepare(&flow).await?;
    for bindings in [
        serde_json::json!({}),
        serde_json::json!({"scalars":{"predicate":true}}),
        serde_json::json!({"exprs":{"unknown":"true"}}),
        serde_json::json!({"exprs":{"predicate":"missing > 0"}}),
    ] {
        let request: QueryRequest =
            serde_json::from_value(serde_json::json!({"bindings":bindings})).unwrap();
        assert!(p
            .query_request(&request, &AssetBindings::new())
            .await
            .is_err());
    }
    let request: QueryRequest = serde_json::from_value(serde_json::json!({
        "bindings":{"exprs":{"predicate":"true"}},
        "outputs":{"scalars":[{"output":"value"}]}
    }))
    .unwrap();
    let result = p.query_request(&request, &AssetBindings::new()).await?;
    assert_eq!(
        result.scalar(&flow.interface().root().scalar_output("value")?)?,
        &avenger_datafusion_dataflow::datafusion::common::ScalarValue::Boolean(Some(true))
    );
    Ok(())
}
