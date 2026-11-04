//! Runs return the rows of the uncut plans, keep their results, and hit them.

mod common;

use avenger_query_graph::{Inputs, Outcome, QueryGraph, ResultStore, Run, StoreConfig};
use avenger_selection::SelectionState;
use common::{context, exact, inputs, outcomes, plan, rows, state, store, uncut};
use datafusion::{logical_expr::LogicalPlan, prelude::SessionContext, scalar::ScalarValue};

/// A graph of the plans `sqls`, with the plans.
async fn graph(
    ctx: &SessionContext,
    store: &ResultStore,
    sqls: &[&str],
) -> (QueryGraph, Vec<LogicalPlan>) {
    let mut plans = Vec::new();
    for sql in sqls {
        plans.push(plan(ctx, sql).await);
    }
    (store.compile(&ctx.state(), plans.clone()).unwrap(), plans)
}

/// Run the graph and check each plan's rows against the plan run uncut.
async fn run(
    ctx: &SessionContext,
    graph: &QueryGraph,
    plans: &[LogicalPlan],
    inputs: &Inputs,
) -> Run {
    let run = graph.run(inputs).await.unwrap();
    for (batches, plan) in run.batches.iter().zip(plans) {
        let expected = uncut(ctx, plan, inputs).await;
        assert!(!expected.is_empty());
        assert_eq!(rows(batches), expected);
    }
    run
}

fn year(year: i64) -> (&'static str, ScalarValue) {
    ("year", ScalarValue::from(year))
}

const EXAMPLE: &str = "SELECT carrier, count(*) AS n FROM flights \
                       WHERE year = $year AND selected($brush) GROUP BY carrier";

#[tokio::test]
async fn the_example_keeps_and_widens_its_nodes() {
    let ctx = context();
    let store = store();
    let (graph, plans) = graph(&ctx, &store, &[EXAMPLE]).await;
    let brush = |low, high| state(vec![("delay", exact("delay", low, high))]);
    let empty = SelectionState::default();

    let first = run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[year(2023)], &[("brush", empty.clone())]),
    )
    .await;
    assert_eq!(outcomes(&first), ["computed", "computed"]);
    // The first box tests `delay`, which the kept year filter lacks.
    let boxed = run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[year(2023)], &[("brush", brush(0.0, 40.0))]),
    )
    .await;
    assert_eq!(outcomes(&boxed), ["widened", "computed"]);
    let Outcome::Computed { widened, .. } = &boxed.outcomes[0] else {
        unreachable!()
    };
    assert_eq!(widened, &["delay"]);
    let moved = run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[year(2023)], &[("brush", brush(10.0, 60.0))]),
    )
    .await;
    assert_eq!(outcomes(&moved), ["hit", "computed"]);
    let other = run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[year(2024)], &[("brush", brush(10.0, 60.0))]),
    )
    .await;
    assert_eq!(outcomes(&other), ["computed", "computed"]);
    // A slider that toggles back hits the top node, and reads nothing below it.
    let back = run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[year(2023)], &[("brush", brush(10.0, 60.0))]),
    )
    .await;
    assert_eq!(outcomes(&back), ["not read", "hit"]);
}

#[tokio::test]
async fn a_cross_filter_reruns_only_the_plots_that_read_the_move() {
    let ctx = context();
    let store = store();
    let carriers = "SELECT carrier, count(*) AS n FROM flights \
                    WHERE selected($s, exclude_own => true, owner => 'carriers') GROUP BY carrier";
    let delays = "SELECT floor(delay / 20) AS bin, count(*) AS n FROM flights \
                  WHERE selected($s, exclude_own => true, owner => 'delays') \
                  GROUP BY floor(delay / 20)";
    let (graph, plans) = graph(&ctx, &store, &[carriers, delays]).await;
    let brush = |low, high| state(vec![("delays", exact("delay", low, high))]);
    run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[], &[("s", brush(0.0, 30.0))]),
    )
    .await;
    let moved = run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[], &[("s", brush(20.0, 80.0))]),
    )
    .await;
    let tops = graph.nodes(None).unwrap();
    let delay_top = (tops.iter()).position(|node| node.top_of() == [1]).unwrap();
    let carrier_top = (tops.iter()).position(|node| node.top_of() == [0]).unwrap();
    assert!(matches!(moved.outcomes[delay_top], Outcome::Hit(_)));
    assert!(matches!(
        moved.outcomes[carrier_top],
        Outcome::Computed { .. }
    ));
}

