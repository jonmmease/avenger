//! The plans that a graph keeps for its nodes: a brush binds the physical plan that the first run
//! planned, over a file table, in an aggregate's argument, and below a shared input, and a node
//! that is not kept plans each run with the run's states.

use std::{collections::BTreeMap, sync::Arc};

use avenger_selection::{Clause, Condition, Resolution, SelectionState};
use avenger_selection_datafusion::{
    with_selection_nodes, with_selection_states, QueryPlanner, SelectionFields,
};
use datafusion::{
    arrow::{array::Float64Array, util::pretty::pretty_format_batches},
    common::ParamValues,
    dataframe::DataFrame,
    execution::SessionStateBuilder,
    logical_expr::LogicalPlan,
    prelude::{CsvReadOptions, SessionConfig, SessionContext},
};

use crate::{store::lock, Inputs, Outcome, QueryGraph, ResultStore, SourceOptions, StoreConfig};

fn brush(low: f64, high: f64) -> SelectionState {
    let clause = Clause::new("scatter").condition(Condition::Range {
        column: "x".into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, high])),
        exclude_end: false,
        cells: None,
    });
    SelectionState::new(Resolution::Global, vec![clause])
}

/// A session with the table `t` of a CSV file, whose scans are not nodes of their own, as a
/// project's data files are not.
async fn context(name: &str) -> (SessionContext, ResultStore) {
    let state = SessionStateBuilder::new()
        .with_config(SessionConfig::new().with_target_partitions(2))
        .with_default_features()
        .with_query_planner(Arc::new(QueryPlanner))
        .build();
    let mut ctx = SessionContext::new_with_state(state);
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    let dir =
        std::env::temp_dir().join(format!("avenger-query-graph-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rows: String = (0..60)
        .map(|i| format!("{},{i}\n", ["a", "b", "c"][i % 3]))
        .collect();
    let path = dir.join("t.csv");
    std::fs::write(&path, format!("k,x\n{rows}")).unwrap();
    (ctx.register_csv("t", path.to_str().unwrap(), CsvReadOptions::new())
        .await)
        .unwrap();
    let store = ResultStore::new(StoreConfig {
        budget: 64 << 20,
        max_result: 16 << 20,
    });
    let options = SourceOptions {
        materialize: false,
        ttl: None,
    };
    store.set_source_options("t", options);
    (ctx, store)
}

/// The fields of the selection `s`, whose tools test `x`.
fn fields() -> SelectionFields {
    SelectionFields::from([("s".to_string(), Some(vec!["x".to_string()]))])
}

/// The graph of `plan`.
fn graph(ctx: &SessionContext, store: &ResultStore, plan: &LogicalPlan) -> QueryGraph {
    (store.compile_with_fields(&ctx.state(), vec![plan.clone()], fields())).unwrap()
}

/// The outcomes of runs of `graph` for several brushes, whose rows equal the rows of `whole` run
/// whole.
async fn runs(ctx: &SessionContext, graph: &QueryGraph, whole: &LogicalPlan) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for (low, high) in [(0.0, 20.0), (10.0, 45.0), (0.0, 20.0), (30.0, 59.0)] {
        let state = brush(low, high);
        let states = BTreeMap::from([("s".to_string(), state)]);
        let inputs = Inputs {
            params: ParamValues::List(Vec::new()),
            selections: states.clone(),
        };
        let run = graph.run(&inputs).await.unwrap();
        let whole = with_selection_nodes(whole.clone(), &fields()).unwrap();
        let session = with_selection_states(&ctx.state(), states);
        let expected = DataFrame::new(session, whole).collect().await.unwrap();
        assert_eq!(
            pretty_format_batches(&run.batches[0]).unwrap().to_string(),
            pretty_format_batches(&expected).unwrap().to_string(),
        );
        outcomes.extend(run.outcomes);
    }
    outcomes
}

/// The number of physical plans in each memo of the graph.
fn planned(graph: &QueryGraph) -> Vec<usize> {
    let memos = lock(&graph.inner.memos);
    (memos.entries.values())
        .map(|memo| lock(&memo.physical).len())
        .collect()
}

/// A filter by a selection stays above the scan of a CSV file, so a brush over the file binds
/// the plan that the first run planned, as a selection in an aggregate's `FILTER` does.
#[tokio::test]
async fn a_brush_binds_the_plan_that_its_first_run_planned() {
    let (ctx, store) = context("binds").await;
    for sql in [
        "SELECT k, count(*) AS n FROM t WHERE selected($s) GROUP BY k ORDER BY k",
        "SELECT count(*) FILTER (WHERE selected($s)) AS n FROM t",
    ] {
        let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
        let graph = graph(&ctx, &store, &plan);
        runs(&ctx, &graph, &plan).await;
        let planned = planned(&graph);
        assert!(
            !planned.is_empty() && planned.iter().all(|n| *n == 1),
            "{sql}: {planned:?}"
        );
    }
}

/// A shared input is a node of its own, so a brush below it binds the plan of each node that
/// reads it.
#[tokio::test]
async fn a_brush_below_a_shared_input_binds_its_plan() {
    let (ctx, store) = context("shared").await;
    let input = (ctx.sql("SELECT k, x FROM t WHERE selected($s)").await)
        .unwrap()
        .into_unoptimized_plan();
    let counts = |input: LogicalPlan| {
        datafusion::logical_expr::LogicalPlanBuilder::from(input)
            .aggregate(
                vec![datafusion::logical_expr::col("k")],
                vec![datafusion::functions_aggregate::count::count_all()],
            )
            .unwrap()
            .sort(vec![datafusion::logical_expr::col("k").sort(true, false)])
            .unwrap()
            .build()
            .unwrap()
    };
    let shared = counts(avenger_datafusion_shared::shared(input.clone()));
    let graph = graph(&ctx, &store, &shared);
    assert!(graph.nodes().len() > 1);
    runs(&ctx, &graph, &counts(input)).await;
    assert!(planned(&graph).iter().all(|n| *n == 1));
}

/// A node that calls `now()` is not kept, so each run plans it with the run's states.
#[tokio::test]
async fn a_node_that_is_not_kept_plans_each_run() {
    let (ctx, store) = context("now").await;
    let sql = "SELECT count(*) FILTER (WHERE selected($s) AND now() IS NOT NULL) AS n FROM t";
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let graph = graph(&ctx, &store, &plan);
    let outcomes = runs(&ctx, &graph, &plan).await;
    assert!(outcomes
        .iter()
        .any(|outcome| matches!(outcome, Outcome::Uncached { .. })));
    assert!(planned(&graph).is_empty());
}
