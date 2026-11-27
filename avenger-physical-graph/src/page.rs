//! A graph that a host planned for a page, as its file holds it, and its runs: the results of the
//! nodes that read no selection, the physical plans of the nodes that read one, which each run
//! binds to the states of its selections, and the results of the run with empty selections, which
//! the first run reads without computing anything.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::Cursor,
    sync::{Arc, Mutex},
};

use arrow::{
    array::RecordBatch,
    compute::SortOptions,
    datatypes::{Schema, SchemaRef},
    ipc::{reader::StreamReader, writer::StreamWriter},
};
use avenger_selection::{Clause, Resolution, SelectionState};
use avenger_selection_datafusion::{bind_selections, Owners};
use datafusion_common::{exec_datafusion_err, project_schema, Result};
use datafusion_execution::{config::SessionConfig, runtime_env::RuntimeEnv, TaskContext};
use datafusion_physical_plan::{execution_plan::reset_plan_states, ExecutionPlan};
use datafusion_proto::{physical_plan::AsExecutionPlan, protobuf::PhysicalPlanNode};
use datafusion_proto_common as protobuf;
use prost::Message;

use crate::{
    codec::{GraphCodec, Hosts},
    kept::{substitute, Kept},
    keys::{resolution_code, SelectionKey},
};

/// The version of a graph's file, which a reader checks before it reads the rest.
pub const GRAPH_VERSION: u32 = 1;

/// The input of a node that its plan does not scan, in [`NodeFile::inputs`].
pub const ABSENT: u32 = u32::MAX;

/// A graph's file.
#[derive(Clone, PartialEq, Message)]
pub struct GraphFile {
    #[prost(uint32, tag = "1")]
    pub version: u32,
    #[prost(message, repeated, tag = "2")]
    pub nodes: Vec<NodeFile>,
    #[prost(message, repeated, tag = "3")]
    pub outputs: Vec<OutputFile>,
    #[prost(message, repeated, tag = "4")]
    pub selections: Vec<SelectionFile>,
}

/// A node of a graph's file: the nodes that it reads, in an order where each node follows them,
/// the columns of its output that it computes, and either its result, for a node that reads no
/// selection, or its physical plan, with what it reads of the selections and its result with every
/// selection empty.
#[derive(Clone, PartialEq, Message)]
pub struct NodeFile {
    #[prost(uint32, repeated, tag = "1")]
    pub inputs: Vec<u32>,
    #[prost(uint32, repeated, tag = "2")]
    pub columns: Vec<u32>,
    #[prost(message, optional, tag = "3")]
    pub result: Option<ResultFile>,
    #[prost(bytes = "vec", optional, tag = "4")]
    pub plan: Option<Vec<u8>>,
    #[prost(message, repeated, tag = "5")]
    pub reads: Vec<ReadFile>,
    #[prost(message, optional, tag = "6")]
    pub initial: Option<ResultFile>,
}

