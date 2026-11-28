//! A chart graph: live, baked, and read from its file, it returns the rows of the plans run uncut
//! for each state of the selections. A baked graph holds the tables and the columns that its
//! plans can read, and its file falls back to a chart's first rows when the chart's plan does not
//! travel.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use avenger_query_graph::{
    chart_graph::{ChartGraph, ChartSelection, Encoded},
    proto::{self, ChartGraphFile},
    Inputs,
};
use avenger_selection::{Clause, Resolution, SelectionState};
use common::{context, exact, inputs, outcomes, plan, rows, store, uncut};
use datafusion::{
    common::ParamValues,
    logical_expr::LogicalPlan,
    prelude::{SessionConfig, SessionContext},
    scalar::ScalarValue,
};

/// The session of a baked graph: one partition, with `selected`.
fn baked_session() -> SessionContext {
    let config = SessionConfig::new().with_target_partitions(1);
    let mut ctx = SessionContext::new_with_config(config);
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    ctx
}

/// The selection `name`, whose placeholder is its name, with the columns `fields` of its tools.
fn selection(name: &str, fields: Option<&[&str]>) -> ChartSelection {
    ChartSelection {
        name: name.to_string(),
        placeholder: name.to_string(),
        resolution: Resolution::Intersect,
        fields: fields.map(|fields| fields.iter().map(|field| field.to_string()).collect()),
    }
}

fn year() -> ParamValues {
    ParamValues::from(vec![("year", ScalarValue::from(2023_i64))])
}

/// The plans of charts, their live graph, the graph baked, and the baked graph's file.
struct Graphs {
    plans: Vec<LogicalPlan>,
    live: ChartGraph,
    baked: ChartGraph,
    encoded: Encoded,
}

impl Graphs {
    /// The graph that the file gives.
    fn decoded(&self) -> ChartGraph {
        ChartGraph::decode(&self.encoded.bytes, &baked_session().state()).unwrap()
    }
}

/// The graphs of the plans of `sqls`, for the selections `selections`.
async fn graphs(sqls: &[&str], selections: &[ChartSelection]) -> Graphs {
    let ctx = context();
    let mut plans = Vec::new();
    for sql in sqls {
        plans.push(plan(&ctx, sql).await);
    }
    let selections = selections.to_vec();
    let live = ChartGraph::new(&store(), &ctx.state(), plans.clone(), year(), selections).unwrap();
    let baked = live.bake(&baked_session().state()).await.unwrap();
    let encoded = baked.encode().await.unwrap();
    Graphs {
        plans,
        live,
        baked,
        encoded,
    }
}

/// The clauses of a brush of `delay` from `low` to `high`, by the name of the selection `name`.
fn brushed(name: &str, range: Option<(f64, f64)>) -> BTreeMap<String, Vec<Clause>> {
    let clauses = (range.into_iter())
        .map(|(low, high)| Clause::new("delay").condition(exact("delay", low, high)))
        .collect();
    BTreeMap::from([(name.to_string(), clauses)])
}

/// Checks the rows of each chart of `graph` for `clauses` against its plan run uncut.
async fn check(graph: &ChartGraph, plans: &[LogicalPlan], clauses: &BTreeMap<String, Vec<Clause>>) {
    let ctx = context();
    let states: Vec<(&str, SelectionState)> = (clauses.iter())
        .map(|(name, clauses)| {
            let state = SelectionState::new(Resolution::Intersect, clauses.clone());
            (name.as_str(), state)
        })
        .collect();
    let inputs = inputs(&[("year", ScalarValue::from(2023_i64))], &states);
    let outputs = graph.run(clauses).await.unwrap();
    for (position, plan) in plans.iter().enumerate() {
        let expected = uncut(&ctx, plan, &inputs).await;
        assert!(!expected.is_empty());
        assert_eq!(rows(&outputs[position]), expected, "chart {position}");
    }
}

const CHARTS: [&str; 4] = [
    "SELECT carrier, count(*) AS n FROM flights WHERE year = $year AND selected($brush) \
     GROUP BY carrier ORDER BY carrier",
    "SELECT carrier, avg(delay) AS d FROM flights WHERE selected($brush) GROUP BY carrier \
     ORDER BY carrier",
    "SELECT year, count(*) AS n FROM flights GROUP BY year ORDER BY year",
    "SELECT carrier, count(*) AS n FROM flights WHERE selected($brush) OR distance > 2000 \
     GROUP BY carrier ORDER BY carrier",
];

