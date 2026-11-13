//! Helpers that the tests share.

#![allow(dead_code)]

pub mod calls;

use std::sync::Arc;

use datafusion::arrow::array::{ArrayRef, RecordBatch};
use datafusion::arrow::datatypes::{Field, Schema};
use datafusion::catalog::MemTable;
use datafusion::common::Result;
use datafusion::prelude::SessionContext;

/// A session with the transforms, without the optimizer rules that `register` adds if not
/// `rules`, so that the window functions run through their own evaluators.
pub fn session(rules: bool) -> SessionContext {
    let ctx = SessionContext::new();
    let before: Vec<String> = ctx
        .state()
        .optimizers()
        .iter()
        .map(|r| r.name().to_string())
        .collect();
    avenger_datafusion_transforms::register(&ctx).expect("register the transforms");
    if !rules {
        let added: Vec<String> = ctx
            .state()
            .optimizers()
            .iter()
            .map(|r| r.name().to_string())
            .filter(|n| !before.contains(n))
            .collect();
        for name in added {
            ctx.remove_optimizer_rule(&name);
        }
    }
    ctx
}

/// Registers the columns as the table `name`, in place of any table of that name.
pub fn register(ctx: &SessionContext, name: &str, columns: Vec<(&str, ArrayRef)>) -> Result<()> {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(n, a)| Field::new(*n, a.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(
        schema.clone(),
        columns.into_iter().map(|(_, a)| a).collect(),
    )?;
    ctx.deregister_table(name)?;
    ctx.register_table(
        name,
        Arc::new(MemTable::try_new(schema, vec![vec![batch]])?),
    )?;
    Ok(())
}

/// The error of planning and running a query, or None.
pub async fn error(ctx: &SessionContext, sql: &str) -> Option<String> {
    match calls::sql(ctx, sql).await {
        Ok(df) => df.collect().await.err().map(|e| e.to_string()),
        Err(e) => Some(e.to_string()),
    }
}
