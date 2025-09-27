//! LogicalExtensionCodec implementation for scale UDFs
//!
//! This module provides a custom codec that can serialize and deserialize
//! scale UDFs, allowing them to work on fresh SessionContexts without
//! requiring pre-registration.

use datafusion::arrow::datatypes::{DataType, Field, SchemaRef};
use datafusion::datasource::TableProvider;
use datafusion::error::{DataFusionError, Result as DataFusionResult};
use datafusion::logical_expr::{Extension, LogicalPlan, ScalarUDF};
use datafusion::prelude::SessionContext;
use datafusion_common::TableReference;
use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, LogicalExtensionCodec};
use std::sync::Arc;

/// Magic header for identifying serialized scale UDFs
const SCALE_UDF_MAGIC: &[u8] = b"SCALE_UDF_V2";

/// Extension codec for avenger-chart that handles scale UDF serialization
///
/// This codec implements a hybrid approach:
/// - Internal scale UDFs are fully serialized and can be deserialized on a fresh context
/// - User UDFs are referenced by name only and must be registered by the user
#[derive(Debug)]
pub struct AvengerChartExtensionCodec {
    /// Default codec for fallback behavior
    default_codec: DefaultLogicalExtensionCodec,
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
            default_codec: DefaultLogicalExtensionCodec {},
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
        // Delegate to default codec
        self.default_codec.try_decode(buf, inputs, ctx)
    }

    fn try_encode(&self, node: &Extension, buf: &mut Vec<u8>) -> DataFusionResult<()> {
        // Delegate to default codec
        self.default_codec.try_encode(node, buf)
    }

    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        table_ref: &TableReference,
        schema: SchemaRef,
        ctx: &SessionContext,
    ) -> DataFusionResult<Arc<dyn TableProvider>> {
        // Delegate to default codec
        self.default_codec
            .try_decode_table_provider(buf, table_ref, schema, ctx)
    }

    fn try_encode_table_provider(
        &self,
        table_ref: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> DataFusionResult<()> {
        // Delegate to default codec
        self.default_codec
            .try_encode_table_provider(table_ref, node, buf)
    }

    fn try_decode_file_format(
        &self,
        buf: &[u8],
        ctx: &SessionContext,
    ) -> DataFusionResult<Arc<dyn datafusion::datasource::file_format::FileFormatFactory>> {
        // Delegate to default codec
        self.default_codec.try_decode_file_format(buf, ctx)
    }

    fn try_encode_file_format(
        &self,
        buf: &mut Vec<u8>,
        node: Arc<dyn datafusion::datasource::file_format::FileFormatFactory>,
    ) -> DataFusionResult<()> {
        // Delegate to default codec
        self.default_codec.try_encode_file_format(buf, node)
    }

    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> DataFusionResult<()> {
        use crate::scales::udf::ScaleUDF;

        // Check if this is an internal scale UDF
        if node.name() == "scale" {
            // Try to downcast to ScaleUDF
            if let Some(scale_udf) = node.inner().as_any().downcast_ref::<ScaleUDF>() {
                // Extract metadata from ScaleUDF
                let metadata = scale_udf.metadata();

                // Serialize the metadata as JSON
                let json_bytes = serde_json::to_vec(&metadata)
                    .map_err(|e| DataFusionError::External(Box::new(e)))?;

                // Write magic header, length, and JSON data
                buf.extend_from_slice(SCALE_UDF_MAGIC);
                buf.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(&json_bytes);
            }
            // If it's not a ScaleUDF implementation, don't serialize
        }
        // User UDFs: don't serialize, rely on name lookup
        Ok(())
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> DataFusionResult<Arc<ScalarUDF>> {
        use crate::scales::udf::{ScaleUDF, ScaleUDFMetadata};

        // Check if this is a scale UDF with serialized data
        if name == "scale" && buf.len() > SCALE_UDF_MAGIC.len() + 4 {
            if buf.starts_with(SCALE_UDF_MAGIC) {
                // Skip magic header
                let buf = &buf[SCALE_UDF_MAGIC.len()..];

                // Read JSON length
                if buf.len() < 4 {
                    return datafusion_common::plan_err!(
                        "Invalid scale UDF serialization: missing length"
                    );
                }
                let json_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

                // Read JSON data
                if buf.len() < 4 + json_len {
                    return datafusion_common::plan_err!(
                        "Invalid scale UDF serialization: truncated data"
                    );
                }
                let json_bytes = &buf[4..4 + json_len];

                // Deserialize metadata
                let metadata: ScaleUDFMetadata = serde_json::from_slice(json_bytes)
                    .map_err(|e| DataFusionError::External(Box::new(e)))?;

                // Recreate the ScaleUDF from metadata
                let scale_udf = ScaleUDF::from_metadata(metadata)
                    .map_err(|e| DataFusionError::External(Box::new(e)))?;

                return Ok(Arc::new(ScalarUDF::new_from_impl(scale_udf)));
            }
        }

        // For backward compatibility: scale UDF without serialized data
        if name == "scale" && buf.is_empty() {
            // Create a default linear scale for backward compatibility
            use crate::scales::{Linear, Scale};

            let scale = Scale::<Linear>::new().into_auto();
            let scale_udf = ScaleUDF::new(
                scale,
                DataType::Float64,
                DataType::Float64,
                DataType::Struct(datafusion::arrow::datatypes::Fields::from(vec![
                    Field::new("", DataType::Null, true),
                ])),
            )
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
    use datafusion::arrow::datatypes::DataType;

    #[test]
    fn test_codec_creation() {
        let codec = AvengerChartExtensionCodec::new();
        // Just verify we can create the codec
        let _ = format!("{:?}", codec);
    }

    #[test]
    fn test_magic_header() {
        assert_eq!(SCALE_UDF_MAGIC.len(), 12);
        assert_eq!(SCALE_UDF_MAGIC, b"SCALE_UDF_V2");
    }
}
