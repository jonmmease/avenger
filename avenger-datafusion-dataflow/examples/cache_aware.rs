use std::{sync::Arc, time::Duration};

use avenger_datafusion_dataflow::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    datafusion::{
        functions_aggregate::expr_fn::sum,
        logical_expr::{col, LogicalPlanBuilder},
    },
    CacheAwareOptions, CacheNode, CacheRead, CacheTargets, DataflowBuilder, Error, QueryInputs,
    Result, Runtime, TableSnapshot,
};

fn snapshot(schema: &SchemaRef, values: &[i64]) -> Result<TableSnapshot> {
    TableSnapshot::from_batches(
        schema.clone(),
        vec![RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(values.to_vec()))],
        )?],
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Int64,
        false,
    )]));
    let mut builder = DataflowBuilder::new();
    let source = builder.table_input("source", schema.clone())?;
    let weight = builder.scalar_input("weight", DataType::Int64)?;
    let brush = builder.scalar_input("brush", DataType::Int64)?;
    let histogram = builder.add_plan(
        "histogram",
        LogicalPlanBuilder::from(source.plan_ref())
            .aggregate(
                vec![col("value")],
                vec![sum(weight.expr_ref()).alias("count")],
            )?
            .build()?,
    )?;
    let view = builder.add_plan(
        "view",
        LogicalPlanBuilder::from(histogram.plan_ref())
            .filter(col("value").gt_eq(brush.expr_ref()))?
            .build()?,
    )?;
    let output = builder.table_output("rows", &view)?;
    let flow = Runtime::new(Default::default())?
        .prepare(&builder.finish()?)
        .await?;

    let previous = snapshot(&schema, &[1, 1, 2, 3])?;
    let latest = snapshot(&schema, &[1, 1, 1, 2, 3, 4])?;
    let previous_inputs = flow
        .inputs()
        .table(&source, previous.clone())?
        .scalar(&weight, 1_i64.into())?
        .scalar(&brush, 0_i64.into())?
        .finish()?;
    flow.query(&[output], &[], &previous_inputs).await?;

    let mut fallback = flow
        .inputs()
        .table(&source, previous)?
        .scalar(&weight, 1_i64.into())?
        .finish_overrides()?;
    let mut latest_inputs = previous_inputs
        .edit()
        .table(&source, latest)?
        .scalar(&weight, 2_i64.into())?
        .finish()?;
    let strict = flow.cache_aware_query(
        &[output],
        &[],
        QueryInputs::new(latest_inputs.clone()),
        Default::default(),
    )?;
    assert!(matches!(
        strict.read(CacheRead::CachedOnly).await,
        Err(Error::CacheMiss { .. })
    ));

    let targets = CacheTargets::Nodes(vec![CacheNode::Plan(histogram)]);
    let mut warming = flow.cache_aware_query(
        &[output],
        &[],
        QueryInputs::new(latest_inputs.clone()).fallbacks([fallback.clone()])?,
        CacheAwareOptions {
            targets: targets.clone(),
            start_latest: true,
        },
    )?;
    let cached = warming.read(CacheRead::CachedOnly).await?;
    println!(
        "Initial cached view: candidate {}",
        cached.candidate_index()
    );

    // The retained warming query progresses independently of interaction reads.
    let mut ticks = tokio::time::interval(Duration::from_millis(20));
    for tick in 0..100 {
        ticks.tick().await;
        if tick == 1 {
            latest_inputs = latest_inputs
                .edit()
                .table(&source, snapshot(&schema, &[1, 1, 1, 2, 3, 4, 5])?)?
                .finish()?;
            // Constructing the replacement first preserves a pending group's queue position.
            let replacement = flow.cache_aware_query(
                &[output],
                &[],
                QueryInputs::new(latest_inputs.clone()).fallbacks([fallback.clone()])?,
                CacheAwareOptions {
                    targets: targets.clone(),
                    start_latest: true,
                },
            )?;
            warming = replacement;
        }
        let current_brush = if tick == 0 { 2_i64 } else { 3_i64 };
        let current_inputs = latest_inputs
            .edit()
            .scalar(&brush, current_brush.into())?
            .finish()?;
        let interaction = flow.cache_aware_query(
            &[output],
            &[],
            QueryInputs::new(current_inputs).fallbacks([fallback.clone()])?,
            CacheAwareOptions {
                targets: targets.clone(),
                start_latest: false,
            },
        )?;
        let result = interaction.read(CacheRead::FromCachedTargets).await?;
        assert_eq!(result.inputs().scalar_value(&brush)?, &current_brush.into());
        println!(
            "Brush {current_brush}: candidate {}, rows: {}, physical plans: {}",
            result.candidate_index(),
            result.result().table(&output)?.num_rows(),
            result.result().report().physical_plans
        );
        fallback = flow
            .inputs()
            .table(&source, result.inputs().table_value(&source)?.clone())?
            .scalar(&weight, result.inputs().scalar_value(&weight)?.clone())?
            .finish_overrides()?;
        if result.candidate_index() == 0 && tick > 0 {
            let confirmed = flow.cache_aware_query(
                &[output],
                &[],
                QueryInputs::new(result.inputs().clone()).fallbacks([fallback])?,
                CacheAwareOptions {
                    targets,
                    start_latest: false,
                },
            )?;
            assert_eq!(
                confirmed
                    .read(CacheRead::CachedOnly)
                    .await?
                    .candidate_index(),
                0
            );
            drop(warming);
            return Ok(());
        }
    }
    Err(Error::InvalidConfig(
        "example did not observe warmed targets within 100 refreshes".into(),
    ))
}