/// A node's result: the columns of its output that it holds, whether its batches are in the order
/// of its rows and by which columns, and the batches as an Arrow IPC stream with their schema.
#[derive(Clone, PartialEq, Message)]
pub struct ResultFile {
    #[prost(uint32, repeated, tag = "1")]
    pub columns: Vec<u32>,
    #[prost(bool, tag = "2")]
    pub ordered: bool,
    #[prost(message, repeated, tag = "3")]
    pub ordering: Vec<OrderFile>,
    #[prost(bytes = "vec", tag = "4")]
    pub ipc: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct OrderFile {
    #[prost(uint32, tag = "1")]
    pub column: u32,
    #[prost(bool, tag = "2")]
    pub descending: bool,
    #[prost(bool, tag = "3")]
    pub nulls_first: bool,
}

/// What a node reads of one selection: the placeholder of the selection, and the owners whose
/// clauses it reads, every owner but `owners` or `owners` alone.
#[derive(Clone, PartialEq, Message)]
pub struct ReadFile {
    #[prost(string, tag = "1")]
    pub parameter: String,
    #[prost(bool, tag = "2")]
    pub all_but: bool,
    #[prost(string, repeated, tag = "3")]
    pub owners: Vec<String>,
}

/// An output of a graph: the node of a plan's result and the schema of the result.
#[derive(Clone, PartialEq, Message)]
pub struct OutputFile {
    #[prost(uint32, tag = "1")]
    pub node: u32,
    #[prost(message, optional, tag = "2")]
    pub schema: Option<protobuf::Schema>,
}

/// A selection of the document: its name, the placeholder of its state, and its resolution, `0`
/// for global, `1` for union, and `2` for intersect.
#[derive(Clone, PartialEq, Message)]
pub struct SelectionFile {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub placeholder: String,
    #[prost(uint32, tag = "3")]
    pub resolution: u32,
}

fn invalid(error: impl std::fmt::Display) -> datafusion_common::DataFusionError {
    exec_datafusion_err!("a graph's file: {error}")
}

impl ResultFile {
    /// The file of `kept`, whose batches take the schema `schema` of the columns that it holds.
    pub fn of(kept: &Kept, schema: SchemaRef) -> Result<Self> {
        let batches = kept.project(&kept.columns, schema.clone())?;
        let mut ipc = Vec::new();
        let mut writer = StreamWriter::try_new(&mut ipc, &schema)?;
        for batch in &batches {
            writer.write(batch)?;
        }
        writer.finish()?;
        drop(writer);
        Ok(Self {
            columns: kept.columns.iter().map(|column| *column as u32).collect(),
            ordered: kept.ordered,
            ordering: (kept.ordering.iter())
                .map(|(column, options)| OrderFile {
                    column: *column as u32,
                    descending: options.descending,
                    nulls_first: options.nulls_first,
                })
                .collect(),
            ipc,
        })
    }

    fn kept(&self) -> Result<Kept> {
        let reader = StreamReader::try_new(Cursor::new(&self.ipc), None)?;
        let batches = reader.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Kept::new(
            self.columns.iter().map(|column| *column as usize).collect(),
            batches,
            self.ordered,
            (self.ordering.iter())
                .map(|order| {
                    let options = SortOptions {
                        descending: order.descending,
                        nulls_first: order.nulls_first,
                    };
                    (order.column as usize, options)
                })
                .collect(),
        ))
    }
}

impl ReadFile {
    /// The file of a read of the selection whose placeholder is `parameter`.
    pub fn of(parameter: &str, owners: &Owners) -> Self {
        let (all_but, owners) = match owners {
            Owners::AllBut(skipped) => (true, skipped),
            Owners::Only(read) => (false, read),
        };
        Self {
            parameter: parameter.to_string(),
            all_but,
            owners: owners.iter().cloned().collect(),
        }
    }

    fn owners(&self) -> Owners {
        let owners: BTreeSet<String> = self.owners.iter().cloned().collect();
        match self.all_but {
            true => Owners::AllBut(owners),
            false => Owners::Only(owners),
        }
    }
}

impl SelectionFile {
    fn resolution(&self) -> Resolution {
        match self.resolution {
            1 => Resolution::Union,
            2 => Resolution::Intersect,
            _ => Resolution::Global,
        }
    }

    /// The file of the selection `name`, whose state is the placeholder `placeholder`.
    pub fn of(name: &str, placeholder: &str, resolution: Resolution) -> Self {
        Self {
            name: name.to_string(),
            placeholder: placeholder.to_string(),
            resolution: u32::from(resolution_code(resolution)),
        }
    }
}

/// The context in which a page's plans decode and run: one partition, with DataFusion's scalar,
/// nested, aggregate, and window functions and the scalar compositions of charts. The bound calls
/// of marks and compositions, the operators of selections, and the functions of `hosts`, such
/// as avenger-sql's library, decode through the graph's codec.
pub fn task_context(hosts: Hosts) -> TaskContext {
    let scalar = (datafusion_functions::all_default_functions().into_iter())
        .chain(datafusion_functions_nested::all_default_nested_functions())
        .chain(crate::functions::Combine::all())
        .flat_map(|function| {
            let names: Vec<String> = std::iter::once(function.name().to_string())
                .chain(function.aliases().iter().cloned())
                .collect();
            names.into_iter().map(move |name| (name, function.clone()))
        })
        .collect();
    let aggregate = (datafusion_functions_aggregate::all_default_aggregate_functions().into_iter())
        .flat_map(|function| {
            let names: Vec<String> = std::iter::once(function.name().to_string())
                .chain(function.aliases().iter().cloned())
                .collect();
            names.into_iter().map(move |name| (name, function.clone()))
        })
        .collect();
    let window = (datafusion_functions_window::all_default_window_functions().into_iter())
        .flat_map(|function| {
            let names: Vec<String> = std::iter::once(function.name().to_string())
                .chain(function.aliases().iter().cloned())
                .collect();
            names.into_iter().map(move |name| (name, function.clone()))
        })
        .collect();
    TaskContext::new(
        None,
        "avenger-page".to_string(),
        SessionConfig::new()
            .with_target_partitions(1)
            .with_extension(Arc::new(hosts)),
        scalar,
        HashMap::new(),
        aggregate,
        window,
        Arc::new(RuntimeEnv::default()),
    )
}

