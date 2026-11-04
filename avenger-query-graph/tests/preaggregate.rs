//! A run with a focus pre-aggregates, keeps its rollups under the keys of the aggregates that
//! they replace, and returns the rows of the direct plans.

mod common;

use std::sync::Arc;

use avenger_query_graph::{Inputs, QueryGraph, SourceOptions};
use avenger_scales::scales::linear::LinearScale;
use avenger_selection::{PixelGrid, SelectionState};
use avenger_selection_datafusion::preaggregate::{Focus, FocusDimension};
use common::{context, gridded, inputs, outcomes, plan, rows, state, store, uncut};
use datafusion::{logical_expr::LogicalPlan, prelude::SessionContext, scalar::ScalarValue};

/// The grid of a delay histogram 240 pixels wide over -20 to 100 minutes, in cells of `cell`
/// pixels.
fn grid(cell: f64) -> PixelGrid {
    PixelGrid::new(LinearScale::configured((-20.0, 100.0), (0.0, 240.0)), cell).unwrap()
}

fn focus(cell: f64) -> Focus {
    Focus {
        parameter: "$brush".into(),
        owner: "delay".into(),
        dimensions: vec![FocusDimension {
            column: "delay".into(),
            grid: Some(grid(cell)),
        }],
    }
}

/// The brush's box over the pixels `low` to `high`, with the cells of the grid of `cell` pixels.
fn brushed(low: f64, high: f64, cell: f64) -> SelectionState {
    state(vec![("delay", gridded("delay", &grid(cell), low, high))])
}

fn gesture(year: i64, brush: SelectionState, focus: Option<Focus>) -> Inputs {
    Inputs {
        focus,
        ..inputs(&[("year", ScalarValue::from(year))], &[("brush", brush)])
    }
}

/// A graph of `sqls` over materialized flights.
async fn graph(ctx: &SessionContext, sqls: &[&str]) -> (QueryGraph, Vec<LogicalPlan>) {
    let store = store();
    store.set_source_options(
        "flights",
        SourceOptions {
            materialize: true,
            ttl: None,
        },
    );
    let mut plans = Vec::new();
    for sql in sqls {
        plans.push(plan(ctx, sql).await);
    }
    (store.compile(&ctx.state(), plans.clone()).unwrap(), plans)
}

/// Run `graph` and check each plan's rows against the plan run uncut, without the focus.
async fn run(
    ctx: &SessionContext,
    graph: &QueryGraph,
    plans: &[LogicalPlan],
    inputs: &Inputs,
) -> Vec<&'static str> {
    let run = graph.run(inputs).await.unwrap();
    for (batches, plan) in run.batches.iter().zip(plans) {
        assert_eq!(rows(batches), uncut(ctx, plan, inputs).await);
    }
    outcomes(&run)
}

const EXAMPLE: &str = "SELECT carrier, count(*) AS n, round(avg(distance), 6) AS mean \
                       FROM flights WHERE year = $year AND selected($brush) GROUP BY carrier";

