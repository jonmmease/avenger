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

#[tokio::test]
async fn a_limit_above_a_cut_keeps_the_rows_of_the_sorted_order() {
    // Batches of 16 rows in two partitions, so a cut that lost the order would change the rows
    // that the limit keeps.
    let ctx = SessionContext::new_with_config(
        datafusion::prelude::SessionConfig::new()
            .with_batch_size(16)
            .with_target_partitions(2),
    );
    ctx.register_batch("flights", common::flights()).unwrap();
    let store = store();
    let (graph, plans) = graph(
        &ctx,
        &store,
        &["SELECT delay FROM flights ORDER BY delay LIMIT $n"],
    )
    .await;
    run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[("n", ScalarValue::from(40i64))], &[]),
    )
    .await;
    run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[("n", ScalarValue::from(25i64))], &[]),
    )
    .await;
}

#[tokio::test]
async fn plans_over_different_providers_of_one_name_are_distinct() {
    let ctx = SessionContext::new();
    let table = |value: i64| {
        datafusion::arrow::array::RecordBatch::try_from_iter(vec![(
            "value",
            std::sync::Arc::new(datafusion::arrow::array::Int64Array::from(vec![value])) as _,
        )])
        .unwrap()
    };
    ctx.register_batch("t", table(1)).unwrap();
    let first = plan(&ctx, "SELECT * FROM t").await;
    ctx.deregister_table("t").unwrap();
    ctx.register_batch("t", table(2)).unwrap();
    let second = plan(&ctx, "SELECT * FROM t").await;
    let graph = store().compile(&ctx.state(), vec![first, second]).unwrap();
    let run = graph.run(&Inputs::default()).await.unwrap();
    assert_eq!(rows(&run.batches[0]), ["1"]);
    assert_eq!(rows(&run.batches[1]), ["2"]);
}

#[tokio::test]
async fn a_function_named_like_a_selection_function_reads_its_parameter() {
    let ctx = SessionContext::new();
    ctx.register_udf(datafusion::logical_expr::create_udf(
        "in_keys",
        vec![datafusion::arrow::datatypes::DataType::Int64],
        datafusion::arrow::datatypes::DataType::Int64,
        datafusion::logical_expr::Volatility::Immutable,
        std::sync::Arc::new(|args| Ok(args[0].clone())),
    ));
    let store = store();
    let (graph, plans) = graph(&ctx, &store, &["SELECT in_keys($n) AS n"]).await;
    for value in [1i64, 2] {
        run(
            &ctx,
            &graph,
            &plans,
            &inputs(&[("n", ScalarValue::from(value))], &[]),
        )
        .await;
    }
}

#[tokio::test]
async fn a_recursive_query_is_not_cut_inside() {
    let ctx = SessionContext::new();
    let store = store();
    let sql = "WITH RECURSIVE t(n) AS (VALUES (1) UNION ALL \
               SELECT n + 1 FROM t WHERE n > 0 AND n < $max) SELECT * FROM t";
    let (graph, plans) = graph(&ctx, &store, &[sql]).await;
    assert_eq!(graph.nodes(None).unwrap().len(), 1);
    run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[("max", ScalarValue::from(3i64))], &[]),
    )
    .await;
}

#[tokio::test]
async fn a_cut_that_the_optimizer_removes_does_not_run() {
    let ctx = context();
    let store = store();
    // A false `$show` removes the cut below it, where the cast would fail for carriers such as
    // `UA`.
    let (graph, plans) = graph(
        &ctx,
        &store,
        &["SELECT delay FROM flights \
           WHERE CAST(carrier AS BIGINT) > 0 AND year = $year AND $show"],
    )
    .await;
    let values = inputs(
        &[
            ("year", ScalarValue::from(2023i64)),
            ("show", ScalarValue::from(false)),
        ],
        &[],
    );
    let result = graph.run(&values).await.unwrap();
    assert!(rows(&result.batches[0]).is_empty());
    assert_eq!(uncut(&ctx, &plans[0], &values).await, Vec::<String>::new());
    // The cast and the year filter are nodes below the removed cut, and neither runs.
    assert_eq!(outcomes(&result), ["not read", "not read", "computed"]);
}

#[tokio::test]
async fn a_result_over_the_budget_evicts_nothing() {
    let ctx = context();
    let count = "SELECT count(*) AS n FROM flights WHERE year = 2022";
    let (probe, _) = graph(&ctx, &common::store(), &[count]).await;
    let Outcome::Computed { output, .. } =
        &probe.run(&Inputs::default()).await.unwrap().outcomes[0]
    else {
        unreachable!()
    };
    let store = ResultStore::new(StoreConfig {
        budget: output.bytes * 2,
        max_result: 64 << 20,
    });
    let (small, _) = graph(&ctx, &store, &[count]).await;
    let (large, _) = graph(&ctx, &store, &["SELECT * FROM flights"]).await;
    small.run(&Inputs::default()).await.unwrap();
    assert_eq!(
        outcomes(&large.run(&Inputs::default()).await.unwrap()),
        ["uncached"]
    );
    assert_eq!(
        outcomes(&small.run(&Inputs::default()).await.unwrap()),
        ["hit"]
    );
}

