//! The codec of the physical plans of a graph: datafusion-proto's messages, with the leaves of cuts,
//! the bound calls of marks and compositions, the operators of selections, and the functions of
//! the host that planned the graph as extensions, so a host that runs a graph builds the same
//! charts and binds the same selections as the host that planned it.

use std::{fmt::Debug, sync::Arc};

use arrow::datatypes::Schema;
use avenger_selection::SelectedOptions;
use avenger_selection_datafusion::SelectionExec;
use datafusion_common::{exec_datafusion_err, not_impl_err, project_schema, Result};
use datafusion_execution::TaskContext;
use datafusion_expr::{AggregateUDF, ScalarUDF};
use datafusion_physical_plan::{empty::EmptyExec, ExecutionPlan};
use datafusion_proto::{
    physical_plan::{
        PhysicalExtensionCodec, PhysicalPlanDecodeContext, PhysicalProtoConverterExtension,
    },
    protobuf::PhysicalExprNode,
};
use datafusion_proto_common as protobuf;
use prost::Message;

use crate::{
    cut::{CutInputExec, CutScan},
    functions::{CompositionInstance, MarkInstance},
};

/// The kinds of the codec's extensions, the first byte of each.
const CUT: u8 = 1;
const MARK: u8 = 2;
const COMPOSITION: u8 = 3;
const SELECTION: u8 = 4;
const HOST: u8 = 5;

/// The codec of the functions that a host adds to a graph's plans beyond DataFusion's, such as the
/// library of avenger-sql, which travel as their names and the bytes of their state, so a page
/// that knows the same host decodes the functions that the host planned.
pub trait HostCodec: Debug + Send + Sync {
    /// The host's name, such as `avenger-sql`, which a session's hosts hold once.
    fn name(&self) -> &str;

    /// The bytes of the state of `udf`, when it is one of the host's functions.
    fn encode_udf(&self, udf: &ScalarUDF) -> Option<Result<Vec<u8>>>;

    /// The function named `name` with the state `bytes`, when it is one of the host's functions.
    fn decode_udf(&self, name: &str, bytes: &[u8]) -> Option<Result<Arc<ScalarUDF>>>;
}

/// The hosts whose functions a graph's plans call, which a session holds as an extension of its
/// configuration, so the export of a page and the page's context find them.
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
    pub fn of(config: &datafusion_execution::config::SessionConfig) -> Hosts {
        (config.get_extension::<Hosts>())
            .map(|hosts| hosts.as_ref().clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, PartialEq, Message)]
struct CutBytes {
    #[prost(uint32, tag = "1")]
    input: u32,
    #[prost(message, optional, tag = "2")]
    schema: Option<protobuf::Schema>,
    #[prost(uint32, repeated, tag = "3")]
    projection: Vec<u32>,
    #[prost(bool, tag = "4")]
    projected: bool,
    #[prost(uint64, optional, tag = "5")]
    limit: Option<u64>,
}

/// An unbound operator of a selection: its placeholder, the options of its test, the names of
/// the columns that it can read, whether its test reads the struct of the row's partition
/// columns, its column, and the expressions of its columns and of the struct.
#[derive(Clone, PartialEq, Message)]
struct SelectionBytes {
    #[prost(string, tag = "1")]
    parameter: String,
    #[prost(string, repeated, tag = "2")]
    exclude_own: Vec<String>,
    #[prost(bool, tag = "3")]
    faceted_option: bool,
    #[prost(bool, tag = "4")]
    empty_selects_none: bool,
    #[prost(string, repeated, tag = "5")]
    without: Vec<String>,
    #[prost(string, repeated, tag = "6")]
    columns: Vec<String>,
    #[prost(bool, tag = "7")]
    faceted: bool,
    #[prost(string, tag = "8")]
    name: String,
    #[prost(message, repeated, tag = "9")]
    exprs: Vec<PhysicalExprNode>,
}

fn invalid(error: impl std::fmt::Display) -> datafusion_common::DataFusionError {
    exec_datafusion_err!("the bytes of a graph's extension: {error}")
}

/// The codec of a graph's physical plans, with the functions of `hosts`.
#[derive(Debug, Default)]
pub struct GraphCodec {
    hosts: Hosts,
}

impl GraphCodec {
    /// The codec of the plans of a session whose configuration holds `hosts`.
    pub fn new(hosts: Hosts) -> GraphCodec {
        GraphCodec { hosts }
    }

    /// The codec of the plans that run in `ctx`, with the hosts of its configuration.
    pub fn of(ctx: &TaskContext) -> GraphCodec {
        GraphCodec::new(Hosts::of(ctx.session_config()))
    }

