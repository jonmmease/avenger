use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use avenger_datafusion_dataflow::{
    DataflowBuilder, Result, Runtime, RuntimeConfig, SemanticConfig, TableSnapshot,
};
use avenger_scales_datafusion::{
    list_literal, options_literal, scale_expr, BuiltinScale, ScaleExtensionCodec,
    SCALE_FUNCTION_VERSION,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    functions_aggregate::expr_fn::max,
    functions_nested::expr_fn::make_array,
    logical_expr::{col, lit, scalar_subquery, Expr, LogicalPlanBuilder},
    prelude::SessionContext,
};

#[tokio::test]
async fn scoped_domains_dynamic_ranges_cache_and_decode_into_a_fresh_runtime() -> Result<()> {
    let versions = BTreeMap::from([("scalar:scale".into(), SCALE_FUNCTION_VERSION.into())]);
    let mut b = DataflowBuilder::with_semantics(SemanticConfig {
        function_versions: versions.clone(),
        ..SemanticConfig::default()
    });
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["East", "East", "West", "West"])),
            Arc::new(Float64Array::from(vec![10.0, 20.0, 50.0, 100.0])),
        ],
    )?;
    let sales = b.table_snapshot("sales", TableSnapshot::from_batches(schema, vec![batch])?)?;
    let width = b.scalar_input("width", DataType::Float64)?;
    b.partition_by("regions", sales.plan_ref(), vec![col("region")], |s| {
        let zoom = s.scalar_input("zoom", DataType::Float64)?;
        let maximum = s.add_plan(
            "maximum",
            LogicalPlanBuilder::from(s.rows().plan_ref())
                .aggregate(Vec::<Expr>::new(), vec![max(col("value"))])?
                .build()?,
        )?;
        let upper = s.add_expr("upper", scalar_subquery(Arc::new(maximum.plan_ref())))?;
        let x = scale_expr(
            BuiltinScale::Linear,
            make_array(vec![lit(0.0_f64), upper.expr_ref()]),
            make_array(vec![lit(0.0_f64), width.expr_ref() * zoom.expr_ref()]),
            options_literal(&HashMap::new())?,
            col("value"),
        )?;
        let projected = s.add_plan(
            "scaled",
            LogicalPlanBuilder::from(s.rows().plan_ref())
                .project(vec![x.alias("x")])?
                .build()?,
        )?;
        s.table_output("marks", &projected)?;
        let marker = s.add_expr(
            "midpoint",
            scale_expr(
                BuiltinScale::Linear,
                list_literal(Arc::new(Float64Array::from(vec![0.0, 1.0])))?,
                make_array(vec![lit(0.0_f64), width.expr_ref()]),
                options_literal(&HashMap::new())?,
                lit(0.5_f64),
            )?,
        )?;
        s.scalar_output("midpoint", &marker)?;
        Ok(())
    })?;
    let definition = b.finish()?;
    let codec = Arc::new(ScaleExtensionCodec::default());
    let bytes = definition.to_bytes_with_codec(codec.clone())?;
    let runtime = Runtime::with_session_state_and_codec(
        SessionContext::new().state(),
        RuntimeConfig {
            function_versions: versions,
            ..RuntimeConfig::default()
        },
        codec,
    )?;
    let loaded = runtime.decode_dataflow(&bytes)?;
    let interface = loaded.interface().root();
    let regions = interface.scope("regions")?;
    let handle = regions.handle().unwrap();
    let marks = regions.table_output("marks")?;
    let midpoint = regions.scalar_output("midpoint")?;
    let prepared = runtime.prepare(&loaded).await?;
    let inputs = prepared
        .inputs()
        .scalar(&interface.scalar_input("width")?, 200.0_f64.into())?
        .scope_defaults(handle, |i| {
            i.scalar(&regions.scalar_input("zoom")?, 1.0_f64.into())
        })?
        .finish()?;
    let first = prepared.query(&[marks], &[midpoint], &inputs).await?;
    for (_, panel) in first.scope(handle)?.iter() {
        assert_eq!(panel.scalar(&midpoint)?, &ScalarValue::Float32(Some(100.0)));
        assert_eq!(
            panel.table(&marks)?.batches()[0]
                .column(0)
                .as_any()
                .downcast_ref::<datafusion::arrow::array::Float32Array>()
                .unwrap()
                .values()
                .as_ref(),
            &[100.0, 200.0]
        );
    }
    let east = handle.instance(vec![ScalarValue::Utf8(Some("East".into()))])?;
    let changed = inputs
        .edit()
        .at(&east, |i| {
            i.scalar(&regions.scalar_input("zoom")?, 0.5_f64.into())
        })?
        .finish()?;
    let second = prepared.query(&[marks], &[midpoint], &changed).await?;
    assert!(second.report().cache_hits > 0);
    let panels = second.scope(handle)?;
    for (region, expected) in [("East", vec![50.0, 100.0]), ("West", vec![100.0, 200.0])] {
        let key = handle.key(vec![ScalarValue::Utf8(Some(region.into()))])?;
        let panel = panels.get(&key).unwrap();
        assert_eq!(
            panel.table(&marks)?.batches()[0]
                .column(0)
                .as_any()
                .downcast_ref::<datafusion::arrow::array::Float32Array>()
                .unwrap()
                .values()
                .as_ref(),
            expected.as_slice()
        );
    }
    let third = prepared.query(&[marks], &[midpoint], &inputs).await?;
    assert!(third.report().cache_hits > 0);
    assert_eq!(third.report().physical_plans, 0);
    Ok(())
}
