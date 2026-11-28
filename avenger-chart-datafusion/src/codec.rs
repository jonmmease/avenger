//! The codec of the logical plans that build charts, for a host that sends a plan to where it
//! runs, such as a page. DataFusion's messages hold the plan, and the codec adds what they do not
//! know: the bound calls of marks and compositions, the functions and the extension nodes of the
//! host that planned the plan, and the tables that the plan scans.
//!
//! What the codec adds is in the messages of package `avenger.chart.datafusion.v1`, which
//! [`proto`](crate::proto) holds: a scan travels as a `TableScan`, a function with state as its
//! name and a `FunctionState`, and a node of a host as a `HostNode`. A function without state
//! travels as its name alone, and decodes from the functions of the session that reads the plan.

use std::{any::Any, fmt::Debug, sync::Arc};

use arrow::datatypes::SchemaRef;
use datafusion::{
    catalog::TableProvider,
    common::{exec_datafusion_err, not_impl_err, DataFusionError, Result, TableReference},
    execution::{config::SessionConfig, TaskContext},
    logical_expr::{
        AggregateUDF, Extension, LogicalPlan, ScalarUDF, UserDefinedLogicalNode, WindowUDF,
    },
};
use datafusion_proto::{
    bytes::{
        logical_plan_from_bytes_with_extension_codec, logical_plan_to_bytes_with_extension_codec,
    },
    logical_plan::LogicalExtensionCodec,
};
use prost::Message;

use crate::{
    functions::{CompositionInstance, MarkInstance},
    proto::{
        function_state::State, table_scan::Table, CompositionState, FunctionState, HostNode,
        MarkState, TableScan,
    },
};

/// The codec of the functions and the extension nodes that a host adds to its plans beyond
/// DataFusion's, such as the library and the transforms of avenger-sql. Each travels as the bytes
/// of its state, so a reader that knows the same host builds what the host planned. A host
/// leaves out the kinds that it adds none of.
pub trait HostCodec: Debug + Send + Sync {
    /// The host's name, such as `avenger-sql`, which a session's hosts hold once.
    fn name(&self) -> &str;

    /// The bytes of the state of `udf`, when it is one of the host's functions with state.
    fn encode_udf(&self, _udf: &ScalarUDF) -> Option<Result<Vec<u8>>> {
        None
    }

    /// The function named `name` with the state `bytes`, when it is one of the host's functions.
    fn decode_udf(&self, _name: &str, _bytes: &[u8]) -> Option<Result<Arc<ScalarUDF>>> {
        None
    }

    /// The bytes of the state of `udaf`, when it is one of the host's aggregate functions.
    fn encode_udaf(&self, _udaf: &AggregateUDF) -> Option<Result<Vec<u8>>> {
        None
    }

    /// The aggregate function named `name` with the state `bytes`, when it is one of the host's.
    fn decode_udaf(&self, _name: &str, _bytes: &[u8]) -> Option<Result<Arc<AggregateUDF>>> {
        None
    }

    /// The bytes of the state of `udwf`, when it is one of the host's window functions.
    fn encode_udwf(&self, _udwf: &WindowUDF) -> Option<Result<Vec<u8>>> {
        None
    }

    /// The window function named `name` with the state `bytes`, when it is one of the host's.
    fn decode_udwf(&self, _name: &str, _bytes: &[u8]) -> Option<Result<Arc<WindowUDF>>> {
        None
    }

    /// The bytes of `node` without its inputs, when it is one of the host's extension nodes.
    fn encode_node(&self, _node: &dyn UserDefinedLogicalNode) -> Option<Result<Vec<u8>>> {
        None
    }

    /// The extension node of `bytes`, which the host wrote, over the decoded `inputs`.
    fn decode_node(
        &self,
        _bytes: &[u8],
        _inputs: &[LogicalPlan],
    ) -> Result<Arc<dyn UserDefinedLogicalNode>> {
        not_impl_err!("the host {} has no extension nodes", self.name())
    }
}

