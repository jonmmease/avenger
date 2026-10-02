use avenger_chart_definition::protobuf;
use avenger_chart_definition::{
    dataflow::{
        arrow::{array::Float64Array, datatypes::DataType, record_batch::RecordBatch},
        datafusion::{common::ScalarValue, logical_expr::col},
        *,
    },
    *,
};
use prost::Message;
use std::sync::Arc;

fn flow() -> anyhow::Result<(Dataflow, TableOutput, ScalarInput)> {
    let batch = RecordBatch::try_from_iter(vec![(
        "value",
        Arc::new(Float64Array::from(vec![1., 2., 3.])) as _,
    )])?;
    let mut f = DataflowBuilder::new();
    let node = f.table_snapshot(
        "source",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let output = f.table_output("rows", &node)?;
    let input = f.scalar_input("factor", DataType::Float64)?;
    Ok((f.finish()?, output, input))
}
fn chart() -> anyhow::Result<ChartDefinition> {
    let (f, t, input) = flow()?;
    let mut c = ChartDefinition::builder(f);
    c.parameter("factor", &input, ScalarValue::Float64(Some(2.)))?;
    c.plot("plot", |p| {
        let x = p.scale(
            "x",
            Scale::linear(Domain::numeric(0., 3.), Range::PlotWidth),
        )?;
        p.symbol(
            "points",
            &t,
            SymbolEncoding::new().x(x.field("value")).y(10.),
        )?;
        p.axis(Axis::bottom(&x))?;
        Ok(())
    })?;
    Ok(c.finish()?)
}
#[test]
fn protobuf_resolves_fresh_native_handles() -> anyhow::Result<()> {
    let original = chart()?;
    let old = original
        .dataflow()
        .interface()
        .root()
        .table_output("rows")?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let bytes = original.to_bytes()?;
    let decoded = ChartDefinition::from_bytes(&bytes, &runtime)?;
    decoded.validate()?;
    assert!(decoded.dataflow().interface().table_metadata(&old).is_err());
    let fresh = decoded.dataflow().interface().root().table_output("rows")?;
    let metadata = decoded.dataflow().interface().table_metadata(&fresh)?;
    assert!(metadata.reference.scope.is_empty());
    assert_eq!(metadata.reference.name, "rows");
    assert_eq!(
        decoded.parameters()[0].initial,
        Some(ScalarValue::Float64(Some(2.)))
    );
    assert_eq!(
        protobuf::ChartArtifact::decode(bytes.as_slice())?,
        protobuf::ChartArtifact::decode(decoded.to_bytes()?.as_slice())?
    );
    Ok(())
}
#[test]
fn malformed_artifacts_fail_before_preparation() -> anyhow::Result<()> {
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let mut wire = protobuf::ChartArtifact::decode(chart()?.to_bytes()?.as_slice())?;
    wire.version = 2;
    assert!(ChartDefinition::from_bytes(&wire.encode_to_vec(), &runtime)
        .err()
        .unwrap()
        .to_string()
        .contains("version"));
    wire.version = 1;
    wire.root = None;
    assert!(ChartDefinition::from_bytes(&wire.encode_to_vec(), &runtime).is_err());
    Ok(())
}
#[test]
fn invalid_columns_foreign_handles_and_scale_owners_are_rejected() -> anyhow::Result<()> {
    let (f, t, _) = flow()?;
    let mut c = ChartDefinition::builder(f.clone());
    c.plot("bad", |p| {
        p.symbol(
            "points",
            &t,
            SymbolEncoding::new().x(Value::field("missing")).y(0.),
        )
    })?;
    assert!(c.finish().is_err());
    let (_, foreign, _) = flow()?;
    let mut c = ChartDefinition::builder(f.clone());
    c.plot("bad", |p| {
        p.symbol("points", &foreign, SymbolEncoding::new().x(1.).y(0.))
    })?;
    assert!(c.finish().is_err());
    let mut c = ChartDefinition::builder(f);
    let mut scale = None;
    c.plot("first", |p| {
        scale = Some(p.scale(
            "x",
            Scale::linear(Domain::numeric(0., 1.), Range::PlotWidth),
        )?);
        Ok(())
    })?;
    c.plot("second", |p| {
        p.symbol(
            "points",
            &t,
            SymbolEncoding::new()
                .x(scale.as_ref().unwrap().field("value"))
                .y(0.),
        )
    })?;
    assert!(c.finish().is_err());
    Ok(())
}
#[test]
fn scope_metadata_and_parameter_types_are_checked() -> anyhow::Result<()> {
    let (f, _, input) = flow()?;
    let mut c = ChartDefinition::builder(f);
    c.parameter("factor", &input, ScalarValue::Int32(Some(1)))?;
    assert!(c.finish().is_err());
    let mut f = DataflowBuilder::new();
    let schema = Arc::new(dataflow::arrow::datatypes::Schema::new(vec![
        dataflow::arrow::datatypes::Field::new("key", DataType::Int32, false),
    ]));
    let source = f.table_input("source", schema)?;
    let (scope, rows) = f.partition_by("keys", source.plan_ref(), vec![col("key")], |s| {
        s.table_output("rows", &s.rows())
    })?;
    let f = f.finish()?;
    assert_eq!(f.interface().scope_path(&scope)?, vec!["keys"]);
    assert_eq!(
        f.interface().table_metadata(&rows)?.reference.scope,
        vec!["keys"]
    );
    let mut c = ChartDefinition::builder(f);
    c.plot("illegal", |p| {
        p.symbol("points", &rows, SymbolEncoding::new().x(1.).y(0.))
    })?;
    assert!(c.finish().is_err());
    Ok(())
}

#[tokio::test]
async fn transform_udfs_round_trip_with_the_dataflow_codec() -> anyhow::Result<()> {
    use dataflow::datafusion::{logical_expr::scalar_subquery, prelude::SessionContext};
    let batch = RecordBatch::try_from_iter(vec![(
        "value",
        Arc::new(Float64Array::from(vec![1., 2., 3.])) as _,
    )])?;
    let mut f = DataflowBuilder::with_semantics(SemanticConfig {
        function_versions: avenger_transform::function_versions(),
        ..Default::default()
    });
    let source = f.table_snapshot(
        "source",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let extent = f.add_scalar(
        "extent",
        scalar_subquery(Arc::new(avenger_transform::extent(
            source.plan_ref(),
            col("value"),
        )?)),
    )?;
    let rows = f.table_output("rows", &source)?;
    let extent = f.scalar_output("extent", &extent)?;
    let mut c = ChartDefinition::builder(f.finish()?);
    c.plot("plot", |p| {
        let x = p.scale(
            "x",
            Scale::linear(Domain::extent(&extent), Range::PlotWidth),
        )?;
        p.symbol(
            "points",
            &rows,
            SymbolEncoding::new().x(x.field("value")).y(10.),
        )
    })?;
    let codec = Arc::new(avenger_transform::TransformExtensionCodec::default());
    let bytes = c.finish()?.to_bytes_with_codec(codec.clone())?;
    let runtime = Runtime::with_session_state_and_codec(
        SessionContext::new().state(),
        RuntimeConfig {
            function_versions: avenger_transform::function_versions(),
            ..Default::default()
        },
        codec,
    )?;
    let definition = ChartDefinition::from_bytes(&bytes, &runtime)?;
    let chart = avenger_chart::Chart::prepare(
        definition,
        avenger_chart::ChartOptions {
            dataflow: Some(runtime),
            ..Default::default()
        }
        .with_formatting(d3_formatting()),
    )
    .await?;
    let frame = chart
        .render(avenger_chart::RenderOptions::default())
        .await?;
    assert_eq!(
        frame.plots()[0].scales["x"].numeric_interval_domain_f64()?,
        (1., 3.)
    );
    Ok(())
}

fn d3_formatting() -> avenger_scales::formatter::ScaleFormatting {
    avenger_scales::formatter::ScaleFormatting::d3(Default::default(), Default::default())
}