#[tokio::test]
async fn the_example_gestures_rerun_only_the_rollup() {
    let ctx = context();
    let (graph, plans) = graph(&ctx, &[EXAMPLE]).await;
    let focused = Some(focus(4.0));
    let empty = SelectionState::default;
    // The direct plan has the scan, the year filter, and the brush filter with the aggregate.
    assert_eq!(graph.nodes(None).unwrap().len(), 3);
    // The variant has the scan, the year filter, the cells, and the rollup.
    let variant = graph.nodes(focused.as_ref()).unwrap();
    assert_eq!(variant.len(), 4);
    assert_eq!(variant[3].replaces().len(), 1);

    let opens = run(&ctx, &graph, &plans, &gesture(2023, empty(), None)).await;
    assert_eq!(opens, ["computed", "computed", "computed"]);
    // The pointer enters: the rollup hits under the direct key, and the cells compute, which
    // widens the nodes below them by `delay`.
    let enters = run(
        &ctx,
        &graph,
        &plans,
        &gesture(2023, empty(), focused.clone()),
    )
    .await;
    assert_eq!(enters, ["widened", "widened", "computed", "hit"]);
    let moves = run(
        &ctx,
        &graph,
        &plans,
        &gesture(2023, brushed(40.0, 120.0, 4.0), focused.clone()),
    )
    .await;
    assert_eq!(moves, ["not read", "not read", "hit", "computed"]);
    let again = run(
        &ctx,
        &graph,
        &plans,
        &gesture(2023, brushed(60.0, 180.0, 4.0), focused.clone()),
    )
    .await;
    assert_eq!(again, ["not read", "not read", "hit", "computed"]);
    let slides = run(
        &ctx,
        &graph,
        &plans,
        &gesture(2024, brushed(60.0, 180.0, 4.0), focused),
    )
    .await;
    assert_eq!(slides, ["hit", "computed", "computed", "computed"]);
    // The pointer leaves: the direct plan's top node hits the rollup's result.
    let leaves = run(
        &ctx,
        &graph,
        &plans,
        &gesture(2024, brushed(60.0, 180.0, 4.0), None),
    )
    .await;
    assert_eq!(leaves, ["not read", "not read", "hit"]);
}

#[tokio::test]
async fn a_brush_of_another_grid_runs_the_direct_plan() {
    let ctx = context();
    let (graph, plans) = graph(&ctx, &[EXAMPLE]).await;
    let resized = run(
        &ctx,
        &graph,
        &plans,
        &gesture(2023, brushed(40.0, 120.0, 8.0), Some(focus(4.0))),
    )
    .await;
    assert_eq!(resized.len(), 3);
}

#[tokio::test]
async fn operators_above_a_rollup_keep_the_direct_keys() {
    let ctx = context();
    let ordered = "SELECT carrier, n FROM (SELECT carrier, count(*) AS n FROM flights \
                   WHERE year = $year AND selected($brush) GROUP BY carrier) \
                   ORDER BY n DESC, carrier LIMIT 3";
    let plots = "SELECT 'carriers' AS plot, carrier AS key, count(*) AS n FROM flights \
                 WHERE year = $year AND selected($brush) GROUP BY carrier \
                 UNION ALL \
                 SELECT 'distances', CAST(floor(distance / 500) AS VARCHAR), count(*) FROM flights \
                 WHERE year = $year AND selected($brush) GROUP BY floor(distance / 500)";
    let (graph, plans) = graph(&ctx, &[ordered, plots]).await;
    let variant = graph.nodes(Some(&focus(4.0))).unwrap();
    let union = (variant.iter()).find(|node| node.top_of() == [1]).unwrap();
    assert_eq!(union.replaces().len(), 2);
    let brush = || brushed(40.0, 120.0, 4.0);
    run(
        &ctx,
        &graph,
        &plans,
        &gesture(2023, brush(), Some(focus(4.0))),
    )
    .await;
    let direct = graph.nodes(None).unwrap();
    let leaves = run(&ctx, &graph, &plans, &gesture(2023, brush(), None)).await;
    for (node, outcome) in direct.iter().zip(&leaves) {
        if !node.top_of().is_empty() {
            assert_eq!(*outcome, "hit");
        }
    }
}

#[tokio::test]
async fn a_changed_focus_reuses_the_results_above_the_aggregate() {
    let ctx = context();
    let (graph, plans) = graph(&ctx, &[EXAMPLE]).await;
    let brush = || brushed(40.0, 120.0, 4.0);
    run(
        &ctx,
        &graph,
        &plans,
        &gesture(2023, brush(), Some(focus(4.0))),
    )
    .await;
    // A focus of another grid does not serve the brush, so the run falls back to the direct
    // plan, whose top node hits the result that the first focus's rollup computed.
    let changed = run(
        &ctx,
        &graph,
        &plans,
        &gesture(2023, brush(), Some(focus(2.0))),
    )
    .await;
    assert_eq!(changed, ["not read", "not read", "hit"]);
}

