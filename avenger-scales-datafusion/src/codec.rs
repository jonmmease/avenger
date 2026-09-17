use std::sync::Arc;

use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::TableProvider,
    common::{plan_err, DataFusionError, Result, TableReference},
    datasource::file_format::FileFormatFactory,
    execution::TaskContext,
    logical_expr::{AggregateUDF, Extension, HigherOrderUDF, LogicalPlan, ScalarUDF, WindowUDF},
};
use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, LogicalExtensionCodec};
use serde::{Deserialize, Serialize};

use crate::{ScaleSpec, ScaleUDF, SCALE_FUNCTION_NAME, SCALE_FUNCTION_VERSION};

const MAGIC: &[u8] = b"AVENGER_SCALE\0";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: String,
    spec: Box<dyn ScaleSpec>,
}

/// Serialize scale descriptors and delegate other extensions to another codec.
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

impl LogicalExtensionCodec for ScaleExtensionCodec {
    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        let any = node.inner().as_ref() as &dyn std::any::Any;
        if let Some(scale) = any.downcast_ref::<ScaleUDF>() {
            // The descriptor bytes are canonical and captured at construction.
            let spec: serde_json::Value = serde_json::from_slice(scale.payload())
                .map_err(|e| DataFusionError::External(Box::new(e)))?;
            let payload = serde_json::to_vec(&serde_json::json!({
                "version": SCALE_FUNCTION_VERSION,
                "spec": spec,
            }))
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
            buf.extend_from_slice(MAGIC);
            buf.extend_from_slice(&payload);
            Ok(())
        } else {
            self.fallback.try_encode_udf(node, buf)
        }
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        if !buf.starts_with(MAGIC) {
            return self.fallback.try_decode_udf(name, buf);
        }
        if name != SCALE_FUNCTION_NAME {
            return plan_err!("Scale payload supplied for function {name}");
        }
        let payload: Payload = serde_json::from_slice(&buf[MAGIC.len()..])
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
        if payload.version != SCALE_FUNCTION_VERSION {
            return plan_err!("Unsupported scale function version {}", payload.version);
        }
        Ok(Arc::new(ScalarUDF::new_from_impl(ScaleUDF::new(
            Arc::from(payload.spec),
        )?)))
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
    fn try_decode_udaf(&self, name: &str, buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        self.fallback.try_decode_udaf(name, buf)
    }
    fn try_encode_udaf(&self, node: &AggregateUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.fallback.try_encode_udaf(node, buf)
    }
    fn try_decode_udwf(&self, name: &str, buf: &[u8]) -> Result<Arc<WindowUDF>> {
        self.fallback.try_decode_udwf(name, buf)
    }
    fn try_encode_udwf(&self, node: &WindowUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.fallback.try_encode_udwf(node, buf)
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
}
