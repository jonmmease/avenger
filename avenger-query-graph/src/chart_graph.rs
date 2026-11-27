//! The charts of one document as one graph: [`ChartGraph`] runs the plans of the charts for the
//! clauses of the document's selections, and returns the rows of each chart.
//!
//! [`ChartGraph::new`] compiles the plans over their sources, and its graph is live.
//! [`ChartGraph::bake`] runs a live graph once for empty selections and keeps what the plans read
//! below their selections, so the baked graph reads no source, and runs where the sources are
//! not, such as a page, without a server. A baked graph holds the tables that its plans scan,
//! each the result of a node that reads no selection, the plan of each chart that reads a
//! selection, above those tables, and each chart's rows for empty selections, which a host
//! draws before any selection changes. [`ChartGraph::encode`] writes a baked graph as a file,
//! and [`ChartGraph::decode`] reads the file as a baked graph, which runs as a native host runs
//! a graph.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    sync::Arc,
};

use avenger_chart_datafusion::codec::{Hosts, PlanCodec};
use avenger_selection::{Clause, Resolution, SelectionState};
use datafusion::{
    arrow::{
        array::RecordBatch,
        compute::SortOptions,
        datatypes::{DataType, Schema, SchemaRef},
        ipc::{reader::StreamReader, writer::StreamWriter},
    },
    catalog::TableProvider,
    common::{
        exec_datafusion_err, internal_err, plan_err, project_schema,
        tree_node::{Transformed, TransformedResult, TreeNode, TreeNodeRecursion},
        Column, DataFusionError, ParamValues, Result,
    },
    datasource::{provider_as_source, source_as_provider},
    execution::{SessionState, TaskContext},
    logical_expr::{
        expr::{Placeholder, ScalarFunction},
        expr_rewriter::NamePreserver,
        utils::merge_schema,
        ColumnarValue, Expr, LogicalPlan, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature,
        Volatility,
    },
};
use datafusion_proto_common as protobuf;
use prost::Message;

use crate::{
    cut::{cut_requests, cut_schema, kept_table, scan_under, CutSource, Kept},
    graph::{Nodes, QueryGraph},
    key::resolution_code,
    run::{node_key, projected, resolve, versions, Inputs},
    store::{ResultStore, StoreConfig},
};

/// The version of a graph's file, which a reader checks before it reads the rest.
pub const FILE_VERSION: u32 = 1;

/// The bytes of the results that a baked graph's store keeps, and of the largest that it keeps.
const STORE: StoreConfig = StoreConfig {
    budget: 256 << 20,
    max_result: 64 << 20,
};

/// The file of a baked graph.
#[derive(Clone, PartialEq, Message)]
pub struct ChartGraphFile {
    #[prost(uint32, tag = "1")]
    pub version: u32,
    #[prost(message, repeated, tag = "2")]
    pub tables: Vec<TableFile>,
    #[prost(message, repeated, tag = "3")]
    pub charts: Vec<ChartFile>,
    #[prost(message, repeated, tag = "4")]
    pub selections: Vec<SelectionFile>,
}

/// A table that the plans scan: the schema of its scan, whose columns are named by position,
/// the names of those columns in the plan that computed the table, and its rows, with the
/// columns that the plans can read.
#[derive(Clone, PartialEq, Message)]
pub struct TableFile {
    #[prost(message, optional, tag = "1")]
    pub schema: Option<protobuf::Schema>,
    #[prost(string, repeated, tag = "2")]
    pub names: Vec<String>,
    #[prost(message, optional, tag = "3")]
    pub result: Option<ResultFile>,
}

/// A chart of the document: the schema of its rows, its rows for empty selections, and its plan
/// when it reads a selection and its plan travels.
#[derive(Clone, PartialEq, Message)]
pub struct ChartFile {
    #[prost(message, optional, tag = "1")]
    pub schema: Option<protobuf::Schema>,
    #[prost(message, optional, tag = "2")]
    pub first: Option<ResultFile>,
    #[prost(bytes = "vec", optional, tag = "3")]
    pub plan: Option<Vec<u8>>,
}

