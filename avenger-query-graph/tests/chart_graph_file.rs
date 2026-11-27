//! The file of a baked graph that is checked in, as avenger-sql-project's test of chart graphs
//! wrote it, loads and runs, and builds the chart that the project's run built for a brush, so a
//! change of the file's format or of the codec of its plans shows here. The file and the chart are
//! `tests/baselines/chart_graph/bars.graph` and `tests/baselines/chart_graph/bars-brushed.chart`,
//! which that test writes to `tests/output/`.

use std::{collections::BTreeMap, sync::Arc};

use avenger_query_graph::chart_graph::ChartGraph;
use avenger_selection::{Clause, Condition};
use datafusion::arrow::array::{AsArray, Int64Array, RecordBatch};

const FILE: &[u8] = include_bytes!("baselines/chart_graph/bars.graph");
const BRUSHED: &[u8] = include_bytes!("baselines/chart_graph/bars-brushed.chart");

fn brushed() -> BTreeMap<String, Vec<Clause>> {
    let clause = Clause::new("scatter").condition(Condition::Range {
        column: "v".into(),
        channel: None,
        ends: Arc::new(Int64Array::from(vec![2, 3])),
        exclude_end: false,
        cells: None,
    });
    BTreeMap::from([("brush".to_string(), vec![clause])])
}

fn chart(batches: &[RecordBatch]) -> Vec<u8> {
    batches[0].column(0).as_binary_view().value(0).to_vec()
}

#[tokio::test]
async fn a_checked_in_file_runs() {
    let session = avenger_sql::bake::baked_session().unwrap();
    let graph = ChartGraph::decode(FILE, &session.state()).unwrap();
    let empty = graph.run(&BTreeMap::new()).await.unwrap();
    let brushed = graph.run(&brushed()).await.unwrap();
    assert_eq!(chart(&brushed[0]), BRUSHED);
    assert_ne!(chart(&empty[0]), BRUSHED);
}
