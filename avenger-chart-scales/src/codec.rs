//! LogicalExtensionCodec implementation for scale UDFs
//!
//! This module provides a custom codec that can serialize and deserialize
//! scale UDFs, allowing them to work on fresh SessionContexts without
//! requiring pre-registration.

use std::sync::Arc;

use avenger_chart_core::serialization::AvengerCoreExtensionCodec;
use datafusion::{
    arrow::datatypes::SchemaRef,
    datasource::TableProvider,
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{Extension, LogicalPlan, ScalarUDF},
    prelude::SessionContext,
};
use datafusion_common::TableReference;
use datafusion_proto::logical_plan::LogicalExtensionCodec;

use crate::udf::ScaleUDF;

/// Magic header for identifying serialized scale UDFs
const SCALE_UDF_MAGIC: &[u8] = b"SCALE_UDF_V1";

/// Extension codec for avenger-chart that handles scale UDF serialization
///
/// This codec implements a hybrid approach:
/// - Internal scale UDFs are fully serialized and can be deserialized on a fresh context
/// - User UDFs are referenced by name only and must be registered by the user
#[derive(Debug)]
pub struct AvengerChartExtensionCodec {
    core_codec: AvengerCoreExtensionCodec,
}

impl Default for AvengerChartExtensionCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl AvengerChartExtensionCodec {
    /// Create a new instance of the codec
    pub fn new() -> Self {
        Self {
            core_codec: AvengerCoreExtensionCodec::new(),
        }
    }
}

impl LogicalExtensionCodec for AvengerChartExtensionCodec {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[LogicalPlan],
        ctx: &SessionContext,
    ) -> DataFusionResult<Extension> {
        self.core_codec.try_decode(buf, inputs, ctx)
    }

    fn try_encode(&self, node: &Extension, buf: &mut Vec<u8>) -> DataFusionResult<()> {
        self.core_codec.try_encode(node, buf)
    }

    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        table_ref: &TableReference,
        schema: SchemaRef,
        ctx: &SessionContext,
    ) -> DataFusionResult<Arc<dyn TableProvider>> {
        self.core_codec
            .try_decode_table_provider(buf, table_ref, schema, ctx)
    }

    fn try_encode_table_provider(
        &self,
        table_ref: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> DataFusionResult<()> {
        self.core_codec
            .try_encode_table_provider(table_ref, node, buf)
    }

    fn try_decode_file_format(
        &self,
        buf: &[u8],
        ctx: &SessionContext,
    ) -> DataFusionResult<Arc<dyn datafusion::datasource::file_format::FileFormatFactory>> {
        self.core_codec.try_decode_file_format(buf, ctx)
    }

    fn try_encode_file_format(
        &self,
        buf: &mut Vec<u8>,
        node: Arc<dyn datafusion::datasource::file_format::FileFormatFactory>,
    ) -> DataFusionResult<()> {
        self.core_codec.try_encode_file_format(buf, node)
    }

    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> DataFusionResult<()> {
        // Check if this is an internal scale UDF
        if node.name() == "scale" {
            // Try to downcast to ScaleUDF
            if let Some(scale_udf) = node.inner().as_any().downcast_ref::<ScaleUDF>() {
                // Serialize the ScaleUDF with serde_json. The payload is small,
                // and JSON supports the internally tagged enums and maps used by
                // chart scale specs without requiring known sequence lengths.
                let payload = serde_json::to_vec(scale_udf)
                    .map_err(|e| DataFusionError::External(Box::new(e)))?;

                // Write magic header, length, and payload data
                buf.extend_from_slice(SCALE_UDF_MAGIC);
                buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                buf.extend_from_slice(&payload);
            }
            // If it's not a ScaleUDF implementation, don't serialize
        }
        // User UDFs: don't serialize, rely on name lookup
        Ok(())
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> DataFusionResult<Arc<ScalarUDF>> {
        // Check if this is a scale UDF with serialized data
        if name == "scale"
            && buf.len() > SCALE_UDF_MAGIC.len() + 4
            && buf.starts_with(SCALE_UDF_MAGIC)
        {
            // Skip magic header
            let buf = &buf[SCALE_UDF_MAGIC.len()..];

            // Read payload length
            if buf.len() < 4 {
                return datafusion_common::plan_err!(
                    "Invalid scale UDF serialization: missing length"
                );
            }
            let payload_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

            // Read payload data
            if buf.len() < 4 + payload_len {
                return datafusion_common::plan_err!(
                    "Invalid scale UDF serialization: truncated data"
                );
            }
            let payload = &buf[4..4 + payload_len];

            // Deserialize ScaleUDF
            let scale_udf: ScaleUDF = serde_json::from_slice(payload)
                .map_err(|e| DataFusionError::External(Box::new(e)))?;

            return Ok(Arc::new(ScalarUDF::new_from_impl(scale_udf)));
        }

        // User UDF - fail with informative error
        datafusion_common::not_impl_err!(
            "User UDF '{}' not registered. Please register your UDFs with the SessionContext before deserialization.",
            name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Linear, Scale, create_scale_udf};
    use datafusion::arrow::datatypes::{DataType, Field};

    #[test]
    fn test_codec_creation() {
        let codec = AvengerChartExtensionCodec::new();
        // Just verify we can create the codec
        let _ = format!("{:?}", codec);
    }

    #[test]
    fn test_magic_headers() {
        assert_eq!(SCALE_UDF_MAGIC.len(), 12);
        assert_eq!(SCALE_UDF_MAGIC, b"SCALE_UDF_V1");
    }

    #[test]
    fn scale_udf_codec_encodes_and_decodes_internal_scale() {
        let codec = AvengerChartExtensionCodec::new();
        let udf = create_scale_udf(
            Scale::<Linear>::new().into_auto(),
            DataType::Float64,
            DataType::Float64,
            DataType::Float32,
            DataType::Struct(vec![Arc::new(Field::new("band", DataType::Float64, true))].into()),
        )
        .expect("scale udf");

        let mut buf = Vec::new();
        codec
            .try_encode_udf(&udf, &mut buf)
            .expect("encode scale udf");

        assert!(buf.starts_with(SCALE_UDF_MAGIC));

        let decoded = codec
            .try_decode_udf("scale", &buf)
            .expect("decode scale udf");
        assert_eq!(decoded.name(), "scale");
        assert!(decoded.inner().as_any().is::<ScaleUDF>());
    }
}
