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

#[tokio::test]
async fn expression_overrides_preserve_nested_instances_and_round_trip() -> Result<()> {
    use avenger_datafusion_dataflow::{datafusion::logical_expr::lit, InputHandle};
    let mut b = DataflowBuilder::new();
    let source = b.table_snapshot("sales", scoped::sales())?;
    let global = b.expr_input("global", DataType::Boolean)?;
    let (regions, (regional, years, local, rows)) =
        b.partition_by("regions", source.plan_ref(), vec![col("region")], |s| {
            let regional = s.expr_input("selection", DataType::Boolean)?;
            let (years, (local, rows)) =
                s.partition_by("years", s.rows().plan_ref(), vec![col("year")], |s| {
                    let local = s.expr_input("selection", DataType::Boolean)?;
                    let filtered = s.add_plan(
                        "filtered",
                        LogicalPlanBuilder::from(s.rows().plan_ref())
                            .filter(
                                global
                                    .expr_ref()
                                    .and(regional.expr_ref())
                                    .and(local.expr_ref()),
                            )?
                            .build()?,
                    )?;
                    Ok((local, s.table_output("rows", &filtered)?))
                })?;
            Ok((regional, years, local, rows))
        })?;
    let flow = b.finish()?;
    let runtime = Runtime::new(Default::default())?;
    let p = runtime.prepare(&flow).await?;
    let inputs = p
        .inputs()
        .expr(&global, col("amount").gt(lit(0_i64)))?
        .scope_defaults(&regions, |b| b.expr(&regional, lit(true)))?
        .scope_defaults(&years, |b| b.expr(&local, lit(true)))?
        .finish()?;
    let first = p.query(&[rows], &[], &inputs).await?;
    let east = regions.instance([ScalarValue::from("East")])?;
    let east_2025 = east.child(&years, [2025_i32.into()])?;
    let absent = regions
        .instance([ScalarValue::from("Absent")])?
        .child(&years, [2025_i32.into()])?;
    let edited = inputs
        .edit()
        .at(&east_2025, |b| {
            b.expr(&local, col("amount").gt(lit(50_i64)))
        })?
        .at(&absent, |b| b.expr(&local, lit(false)))?
        .finish()?;
    let result = p.query(&[rows], &[], &edited).await?;
    assert_eq!(result.report().physical_plans, 1);
    assert!(result.report().cache_hits >= 3);
    let east_key = regions.key([ScalarValue::from("East")])?;
    let year_key = years.key([2025_i32.into()])?;
    let values = |result: &avenger_datafusion_dataflow::DataflowResult| -> Result<Vec<i64>> {
        Ok(scoped::amounts(
            result
                .scope(&regions)?
                .get(&east_key)
                .unwrap()
                .scope(&years)?
                .get(&year_key)
                .unwrap()
                .table(&rows)?,
        ))
    };
    assert_eq!(values(&first)?, vec![40, 80]);
    assert_eq!(values(&result)?, vec![80]);
    let restored = edited
        .edit()
        .at(&east_2025, |b| b.unset_expr(&local))?
        .finish()?;
    assert_eq!(
        p.query(&[rows], &[], &restored)
            .await?
            .report()
            .physical_plans,
        0
    );
    assert!(inputs
        .edit()
        .scope_defaults(&regions, |b| b.expr(&local, lit(true)))
        .is_err());
    let missing = inputs
        .edit()
        .scope_defaults(&years, |b| b.unset_expr(&local))?
        .finish()?;
    assert!(p.query(&[rows], &[], &missing).await.is_err());
    let decoded = runtime.decode_dataflow(&flow.to_bytes()?)?;
    let names = decoded.interface().root();
    let region_names = names.scope("regions")?;
    let year_names = region_names.scope("years")?;
    assert_eq!(
        region_names
            .inputs()
            .map(|i| i.name().to_string())
            .collect::<Vec<_>>(),
        ["selection"]
    );
    assert!(matches!(
        year_names.inputs().next(),
        Some(InputHandle::Expr(_))
    ));
    let p = runtime.prepare(&decoded).await?;
    let bound = p
        .inputs()
        .expr(&names.expr_input("global")?, lit(true))?
        .scope_defaults(region_names.handle().unwrap(), |b| {
            b.expr(&region_names.expr_input("selection")?, lit(true))
        })?
        .scope_defaults(year_names.handle().unwrap(), |b| {
            b.expr(
                &year_names.expr_input("selection")?,
                col("amount").gt(lit(50_i64)),
            )
        })?
        .finish()?;
    let out = year_names.table_output("rows")?;
    let result = p.query(&[out], &[], &bound).await?;
    assert_eq!(result.scope(region_names.handle().unwrap())?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn expression_partition_keys_invalidate_discovery_and_composite_addresses() -> Result<()> {
    use avenger_datafusion_dataflow::datafusion::logical_expr::lit;
    let mut b = DataflowBuilder::new();
    let source = b.table_snapshot("sales", scoped::sales())?;
    let key = b.expr_input("key", DataType::Utf8)?;
    let (panels, rows) = b.partition_by(
        "panels",
        source.plan_ref(),
        vec![key.expr_ref(), col("year")],
        |s| s.table_output("rows", &s.rows()),
    )?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let input = p.inputs().expr(&key, col("region"))?.finish()?;
    let first = p.query(&[rows], &[], &input).await?;
    assert_eq!(first.scope(&panels)?.len(), 3);
    let one = input.edit().expr(&key, lit("All"))?.finish()?;
    let second = p.query(&[rows], &[], &one).await?;
    assert_eq!(second.scope(&panels)?.len(), 2);
    let panel_key = panels.key([ScalarValue::from("All"), 2025_i32.into()])?;
    assert_eq!(
        second
            .scope(&panels)?
            .get(&panel_key)
            .unwrap()
            .table(&rows)?
            .num_rows(),
        4
    );
    Ok(())
}
