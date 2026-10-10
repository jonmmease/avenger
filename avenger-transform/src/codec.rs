use crate::udf::{Function, TransformUdf, TRANSFORM_FUNCTION_VERSION};
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::TableProvider,
    common::{plan_err, Result, TableReference},
    datasource::file_format::FileFormatFactory,
    execution::TaskContext,
    logical_expr::{AggregateUDF, Extension, HigherOrderUDF, LogicalPlan, ScalarUDF, WindowUDF},
};
use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, LogicalExtensionCodec};
use std::sync::Arc;

const MAGIC: &[u8] = b"AVENGER_TRANSFORM\0";

/// Encode transform UDF identities and delegate unrelated functions and extensions.
#[derive(Debug)]
pub struct TransformExtensionCodec {
    fallback: Arc<dyn LogicalExtensionCodec>,
}
impl Default for TransformExtensionCodec {
    fn default() -> Self {
        Self::with_fallback(Arc::new(DefaultLogicalExtensionCodec {}))
    }
}
impl TransformExtensionCodec {
    /// Wrap another codec for graphs that also contain other libraries' functions.
    pub fn with_fallback(fallback: Arc<dyn LogicalExtensionCodec>) -> Self {
        Self { fallback }
    }
}
impl LogicalExtensionCodec for TransformExtensionCodec {
    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        if node.inner().is::<TransformUdf>() {
            buf.extend_from_slice(MAGIC);
            buf.extend_from_slice(TRANSFORM_FUNCTION_VERSION.as_bytes());
            buf.push(0);
            buf.extend_from_slice(node.name().as_bytes());
            Ok(())
        } else {
            self.fallback.try_encode_udf(node, buf)
        }
    }
    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        let Some(payload) = buf.strip_prefix(MAGIC) else {
            return self.fallback.try_decode_udf(name, buf);
        };
        let expected = format!("{TRANSFORM_FUNCTION_VERSION}\0{name}");
        if payload != expected.as_bytes() {
            return plan_err!(
                "Unsupported transform function version or mismatched name for {name}"
            );
        }
        match Function::ALL.into_iter().find(|f| f.name() == name) {
            Some(f) => Ok(Arc::new(f.udf())),
            None => plan_err!("Unknown transform function {name}"),
        }
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