/// The hosts whose functions and nodes a session's plans hold. A session holds them as an
/// extension of its configuration, so the host that writes a plan and the session that reads it
/// find them.
#[derive(Clone, Debug, Default)]
pub struct Hosts(pub Vec<Arc<dyn HostCodec>>);

impl Hosts {
    /// The hosts with `host`, unless they hold a host of its name.
    pub fn with(mut self, host: Arc<dyn HostCodec>) -> Hosts {
        if !self.0.iter().any(|known| known.name() == host.name()) {
            self.0.push(host);
        }
        self
    }

    /// The hosts that `config` holds, or none.
    pub fn of(config: &SessionConfig) -> Hosts {
        (config.get_extension::<Hosts>())
            .map(|hosts| hosts.as_ref().clone())
            .unwrap_or_default()
    }
}

fn invalid(error: impl std::fmt::Display) -> DataFusionError {
    exec_datafusion_err!("the bytes of a plan's extension: {error}")
}

/// Writes `state`, the state of a function, into `buf`. DataFusion drops an error of a function's
/// state, and a reader finds a function that wrote nothing by its name in its session. So a state
/// that failed writes `failed`, a state of its kind that holds nothing, which keeps the reader
/// from its session: a mark or a composition that holds nothing is an error there, and a host
/// reads its own.
fn encode_state(state: Result<State>, failed: State, buf: &mut Vec<u8>) -> Result<()> {
    let (state, result) = match state {
        Ok(state) => (state, Ok(())),
        Err(error) => (failed, Err(error)),
    };
    let state = FunctionState { state: Some(state) };
    state.encode(buf).map_err(invalid)?;
    result
}

/// Writes the state `bytes` of a function of a host into `buf`.
fn encode_host(bytes: Result<Vec<u8>>, buf: &mut Vec<u8>) -> Result<()> {
    encode_state(bytes.map(State::Host), State::Host(Vec::new()), buf)
}

/// The state of a function in `buf`, or `None` when `buf` holds no state.
fn decode_state(buf: &[u8]) -> Result<Option<State>> {
    Ok(FunctionState::decode(buf).map_err(invalid)?.state)
}

/// The codec of a logical plan, with the functions and the nodes of `hosts`. A scan travels as
/// the position of its table in `tables`.
#[derive(Debug, Default)]
pub struct PlanCodec {
    hosts: Hosts,
    tables: Vec<Arc<dyn TableProvider>>,
}

impl PlanCodec {
    /// The codec of the plans that scan `tables`, with the functions and the nodes of `hosts`.
    pub fn new(hosts: Hosts, tables: Vec<Arc<dyn TableProvider>>) -> PlanCodec {
        PlanCodec { hosts, tables }
    }

    /// The bytes of `plan`.
    pub fn encode(&self, plan: &LogicalPlan) -> Result<Vec<u8>> {
        Ok(logical_plan_to_bytes_with_extension_codec(plan, self)?.to_vec())
    }

    /// The plan of `bytes`, whose functions without state are the functions of `ctx`.
    pub fn decode(&self, bytes: &[u8], ctx: &TaskContext) -> Result<LogicalPlan> {
        logical_plan_from_bytes_with_extension_codec(bytes, ctx, self)
    }
}

