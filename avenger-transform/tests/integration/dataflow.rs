use super::common;
use avenger_datafusion_dataflow::{
    CacheConfig, CachePolicy, DataflowBuilder, Result, Runtime, RuntimeConfig, TableSnapshot,
};
use avenger_transform::{self as t, expr_fn as tf, BinOptions};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::DataType,
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    logical_expr::{col, lit, scalar_subquery},
};
use std::sync::Arc;

fn runtime() -> Result<Runtime> {
    Runtime::new(RuntimeConfig {
        cache: CachePolicy::Lru(CacheConfig {
            max_bytes: 16 * 1024 * 1024,
            max_entries: 256,
        }),
        ..Default::default()
    })
}
#[tokio::test]
async fn independent_nodes_reuse_and_invalidate_by_dependency() -> Result<()> {
    let batch = common::batch(vec![Some(0.0), Some(5.0), Some(10.0), Some(29.0)]);
    let snapshot = TableSnapshot::from_batches(batch.schema(), vec![batch.clone()])?;
    let mut b = DataflowBuilder::new();
    let rows = b.table_input("rows", batch.schema())?;
    let maxbins = b.scalar_input("maxbins", DataType::Float64)?;
    let selection = b.expr_input("selection", DataType::Boolean)?;
    let extent = b.add_plan("extent", t::extent(rows.plan_ref(), col("x"))?)?;
    let extent = b.add_scalar("extent_value", scalar_subquery(Arc::new(extent.plan_ref())))?;
    let parameters = b.add_scalar(
        "parameters",
        t::bin_parameters(
            extent.expr_ref(),
            BinOptions {
                maxbins: Some(maxbins.expr_ref()),
                ..Default::default()
            },
        )?,
    )?;
    let bins = b.add_plan(
        "bins",
        t::bin(
            rows.plan_ref(),
            col("x"),
            parameters.expr_ref(),
            ["lo", "hi"],
        )?,
    )?;
    let counts = b.add_plan(
        "counts",
        t::aggregate(
            t::filter(bins.plan_ref(), selection.expr_ref())?,
            vec![col("lo"), col("hi")],
            vec![tf::count().alias("count")],
        )?,
    )?;
    let output = b.table_output("histogram", &counts)?;
    let extent_output = b.scalar_output("extent", &extent)?;
    let params_output = b.scalar_output("parameters", &parameters)?;
    let prepared = runtime()?.prepare(&b.finish()?).await?;
    let inputs = prepared
        .inputs()
        .table(&rows, snapshot.clone())?
        .scalar(&maxbins, 4.0.into())?
        .expr(&selection, lit(true))?
        .finish()?;
    let independent = prepared
        .query(&[], &[extent_output, params_output], &inputs)
        .await?;
    assert_eq!(
        common::struct_values(independent.scalar(&extent_output)?),
        vec![Some(0.0), Some(29.0)]
    );
    let first = prepared.query(&[output], &[], &inputs).await?;
    assert!(first.report().executed_nodes.iter().any(|n| n == "bins"));
    let brush = inputs
        .edit()
        .expr(&selection, col("x").lt(lit(10.0)))?
        .finish()?;
    let changed = prepared.query(&[output], &[], &brush).await?;
    assert_eq!(changed.report().executed_nodes, vec!["counts"]);
    let options = brush.edit().scalar(&maxbins, 10.0.into())?.finish()?;
    let changed = prepared.query(&[output], &[], &options).await?;
    assert!(changed
        .report()
        .executed_nodes
        .iter()
        .any(|n| n == "parameters"));
    assert!(changed.report().executed_nodes.iter().any(|n| n == "bins"));
    assert!(!changed
        .report()
        .executed_nodes
        .iter()
        .any(|n| n.starts_with("extent")));
    let replacement = TableSnapshot::from_batches(
        batch.schema(),
        vec![common::batch(vec![Some(-10.0), Some(40.0)])],
    )?;
    let replaced = options.edit().table(&rows, replacement)?.finish()?;
    let changed = prepared.query(&[output], &[], &replaced).await?;
    assert!(changed
        .report()
        .executed_nodes
        .iter()
        .any(|n| n == "extent"));
    let prior = prepared.query(&[output], &[], &inputs).await?;
    assert!(prior.report().executed_nodes.is_empty());
    Ok(())
}

