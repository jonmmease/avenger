//! A graph's page: its file decodes into a graph that runs without the plans' session, and returns
//! the rows of the graph run natively for each state of its selections.

mod common;

use std::{collections::BTreeMap, sync::Arc};

use avenger_physical_graph::page::{task_context, PageGraph, SelectionFile};
use avenger_selection::{Clause, Resolution, SelectionState};
use common::{context, exact, inputs, plan, rows, store};
use datafusion::scalar::ScalarValue;

const PLANS: [&str; 3] = [
    "SELECT carrier, count(*) AS n, sum(distance) AS total FROM flights \
     WHERE year = $year AND selected($brush) GROUP BY carrier ORDER BY carrier",
    "SELECT carrier, avg(delay) AS mean FROM flights \
     WHERE selected($brush, exclude_own => true, owner => 'delays') GROUP BY carrier ORDER BY carrier",
    "SELECT year, count(*) AS n FROM flights GROUP BY year ORDER BY year",
];

fn clauses(low: f64, high: f64) -> Vec<Clause> {
    vec![Clause::new("delays").condition(exact("delay", low, high))]
}

#[tokio::test]
async fn a_page_runs_the_rows_of_the_native_graph() {
    let ctx = context();
    let mut plans = Vec::new();
    for sql in PLANS {
        plans.push(plan(&ctx, sql).await);
    }
    let fields = BTreeMap::from([("brush".to_string(), Some(vec!["delay".to_string()]))]);
    let page_graph = (store().compile_page(&ctx.state(), plans.clone(), fields.clone())).unwrap();
    let native = (store().compile_with_fields(&ctx.state(), plans, fields)).unwrap();
    let year = [("year", ScalarValue::from(2023i64))];
    let selections = vec![SelectionFile::of("brush", "brush", Resolution::Intersect)];
    let page = page_graph
        .page(&inputs(&year, &[]), selections)
        .await
        .unwrap();
    assert!(page.warnings.is_empty(), "{:?}", page.warnings);
    let graph = PageGraph::decode(&page.bytes, Arc::new(task_context())).unwrap();
    assert_eq!(graph.reads(), ["brush".to_string()].into());
    let states = [
        None,
        Some(clauses(0.0, 40.0)),
        Some(clauses(10.0, 60.0)),
        None,
        Some(clauses(0.0, 40.0)),
    ];
    for clauses in states {
        let state = SelectionState::new(Resolution::Intersect, clauses.clone().unwrap_or_default());
        let expected = native
            .run(&inputs(&year, &[("brush", state)]))
            .await
            .unwrap();
        let named = BTreeMap::from_iter(clauses.map(|clauses| ("brush".to_string(), clauses)));
        let ran = graph.run(&named).await.unwrap();
        assert_eq!(ran.len(), expected.batches.len());
        for (ran, expected) in ran.iter().zip(&expected.batches) {
            assert_eq!(rows(ran), rows(expected));
        }
    }
}