/// A result: the columns of its node's output that it holds, whether its batches are in the order
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

/// A selection of the document: its name, the placeholder of its state without `$`, and its
/// resolution, `0` for global, `1` for union, and `2` for intersect.
#[derive(Clone, PartialEq, Message)]
pub struct SelectionFile {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub placeholder: String,
    #[prost(uint32, tag = "3")]
    pub resolution: u32,
}

fn invalid(error: impl std::fmt::Display) -> DataFusionError {
    exec_datafusion_err!("the file of a chart graph: {error}")
}

fn schema_file(schema: &Schema) -> Result<protobuf::Schema> {
    protobuf::Schema::try_from(schema).map_err(invalid)
}

fn schema_of(file: &Option<protobuf::Schema>) -> Result<SchemaRef> {
    let file = file.as_ref().ok_or_else(|| invalid("no schema"))?;
    Ok(Arc::new(Schema::try_from(file).map_err(invalid)?))
}

impl ChartGraphFile {
    /// The file of `bytes`, of a version that this reader reads.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let file = ChartGraphFile::decode(bytes).map_err(invalid)?;
        if file.version != FILE_VERSION {
            return Err(invalid(format!(
                "version {} of the file, which this reader, of version {FILE_VERSION}, does not read",
                file.version
            )));
        }
        Ok(file)
    }
}

