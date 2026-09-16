#[path = "common/scoped.rs"]
mod scoped;
use avenger_datafusion_dataflow::{
    arrow::datatypes::DataType,
    datafusion::{
        common::ScalarValue,
        logical_expr::{col, LogicalPlanBuilder},
    },
    DataflowBuilder, Result, Runtime, RuntimeConfig,
};

#[tokio::test]
async fn local_overrides_reuse_siblings_and_table_replacement_invalidates_partitions() -> Result<()>
{
    let mut b = DataflowBuilder::new();
    let input = b.table_input("sales", scoped::schema())?;
    let (regions, (cutoff, rows)) =
        b.partition_by("regions", input.plan_ref(), vec![col("region")], |s| {
            let cutoff = s.scalar_input("cutoff", DataType::Int64)?;
            let filtered = s.add_plan(
                "filtered",
                LogicalPlanBuilder::from(s.rows().plan_ref())
                    .filter(col("amount").gt(cutoff.expr_ref()))?
                    .build()?,
            )?;
            Ok((cutoff, s.table_output("rows", &filtered)?))
        })?;
    let p = Runtime::new(RuntimeConfig::default())?
        .prepare(&b.finish()?)
        .await?;
    let inputs = p
        .inputs()
        .table(&input, scoped::sales())?
        .scope_defaults(&regions, |b| b.scalar(&cutoff, 0_i64.into()))?
        .finish()?;
    p.query(&[rows], &[], &inputs).await?;
    let east = regions.instance([ScalarValue::from("East")])?;
    let changed = inputs
        .edit()
        .at(&east, |b| b.scalar(&cutoff, 100_i64.into()))?
        .finish()?;
    let result = p.query(&[rows], &[], &changed).await?;
    assert_eq!(result.report().cache_hits, 2); // Discovery and West.
    assert_eq!(result.report().physical_plans, 1);
    assert_eq!(
        result
            .scope(&regions)?
            .get(&regions.key([ScalarValue::from("East")])?)
            .unwrap()
            .table(&rows)?
            .num_rows(),
        0
    );
    let inherited = inputs
        .edit()
        .at(&east, |b| b.scalar(&cutoff, 0_i64.into()))?
        .finish()?;
    assert_eq!(
        p.query(&[rows], &[], &inherited)
            .await?
            .report()
            .physical_plans,
        0
    );
    let replaced = inherited.edit().table(&input, scoped::sales())?.finish()?;
    assert_eq!(
        p.query(&[rows], &[], &replaced).await?.report().cache_hits,
        0
    );
    Ok(())
}
