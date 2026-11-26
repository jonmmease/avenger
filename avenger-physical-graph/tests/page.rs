//! A page's graph, as avenger-sql-project's test of pages writes its file, decodes and runs
//! natively and, under `wasm-pack test`, in a browser, and builds the chart that the native run
//! builds for a brush. The file and the chart are `tests/baselines/bars.graph` and
//! `tests/baselines/bars-brushed.chart`, which that test writes to `tests/output/`.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{AsArray, Int64Array, RecordBatch};
use avenger_physical_graph::page::{task_context, PageGraph};
use avenger_selection::{Clause, Condition};

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const GRAPH: &[u8] = include_bytes!("baselines/bars.graph");
const BRUSHED: &[u8] = include_bytes!("baselines/bars-brushed.chart");

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
    let graph = PageGraph::decode(GRAPH, Arc::new(task_context(Default::default()))).unwrap();
    let empty = graph.run(&BTreeMap::new()).await.unwrap();
    let brushed = graph.run(&brushed()).await.unwrap();
    assert_eq!(chart(&brushed[0]), BRUSHED);
    assert_ne!(chart(&empty[0]), BRUSHED);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_page_s_graph_runs_natively() {
    futures::executor::block_on(runs());
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test::wasm_bindgen_test]
async fn a_page_s_graph_runs_in_a_browser() {
    runs().await;
}