#[tokio::test]
async fn the_graph_keeps_the_variants_of_its_four_latest_foci() {
    let ctx = context();
    let (graph, _) = graph(&ctx, &[EXAMPLE]).await;
    let cells = [1.0, 2.0, 3.0, 4.0, 5.0];
    let first = graph.nodes(Some(&focus(cells[0]))).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &graph.nodes(Some(&focus(cells[0]))).unwrap()
    ));
    let mut latest = first.clone();
    for cell in &cells[1..] {
        latest = graph.nodes(Some(&focus(*cell))).unwrap();
    }
    assert!(Arc::ptr_eq(
        &latest,
        &graph.nodes(Some(&focus(5.0))).unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &first,
        &graph.nodes(Some(&focus(cells[0]))).unwrap()
    ));
}

#[tokio::test]
async fn the_cells_below_a_hit_compute_with_every_column_of_their_rollup() {
    let ctx = context();
    // The rollup is in its own node below a top node that reads `$min`.
    let sql = "SELECT * FROM (SELECT carrier, count(*) AS n, round(avg(distance), 6) AS mean \
               FROM flights WHERE year = $year AND selected($brush) GROUP BY carrier) \
               WHERE n > $min";
    let (graph, plans) = graph(&ctx, &[sql]).await;
    let at = |brush: SelectionState, focus: Option<Focus>| Inputs {
        focus,
        ..inputs(
            &[
                ("year", ScalarValue::from(2023i64)),
                ("min", ScalarValue::from(0i64)),
            ],
            &[("brush", brush)],
        )
    };
    run(&ctx, &graph, &plans, &at(SelectionState::default(), None)).await;
    let enters = run(
        &ctx,
        &graph,
        &plans,
        &at(SelectionState::default(), Some(focus(4.0))),
    )
    .await;
    assert_eq!(enters.last(), Some(&"hit"));
    assert!(
        enters.contains(&"computed"),
        "the cells compute: {enters:?}"
    );
    // The first move reads the cells that the hover computed, with every column.
    let moves = run(
        &ctx,
        &graph,
        &plans,
        &at(brushed(40.0, 120.0, 4.0), Some(focus(4.0))),
    )
    .await;
    assert_eq!(
        moves,
        ["not read", "not read", "hit", "computed", "computed"]
    );
}

#[tokio::test]
async fn a_focus_keeps_plans_over_different_providers_of_one_name_apart() {
    let mut ctx = SessionContext::new();
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    let table = |values: Vec<i64>| {
        datafusion::arrow::array::RecordBatch::try_from_iter(vec![(
            "delay",
            Arc::new(datafusion::arrow::array::Int64Array::from(values)) as _,
        )])
        .unwrap()
    };
    let sql = "SELECT count(*) AS n FROM t WHERE selected($brush)";
    ctx.register_batch("t", table(vec![1])).unwrap();
    let first = plan(&ctx, sql).await;
    ctx.deregister_table("t").unwrap();
    ctx.register_batch("t", table(vec![1, 2])).unwrap();
    let second = plan(&ctx, sql).await;
    let graph = store().compile(&ctx.state(), vec![first, second]).unwrap();
    let keys = Focus {
        parameter: "$brush".into(),
        owner: "delay".into(),
        dimensions: vec![FocusDimension {
            column: "delay".into(),
            grid: None,
        }],
    };
    let run = graph
        .run(&Inputs {
            focus: Some(keys),
            ..inputs(&[], &[("brush", SelectionState::default())])
        })
        .await
        .unwrap();
    assert_eq!(rows(&run.batches[0]), ["1"]);
    assert_eq!(rows(&run.batches[1]), ["2"]);
}
