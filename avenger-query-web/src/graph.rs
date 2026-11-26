//! The graph of a document's queries in a page, which builds its charts and follows the reader's
//! selections without a server: [`ChartGraph`] reads the file of a baked graph, takes the
//! selections of the changes that its views report, and runs the graph for them.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use avenger_chart::{results::is_chart_field, ChangeMessage};
use avenger_chart_app::ChartChange;
use avenger_selection::Clause;
use js_sys::{Array, Promise, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

struct Inner {
    graph: avenger_query_graph::chart_graph::ChartGraph,
    clauses: RefCell<BTreeMap<String, Vec<Clause>>>,
    reads: BTreeSet<String>,
}

/// A document's graph, as `avenger render` and the Quarto engine write its file, which runs the
/// document's queries for the clauses of its selections.
#[wasm_bindgen]
pub struct ChartGraph {
    inner: Rc<Inner>,
}

#[wasm_bindgen]
impl ChartGraph {
    /// Reads the file `bytes` of a baked graph, and compiles its plans in a baked graph's session.
    /// Throws for bytes of no file, or of a file whose plans name a function or a node that the
    /// library lacks.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<ChartGraph, JsError> {
        let graph = avenger_sql::bake::baked_session()
            .and_then(|session| {
                avenger_query_graph::chart_graph::ChartGraph::decode(bytes, &session.state())
            })
            .map_err(|error| JsError::new(&error.to_string()))?;
        let reads = graph.reads();
        Ok(ChartGraph {
            inner: Rc::new(Inner {
                graph,
                clauses: RefCell::new(BTreeMap::new()),
                reads,
            }),
        })
    }

    /// The names of the selections that the graph's queries read, so a page runs the graph when
    /// one of them changes.
    #[wasm_bindgen(getter)]
    pub fn reads(&self) -> Vec<String> {
        self.inner.reads.iter().cloned().collect()
    }

    /// Takes the clauses of the selections that the change message `bytes`, as a view's
    /// `onChange` gives it, writes. Returns whether a selection that the graph reads changed.
    pub fn change(&self, bytes: Vec<u8>) -> Result<bool, JsError> {
        let message =
            ChangeMessage::from_bytes(bytes).map_err(|error| JsError::new(&error.to_string()))?;
        let change = ChartChange::from_message(&message)
            .map_err(|error| JsError::new(&error.to_string()))?;
        let mut clauses = self.inner.clauses.borrow_mut();
        let mut read = false;
        for (name, written) in change.selections {
            read |= self.inner.reads.contains(&name);
            clauses.insert(name, written);
        }
        Ok(read)
    }

    /// Runs the graph with the clauses that it took, and resolves to the chart of each of its
    /// outputs, in order: the bytes of the first chart of the output's first row, or `null` for an
    /// output without a chart.
    pub fn run(&self) -> Promise {
        let inner = self.inner.clone();
        future_to_promise(async move {
            let clauses = inner.clauses.borrow().clone();
            let outputs = (inner.graph.run(&clauses).await)
                .map_err(|error| JsError::new(&error.to_string()))?;
            let charts = Array::new();
            for batches in outputs {
                let chart = (batches.iter())
                    .find(|batch| batch.num_rows() > 0)
                    .and_then(|batch| {
                        let schema = batch.schema();
                        let column =
                            (schema.fields().iter()).position(|field| is_chart_field(field))?;
                        chart_bytes(batch.column(column).as_ref())
                    });
                match chart {
                    Some(bytes) => charts.push(&Uint8Array::from(bytes.as_slice())),
                    None => charts.push(&JsValue::NULL),
                };
            }
            Ok(charts.into())
        })
    }
}

/// The bytes of the first chart of `column`, a column of charts.
fn chart_bytes(column: &dyn arrow::array::Array) -> Option<Vec<u8>> {
    use arrow::array::AsArray;
    use arrow::datatypes::DataType;
    if column.is_null(0) {
        return None;
    }
    match column.data_type() {
        DataType::BinaryView => Some(column.as_binary_view().value(0).to_vec()),
        DataType::Binary => Some(column.as_binary::<i32>().value(0).to_vec()),
        DataType::LargeBinary => Some(column.as_binary::<i64>().value(0).to_vec()),
        _ => None,
    }
}
