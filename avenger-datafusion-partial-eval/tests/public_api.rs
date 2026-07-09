//! Integration tests exercising ONLY the public API, written from the
//! perspective of an external consumer (e.g., a server that bakes a plan and
//! ships the residual to an environment with no access to the original
//! sources).

use std::sync::Arc;

use arrow::array::{Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use arrow::util::pretty::pretty_format_batches;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion_common::ScalarValue;

use avenger_datafusion_partial_eval::{
    PartialEvalPolicy, SkipReason, partial_evaluate, partial_evaluate_set,
};

fn sales_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["EU", "EU", "NA", "NA", "APAC"])) as _,
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _,
        ],
    )
    .unwrap()
}

async fn server_ctx_and_plan(sql: &str) -> (SessionContext, LogicalPlan) {
    let ctx = SessionContext::new();
    ctx.register_batch("sales", sales_batch()).unwrap();
    let plan = ctx.sql(sql).await.unwrap().logical_plan().clone();
    (ctx, plan)
}

fn sorted_rows(batches: &[RecordBatch]) -> Vec<String> {
    let mut lines = pretty_format_batches(batches)
        .unwrap()
        .to_string()
        .lines()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    lines.sort();
    lines
}

/// The flagship consumer scenario: bake on a "server" session that has the
/// data, then bind params and execute the residual in a FRESH session with no
/// tables registered. The baked residual must be self-contained.
#[tokio::test]
async fn bake_on_server_execute_residual_in_fresh_session() {
    let (server_ctx, plan) = server_ctx_and_plan(
        "SELECT * FROM (SELECT region, sum(value) AS total \
         FROM sales GROUP BY region) q WHERE total > $min",
    )
    .await;

    let output = partial_evaluate(plan.clone(), &server_ctx, &PartialEvalPolicy::default())
        .await
        .unwrap();

    // Report shape: the aggregate folded, the original source is recorded,
    // and the live param is inventoried for the consumer.
    assert_eq!(output.report.baked.len(), 1);
    assert!(output.report.baked[0].rows > 0);
    assert!(output.report.baked[0].bytes > 0);
    assert_eq!(output.report.baked[0].source_tables, vec!["sales"]);
    assert_eq!(output.report.source_tables, vec!["sales".to_string()]);
    assert_eq!(output.report.remaining_params, vec!["$min".to_string()]);

    // "Client": a fresh context that has never seen the `sales` table.
    let client_ctx = SessionContext::new();
    for min in [2.5_f64, 4.5_f64] {
        let params = vec![("min", ScalarValue::Float64(Some(min)))];

        let expected = DataFrame::new(server_ctx.state(), plan.clone())
            .with_param_values(params.clone())
            .unwrap()
            .collect()
            .await
            .unwrap();
        let actual = DataFrame::new(client_ctx.state(), output.residual.clone())
            .with_param_values(params)
            .unwrap()
            .collect()
            .await
            .unwrap();

        assert_eq!(sorted_rows(&expected), sorted_rows(&actual), "min={min}");
    }
}

/// Fixed params bind before folding: the specialized plan folds completely,
/// the report distinguishes applied/unused/remaining params, and `$`-prefix
/// normalization is accepted on input.
#[tokio::test]
async fn fixed_params_fold_more_and_are_inventoried() {
    let sql = "SELECT * FROM (SELECT region, sum(value) AS total \
               FROM sales GROUP BY region) q WHERE total > $min";

    let (ctx, plan) = server_ctx_and_plan(sql).await;
    let live = partial_evaluate(plan.clone(), &ctx, &PartialEvalPolicy::default())
        .await
        .unwrap();
    assert_eq!(live.report.remaining_params, vec!["$min".to_string()]);

    let policy = PartialEvalPolicy {
        fixed_params: vec![
            ("min".to_string(), ScalarValue::Float64(Some(4.5))),
            ("$ghost".to_string(), ScalarValue::Float64(Some(0.0))),
        ],
        ..PartialEvalPolicy::default()
    };
    let fixed = partial_evaluate(plan.clone(), &ctx, &policy).await.unwrap();

    assert!(fixed.report.remaining_params.is_empty());
    assert_eq!(
        fixed.report.fixed_params_applied,
        vec![("min".to_string(), ScalarValue::Float64(Some(4.5)))]
    );
    assert_eq!(fixed.report.unused_fixed_params, vec!["$ghost".to_string()]);

    // The specialized residual needs no params and matches the original plan
    // bound to the same value.
    let expected = DataFrame::new(ctx.state(), plan)
        .with_param_values(vec![("min", ScalarValue::Float64(Some(4.5)))])
        .unwrap()
        .collect()
        .await
        .unwrap();
    let actual = DataFrame::new(SessionContext::new().state(), fixed.residual)
        .collect()
        .await
        .unwrap();
    assert_eq!(sorted_rows(&expected), sorted_rows(&actual));
}

/// `partial_evaluate_set` shares one bake registry across plans (identical
/// subtrees materialize once), and policy-excluded tables stay symbolic in
/// the residual with an `ExcludedTable` skip recorded.
#[tokio::test]
async fn set_dedups_shared_bakes_and_respects_excluded_tables() {
    let sql = "SELECT region, sum(value) AS total FROM sales GROUP BY region";
    let (ctx, plan) = server_ctx_and_plan(sql).await;

    // Two identical param-free plans: one baked table, two occurrences.
    let (residuals, report) = partial_evaluate_set(
        vec![plan.clone(), plan.clone()],
        &ctx,
        &PartialEvalPolicy::default(),
    )
    .await
    .unwrap();
    assert_eq!(residuals.len(), 2);
    assert_eq!(report.baked.len(), 1);
    assert_eq!(report.baked[0].occurrences, 2);

    // Excluded table: the scan stays symbolic and is reported.
    let policy = PartialEvalPolicy {
        unfoldable_tables: ["sales".to_string()].into_iter().collect(),
        ..PartialEvalPolicy::default()
    };
    let output = partial_evaluate(plan, &ctx, &policy).await.unwrap();
    assert!(output.report.baked.is_empty());
    assert!(
        output
            .report
            .skipped
            .iter()
            .any(|skip| skip.reason == SkipReason::ExcludedTable)
    );
    let display = output.residual.display_indent().to_string();
    assert!(display.contains("sales"), "{display}");
}
