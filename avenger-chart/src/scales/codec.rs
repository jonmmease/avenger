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
        use datafusion::datasource::MemTable;

        // Check if this is a serialized MemTable
        const MEMTABLE_MAGIC: &[u8] = b"MEMTABLE_V1";
        if buf.starts_with(MEMTABLE_MAGIC) {
            // Skip magic header
            let buf = &buf[MEMTABLE_MAGIC.len()..];

            // Read the length of the serialized batches
            if buf.len() < 8 {
                return datafusion_common::plan_err!(
                    "Invalid MemTable serialization: missing length"
                );
            }
            let batch_len = u64::from_le_bytes(
                buf[0..8].try_into()
                    .map_err(|_| datafusion_common::plan_datafusion_err!("Invalid length bytes"))?
            ) as usize;

            // Read the batch data
            let batch_data = &buf[8..8 + batch_len];

            // Deserialize the batches using Arrow IPC format
            use datafusion::arrow::ipc::reader::StreamReader;
            use std::io::Cursor;

            let cursor = Cursor::new(batch_data);
            let reader = StreamReader::try_new(cursor, None)
                .map_err(|e| DataFusionError::External(Box::new(e)))?;

            // Collect all batches
            let mut batches = Vec::new();
            for batch_result in reader {
                let batch = batch_result
                    .map_err(|e| DataFusionError::External(Box::new(e)))?;
                batches.push(batch);
            }

            // Create a new MemTable with the deserialized batches
            // MemTable expects partitions, so we wrap our batches in a single partition
            let mem_table = MemTable::try_new(schema, vec![batches])?;

            Ok(Arc::new(mem_table))
        } else {
            // Delegate to default codec for other table types
            self.default_codec
                .try_decode_table_provider(buf, table_ref, schema, ctx)
        }
    }

    fn try_encode_table_provider(
        &self,
        table_ref: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> DataFusionResult<()> {
        use datafusion::datasource::MemTable;

        // Check if this is a MemTable
        if let Some(mem_table) = node.as_any().downcast_ref::<MemTable>() {
            // Serialize MemTable
            // Write a magic header to identify this as a serialized MemTable
            buf.extend_from_slice(b"MEMTABLE_V1");

            // Get the record batches from the MemTable
            // MemTable stores data as partitions of RecordBatches
            use datafusion::execution::context::SessionState;
            let state = SessionState::new_with_config_rt(
                datafusion::execution::context::SessionConfig::default(),
                Arc::new(datafusion::execution::runtime_env::RuntimeEnv::default()),
            );
            let batches = futures::executor::block_on(async {
                mem_table.scan(&state, None, &[], None).await
            })?;

            // Collect all batches from the execution plan
            use datafusion::physical_plan::ExecutionPlan;
            use datafusion::arrow::record_batch::RecordBatch;
            use datafusion::arrow::ipc::writer::StreamWriter;

            let task_ctx = Arc::new(datafusion::execution::context::TaskContext::default());
            let stream = batches.execute(0, task_ctx)?;

            // Collect batches
            let collected_batches: Vec<RecordBatch> = futures::executor::block_on(async {
                use futures::TryStreamExt;

                stream.try_collect::<Vec<_>>().await
            }).map_err(|e| DataFusionError::External(Box::new(e)))?;

            // Serialize the batches using Arrow IPC format
            let mut batch_buffer = Vec::new();
            if !collected_batches.is_empty() {
                let schema = collected_batches[0].schema();
                let mut writer = StreamWriter::try_new(&mut batch_buffer, &schema)
                    .map_err(|e| DataFusionError::External(Box::new(e)))?;

                for batch in &collected_batches {
                    writer.write(batch)
                        .map_err(|e| DataFusionError::External(Box::new(e)))?;
                }

                writer.finish()
                    .map_err(|e| DataFusionError::External(Box::new(e)))?;
            }

            // Write the length of the serialized batches
            buf.extend_from_slice(&(batch_buffer.len() as u64).to_le_bytes());
            // Write the batches
            buf.extend_from_slice(&batch_buffer);

            Ok(())
        } else {
            // Delegate to default codec for other table types
            self.default_codec
                .try_encode_table_provider(table_ref, node, buf)
        }
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
