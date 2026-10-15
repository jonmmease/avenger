use std::any::Any;
use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use datafusion::catalog::TableProvider;
use datafusion::common::{plan_err, Result, TableReference};
use datafusion::datasource::file_format::FileFormatFactory;
use datafusion::execution::TaskContext;
use datafusion::logical_expr::{
    AggregateUDF, AggregateUDFImpl, Extension, HigherOrderUDF, LogicalPlan, ScalarUDF, WindowUDF,
};
use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, LogicalExtensionCodec};

use super::fit::{self, FitUdf};
use super::scale::{self, ScaleUdf};
use super::summary::{MergeSummariesUdaf, SummaryUdaf, MERGE_SUMMARIES};
use super::{external, SerializedScale, SCALE_PAYLOAD_VERSION};

/// The bytes that begin a serialized scale.
const MAGIC: &[u8] = b"AVENGER_SCALE\0";

/// A plan codec that serializes the functions of [`scale_expr`](super::scale_expr) and
/// [`summary_expr`](super::summary_expr) with their scales, and delegates other extensions to a
/// fallback codec. Decoding needs no registration: the scale decodes through typetag.
#[derive(Debug)]
pub struct ScaleExtensionCodec {
    fallback: Arc<dyn LogicalExtensionCodec>,
}

impl Default for ScaleExtensionCodec {
    fn default() -> Self {
        Self::with_fallback(Arc::new(DefaultLogicalExtensionCodec {}))
    }
}

impl ScaleExtensionCodec {
    pub fn with_fallback(fallback: Arc<dyn LogicalExtensionCodec>) -> Self {
        Self { fallback }
    }
}

/// Writes `payload` with the payload version.
fn write_payload(mut payload: serde_json::Value, buf: &mut Vec<u8>) -> Result<()> {
    payload["version"] = SCALE_PAYLOAD_VERSION.into();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&serde_json::to_vec(&payload).map_err(external)?);
    Ok(())
}

/// The payload of a function of `scale`: its serialized form.
fn encode(scale: &SerializedScale, buf: &mut Vec<u8>) -> Result<()> {
    let scale: serde_json::Value = serde_json::from_slice(scale.json()).map_err(external)?;
    write_payload(serde_json::json!({ "scale": scale }), buf)
}

/// A payload that this codec wrote, or `None` for another codec's payload.
fn read_payload(buf: &[u8]) -> Result<Option<serde_json::Value>> {
    let Some(payload) = buf.strip_prefix(MAGIC) else {
        return Ok(None);
    };
    let payload: serde_json::Value = serde_json::from_slice(payload).map_err(external)?;
    if payload["version"] != SCALE_PAYLOAD_VERSION {
        return plan_err!("unsupported scale payload version {}", payload["version"]);
    }
    Ok(Some(payload))
}

/// The scale of a function's payload.
fn decode(payload: &serde_json::Value) -> Result<SerializedScale> {
    let mut scale = payload["scale"].clone();
    scale.sort_all_objects();
    SerializedScale::from_json(&scale)
}

impl LogicalExtensionCodec for ScaleExtensionCodec {
    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        let inner = node.inner().as_ref() as &dyn Any;
        if let Some(udf) = inner.downcast_ref::<ScaleUdf>() {
            encode(udf.serialized_scale(), buf)
        } else if let Some(udf) = inner.downcast_ref::<FitUdf>() {
            encode(udf.serialized_scale(), buf)
        } else {
            self.fallback.try_encode_udf(node, buf)
        }
    }

    /// The function's name tells which function of the payload's scale it is.
    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        let Some(payload) = read_payload(buf)? else {
            return self.fallback.try_decode_udf(name, buf);
        };
        let scale = decode(&payload)?;
        let udf = if name == scale::function_name(scale.scale().as_ref()) {
            ScalarUDF::new_from_impl(ScaleUdf::from_serialized(scale))
        } else if name == fit::function_name(scale.scale().as_ref()) {
            ScalarUDF::new_from_impl(FitUdf::from_serialized(scale))
        } else {
            return plan_err!(
                "a payload of a {} scale cannot decode {name}",
                scale.scale().scale_type()
            );
        };
        Ok(Arc::new(udf))
    }

    fn try_encode_udaf(&self, node: &AggregateUDF, buf: &mut Vec<u8>) -> Result<()> {
        let inner = node.inner().as_ref() as &dyn Any;
        if let Some(udaf) = inner.downcast_ref::<SummaryUdaf>() {
            encode(udaf.serialized_scale(), buf)
        } else if inner.is::<MergeSummariesUdaf>() {
            // The function holds nothing, but a payload decodes it without registration.
            write_payload(serde_json::json!({ "function": MERGE_SUMMARIES }), buf)
        } else {
            self.fallback.try_encode_udaf(node, buf)
        }
    }

    fn try_decode_udaf(&self, name: &str, buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        let Some(payload) = read_payload(buf)? else {
            return self.fallback.try_decode_udaf(name, buf);
        };
        if payload["function"] == MERGE_SUMMARIES && name == MERGE_SUMMARIES {
            return Ok(Arc::new(AggregateUDF::new_from_impl(
                MergeSummariesUdaf::new(),
            )));
        }
        let udaf = SummaryUdaf::from_serialized(decode(&payload)?);
        if udaf.name() != name {
            return plan_err!("a payload of {} cannot decode {name}", udaf.name());
        }
        Ok(Arc::new(AggregateUDF::new_from_impl(udaf)))
    }

    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[LogicalPlan],
        ctx: &TaskContext,
    ) -> Result<Extension> {
        self.fallback.try_decode(buf, inputs, ctx)
    }

    fn try_encode(&self, node: &Extension, buf: &mut Vec<u8>) -> Result<()> {
        self.fallback.try_encode(node, buf)
    }

    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        table_ref: &TableReference,
        schema: SchemaRef,
        ctx: &TaskContext,
    ) -> Result<Arc<dyn TableProvider>> {
        self.fallback
            .try_decode_table_provider(buf, table_ref, schema, ctx)
    }

    fn try_encode_table_provider(
        &self,
        table_ref: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> Result<()> {
        self.fallback
            .try_encode_table_provider(table_ref, node, buf)
    }

    fn try_decode_file_format(
        &self,
        buf: &[u8],
        ctx: &TaskContext,
    ) -> Result<Arc<dyn FileFormatFactory>> {
        self.fallback.try_decode_file_format(buf, ctx)
    }

    fn try_encode_file_format(
        &self,
        buf: &mut Vec<u8>,
        node: Arc<dyn FileFormatFactory>,
    ) -> Result<()> {
        self.fallback.try_encode_file_format(buf, node)
    }

    fn try_decode_higher_order_function(
        &self,
        name: &str,
        buf: &[u8],
    ) -> Result<Arc<HigherOrderUDF>> {
        self.fallback.try_decode_higher_order_function(name, buf)
    }

    fn try_encode_higher_order_function(
        &self,
        node: &HigherOrderUDF,
        buf: &mut Vec<u8>,
    ) -> Result<()> {
        self.fallback.try_encode_higher_order_function(node, buf)
    }

    fn try_decode_udwf(&self, name: &str, buf: &[u8]) -> Result<Arc<WindowUDF>> {
        self.fallback.try_decode_udwf(name, buf)
    }

    fn try_encode_udwf(&self, node: &WindowUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.fallback.try_encode_udwf(node, buf)
    }
}
