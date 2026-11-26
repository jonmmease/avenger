//! A graph's page: its file, which a page runs without a server. The nodes that read no selection
//! travel as their results, and the nodes that read one as their physical plans, with their results
//! for empty selections, so a page draws its charts before any selection changes.

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use avenger_physical_graph::{
    kept::{shapes, Kept},
    page::{
        decode_plan, encode_plan, projected, task_context, GraphFile, NodeFile, OutputFile,
        ReadFile, ResultFile, SelectionFile, ABSENT, GRAPH_VERSION,
    },
};
use datafusion::{
    arrow::datatypes::SchemaRef,
    common::{plan_err, Result},
};
use datafusion_proto_common as protobuf;
use prost::Message;

use crate::{
    cut::cut_schema,
    graph::QueryGraph,
    run::{physical, planned, request, versions, Inputs},
};

/// A graph's file for a page, with a warning for each node that reads a selection and travels as
/// its result, so its charts do not filter in the page.
#[derive(Clone, Debug)]
pub struct Page {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

impl QueryGraph {
    /// The file of the graph for a page, with the values of `inputs`' parameters and each
    /// selection empty, and `selections`, the document's selections. Compile the graph with
    /// [`crate::ResultStore::compile_page`], so it plans with one partition and reads each table
    /// through a node of its own.
    pub async fn page(&self, inputs: &Inputs, selections: Vec<SelectionFile>) -> Result<Page> {
        let inner = &self.inner;
        if !inner.page {
            return plan_err!("a graph for a page compiles with `ResultStore::compile_page`");
        }
        let nodes = inner.nodes.clone();
        let mut inputs = inputs.clone();
        for node in nodes.iter() {
            for parameter in node.reads.selections.keys() {
                let name = parameter.trim_start_matches('$').to_string();
                inputs.selections.entry(name).or_default();
            }
        }
        let inputs = Arc::new(inputs);
        let state = inner.state.clone();
        let versions = versions(inner);
        let count = nodes.len();

        // From the top down: the columns that each node computes, and its plan.
        let mut requests: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
        let mut needed = vec![false; count];
        for top in &inner.tops {
            needed[*top] = true;
            requests[*top] = (0..nodes[*top].plan.schema().fields().len()).collect();
        }
        let mut plans = Vec::with_capacity(count);
        let mut presents = vec![Vec::new(); count];
        let mut columns_of = vec![Vec::new(); count];
        for index in (0..count).rev() {
            if !needed[index] {
                plans.push(None);
                continue;
            }
            let node = &nodes[index];
            let mut columns = requests[index].clone();
            if columns.is_empty() && !node.plan.schema().fields().is_empty() {
                columns.insert(0);
            }
            let plan = planned(inner, index, node, &columns, &inputs, &versions, &state)?;
            presents[index] = request(node, plan.logical(), &mut requests, &mut needed)?;
            columns_of[index] = columns.into_iter().collect::<Vec<_>>();
            plans.push(Some(plan));
        }
        plans.reverse();

        // From the bottom up: each node's result for empty selections, and its plan's file when
        // it reads a selection.
        let mut results: Vec<Option<Arc<Kept>>> = vec![None; count];
        let mut page_plans: Vec<Option<Vec<u8>>> = vec![None; count];
        let mut warnings = Vec::new();
        let target = state.config_options().execution.target_partitions;
        let page_context = task_context();
        for index in 0..count {
            let Some(plan) = plans[index].take() else {
                continue;
            };
            let node = &nodes[index];
            let read: Vec<Option<Arc<Kept>>> = (node.inputs.iter().zip(&presents[index]))
                .map(|(input, present)| match present {
                    true => results[*input].clone(),
                    false => None,
                })
                .collect();
            let memo = plan.memo();
            let physical_plan = physical(plan, &read, &state).await?;
            let kept = Kept::compute(physical_plan, columns_of[index].clone(), state.task_ctx());
            results[index] = Some(Arc::new(kept.await?));
            if node.reads.selections.is_empty() {
                continue;
            }
            let readable = (node.inputs.iter().zip(&presents[index])).all(|(input, present)| {
                !present
                    || page_plans[*input].is_some()
                    || nodes[*input].reads.selections.is_empty()
            });
            let template = memo.and_then(|memo| memo.physical(&shapes(&read, target)));
            let encoded = match (template, readable) {
                // The plan runs in a page when it decodes with the page's functions alone.
                (Some(template), true) => encode_plan(template)
                    .and_then(|bytes| decode_plan(&bytes, &page_context).map(|_| bytes))
                    .map_err(|error| error.to_string()),
                (None, _) => {
                    Err("it is not kept, since it calls a volatile or stable function".to_string())
                }
                (Some(_), false) => Err("it reads a node that does not run in a page".to_string()),
            };
            match encoded {
                Ok(bytes) => page_plans[index] = Some(bytes),
                Err(reason) => warnings.push(format!(
                    "node {index} reads {} and does not run in a page: {reason}",
                    (node.reads.selections.keys().cloned())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }

        // The nodes that the page reads: the tops, the nodes that it runs, and their inputs.
        let mut kept_nodes = vec![false; count];
        for top in &inner.tops {
            kept_nodes[*top] = true;
        }
        for index in 0..count {
            if page_plans[index].is_some() {
                kept_nodes[index] = true;
                for (input, present) in nodes[index].inputs.iter().zip(&presents[index]) {
                    if *present {
                        kept_nodes[*input] = true;
                    }
                }
            }
        }
        let mut renumbered = HashMap::new();
        for index in (0..count).filter(|index| kept_nodes[*index]) {
            renumbered.insert(index, renumbered.len() as u32);
        }
        let mut files = Vec::with_capacity(renumbered.len());
        for index in (0..count).filter(|index| kept_nodes[*index]) {
            let node = &nodes[index];
            let kept = results[index]
                .as_ref()
                .expect("a node of the page has a result");
            let schema: SchemaRef = projected(&cut_schema(&node.plan), &kept.columns)?;
            let result = ResultFile::of(kept, schema)?;
            let columns = columns_of[index]
                .iter()
                .map(|column| *column as u32)
                .collect();
            let file = match page_plans[index].take() {
                Some(plan) => NodeFile {
                    inputs: (node.inputs.iter().zip(&presents[index]))
                        .map(|(input, present)| match present {
                            true => renumbered[input],
                            false => ABSENT,
                        })
                        .collect(),
                    columns,
                    result: None,
                    plan: Some(plan),
                    reads: (node.reads.selections.iter())
                        .map(|(parameter, owners)| ReadFile::of(parameter, owners))
                        .collect(),
                    initial: Some(result),
                },
                None => NodeFile {
                    inputs: Vec::new(),
                    columns,
                    result: Some(result),
                    plan: None,
                    reads: Vec::new(),
                    initial: None,
                },
            };
            files.push(file);
        }
        let outputs = (inner.tops.iter())
            .map(|top| {
                let schema = nodes[*top].plan.schema().as_arrow().clone();
                Ok(OutputFile {
                    node: renumbered[top],
                    schema: Some(protobuf::Schema::try_from(&schema).map_err(|error| {
                        datafusion::common::DataFusionError::External(Box::new(error))
                    })?),
                })
            })
            .collect::<Result<_>>()?;
        let file = GraphFile {
            version: GRAPH_VERSION,
            nodes: files,
            outputs,
            selections,
        };
        Ok(Page {
            bytes: file.encode_to_vec(),
            warnings,
        })
    }
}