/// The physical plan of the bytes `bytes`, a node's plan in a graph's file, decoded in `ctx`.
pub fn decode_plan(bytes: &[u8], ctx: &TaskContext) -> Result<Arc<dyn ExecutionPlan>> {
    let node = PhysicalPlanNode::decode(bytes).map_err(invalid)?;
    crate::decode::decode(&node, ctx)
}

/// The bytes of the physical plan `plan` in a graph's file, with the functions of `hosts`.
pub fn encode_plan(plan: Arc<dyn ExecutionPlan>, hosts: &Hosts) -> Result<Vec<u8>> {
    let node = PhysicalPlanNode::try_from_physical_plan(plan, &GraphCodec::new(hosts.clone()))?;
    Ok(node.encode_to_vec())
}

/// The results that a page's runs computed, by their nodes and the parts of their keys that the
/// selections give.
type Cache = HashMap<(usize, Vec<SelectionKey>), Arc<Kept>>;

enum PageNode {
    Fixed(Arc<Kept>),
    Plan {
        plan: Arc<dyn ExecutionPlan>,
        reads: Vec<(String, Owners)>,
    },
}

/// A graph that runs in a page, as [`PageGraph::decode`] reads it from its file.
pub struct PageGraph {
    nodes: Vec<(PageNode, Vec<Option<usize>>, Vec<usize>)>,
    outputs: Vec<(usize, SchemaRef)>,
    selections: Vec<SelectionFile>,
    ctx: Arc<TaskContext>,
    cache: Mutex<Cache>,
}

impl PageGraph {
    /// The graph of the file `bytes`, whose plans decode in `ctx`, such as [`task_context`].
    pub fn decode(bytes: &[u8], ctx: Arc<TaskContext>) -> Result<Self> {
        let file = GraphFile::decode(bytes).map_err(invalid)?;
        if file.version != GRAPH_VERSION {
            return Err(invalid(format!(
                "version {} of the file, which this reader, of version {GRAPH_VERSION}, does not read",
                file.version
            )));
        }
        let mut cache = HashMap::new();
        let mut nodes = Vec::with_capacity(file.nodes.len());
        for (index, node) in file.nodes.iter().enumerate() {
            // An input that the node's plan does not scan, such as below a filter that is false,
            // has no node.
            let inputs = (node.inputs.iter())
                .map(|input| (*input != ABSENT).then_some(*input as usize))
                .collect();
            let columns = node.columns.iter().map(|column| *column as usize).collect();
            let page = match (&node.result, &node.plan) {
                (Some(result), _) => PageNode::Fixed(Arc::new(result.kept()?)),
                (None, Some(plan)) => {
                    let reads: Vec<(String, Owners)> = (node.reads.iter())
                        .map(|read| (read.parameter.clone(), read.owners()))
                        .collect();
                    if let Some(initial) = &node.initial {
                        let key = (reads.iter())
                            .map(|_| SelectionKey {
                                resolution: None,
                                clauses: Vec::new(),
                            })
                            .collect();
                        cache.insert((index, key), Arc::new(initial.kept()?));
                    }
                    PageNode::Plan {
                        plan: decode_plan(plan, &ctx)?,
                        reads,
                    }
                }
                (None, None) => {
                    return Err(invalid(format!("node {index} has no result and no plan")))
                }
            };
            nodes.push((page, inputs, columns));
        }
        let outputs = (file.outputs.iter())
            .map(|output| {
                let schema = output.schema.as_ref().ok_or_else(|| invalid("no schema"))?;
                let schema = Arc::new(Schema::try_from(schema).map_err(invalid)?);
                Ok((output.node as usize, schema))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            nodes,
            outputs,
            selections: file.selections,
            ctx,
            cache: Mutex::new(cache),
        })
    }

