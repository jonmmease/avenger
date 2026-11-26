//! The file of a real chart's baked graph loads and runs in a baked graph's session, natively and,
//! under `wasm-pack test`, in a browser, and builds the chart that the project's run built for a
//! brush. The file and the chart are avenger-query-graph's baselines,
//! `tests/baselines/chart_graph/bars.graph` and `tests/baselines/chart_graph/bars-brushed.chart`.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{AsArray, Int64Array, RecordBatch};
use avenger_query_graph::chart_graph::ChartGraph;
use avenger_selection::{Clause, Condition};

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const FILE: &[u8] =
    include_bytes!("../../avenger-query-graph/tests/baselines/chart_graph/bars.graph");
const BRUSHED: &[u8] =
    include_bytes!("../../avenger-query-graph/tests/baselines/chart_graph/bars-brushed.chart");

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

async fn runs() {
    let session = avenger_sql::bake::baked_session().unwrap();
    let graph = ChartGraph::decode(FILE, &session.state()).unwrap();
    let empty = graph.run(&BTreeMap::new()).await.unwrap();
    let brushed = graph.run(&brushed()).await.unwrap();
    assert_eq!(chart(&brushed[0]), BRUSHED);
    assert_ne!(chart(&empty[0]), BRUSHED);
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn a_graph_s_file_runs_natively() {
    runs().await;
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test::wasm_bindgen_test]
async fn a_graph_s_file_runs_in_a_browser() {
    runs().await;
}