#[tokio::test]
async fn one_branch_of_a_union_reruns_when_its_parameter_changes() {
    let ctx = context();
    let store = store();
    let sql = "SELECT carrier, count(*) AS n FROM flights WHERE year = $a GROUP BY carrier \
               UNION ALL \
               SELECT carrier, count(*) AS n FROM flights WHERE delay > $b GROUP BY carrier";
    let (graph, plans) = graph(&ctx, &store, &[sql]).await;
    let values = |a: i64, b: f64| {
        inputs(
            &[("a", ScalarValue::from(a)), ("b", ScalarValue::from(b))],
            &[],
        )
    };
    run(&ctx, &graph, &plans, &values(2023, 10.0)).await;
    let changed = run(&ctx, &graph, &plans, &values(2023, 50.0)).await;
    assert_eq!(outcomes(&changed), ["hit", "computed", "computed"]);
}

#[tokio::test]
async fn the_budget_evicts_the_least_recently_used_results() {
    let ctx = context();
    let sql = "SELECT carrier, count(*) AS n FROM flights WHERE year = $year GROUP BY carrier";
    // A budget of two and a half results.
    let (probe, _) = graph(&ctx, &common::store(), &[sql]).await;
    let Outcome::Computed { output, .. } = &probe
        .run(&inputs(&[year(2022)], &[]))
        .await
        .unwrap()
        .outcomes[0]
    else {
        unreachable!()
    };
    let store = ResultStore::new(StoreConfig {
        budget: output.bytes * 5 / 2,
        max_result: output.bytes * 2,
    });
    let (graph, plans) = graph(&ctx, &store, &[sql]).await;
    for value in [2022, 2023, 2024] {
        run(&ctx, &graph, &plans, &inputs(&[year(value)], &[])).await;
    }
    let latest = run(&ctx, &graph, &plans, &inputs(&[year(2024)], &[])).await;
    assert_eq!(outcomes(&latest), ["hit"]);
    let oldest = run(&ctx, &graph, &plans, &inputs(&[year(2022)], &[])).await;
    assert_eq!(outcomes(&oldest), ["computed"]);
}

#[tokio::test]
async fn a_result_over_the_limit_is_used_and_not_kept() {
    let ctx = context();
    let store = ResultStore::new(StoreConfig {
        budget: 64 << 20,
        max_result: 64,
    });
    let sql = "SELECT carrier, delay FROM flights WHERE year = $year";
    let (graph, plans) = graph(&ctx, &store, &[sql]).await;
    for _ in 0..2 {
        let run = run(&ctx, &graph, &plans, &inputs(&[year(2023)], &[])).await;
        assert_eq!(outcomes(&run), ["uncached"]);
    }
}

#[tokio::test]
async fn a_volatile_plan_computes_on_every_run() {
    let ctx = context();
    let store = store();
    let sql = "SELECT carrier, now() AS at FROM flights WHERE year = $year";
    let (graph, _) = graph(&ctx, &store, &[sql]).await;
    let values = inputs(&[year(2023)], &[]);
    // The year filter below the call is kept, and the projection that calls `now()` is not.
    let first = graph.run(&values).await.unwrap();
    assert_eq!(outcomes(&first), ["computed", "uncached"]);
    let second = graph.run(&values).await.unwrap();
    assert_eq!(outcomes(&second), ["hit", "uncached"]);
    assert_ne!(graph.key(&values).unwrap(), graph.key(&values).unwrap());
}

#[tokio::test]
async fn keys_change_only_with_what_the_plans_read() {
    let ctx = context();
    let store = store();
    let (graph, _) = graph(&ctx, &store, &[EXAMPLE]).await;
    let key = |value, owner: &str| {
        let brush = state(vec![(owner, exact("delay", 0.0, 40.0))]);
        graph
            .key(&inputs(
                &[year(value), ("unused", ScalarValue::from(1))],
                &[("brush", brush)],
            ))
            .unwrap()
    };
    assert_eq!(key(2023, "delays"), key(2023, "delays"));
    assert_ne!(key(2023, "delays"), key(2024, "delays"));
    assert_ne!(key(2023, "delays"), key(2023, "other"));
}

#[tokio::test]
async fn a_changed_table_misses_every_node_above_it() {
    let ctx = context();
    let store = store();
    let (graph, plans) = graph(&ctx, &store, &[EXAMPLE]).await;
    let values = inputs(&[year(2023)], &[("brush", SelectionState::default())]);
    run(&ctx, &graph, &plans, &values).await;
    store.table_changed("flights");
    let changed = run(&ctx, &graph, &plans, &values).await;
    assert_eq!(outcomes(&changed), ["computed", "computed"]);
}

#[tokio::test]
async fn a_missing_input_fails_the_run_and_names_it() {
    let ctx = context();
    let store = store();
    let (graph, _) = graph(&ctx, &store, &[EXAMPLE]).await;
    let error = graph
        .run(&inputs(&[], &[("brush", SelectionState::default())]))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("$year"), "{error}");
    let error = graph.run(&inputs(&[year(2023)], &[])).await.unwrap_err();
    assert!(error.to_string().contains("$brush"), "{error}");
}
