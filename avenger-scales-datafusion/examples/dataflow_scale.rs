//! Derive a scale domain from data, serialize the graph, and change its range.
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use avenger_datafusion_dataflow::{
    DataflowBuilder, Result, Runtime, RuntimeConfig, SemanticConfig, TableSnapshot,
};
use avenger_scales_datafusion::{
    options_literal, scale_expr, BuiltinScale, ScaleExtensionCodec, SCALE_FUNCTION_VERSION,
};
use datafusion::{
    arrow::{
        array::Float64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    functions_aggregate::expr_fn::max,
    functions_nested::expr_fn::make_array,
    logical_expr::{col, lit, scalar_subquery, Expr, LogicalPlanBuilder},
    prelude::SessionContext,
};

#[tokio::main]
async fn main() -> Result<()> {
    let versions = BTreeMap::from([("scalar:scale".into(), SCALE_FUNCTION_VERSION.into())]);
    let mut b = DataflowBuilder::with_semantics(SemanticConfig {
        function_versions: versions.clone(),
        ..SemanticConfig::default()
    });
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Float64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Float64Array::from(vec![0.0, 25.0, 100.0]))],
    )?;
    let data = b.table_snapshot("data", TableSnapshot::from_batches(schema, vec![batch])?)?;
    let maximum = b.add_plan(
        "maximum",
        LogicalPlanBuilder::from(data.plan_ref())
            .aggregate(Vec::<Expr>::new(), vec![max(col("value"))])?
            .build()?,
    )?;
    let upper = b.add_scalar("upper", scalar_subquery(Arc::new(maximum.plan_ref())))?;
    let width = b.scalar_input("width", DataType::Float64)?;
    let x = scale_expr(
        BuiltinScale::Linear,
        make_array(vec![lit(0.0_f64), upper.expr_ref()]),
        make_array(vec![lit(0.0_f64), width.expr_ref()]),
        options_literal(&HashMap::new())?,
        col("value"),
    )?;
    let scaled = b.add_plan(
        "scaled",
        LogicalPlanBuilder::from(data.plan_ref())
            .project(vec![col("value"), x.alias("x")])?
            .build()?,
    )?;
    b.table_output("marks", &scaled)?;

    let codec = Arc::new(ScaleExtensionCodec::default());
    let bytes = b.finish()?.to_bytes_with_codec(codec.clone())?;
    println!("Serialized dataflow: {} bytes", bytes.len());
    let runtime = Runtime::with_session_state_and_codec(
        SessionContext::new().state(),
        RuntimeConfig {
            function_versions: versions,
            ..RuntimeConfig::default()
        },
        codec,
    )?;
    let dataflow = runtime.decode_dataflow(&bytes)?;
    let root = dataflow.interface().root();
    let prepared = runtime.prepare(&dataflow).await?;
    let output = root.table_output("marks")?;
    for width in [200.0, 400.0, 200.0] {
        let inputs = prepared
            .inputs()
            .scalar(&root.scalar_input("width")?, width.into())?
            .finish()?;
        let result = prepared.query(&[output], &[], &inputs).await?;
        println!(
            "width={width}, cache hits={}, physical plans={}\n{}",
            result.report().cache_hits,
            result.report().physical_plans,
            pretty_format_batches(result.table(&output)?.batches())?
        );
    }
    Ok(())
}