impl ResultFile {
    /// The file of `kept`, whose batches take the schema `schema` of the columns that it holds.
    fn of(kept: &Kept, schema: SchemaRef) -> Result<Self> {
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

impl SelectionFile {
    fn resolution(&self) -> Resolution {
        match self.resolution {
            1 => Resolution::Union,
            2 => Resolution::Intersect,
            _ => Resolution::Global,
        }
    }
}

/// A selection of a document: its name, the placeholder of its state without `$`, its
/// resolution, and the columns that its tools test, or `None` when they are unknown, as in a
/// graph that a file gave.
#[derive(Clone, Debug)]
pub struct ChartSelection {
    pub name: String,
    pub placeholder: String,
    pub resolution: Resolution,
    pub fields: Option<Vec<String>>,
}

/// The file of a baked graph, with the number of its charts that travel with their plans, which
/// run again where the file runs, and a warning for each chart that reads a selection and
/// travels without its plan, so it does not follow the selections there. A file without a plan
/// shows the charts of its rows for every state of its selections, so a host can draw them
/// without the file.
#[derive(Clone, Debug)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub plans: usize,
    pub warnings: Vec<String>,
}

/// The columns that the tools of each of `selections` test, by its placeholder.
fn fields(selections: &[ChartSelection]) -> BTreeMap<String, Option<Vec<String>>> {
    (selections.iter())
        .map(|selection| (selection.placeholder.clone(), selection.fields.clone()))
        .collect()
}

/// A stand-in for a `selected` call whose state is unknown: a predicate that reads the columns
/// that the selection's tools test. It is volatile, so the optimizer neither folds nor drops it,
/// and a plan with it reads the columns that the plan reads for any state. It never runs.
#[derive(Debug, PartialEq, Eq, Hash)]
struct AnyState {
    signature: Signature,
}

impl ScalarUDFImpl for AnyState {
    fn name(&self) -> &str {
        "selected_for_any_state"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn invoke_with_args(&self, _: ScalarFunctionArgs) -> Result<ColumnarValue> {
        internal_err!("the stand-in of a selection's test never runs")
    }
}

/// The placeholder of `expr` when `expr` is a `selected` call on a placeholder.
fn selected_placeholder(expr: &Expr) -> Option<&str> {
    let Expr::ScalarFunction(ScalarFunction { func, args }) = expr else {
        return None;
    };
    match args.first() {
        Some(Expr::Placeholder(Placeholder { id, .. })) if func.name() == "selected" => Some(id),
        _ => None,
    }
}

/// `plan` with each `selected` call replaced by a stand-in that reads the call's other arguments
/// and the columns that `fields` names for its selection, by its placeholder without `$`, or
/// every column of the call's input when `fields` names none.
fn for_any_state(
    plan: LogicalPlan,
    fields: &BTreeMap<String, Option<Vec<String>>>,
) -> Result<LogicalPlan> {
    let stand_in = Arc::new(ScalarUDF::new_from_impl(AnyState {
        signature: Signature::variadic_any(Volatility::Volatile),
    }));
    plan.transform_up_with_subqueries(|plan| {
        let schema = merge_schema(&plan.inputs());
        let names = NamePreserver::new(&plan);
        plan.map_expressions(|expr| {
            let name = names.save(&expr);
            let replaced = expr.transform_up(|expr| {
                let Some(id) = selected_placeholder(&expr) else {
                    return Ok(Transformed::no(expr));
                };
                let known = fields.get(id.trim_start_matches('$')).cloned().flatten();
                let columns: Vec<Column> = match known {
                    Some(names) => (names.iter())
                        .flat_map(|name| schema.columns_with_unqualified_name(name))
                        .collect(),
                    None => schema.columns(),
                };
                let Expr::ScalarFunction(ScalarFunction { args, .. }) = expr else {
                    unreachable!("a `selected` call");
                };
                // The first argument is the placeholder, and the others can read columns, as the
                // struct of a faceted plot's partition does.
                let mut read: Vec<Expr> = args.into_iter().skip(1).collect();
                read.extend(columns.into_iter().map(Expr::Column));
                // A call always has an argument, so the stand-in is a call of one at least.
                read.push(Expr::Literal(true.into(), None));
                Ok(Transformed::yes(stand_in.call(read)))
            })?;
            Ok(replaced.update_data(|expr| name.restore(expr)))
        })
    })
    .data()
}

/// `plan` with the placeholder of each parameter of `ids` replaced by its value in `params`. The
/// placeholders of selections stay, where `LogicalPlan::with_param_values` fails on a
/// placeholder without a value.
fn with_params(
    plan: LogicalPlan,
    ids: &BTreeSet<String>,
    params: &ParamValues,
) -> Result<LogicalPlan> {
    plan.transform_up_with_subqueries(|plan| {
        let names = NamePreserver::new(&plan);
        plan.map_expressions(|expr| {
            let name = names.save(&expr);
            let bound = expr.transform_up(|expr| match &expr {
                Expr::Placeholder(Placeholder { id, .. }) if ids.contains(id) => {
                    let value = params.get_placeholders_with_values(id)?;
                    Ok(Transformed::yes(Expr::Literal(value.value, value.metadata)))
                }
                _ => Ok(Transformed::no(expr)),
            })?;
            Ok(bound.update_data(|expr| name.restore(expr)))
        })
    })
    .data()
}

/// The names of the columns of `plan`'s output.
fn names(plan: &LogicalPlan) -> Vec<String> {
    let fields = plan.schema().fields().iter();
    fields.map(|field| field.name().clone()).collect()
}

/// A table of a baked graph: the schema of its scan, the names of its scan's columns in the plan
/// that computed the table, its rows, with the columns that the plans can read, and the provider
/// through which the graph's plans scan it.
#[derive(Clone)]
struct BakedTable {
    scan: SchemaRef,
    names: Vec<String>,
    kept: Arc<Kept>,
    provider: Arc<dyn TableProvider>,
}

/// The tables of a baked graph as its bake finds them, with the node of each.
#[derive(Default)]
struct Tables {
    nodes: Vec<usize>,
    tables: Vec<BakedTable>,
}

/// What a bake knows of the graph that it runs: the nodes, the columns of each node that the
/// plans read for any state, and the result of each node for empty selections.
struct Bake<'a> {
    nodes: &'a Nodes,
    columns: Vec<BTreeSet<usize>>,
    results: Vec<Option<Arc<Kept>>>,
}

impl Bake<'_> {
    /// The columns of the node at `index` that its table holds: the columns that the plans read,
    /// or its first column when they read only its rows.
    fn table_columns(&self, index: usize) -> Vec<usize> {
        let columns: Vec<usize> = self.columns[index].iter().copied().collect();
        let empty = self.nodes.nodes[index].plan.schema().fields().is_empty();
        match columns.is_empty() && !empty {
            true => vec![0],
            false => columns,
        }
    }