    /// The names of the document's selections.
    pub fn selections(&self) -> impl Iterator<Item = &str> {
        self.selections
            .iter()
            .map(|selection| selection.name.as_str())
    }

    /// The names of the selections that the outputs read.
    pub fn reads(&self) -> BTreeSet<String> {
        let placeholders: BTreeSet<&str> = (self.nodes.iter())
            .flat_map(|(node, _, _)| match node {
                PageNode::Plan { reads, .. } => reads.iter().map(|(p, _)| p.as_str()).collect(),
                PageNode::Fixed(_) => Vec::new(),
            })
            .collect();
        (self.selections.iter())
            .filter(|selection| {
                placeholders.contains(format!("${}", selection.placeholder).as_str())
                    || placeholders.contains(selection.placeholder.as_str())
            })
            .map(|selection| selection.name.clone())
            .collect()
    }

    /// The batches of each output, with the clauses `clauses` of the selections, by their names.
    /// A selection that `clauses` does not name is empty.
    pub async fn run(
        &self,
        clauses: &BTreeMap<String, Vec<Clause>>,
    ) -> Result<Vec<Vec<RecordBatch>>> {
        let states: BTreeMap<String, SelectionState> = (self.selections.iter())
            .map(|selection| {
                let clauses = clauses.get(&selection.name).cloned().unwrap_or_default();
                let state = SelectionState::new(selection.resolution(), clauses);
                (selection.placeholder.clone(), state)
            })
            .collect();
        let empty = SelectionState::default();
        let state_of = |parameter: &str| {
            states
                .get(parameter.trim_start_matches('$'))
                .unwrap_or(&empty)
        };
        let count = self.nodes.len();
        let mut results: Vec<Option<Arc<Kept>>> = vec![None; count];
        let mut keys: Vec<Option<Vec<SelectionKey>>> = vec![None; count];
        let mut needed = vec![false; count];
        for (node, _) in &self.outputs {
            needed[*node] = true;
        }
        // From the top down: a hit or a fixed result serves a node, and a miss needs its inputs.
        for index in (0..count).rev() {
            if !needed[index] {
                continue;
            }
            let (node, inputs, _) = &self.nodes[index];
            match node {
                PageNode::Fixed(kept) => results[index] = Some(kept.clone()),
                PageNode::Plan { reads, .. } => {
                    let key: Vec<SelectionKey> = (reads.iter())
                        .map(|(parameter, owners)| SelectionKey::new(state_of(parameter), owners))
                        .collect();
                    let hit = (self.cache.lock().unwrap_or_else(|e| e.into_inner()))
                        .get(&(index, key.clone()))
                        .cloned();
                    match hit {
                        Some(kept) => results[index] = Some(kept),
                        None => {
                            for input in inputs.iter().flatten() {
                                needed[*input] = true;
                            }
                        }
                    }
                    keys[index] = Some(key);
                }
            }
        }
        // From the bottom up: compute each miss over its inputs' results.
        for index in 0..count {
            if !needed[index] || results[index].is_some() {
                continue;
            }
            let (PageNode::Plan { plan, .. }, inputs, columns) = &self.nodes[index] else {
                continue;
            };
            let read: Vec<Option<Arc<Kept>>> = (inputs.iter())
                .map(|input| input.and_then(|input| results[input].clone()))
                .collect();
            let plan = substitute(plan.clone(), &read, 1)?;
            let plan = reset_plan_states(bind_selections(plan, &states)?)?;
            let kept = Arc::new(Kept::compute(plan, columns.clone(), self.ctx.clone()).await?);
            if let Some(key) = keys[index].take() {
                (self.cache.lock().unwrap_or_else(|e| e.into_inner()))
                    .insert((index, key), kept.clone());
            }
            results[index] = Some(kept);
        }
        (self.outputs.iter())
            .map(|(node, schema)| {
                let kept = results[*node]
                    .as_ref()
                    .ok_or_else(|| invalid(format!("output node {node} has no result")))?;
                let columns: Vec<usize> = (0..schema.fields().len()).collect();
                kept.project(&columns, schema.clone())
            })
            .collect()
    }
}

/// The schema of the columns `columns` of `schema`.
pub fn projected(schema: &SchemaRef, columns: &[usize]) -> Result<SchemaRef> {
    project_schema(schema, Some(&columns.to_vec()))
}