#[tokio::test]
async fn a_limit_above_a_sort_by_an_expression_keeps_the_sorted_rows() {
    // The sort key is not an output column, so the lower node's plan declares no ordering,
    // although its one partition gives the rows in order.
    let ctx = SessionContext::new_with_config(
        datafusion::prelude::SessionConfig::new()
            .with_batch_size(16)
            .with_target_partitions(2),
    );
    ctx.register_batch("flights", common::flights()).unwrap();
    let store = store();
    let (graph, plans) = graph(
        &ctx,
        &store,
        &["SELECT carrier FROM flights ORDER BY delay + distance, carrier LIMIT $n"],
    )
    .await;
    assert_eq!(graph.nodes(None).unwrap().len(), 2);
    run(
        &ctx,
        &graph,
        &plans,
        &inputs(&[("n", ScalarValue::from(40i64))], &[]),
    )
    .await;
}

#[tokio::test]
async fn a_parameter_can_share_the_name_of_a_selection() {
    let mut ctx = SessionContext::new();
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    let store = store();
    let (graph, plans) = graph(&ctx, &store, &["SELECT selected($s) AND $s AS v"]).await;
    for value in [true, false] {
        let values = inputs(
            &[("s", ScalarValue::from(value))],
            &[("s", SelectionState::default())],
        );
        run(&ctx, &graph, &plans, &values).await;
    }
}

#[tokio::test]
async fn a_plan_without_columns_returns_its_rows() {
    let ctx = SessionContext::new();
    let plan = datafusion::logical_expr::LogicalPlanBuilder::empty(true)
        .build()
        .unwrap();
    let graph = store().compile(&ctx.state(), vec![plan]).unwrap();
    let run = graph.run(&Inputs::default()).await.unwrap();
    let rows: usize = run.batches[0].iter().map(|batch| batch.num_rows()).sum();
    assert_eq!(rows, 1);
}

/// A query planner that fails its first plan, by an error or a panic, and plans the others.
#[derive(Debug)]
struct FailsOnce {
    plans: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    panics: bool,
}

#[async_trait::async_trait]
impl datafusion::execution::context::QueryPlanner for FailsOnce {
    async fn create_physical_plan(
        &self,
        plan: &LogicalPlan,
        session: &dyn datafusion::catalog::Session,
    ) -> datafusion::common::Result<std::sync::Arc<dyn datafusion::physical_plan::ExecutionPlan>>
    {
        use datafusion::physical_planner::PhysicalPlanner;
        if self.plans.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            if self.panics {
                panic!("a planner that panics once");
            }
            return Err(datafusion::common::DataFusionError::Execution(
                "a planner that fails once".into(),
            ));
        }
        datafusion::physical_planner::DefaultPhysicalPlanner::default()
            .create_physical_plan(plan, session)
            .await
    }
}

/// A failed computation leaves the store, so the next run computes the node again.
async fn retries_after_a_failure(panics: bool) {
    let plans = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let state = datafusion::execution::session_state::SessionStateBuilder::new()
        .with_default_features()
        .with_query_planner(std::sync::Arc::new(FailsOnce {
            plans: plans.clone(),
            panics,
        }))
        .build();
    let ctx = SessionContext::new_with_state(state);
    let graph = store()
        .compile(&ctx.state(), vec![plan(&ctx, "SELECT 1 AS n").await])
        .unwrap();
    assert!(graph.run(&Inputs::default()).await.is_err());
    graph.run(&Inputs::default()).await.unwrap();
    assert_eq!(plans.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_computation_that_fails_leaves_the_store() {
    retries_after_a_failure(false).await;
}

#[tokio::test]
async fn a_computation_that_panics_leaves_the_store() {
    retries_after_a_failure(true).await;
}

#[tokio::test]
async fn a_new_version_frees_the_results_of_the_old_one() {
    let ctx = context();
    let flights = "SELECT carrier, count(*) AS n FROM flights GROUP BY carrier";
    let carriers = "SELECT code, count(*) AS n FROM carriers GROUP BY code";
    let bytes = |run: &Run| match &run.outcomes[0] {
        Outcome::Computed { output, .. } => output.bytes,
        _ => unreachable!(),
    };
    let mut measured = Vec::new();
    for sql in [flights, carriers] {
        let (probe, _) = graph(&ctx, &common::store(), &[sql]).await;
        measured.push(bytes(&probe.run(&Inputs::default()).await.unwrap()));
    }
    let (a, b) = (measured[0], measured[1]);
    // The budget holds one result of each plan, and not a third result.
    let store = ResultStore::new(StoreConfig {
        budget: a + b + a.min(b) / 2,
        max_result: 64 << 20,
    });
    let (of_carriers, _) = graph(&ctx, &store, &[carriers]).await;
    let (of_flights, _) = graph(&ctx, &store, &[flights]).await;
    of_carriers.run(&Inputs::default()).await.unwrap();
    of_flights.run(&Inputs::default()).await.unwrap();
    // The flights count of the old version leaves the store, so the new one fits beside the
    // carriers count, which is the least recently used.
    store.table_changed("flights");
    assert_eq!(
        outcomes(&of_flights.run(&Inputs::default()).await.unwrap()),
        ["computed"]
    );
    assert_eq!(
        outcomes(&of_carriers.run(&Inputs::default()).await.unwrap()),
        ["hit"]
    );
}