    /// The result of the node at `index` with its table's columns.
    fn table(&self, index: usize) -> Result<Kept> {
        let Some(kept) = &self.results[index] else {
            return internal_err!("the bake of a graph has no result of the node {index}");
        };
        let columns = self.table_columns(index);
        let scan = cut_schema(&self.nodes.nodes[index].plan);
        let schema = project_schema(&scan, Some(&columns))?;
        let batches = kept.project(&columns, schema)?;
        Ok(Kept::new(
            columns,
            batches,
            kept.ordered,
            kept.ordering.clone(),
        ))
    }

    /// The plan of the node at `index` with the plans of the nodes below it that read a
    /// selection in place of their cuts, and a scan of a table of `tables` at each cut over a
    /// node that reads none. A cut is `cut_scan`'s projection over the scan of an input, with the
    /// schema of the plan below it, so the plan below replaces the projection.
    fn assemble(&self, index: usize, tables: &mut Tables) -> Result<LogicalPlan> {
        let node = &self.nodes.nodes[index];
        node.plan
            .clone()
            .transform_down_with_subqueries(|plan| {
                let LogicalPlan::Projection(projection) = &plan else {
                    return Ok(Transformed::no(plan));
                };
                let LogicalPlan::TableScan(scan) = projection.input.as_ref() else {
                    return Ok(Transformed::no(plan));
                };
                let Some(cut) = scan.source.as_ref().downcast_ref::<CutSource>() else {
                    return Ok(Transformed::no(plan));
                };
                let input = node.inputs[cut.input];
                if !self.nodes.nodes[input].reads.selections.is_empty() {
                    let below = self.assemble(input, tables)?;
                    return Ok(Transformed::new(below, true, TreeNodeRecursion::Jump));
                }
                let position = match tables.nodes.iter().position(|node| *node == input) {
                    Some(position) => position,
                    None => {
                        let below = &self.nodes.nodes[input].plan;
                        let kept = Arc::new(self.table(input)?);
                        let scan = cut_schema(below);
                        let names = names(below);
                        tables.nodes.push(input);
                        tables.tables.push(BakedTable {
                            provider: kept_table(scan.clone(), kept.clone(), names.clone()),
                            scan,
                            names,
                            kept,
                        });
                        tables.nodes.len() - 1
                    }
                };
                let source = provider_as_source(tables.tables[position].provider.clone());
                let scan = scan_under(&projection.schema, &format!("table_{position}"), source)?;
                Ok(Transformed::new(scan, true, TreeNodeRecursion::Jump))
            })
            .data()
    }
}