impl LogicalExtensionCodec for PlanCodec {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[LogicalPlan],
        _ctx: &TaskContext,
    ) -> Result<Extension> {
        let decoded = HostNode::decode(buf).map_err(invalid)?;
        let Some(host) = (self.hosts.0.iter()).find(|host| host.name() == decoded.host) else {
            return not_impl_err!(
                "a node of the host {}, which the reader lacks",
                decoded.host
            );
        };
        Ok(Extension {
            node: host.decode_node(&decoded.bytes, inputs)?,
        })
    }

    fn try_encode(&self, node: &Extension, buf: &mut Vec<u8>) -> Result<()> {
        let encoded = (self.hosts.0.iter()).find_map(|host| {
            let bytes = host.encode_node(node.node.as_ref())?;
            Some((host.name().to_string(), bytes))
        });
        let Some((host, bytes)) = encoded else {
            return not_impl_err!("no host encodes the node {}", node.node.name());
        };
        let bytes = bytes?;
        HostNode { host, bytes }.encode(buf).map_err(invalid)
    }

    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        table_ref: &TableReference,
        _schema: SchemaRef,
        _ctx: &TaskContext,
    ) -> Result<Arc<dyn TableProvider>> {
        let table = TableScan::decode(buf).ok().and_then(|scan| scan.table);
        match table.and_then(|Table::Position(at)| self.tables.get(at as usize)) {
            Some(table) => Ok(table.clone()),
            None => Err(invalid(format!("no table for the scan of {table_ref}"))),
        }
    }

    fn try_encode_table_provider(
        &self,
        table_ref: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> Result<()> {
        let position = (self.tables.iter()).position(|table| Arc::ptr_eq(table, &node));
        match position {
            Some(position) => {
                let table = Table::Position(position as u32);
                let scan = TableScan { table: Some(table) };
                scan.encode(buf).map_err(invalid)
            }
            None => not_impl_err!("the plan scans {table_ref}, which is not one of its tables"),
        }
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        if let Some(State::Host(bytes)) = decode_state(buf)? {
            if let Some(udf) = (self.hosts.0.iter()).find_map(|host| host.decode_udf(name, &bytes))
            {
                return udf;
            }
        }
        not_impl_err!("the reader does not know the function {name}")
    }

    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        match (self.hosts.0.iter()).find_map(|host| host.encode_udf(node)) {
            Some(bytes) => encode_host(bytes, buf),
            None => Ok(()),
        }
    }

    fn try_decode_udaf(&self, name: &str, buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        match decode_state(buf)? {
            Some(State::Mark(mark)) => Ok(Arc::new(AggregateUDF::new_from_impl(
                MarkInstance::from_state(&mark)?,
            ))),
            Some(State::Composition(composition)) => Ok(Arc::new(AggregateUDF::new_from_impl(
                CompositionInstance::from_state(&composition)?,
            ))),
            Some(State::Host(bytes)) => (self.hosts.0.iter())
                .find_map(|host| host.decode_udaf(name, &bytes))
                .unwrap_or_else(|| not_impl_err!("the reader does not know the aggregate {name}")),
            None => not_impl_err!("the reader does not know the aggregate {name}"),
        }
    }

    fn try_encode_udaf(&self, node: &AggregateUDF, buf: &mut Vec<u8>) -> Result<()> {
        let inner = node.inner().as_ref() as &dyn Any;
        if let Some(mark) = inner.downcast_ref::<MarkInstance>() {
            let failed = State::Mark(MarkState::default());
            encode_state(mark.to_state().map(State::Mark), failed, buf)
        } else if let Some(composition) = inner.downcast_ref::<CompositionInstance>() {
            let failed = State::Composition(CompositionState::default());
            encode_state(composition.to_state().map(State::Composition), failed, buf)
        } else if let Some(bytes) = (self.hosts.0.iter()).find_map(|host| host.encode_udaf(node)) {
            encode_host(bytes, buf)
        } else {
            Ok(())
        }
    }

    fn try_decode_udwf(&self, name: &str, buf: &[u8]) -> Result<Arc<WindowUDF>> {
        if let Some(State::Host(bytes)) = decode_state(buf)? {
            let udwf = (self.hosts.0.iter()).find_map(|host| host.decode_udwf(name, &bytes));
            if let Some(udwf) = udwf {
                return udwf;
            }
        }
        not_impl_err!("the reader does not know the window function {name}")
    }

    fn try_encode_udwf(&self, node: &WindowUDF, buf: &mut Vec<u8>) -> Result<()> {
        match (self.hosts.0.iter()).find_map(|host| host.encode_udwf(node)) {
            Some(bytes) => encode_host(bytes, buf),
            None => Ok(()),
        }
    }
}
