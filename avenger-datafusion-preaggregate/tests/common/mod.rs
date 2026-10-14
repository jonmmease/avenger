#![allow(dead_code)]
#[path = "../../../avenger-datafusion-aggregate-state/tests/common/mod.rs"]
mod fixture;
use avenger_datafusion_preaggregate::*;
use datafusion::{
    common::Result,
    datasource::{view::ViewTable, MemTable},
    logical_expr::{Expr, LogicalPlan},
    prelude::SessionContext,
};
pub use fixture::{context, rows};
use std::sync::Arc;

pub async fn query(ctx: &SessionContext, sql: &str) -> Result<FilterQuery> {
    let builder = FilterQuery::builder(ctx.table("t").await?.into_unoptimized_plan());
    ctx.register_table("rows", Arc::new(ViewTable::new(builder.rows(), None)))?;
    let plan = ctx.sql(sql).await?.into_unoptimized_plan();
    ctx.deregister_table("rows")?;
    builder.finish(plan)
}
pub async fn compare(
    ctx: &SessionContext,
    prepared: &PreparedQuery,
    predicate: Expr,
) -> Result<()> {
    compare_impl(ctx, prepared, predicate, false).await
}
pub async fn compare_ordered(
    ctx: &SessionContext,
    prepared: &PreparedQuery,
    predicate: Expr,
) -> Result<()> {
    compare_impl(ctx, prepared, predicate, true).await
}
async fn compare_impl(
    ctx: &SessionContext,
    prepared: &PreparedQuery,
    predicate: Expr,
    ordered: bool,
) -> Result<()> {
    let direct = prepared.bind_with_policy(predicate.clone(), QueryPolicy::ForceDirect)?;
    let BoundQuery::Direct { plan: direct, .. } = direct else {
        unreachable!()
    };
    let BoundQuery::Preaggregated {
        materialization,
        rollup,
        ..
    } = prepared.bind(predicate)?
    else {
        panic!("ineligible: {:?}", prepared.explain())
    };
    let schema = Arc::new(materialization.schema().as_arrow().clone());
    let batches = ctx
        .execute_logical_plan(materialization)
        .await?
        .collect()
        .await?;
    let stored = ctx
        .read_table(Arc::new(MemTable::try_new(schema, vec![batches])?))?
        .into_unoptimized_plan();
    let actual: LogicalPlan = rollup.with_materialization(stored)?;
    assert_eq!(actual.schema(), direct.schema());
    let expected = ctx.execute_logical_plan(direct).await?.collect().await?;
    let actual = ctx.execute_logical_plan(actual).await?.collect().await?;
    let mut a = rows(&actual)?;
    let mut b = rows(&expected)?;
    if !ordered {
        a.sort_by(|x, y| x.partial_cmp(y).unwrap());
        b.sort_by(|x, y| x.partial_cmp(y).unwrap());
    }
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(&b) {
        for (a, b) in a.iter().zip(b) {
            match (a, b) {
                (
                    datafusion::common::ScalarValue::Float64(Some(a)),
                    datafusion::common::ScalarValue::Float64(Some(b)),
                ) => assert!(
                    a == b || a.is_nan() && b.is_nan() || (a - b).abs() < 1e-9 * (1.0 + b.abs()),
                    "{a} != {b}"
                ),
                _ => assert_eq!(a, b),
            }
        }
    }
    Ok(())
}