    /// The unbound operator of a selection over `inputs`, from its bytes.
    fn decode_selection(
        &self,
        bytes: &[u8],
        inputs: &[Arc<dyn ExecutionPlan>],
        ctx: &TaskContext,
        converter: &dyn PhysicalProtoConverterExtension,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let decoded = SelectionBytes::decode(bytes).map_err(invalid)?;
        let [input] = inputs else {
            return Err(invalid("a selection's operator has one input"));
        };
        let schema = input.schema();
        let decode = PhysicalPlanDecodeContext::new(ctx, self);
        let exprs = (decoded.exprs.iter())
            .map(|expr| converter.proto_to_physical_expr(expr, &schema, &decode))
            .collect::<Result<Vec<_>>>()?;
        let options = SelectedOptions {
            exclude_own: decoded.exclude_own,
            faceted: decoded.faceted_option,
            empty_selects_none: decoded.empty_selects_none,
            without: decoded.without,
        };
        let selection = SelectionExec::try_new(
            input.clone(),
            decoded.parameter,
            options,
            decoded.columns,
            exprs,
            decoded.faceted,
            decoded.name,
        )?;
        Ok(Arc::new(selection))
    }
}

impl PhysicalExtensionCodec for GraphCodec {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[Arc<dyn ExecutionPlan>],
        ctx: &TaskContext,
        converter: &dyn PhysicalProtoConverterExtension,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if let Some((&SELECTION, bytes)) = buf.split_first() {
            return self.decode_selection(bytes, inputs, ctx, converter);
        }
        let Some((&CUT, bytes)) = buf.split_first() else {
            return not_impl_err!("a plan that a graph's codec does not know");
        };
        let decoded = CutBytes::decode(bytes).map_err(invalid)?;
        let schema = decoded
            .schema
            .as_ref()
            .ok_or_else(|| invalid("no schema"))?;
        let schema = Arc::new(Schema::try_from(schema).map_err(invalid)?);
        let projection: Option<Vec<usize>> = (decoded.projected).then(|| {
            decoded
                .projection
                .iter()
                .map(|column| *column as usize)
                .collect()
        });
        let cut = CutScan {
            input: decoded.input as usize,
            schema: schema.clone(),
            projection: projection.clone(),
            limit: decoded.limit.map(|limit| limit as usize),
        };
        // The scan is replaced by a scan of each computation's input.
        let projected = project_schema(&schema, projection.as_ref())?;
        let scan = Arc::new(EmptyExec::new(projected));
        Ok(Arc::new(CutInputExec::new(cut, scan)))
    }

    fn try_encode(
        &self,
        node: Arc<dyn ExecutionPlan>,
        buf: &mut Vec<u8>,
        converter: &dyn PhysicalProtoConverterExtension,
    ) -> Result<()> {
        if let Some(selection) = node.downcast_ref::<SelectionExec>() {
            let options = selection.options();
            let bytes = SelectionBytes {
                parameter: selection.parameter().to_string(),
                exclude_own: options.exclude_own.clone(),
                faceted_option: options.faceted,
                empty_selects_none: options.empty_selects_none,
                without: options.without.clone(),
                columns: selection.columns().to_vec(),
                faceted: selection.faceted(),
                name: selection.name().to_string(),
                exprs: (selection.exprs().iter())
                    .map(|expr| converter.physical_expr_to_proto(expr, self))
                    .collect::<Result<_>>()?,
            };
            buf.push(SELECTION);
            return bytes.encode(buf).map_err(invalid);
        }
        let Some(leaf) = node.downcast_ref::<CutInputExec>() else {
            return not_impl_err!("a graph's codec does not encode {}", node.name());
        };
        let cut = leaf.cut();
        let bytes = CutBytes {
            input: cut.input as u32,
            schema: Some(protobuf::Schema::try_from(cut.schema.as_ref()).map_err(invalid)?),
            projection: (cut.projection.iter().flatten())
                .map(|column| *column as u32)
                .collect(),
            projected: cut.projection.is_some(),
            limit: cut.limit.map(|limit| limit as u64),
        };
        buf.push(CUT);
        bytes.encode(buf).map_err(invalid)
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        if let Some((&HOST, bytes)) = buf.split_first() {
            return (self.hosts.0.iter())
                .find_map(|host| host.decode_udf(name, bytes))
                .unwrap_or_else(|| not_impl_err!("a page does not know the function {name}"));
        }
        not_impl_err!("a graph's codec does not know the function {name}")
    }

    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        if let Some(bytes) = (self.hosts.0.iter()).find_map(|host| host.encode_udf(node)) {
            buf.push(HOST);
            buf.extend(bytes?);
            return Ok(());
        }
        Ok(())
    }

    fn try_decode_udaf(&self, name: &str, buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        match buf.split_first() {
            Some((&MARK, bytes)) => Ok(Arc::new(AggregateUDF::new_from_impl(
                MarkInstance::from_bytes(bytes)?,
            ))),
            Some((&COMPOSITION, bytes)) => Ok(Arc::new(AggregateUDF::new_from_impl(
                CompositionInstance::from_bytes(bytes)?,
            ))),
            _ => not_impl_err!("a graph's codec does not know the aggregate {name}"),
        }
    }

    fn try_encode_udaf(&self, node: &AggregateUDF, buf: &mut Vec<u8>) -> Result<()> {
        let inner = node.inner().as_ref() as &dyn std::any::Any;
        if let Some(mark) = inner.downcast_ref::<MarkInstance>() {
            buf.push(MARK);
            buf.extend(mark.to_bytes()?);
        } else if let Some(composition) = inner.downcast_ref::<CompositionInstance>() {
            buf.push(COMPOSITION);
            buf.extend(composition.to_bytes()?);
        }
        Ok(())
    }
}
