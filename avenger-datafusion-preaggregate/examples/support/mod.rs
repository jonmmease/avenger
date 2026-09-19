use avenger_datafusion_preaggregate::{BoundQuery, PreparedQuery, QueryPolicy};
use datafusion::{
    arrow::{
        array::{BooleanArray, Float64Array, Int32Array, StringArray},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    common::Result,
    datasource::MemTable,
    logical_expr::{col, lit},
    prelude::SessionContext,
    sql::unparser::Unparser,
};
use std::sync::Arc;

pub fn context() -> Result<SessionContext> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_from_iter(vec![
        (
            "airline",
            Arc::new(StringArray::from(vec![
                "AA", "AA", "AA", "DL", "DL", "DL", "UA", "UA", "UA",
            ])) as _,
        ),
        (
            "delay",
            Arc::new(Int32Array::from(vec![0, 10, 20, 0, 10, 20, 0, 10, 20])) as _,
        ),
        (
            "distance",
            Arc::new(Float64Array::from(vec![
                Some(500.0),
                Some(800.0),
                None,
                Some(900.0),
                Some(400.0),
                Some(600.0),
                Some(200.0),
                Some(700.0),
                Some(1100.0),
            ])) as _,
        ),
        (
            "domestic",
            Arc::new(BooleanArray::from(vec![
                true, true, false, false, true, true, true, false, true,
            ])) as _,
        ),
    ])?;
    ctx.register_table(
        "flights",
        Arc::new(MemTable::try_new(batch.schema(), vec![vec![batch]])?),
    )?;
    Ok(ctx)
}

pub async fn demonstrate(ctx: &SessionContext, prepared: PreparedQuery) -> Result<()> {
    let report = prepared.explain();
    println!(
        "Prepared {} grouping dimension(s), {} retained dimension(s), and {} state measure(s).",
        report.grouping_dimensions.len(),
        report.retained_dimensions.len(),
        report.aggregate_states.len()
    );
    let unparser = Unparser::default().with_pretty(true);
    let materialization = prepared
        .materialization_plan()
        .expect("this example uses an eligible query");
    println!(
        "\nWarm-up SQL (all delay cells):\n{:#}",
        unparser.plan_to_sql(materialization)?
    );
    let schema = Arc::new(materialization.schema().as_arrow().clone());
    let batches = ctx
        .execute_logical_plan(materialization.clone())
        .await?
        .collect()
        .await?;
    println!(
        "Materialized {} rows once.\n",
        batches.iter().map(|b| b.num_rows()).sum::<usize>()
    );
    // Preserve the declared schema even when the materialization has no batches.
    ctx.register_table(
        "stored_states",
        Arc::new(MemTable::try_new(schema, vec![batches])?),
    )?;
    let stored = ctx.table("stored_states").await?.into_unoptimized_plan();
    for (lower, upper) in [(0, 30), (0, 15), (10, 25), (100, 200)] {
        let predicate = col("delay")
            .gt_eq(lit(lower))
            .and(col("delay").lt(lit(upper)));
        let bound = prepared.bind(predicate.clone())?;
        println!("Delay [{lower}, {upper}): {:?}", bound.diagnostics());
        let BoundQuery::Preaggregated { rollup, .. } = bound else {
            unreachable!()
        };
        assert_eq!(
            rollup.materialization_schema().as_arrow(),
            stored.schema().as_arrow()
        );
        let plan = rollup.with_materialization(stored.clone())?;
        println!("Rollup SQL:\n{:#}", unparser.plan_to_sql(&plan)?);
        let actual = ctx.execute_logical_plan(plan).await?.collect().await?;
        let BoundQuery::Direct { plan, .. } =
            prepared.bind_with_policy(predicate, QueryPolicy::ForceDirect)?
        else {
            unreachable!()
        };
        let expected = ctx.execute_logical_plan(plan).await?.collect().await?;
        let actual = pretty_format_batches(&actual)?.to_string();
        assert_eq!(actual, pretty_format_batches(&expected)?.to_string());
        println!("{actual}\nMatches direct execution.\n");
    }
    Ok(())
}