/// A graph returns the rows of each plan run uncut, for an empty brush, for a brush that moves,
/// and for a brush that the reader clears: the live graph, the graph baked, and the graph that
/// the baked graph's file gives. A chart that reads no selection travels without a plan.
#[tokio::test]
async fn a_graph_returns_the_rows_of_its_plans() {
    let graphs = graphs(&CHARTS, &[selection("brush", Some(&["delay"]))]).await;
    assert_eq!(graphs.encoded.warnings, Vec::<String>::new());
    let file = ChartGraphFile::from_bytes(&graphs.encoded.bytes).unwrap();
    assert_eq!(file.datafusion, datafusion::DATAFUSION_VERSION);
    let resolution = file.selections[0].resolution();
    assert_eq!(resolution, proto::Resolution::Intersect);
    let with_plans: Vec<bool> = (file.charts.iter().map(|chart| chart.plan.is_some())).collect();
    assert_eq!(with_plans, [true, true, false, true]);
    assert_eq!(graphs.encoded.plans, 3);

    let decoded = graphs.decoded();
    assert!(!graphs.live.is_baked());
    assert!(graphs.baked.is_baked());
    assert!(decoded.is_baked());
    for graph in [&graphs.live, &graphs.baked, &decoded] {
        assert_eq!(graph.reads(), BTreeSet::from(["brush".to_string()]));
        for range in [None, Some((0.0, 40.0)), Some((10.0, 60.0)), None] {
            check(graph, &graphs.plans, &brushed("brush", range)).await;
        }
    }
}

/// A live graph has no file, since a file holds no source, and a baked graph bakes to itself.
#[tokio::test]
async fn a_graph_encodes_after_it_bakes() {
    let graphs = graphs(&CHARTS, &[selection("brush", Some(&["delay"]))]).await;
    let error = graphs.live.encode().await.unwrap_err().to_string();
    assert!(error.contains("encodes after it bakes"), "{error}");
    let again = (graphs.baked.bake(&baked_session().state()).await).unwrap();
    assert!(again.is_baked());
    let encoded = again.encode().await.unwrap();
    assert_eq!(encoded.plans, graphs.encoded.plans);
    assert_eq!(encoded.bytes.len(), graphs.encoded.bytes.len());
}

/// The first run of a baked graph is a hit for every chart, since its charts' rows for empty
/// selections are in its store: the graph that a bake gives, and the graph that a file gives.
#[tokio::test]
async fn the_first_run_of_a_baked_graph_computes_nothing() {
    let graphs = graphs(&CHARTS, &[selection("brush", Some(&["delay"]))]).await;
    for baked in [&graphs.baked, &graphs.decoded()] {
        let graph = baked.graph().unwrap();
        let states = BTreeMap::from([("brush".to_string(), SelectionState::default())]);
        let run = (graph.run(&Inputs {
            selections: states,
            ..Inputs::default()
        }))
        .await
        .unwrap();
        let tops: BTreeSet<&str> = (run.nodes.iter().zip(outcomes(&run)))
            .filter(|(node, _)| !node.top_of().is_empty())
            .map(|(_, outcome)| outcome)
            .collect();
        assert_eq!(tops, BTreeSet::from(["hit"]));
        assert!(outcomes(&run)
            .iter()
            .all(|outcome| ["hit", "not read"].contains(outcome)));
    }
}

/// A baked graph holds one table for each node that reads no selection and that a chart's plan
/// scans: the rows of the year, and the rows of the scan, which two charts share. A table holds
/// the columns that the plans read for any state and the columns of the brush's tools, or every
/// column when the tools' columns are unknown.
#[tokio::test]
async fn a_table_holds_the_columns_that_its_plans_can_read() {
    let columns = |encoded: &Encoded| -> Vec<usize> {
        let file = ChartGraphFile::from_bytes(&encoded.bytes).unwrap();
        let mut columns: Vec<usize> = (file.tables.iter())
            .map(|table| table.result.as_ref().unwrap().columns.len())
            .collect();
        columns.sort();
        columns
    };
    // The year's rows: `carrier` and the brush's `delay`. The scan: those, and `distance`, which
    // the fourth chart reads beside its brush.
    let known = graphs(&CHARTS, &[selection("brush", Some(&["delay"]))]).await;
    assert_eq!(columns(&known.encoded), [2, 3]);
    let unknown = graphs(&CHARTS, &[selection("brush", None)]).await;
    assert_eq!(columns(&unknown.encoded), [5, 5]);
}