/// The positions among `tables` of the tables that `plan` scans, in the order of its scans.
fn tables_of(plan: &LogicalPlan, tables: &[BakedTable]) -> Result<Vec<usize>> {
    let mut found = Vec::new();
    plan.apply_with_subqueries(|plan| {
        if let LogicalPlan::TableScan(scan) = plan {
            if let Ok(provider) = source_as_provider(&scan.source) {
                let position =
                    (tables.iter()).position(|table| Arc::ptr_eq(&table.provider, &provider));
                if let Some(position) = position {
                    if !found.contains(&position) {
                        found.push(position);
                    }
                }
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(found)
}

/// A chart of a graph: the schema of its rows, its rows for empty selections, which a baked
/// graph holds, and the position of its plan among the graph's plans, when it has one.
#[derive(Clone)]
struct Chart {
    schema: SchemaRef,
    first: Option<Arc<Kept>>,
    plan: Option<usize>,
}

/// Where the plans of a graph read their rows.
#[derive(Clone)]
enum Source {
    /// The sources of the charts' plans `plans`, which the graph compiled on `store` in `state`.
    Live {
        store: ResultStore,
        state: SessionState,
        plans: Vec<LogicalPlan>,
    },
    /// The graph's own tables `tables`, which its plans `plans` scan in `state`.
    Baked {
        state: SessionState,
        tables: Vec<BakedTable>,
        plans: Vec<LogicalPlan>,
    },
}

/// The charts of one document as one graph, which returns the rows of each chart for the clauses
/// of the document's selections. A live graph reads the sources of its plans. A baked graph
/// reads its own tables, so it runs where the sources are not, and it encodes as a file. Clones
/// share the graph.
#[derive(Clone)]
pub struct ChartGraph {
    graph: Option<QueryGraph>,
    charts: Vec<Chart>,
    selections: Vec<ChartSelection>,
    params: ParamValues,
    source: Source,
}

impl ChartGraph {
    /// The live graph of `plans`, the unoptimized plans of one document's charts, whose
    /// parameters and selection states are placeholders, with the values `params` of the
    /// parameters and the document's selections `selections`. It compiles the plans on `store`
    /// with `state`'s settings, as [`ResultStore::compile`] does, and runs nothing. Its plans
    /// hold the aggregates of their charts and not a front end's declarations of them, so the
    /// graph bakes.
    pub fn new(
        store: &ResultStore,
        state: &SessionState,
        plans: Vec<LogicalPlan>,
        params: ParamValues,
        selections: Vec<ChartSelection>,
    ) -> Result<Self> {
        let charts = (plans.iter().enumerate())
            .map(|(position, plan)| Chart {
                schema: Arc::new(plan.schema().as_arrow().clone()),
                first: None,
                plan: Some(position),
            })
            .collect();
        let graph = match plans.is_empty() {
            true => None,
            false => Some(store.compile(state, plans.clone())?),
        };
        Ok(Self {
            graph,
            charts,
            selections,
            params,
            source: Source::Live {
                store: store.clone(),
                state: state.clone(),
                plans,
            },
        })
    }

    /// Whether the graph is baked, so its plans read no source and it encodes.
    pub fn is_baked(&self) -> bool {
        matches!(self.source, Source::Baked { .. })
    }

    /// The graph baked: a graph that reads no source, whose plans plan in `session`, such as
    /// avenger-sql's `baked_session`. A baked graph bakes to itself.
    ///
    /// The bake runs the graph once for empty selections. A chart that reads no selection keeps
    /// its rows. A chart that reads one keeps its rows and its plan, above the tables of the
    /// nodes that read none, with the values of its parameters. A table holds the columns that
    /// the plans read for any state of the selections, and the columns that each selection's
    /// tools test.
    pub async fn bake(&self, session: &SessionState) -> Result<ChartGraph> {
        let Source::Live {
            store,
            state,
            plans,
        } = &self.source
        else {
            return Ok(self.clone());
        };
        let cut = store.compile_cut_scans(state, plans.clone())?;
        let inner = &cut.inner;
        let params = &self.params;
        let selections = &self.selections;
        let nodes = inner.direct.clone();
        let count = nodes.nodes.len();
        let fields = fields(selections);

        // From the top down: the columns that the plans read of each node for any state of the
        // selections, through the nodes that read a selection.
        let mut columns: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
        let mut reached = vec![false; count];
        for top in &nodes.tops {
            reached[*top] = true;
            columns[*top] = (0..nodes.nodes[*top].plan.schema().fields().len()).collect();
        }
        for index in (0..count).rev() {
            let node = &nodes.nodes[index];
            if !reached[index] || node.reads.selections.is_empty() {
                continue;
            }
            let shared: Vec<&String> = (node.reads.params.iter())
                .filter(|id| node.reads.selections.contains_key(*id))
                .collect();
            if let Some(id) = shared.first() {
                return plan_err!(
                    "the parameter {id} has the name of a selection, which a baked graph's plans cannot hold"
                );
            }
            if columns[index].is_empty() && !node.plan.schema().fields().is_empty() {
                // A reader that needs only the rows keeps one column, as a run does.
                columns[index].insert(0);
            }
            let plan = for_any_state(projected(node, &columns[index])?, &fields)?;
            let plan = inner
                .state
                .optimize(&plan.with_param_values(params.clone())?)?;
            let wanted = cut_requests(&plan, node.inputs.len())?;
            for (position, wanted) in wanted.into_iter().enumerate() {
                if let Some(wanted) = wanted {
                    let input = node.inputs[position];
                    columns[input].extend(wanted);
                    reached[input] = true;
                }
            }
        }

        // The results for empty selections, with each table's columns.
        let states = (selections.iter())
            .map(|selection| {
                let state = SelectionState::new(selection.resolution, Vec::new());
                (selection.placeholder.clone(), state)
            })
            .collect();
        let inputs = Inputs {
            params: params.clone(),
            selections: states,
            ..Inputs::default()
        };
        let extra: Vec<(usize, BTreeSet<usize>)> = (0..count)
            .filter(|index| reached[*index] && nodes.nodes[*index].reads.selections.is_empty())
            .map(|index| (index, columns[index].clone()))
            .collect();
        let resolved = resolve(&cut, &inputs, &extra).await?;
        let bake = Bake {
            nodes: &nodes,
            columns,
            results: resolved.results,
        };

        let mut tables = Tables::default();
        let mut charts = Vec::with_capacity(nodes.tops.len());
        let mut baked = Vec::new();
        for (position, top) in nodes.tops.iter().enumerate() {
            let node = &nodes.nodes[*top];
            let Some(first) = &bake.results[*top] else {
                return internal_err!("the bake of a graph has no result of the chart {position}");
            };
            let plan = match node.reads.selections.is_empty() {
                true => None,
                false => {
                    let plan = bake.assemble(*top, &mut tables)?;
                    baked.push(with_params(plan, &node.reads.params, params)?);
                    Some(baked.len() - 1)
                }
            };
            charts.push(Chart {
                schema: Arc::new(node.plan.schema().as_arrow().clone()),
                first: Some(first.clone()),
                plan,
            });
        }
        Self::baked(session, tables.tables, baked, charts, selections.clone())
    }

    /// The baked graph of `tables`, of the plans `plans` over them, which it compiles in `state`
    /// on a store of its own, and of `charts`. Each chart's rows for empty selections go into
    /// the store, so the first run computes nothing.
    fn baked(
        state: &SessionState,
        tables: Vec<BakedTable>,
        plans: Vec<LogicalPlan>,
        charts: Vec<Chart>,
        selections: Vec<ChartSelection>,
    ) -> Result<Self> {
        let graph = match plans.is_empty() {
            true => None,
            false => Some(ResultStore::new(STORE).compile(state, plans.clone())?),
        };
        let graph = ChartGraph {
            graph,
            charts,
            selections,
            params: ParamValues::List(Vec::new()),
            source: Source::Baked {
                state: state.clone(),
                tables,
                plans,
            },
        };
        graph.seed()?;
        Ok(graph)
    }

    /// The file of the graph, which is baked. A chart that reads no selection travels as its
    /// rows. A chart that reads one travels as its rows and its plan, with the tables that the
    /// plan scans. A plan that does not encode, or that does not decode and plan in the graph's
    /// session, does not travel, and a warning names its chart and the reason.
    pub async fn encode(&self) -> Result<Encoded> {
        let Source::Baked {
            state,
            tables,
            plans,
        } = &self.source
        else {
            return plan_err!("a graph encodes after it bakes, since its file holds no source");
        };
        let fields = fields(&self.selections);
        let hosts = Hosts::of(state.config());
        let context = TaskContext::from(state);
        // The tables of the file, by their positions among the graph's.
        let mut used: Vec<usize> = Vec::new();
        let mut charts = Vec::with_capacity(self.charts.len());
        let mut warnings = Vec::new();
        for (position, chart) in self.charts.iter().enumerate() {
            let Some(first) = &chart.first else {
                return internal_err!("the chart {position} of a baked graph has no rows");
            };
            let mut file = ChartFile {
                schema: Some(schema_file(&chart.schema)?),
                first: Some(ResultFile::of(first, chart.schema.clone())?),
                plan: None,
            };
            if let (Some(index), Some(graph)) = (chart.plan, &self.graph) {
                let plan = &plans[index];
                // The tables of a chart whose plan does not travel stay out of the file.
                let mut with_chart = used.clone();
                for table in tables_of(plan, tables)? {
                    if !with_chart.contains(&table) {
                        with_chart.push(table);
                    }
                }
                let travels: Result<Vec<u8>> = async {
                    let providers = (with_chart.iter())
                        .map(|table| tables[*table].provider.clone())
                        .collect();
                    let codec = PlanCodec::new(hosts.clone(), providers);
                    let bytes = codec.encode(plan)?;
                    // The reader's check: the plan decodes, optimizes, and plans in the graph's
                    // session, for a state that tests the selections' columns.
                    let decoded = for_any_state(codec.decode(&bytes, &context)?, &fields)?;
                    state
                        .create_physical_plan(&state.optimize(&decoded)?)
                        .await?;
                    Ok(bytes)
                }
                .await;
                match travels {
                    Ok(bytes) => {
                        file.plan = Some(bytes);
                        used = with_chart;
                    }
                    Err(error) => {
                        let inner = &graph.inner;
                        let node = &inner.direct.nodes[inner.direct.tops[index]];
                        warnings.push(format!(
                            "the chart {position} reads {} and does not follow it from the graph's file: {error}",
                            (node.reads.selections.keys().cloned())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    }
                }
            }
            charts.push(file);
        }

        let tables = (used.iter())
            .map(|table| {
                let table = &tables[*table];
                let schema = project_schema(&table.scan, Some(&table.kept.columns))?;
                Ok(TableFile {
                    schema: Some(schema_file(&table.scan)?),
                    names: table.names.clone(),
                    result: Some(ResultFile::of(&table.kept, schema)?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let file = ChartGraphFile {
            version: FILE_VERSION,
            tables,
            charts,
            selections: (self.selections.iter())
                .map(|selection| SelectionFile {
                    name: selection.name.clone(),
                    placeholder: selection.placeholder.clone(),
                    resolution: u32::from(resolution_code(selection.resolution)),
                })
                .collect(),
        };
        Ok(Encoded {
            plans: (file.charts.iter())
                .filter(|chart| chart.plan.is_some())
                .count(),
            bytes: file.encode_to_vec(),
            warnings,
        })
    }

    /// The baked graph of the file `bytes`, whose plans decode and plan in the session `state`,
    /// such as avenger-sql's `baked_session`.
    pub fn decode(bytes: &[u8], state: &SessionState) -> Result<Self> {
        let file = ChartGraphFile::from_bytes(bytes)?;
        let tables = (file.tables.iter())
            .map(|table| {
                let result = table.result.as_ref().ok_or_else(|| invalid("no rows"))?;
                let scan = schema_of(&table.schema)?;
                let kept = Arc::new(result.kept()?);
                Ok(BakedTable {
                    provider: kept_table(scan.clone(), kept.clone(), table.names.clone()),
                    scan,
                    names: table.names.clone(),
                    kept,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let providers = tables.iter().map(|table| table.provider.clone()).collect();
        let codec = PlanCodec::new(Hosts::of(state.config()), providers);
        let context = TaskContext::from(state);
        let mut plans = Vec::new();
        let mut charts = Vec::with_capacity(file.charts.len());
        for chart in &file.charts {
            let first = chart.first.as_ref().ok_or_else(|| invalid("no rows"))?;
            let plan = match &chart.plan {
                Some(bytes) => {
                    plans.push(codec.decode(bytes, &context)?);
                    Some(plans.len() - 1)
                }
                None => None,
            };
            charts.push(Chart {
                schema: schema_of(&chart.schema)?,
                first: Some(Arc::new(first.kept()?)),
                plan,
            });
        }
        let selections = (file.selections.iter())
            .map(|selection| ChartSelection {
                name: selection.name.clone(),
                placeholder: selection.placeholder.clone(),
                resolution: selection.resolution(),
                fields: None,
            })
            .collect();
        Self::baked(state, tables, plans, charts, selections)
    }

    /// Puts each chart's rows for empty selections into the store, as the kept result of the top
    /// node of its plan. A top node that is not kept, because its plan calls a volatile or a
    /// stable function, takes no rows, and each run computes it.
    fn seed(&self) -> Result<()> {
        let Some(graph) = &self.graph else {
            return Ok(());
        };
        let inner = &graph.inner;
        let inputs = self.inputs(&BTreeMap::new());
        let versions = versions(inner);
        for chart in &self.charts {
            let (Some(plan), Some(first)) = (chart.plan, &chart.first) else {
                continue;
            };
            let node = &inner.direct.nodes[inner.direct.tops[plan]];
            if node.kept {
                let key = node_key(inner, node, &inputs, &versions)?;
                let mut store = inner.store.state();
                store.keep(inner.store.config(), key, first.clone());
            }
        }
        Ok(())
    }

    /// The inputs of a run with `clauses`, by the names of their selections.
    fn inputs(&self, clauses: &BTreeMap<String, Vec<Clause>>) -> Inputs {
        let selections = (self.selections.iter())
            .map(|selection| {
                let clauses = clauses.get(&selection.name).cloned().unwrap_or_default();
                let state = SelectionState::new(selection.resolution, clauses);
                (selection.placeholder.clone(), state)
            })
            .collect();
        Inputs {
            params: self.params.clone(),
            selections,
            ..Inputs::default()
        }
    }

    /// The graph of the charts' plans, or `None` when no chart has a plan, as in a baked graph
    /// whose charts read no selection.
    pub fn graph(&self) -> Option<&QueryGraph> {
        self.graph.as_ref()
    }

    /// The names of the document's selections.
    pub fn selections(&self) -> impl Iterator<Item = &str> {
        self.selections
            .iter()
            .map(|selection| selection.name.as_str())
    }

    /// The names of the selections that the charts read, so a caller runs the graph when one of
    /// them changes.
    pub fn reads(&self) -> BTreeSet<String> {
        let Some(graph) = &self.graph else {
            return BTreeSet::new();
        };
        let placeholders: BTreeSet<&str> = (graph.inner.direct.nodes.iter())
            .flat_map(|node| node.reads.selections.keys())
            .map(|id| id.trim_start_matches('$'))
            .collect();
        (self.selections.iter())
            .filter(|selection| placeholders.contains(selection.placeholder.as_str()))
            .map(|selection| selection.name.clone())
            .collect()
    }

    /// The rows of each chart, in the order of the charts, with the clauses `clauses` of the
    /// selections, by their names. A selection that `clauses` does not name is empty.
    pub async fn run(
        &self,
        clauses: &BTreeMap<String, Vec<Clause>>,
    ) -> Result<Vec<Vec<RecordBatch>>> {
        let mut batches = match &self.graph {
            Some(graph) => graph.run(&self.inputs(clauses)).await?.batches,
            None => Vec::new(),
        };
        (self.charts.iter().enumerate())
            .map(|(position, chart)| match (chart.plan, &chart.first) {
                (Some(plan), _) => Ok(std::mem::take(&mut batches[plan])),
                (None, Some(first)) => {
                    let columns: Vec<usize> = (0..chart.schema.fields().len()).collect();
                    first.project(&columns, chart.schema.clone())
                }
                (None, None) => internal_err!("the chart {position} has no plan and no rows"),
            })
            .collect()
    }
}