#[tokio::test]
async fn global_and_local_extent_in_facets() -> Result<()> {
    let batch = RecordBatch::try_from_iter([
        (
            "region",
            Arc::new(StringArray::from(vec!["East", "East", "West", "West"])) as ArrayRef,
        ),
        (
            "x",
            Arc::new(Float64Array::from(vec![0.0, 10.0, 20.0, 40.0])) as ArrayRef,
        ),
    ])?;
    let mut b = DataflowBuilder::new();
    let rows = b.table_snapshot(
        "rows",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let global = b.add_plan("global_extent", t::extent(rows.plan_ref(), col("x"))?)?;
    let global = b.add_scalar(
        "global_parameters",
        t::bin_parameters(
            scalar_subquery(Arc::new(global.plan_ref())),
            BinOptions {
                maxbins: Some(lit(4.0)),
                ..Default::default()
            },
        )?,
    )?;
    let (regions, (selected, local, global_counts, local_counts)) =
        b.partition_by("regions", rows.plan_ref(), vec![col("region")], |s| {
            let selected = s.expr_input("selected", DataType::Boolean)?;
            let extent = s.add_plan("extent", t::extent(s.rows().plan_ref(), col("x"))?)?;
            let extent =
                s.add_scalar("extent_value", scalar_subquery(Arc::new(extent.plan_ref())))?;
            let parameters = s.add_scalar(
                "parameters",
                t::bin_parameters(
                    extent.expr_ref(),
                    BinOptions {
                        maxbins: Some(lit(4.0)),
                        ..Default::default()
                    },
                )?,
            )?;
            let mut outputs = vec![];
            for (name, p) in [
                ("global", global.expr_ref()),
                ("local", parameters.expr_ref()),
            ] {
                let rows = t::bin(s.rows().plan_ref(), col("x"), p, ["lo", "hi"])?;
                let counts = s.add_plan(
                    name,
                    t::aggregate(
                        t::filter(rows, selected.expr_ref())?,
                        vec![col("lo")],
                        vec![tf::count().alias("count")],
                    )?,
                )?;
                outputs.push(s.table_output(name, &counts)?);
            }
            Ok((
                selected,
                s.scalar_output("extent", &extent)?,
                outputs[0],
                outputs[1],
            ))
        })?;
    let prepared = runtime()?.prepare(&b.finish()?).await?;
    let inputs = prepared
        .inputs()
        .scope_defaults(&regions, |b| b.expr(&selected, lit(true)))?
        .finish()?;
    let result = prepared
        .query(&[global_counts, local_counts], &[local], &inputs)
        .await?;
    assert_eq!(result.scope(&regions)?.len(), 2);
    for (region, min, max) in [("East", 0.0, 10.0), ("West", 20.0, 40.0)] {
        let key = regions.key([ScalarValue::from(region)])?;
        let panel = result.scope(&regions)?.get(&key).unwrap();
        assert_eq!(
            common::struct_values(panel.scalar(&local)?),
            vec![Some(min), Some(max)]
        );
        assert_eq!(panel.table(&global_counts)?.num_rows(), 2);
        assert_eq!(panel.table(&local_counts)?.num_rows(), 2);
    }
    let east = regions.instance([ScalarValue::from("East")])?;
    let changed = inputs
        .edit()
        .at(&east, |b| b.expr(&selected, col("x").gt(lit(5.0))))?
        .finish()?;
    let result = prepared
        .query(&[global_counts, local_counts], &[local], &changed)
        .await?;
    assert_eq!(
        result
            .scope(&regions)?
            .get(&regions.key([ScalarValue::from("East")])?)
            .unwrap()
            .table(&local_counts)?
            .num_rows(),
        1
    );
    assert!(!result
        .report()
        .executed_nodes
        .iter()
        .any(|n| n.contains("extent") || n.contains("parameters")));
    Ok(())
}

#[tokio::test]
async fn supplied_volatile_functions_are_not_eagerly_evaluated_or_cached() -> Result<()> {
    use datafusion::logical_expr::{create_udf, ColumnarValue, LogicalPlanBuilder, Volatility};
    use std::sync::atomic::{AtomicI64, Ordering};
    let calls = Arc::new(AtomicI64::new(0));
    let counter = calls.clone();
    let udf = create_udf(
        "draw",
        vec![],
        DataType::Int64,
        Volatility::Volatile,
        Arc::new(move |_| {
            Ok(ColumnarValue::Scalar(ScalarValue::Int64(Some(
                counter.fetch_add(1, Ordering::SeqCst) + 1,
            ))))
        }),
    );
    let mut b = DataflowBuilder::new();
    let rows = t::formula(
        LogicalPlanBuilder::empty(true).build()?,
        udf.call(vec![]),
        "x",
    )?;
    let rows = b.add_plan("draw", rows)?;
    let values = b.add_plan("values", t::filter(rows.plan_ref(), tf::truthy(col("x")))?)?;
    let values = b.table_output("values", &values)?;
    let prepared = runtime()?.prepare(&b.finish()?).await?;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let inputs = prepared.inputs().finish()?;
    for n in 1..=2 {
        let result = prepared.query(&[values], &[], &inputs).await?;
        assert_eq!(
            ScalarValue::try_from_array(result.table(&values)?.batches()[0].column(0), 0)?,
            ScalarValue::Int64(Some(n))
        );
        assert!(result
            .report()
            .executed_nodes
            .iter()
            .any(|name| name == "draw"));
    }
    Ok(())
}