/// A state that tests a column that a baked graph's tables do not hold fails with the column's
/// name, since the graph has no source to read it from.
#[tokio::test]
async fn a_state_tests_only_the_columns_that_the_tables_hold() {
    let graphs = graphs(&CHARTS, &[selection("brush", Some(&["delay"]))]).await;
    let clause = Clause::new("air").condition(exact("air_time", 0.0, 100.0));
    let clauses = BTreeMap::from([("brush".to_string(), vec![clause])]);
    for baked in [&graphs.baked, &graphs.decoded()] {
        let error = baked.run(&clauses).await.unwrap_err().to_string();
        assert!(
            error.contains("tests the column `air_time`, which the graph's tables do not hold"),
            "{error}"
        );
    }
}

/// A chart whose plan does not travel, here for an `IN` subquery beside its brush, follows the
/// brush in the baked graph. In the graph's file it keeps its first rows for every state, with
/// a warning that names its selection.
#[tokio::test]
async fn a_chart_whose_plan_does_not_travel_keeps_its_first_rows() {
    let sql = "SELECT carrier, count(*) AS n FROM flights \
               WHERE selected($brush) OR carrier IN (SELECT code FROM carriers WHERE name > 'D') \
               GROUP BY carrier ORDER BY carrier";
    let graphs = graphs(&[sql], &[selection("brush", Some(&["delay"]))]).await;
    for range in [None, Some((0.0, 40.0))] {
        check(&graphs.baked, &graphs.plans, &brushed("brush", range)).await;
    }
    let warnings = &graphs.encoded.warnings;
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("$brush"), "{warnings:?}");
    let file = ChartGraphFile::from_bytes(&graphs.encoded.bytes).unwrap();
    assert!(file.charts[0].plan.is_none());
    assert_eq!(graphs.encoded.plans, 0);
    assert!(file.tables.is_empty());

    let decoded = graphs.decoded();
    assert!(decoded.reads().is_empty());
    let ctx = context();
    let empty = inputs(
        &[("year", ScalarValue::from(2023_i64))],
        &[("brush", SelectionState::default())],
    );
    let first = uncut(&ctx, &graphs.plans[0], &empty).await;
    for range in [None, Some((0.0, 40.0))] {
        let outputs = decoded.run(&brushed("brush", range)).await.unwrap();
        assert_eq!(rows(&outputs[0]), first);
    }
}

/// A chart that reads two selections follows each, and a chart that calls a volatile function
/// runs at each run of a baked graph, since no store keeps its result.
#[tokio::test]
async fn a_graph_follows_two_selections_and_runs_a_volatile_chart() {
    let sqls = [
        "SELECT carrier, count(*) AS n FROM flights WHERE selected($brush) AND selected($pick) \
         GROUP BY carrier ORDER BY carrier",
        "SELECT carrier, count(*) AS n FROM flights WHERE selected($brush) AND random() >= 0 \
         GROUP BY carrier ORDER BY carrier",
    ];
    let selections = [
        selection("brush", Some(&["delay"])),
        selection("pick", Some(&["distance"])),
    ];
    let graphs = graphs(&sqls, &selections).await;
    assert_eq!(graphs.encoded.warnings, Vec::<String>::new());
    let decoded = graphs.decoded();
    let both = BTreeSet::from(["brush".to_string(), "pick".to_string()]);
    let far = Clause::new("distance").condition(exact("distance", 1000.0, 2500.0));
    for graph in [&graphs.live, &graphs.baked, &decoded] {
        assert_eq!(graph.reads(), both);
        for (range, picked) in [
            (None, false),
            (Some((0.0, 40.0)), false),
            (Some((0.0, 40.0)), true),
        ] {
            let mut clauses = brushed("brush", range);
            let pick = picked.then(|| far.clone()).into_iter().collect();
            clauses.insert("pick".to_string(), pick);
            check(graph, &graphs.plans, &clauses).await;
        }
    }
}

/// A chart that counts the rows of a brushed subquery reads none of the subquery's columns, and
/// follows the brush.
#[tokio::test]
async fn a_chart_reads_only_the_rows_of_a_brushed_subquery() {
    let sql = "SELECT count(*) AS n FROM (SELECT * FROM flights WHERE selected($brush)) AS t \
               WHERE $year > 0";
    let graphs = graphs(&[sql], &[selection("brush", Some(&["delay"]))]).await;
    assert_eq!(graphs.encoded.warnings, Vec::<String>::new());
    let decoded = graphs.decoded();
    for graph in [&graphs.live, &graphs.baked, &decoded] {
        for range in [None, Some((0.0, 40.0))] {
            check(graph, &graphs.plans, &brushed("brush", range)).await;
        }
    }
}
