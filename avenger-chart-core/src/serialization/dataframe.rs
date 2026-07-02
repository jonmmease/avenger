//! Serializable wrapper for DataFusion DataFrames.
//!
//! The wrapper stores a `LogicalPlanNode` as protobuf bytes. It is intentionally
//! core-owned because shared scale-domain specs and compiled mark state both
//! need to serialize logical plans without depending on the top-level chart
//! facade.

use std::{io::Cursor, sync::Arc};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use datafusion::{
    arrow::{
        datatypes::SchemaRef,
        ipc::{reader::StreamReader, writer::StreamWriter},
        record_batch::RecordBatch,
    },
    dataframe::DataFrame,
    datasource::{MemTable, TableProvider},
    error::{DataFusionError, Result as DataFusionResult},
    execution::context::TaskContext,
    logical_expr::{Extension, LogicalPlan, ScalarUDF, UNNAMED_TABLE},
    prelude::SessionContext,
};
use datafusion_common::TableReference;
use datafusion_proto::{
    logical_plan::{AsLogicalPlan, DefaultLogicalExtensionCodec, LogicalExtensionCodec},
    protobuf::LogicalPlanNode,
};
use futures::TryStreamExt;
use prost::Message;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::AvengerChartError;

/// Magic header for identifying serialized in-memory tables.
const MEMTABLE_MAGIC: &[u8] = b"MEMTABLE_V1";
/// Magic header for identifying named in-session in-memory table references.
const MEMTABLE_REF_MAGIC: &[u8] = b"MEMTABLE_REF_V1";

/// Extension codec for core logical-plan serialization.
///
/// Core owns the generic ability to serialize in-memory DataFrames because
/// mark state and plot data both need this without depending on the scales
/// crate. Higher-level crates can wrap this codec to add their own extension
/// payloads, such as scale UDFs.
#[derive(Debug)]
pub struct AvengerCoreExtensionCodec {
    default_codec: DefaultLogicalExtensionCodec,
}

impl Default for AvengerCoreExtensionCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl AvengerCoreExtensionCodec {
    pub fn new() -> Self {
        Self {
            default_codec: DefaultLogicalExtensionCodec {},
        }
    }
}

impl LogicalExtensionCodec for AvengerCoreExtensionCodec {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[LogicalPlan],
        ctx: &SessionContext,
    ) -> DataFusionResult<Extension> {
        self.default_codec.try_decode(buf, inputs, ctx)
    }

    fn try_encode(&self, node: &Extension, buf: &mut Vec<u8>) -> DataFusionResult<()> {
        self.default_codec.try_encode(node, buf)
    }

    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        table_ref: &TableReference,
        schema: SchemaRef,
        ctx: &SessionContext,
    ) -> DataFusionResult<Arc<dyn TableProvider>> {
        if buf.starts_with(MEMTABLE_REF_MAGIC) {
            futures::executor::block_on(ctx.table_provider(table_ref.clone()))
        } else if buf.starts_with(MEMTABLE_MAGIC) {
            let buf = &buf[MEMTABLE_MAGIC.len()..];
            if buf.len() < 8 {
                return datafusion_common::plan_err!(
                    "Invalid MemTable serialization: missing length"
                );
            }
            let batch_len = u64::from_le_bytes(
                buf[0..8]
                    .try_into()
                    .map_err(|_| datafusion_common::plan_datafusion_err!("Invalid length bytes"))?,
            ) as usize;

            let batch_data = &buf[8..8 + batch_len];
            let cursor = Cursor::new(batch_data);
            let reader = StreamReader::try_new(cursor, None)
                .map_err(|err| DataFusionError::External(Box::new(err)))?;

            let mut batches = Vec::new();
            for batch_result in reader {
                let batch = batch_result.map_err(|err| DataFusionError::External(Box::new(err)))?;
                batches.push(batch);
            }

            let mem_table = MemTable::try_new(schema, vec![batches])?;
            Ok(Arc::new(mem_table))
        } else {
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
        if let Some(mem_table) = node.as_any().downcast_ref::<MemTable>() {
            if table_ref.table() != UNNAMED_TABLE {
                buf.extend_from_slice(MEMTABLE_REF_MAGIC);
                return Ok(());
            }

            buf.extend_from_slice(MEMTABLE_MAGIC);

            let state = SessionContext::new().state();
            let batches = futures::executor::block_on(async {
                mem_table.scan(&state, None, &[], None).await
            })?;

            let task_ctx = Arc::new(TaskContext::default());
            let stream = batches.execute(0, task_ctx)?;
            let collected_batches: Vec<RecordBatch> =
                futures::executor::block_on(async { stream.try_collect::<Vec<_>>().await })
                    .map_err(|err| DataFusionError::External(Box::new(err)))?;

            let mut batch_buffer = Vec::new();
            if !collected_batches.is_empty() {
                let schema = collected_batches[0].schema();
                let mut writer = StreamWriter::try_new(&mut batch_buffer, &schema)
                    .map_err(|err| DataFusionError::External(Box::new(err)))?;

                for batch in &collected_batches {
                    writer
                        .write(batch)
                        .map_err(|err| DataFusionError::External(Box::new(err)))?;
                }

                writer
                    .finish()
                    .map_err(|err| DataFusionError::External(Box::new(err)))?;
            }

            buf.extend_from_slice(&(batch_buffer.len() as u64).to_le_bytes());
            buf.extend_from_slice(&batch_buffer);

            Ok(())
        } else {
            self.default_codec
                .try_encode_table_provider(table_ref, node, buf)
        }
    }

    fn try_decode_file_format(
        &self,
        buf: &[u8],
        ctx: &SessionContext,
    ) -> DataFusionResult<Arc<dyn datafusion::datasource::file_format::FileFormatFactory>> {
        self.default_codec.try_decode_file_format(buf, ctx)
    }

    fn try_encode_file_format(
        &self,
        buf: &mut Vec<u8>,
        node: Arc<dyn datafusion::datasource::file_format::FileFormatFactory>,
    ) -> DataFusionResult<()> {
        self.default_codec.try_encode_file_format(buf, node)
    }

    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> DataFusionResult<()> {
        self.default_codec.try_encode_udf(node, buf)
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> DataFusionResult<Arc<ScalarUDF>> {
        self.default_codec.try_decode_udf(name, buf)
    }
}

/// Extension trait for converting between logical plans and protobuf nodes.
pub trait LogicalPlanNodeExt: Sized {
    /// Create from a logical plan.
    fn from_logical_plan(plan: &LogicalPlan) -> Result<Self, AvengerChartError>;

    /// Convert to a logical plan using the provided session context.
    fn to_logical_plan(&self, ctx: &SessionContext) -> Result<LogicalPlan, AvengerChartError>;
}

impl LogicalPlanNodeExt for LogicalPlanNode {
    fn from_logical_plan(plan: &LogicalPlan) -> Result<Self, AvengerChartError> {
        let codec = AvengerCoreExtensionCodec::new();
        <Self as AsLogicalPlan>::try_from_logical_plan(plan, &codec).map_err(|err| {
            AvengerChartError::InternalError(format!("Failed to serialize logical plan: {}", err))
        })
    }

    fn to_logical_plan(&self, ctx: &SessionContext) -> Result<LogicalPlan, AvengerChartError> {
        let codec = AvengerCoreExtensionCodec::new();
        <Self as AsLogicalPlan>::try_into_logical_plan(self, ctx, &codec).map_err(|err| {
            AvengerChartError::InternalError(format!("Failed to parse logical plan: {}", err))
        })
    }
}

/// A serializable wrapper for DataFrames that stores protobuf logical-plan bytes.
///
/// In-memory `MemTable` inputs have two serialization modes:
///
/// - unnamed tables are serialized inline as Arrow IPC payloads and can be
///   reconstructed in a fresh `SessionContext`;
/// - named tables are serialized as references to tables registered in the
///   provided `SessionContext`.
///
/// Named references keep large cached sources small and avoid copying record
/// batches into every compiled plan or async materialization request. They are
/// intentionally session-local: deserializing or materializing the logical plan
/// requires the same table name to be registered in the target
/// `SessionContext`.
#[derive(Clone, Debug, PartialEq)]
pub struct SerializableDataFrame(pub Vec<u8>);

impl SerializableDataFrame {
    /// Create from a DataFrame by converting its logical plan to protobuf bytes.
    pub fn from_dataframe(df: DataFrame) -> Result<Self, AvengerChartError> {
        let plan = df.logical_plan().clone();
        let node = LogicalPlanNode::from_logical_plan(&plan)?;
        Ok(Self::from(node))
    }

    /// Convert to a DataFrame using the provided SessionContext.
    pub fn to_dataframe(&self, ctx: &SessionContext) -> Result<DataFrame, AvengerChartError> {
        let node: LogicalPlanNode = self.clone().into();
        let plan = node.to_logical_plan(ctx)?;
        Ok(DataFrame::new(ctx.state().clone(), plan))
    }
}

impl From<LogicalPlanNode> for SerializableDataFrame {
    fn from(node: LogicalPlanNode) -> Self {
        let mut buf = Vec::new();
        node.encode(&mut buf)
            .expect("Failed to encode LogicalPlanNode");
        SerializableDataFrame(buf)
    }
}

impl From<SerializableDataFrame> for LogicalPlanNode {
    fn from(wrapper: SerializableDataFrame) -> Self {
        LogicalPlanNode::decode(&wrapper.0[..])
            .expect("Failed to decode LogicalPlanNode from SerializableDataFrame")
    }
}

impl Serialize for SerializableDataFrame {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            BASE64.encode(&self.0).serialize(serializer)
        } else {
            self.0.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for SerializableDataFrame {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let base64_str = String::deserialize(deserializer)?;
            let bytes = BASE64.decode(&base64_str).map_err(|err| {
                serde::de::Error::custom(format!("Failed to decode base64: {}", err))
            })?;
            Ok(SerializableDataFrame(bytes))
        } else {
            let bytes = Vec::<u8>::deserialize(deserializer)?;
            Ok(SerializableDataFrame(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::{
        arrow::{
            array::{Int32Array, RecordBatch},
            datatypes::{DataType, Field, Schema},
        },
        datasource::MemTable,
        prelude::SessionContext,
    };

    use super::*;

    fn int_batch(values: Vec<i32>) -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)])),
            vec![Arc::new(Int32Array::from(values))],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn serializable_dataframe_roundtrips_through_json() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1, 'a'), (2, 'b'), (3, 'c')) AS t(id, name)")
            .await
            .unwrap();

        let serializable = SerializableDataFrame::from_dataframe(df).unwrap();
        let json = serde_json::to_string(&serializable).unwrap();
        let deserialized: SerializableDataFrame = serde_json::from_str(&json).unwrap();

        let new_ctx = SessionContext::new();
        let restored_df = deserialized.to_dataframe(&new_ctx).unwrap();

        assert_eq!(restored_df.logical_plan().schema().fields().len(), 2);
    }

    #[tokio::test]
    async fn optional_serializable_dataframe_roundtrips_through_json() {
        let none_df: Option<SerializableDataFrame> = None;
        let json = serde_json::to_string(&none_df).unwrap();
        assert_eq!(json, "null");

        let deserialized: Option<SerializableDataFrame> = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_none());

        let ctx = SessionContext::new();
        let df = ctx.sql("SELECT 1 as id").await.unwrap();
        let some_df = Some(SerializableDataFrame::from_dataframe(df).unwrap());

        let json = serde_json::to_string(&some_df).unwrap();
        let deserialized: Option<SerializableDataFrame> = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_some());
    }

    #[tokio::test]
    async fn named_memtable_serializes_as_session_table_reference() {
        let ctx = SessionContext::new();
        ctx.register_batch("cached_values", int_batch(vec![1, 2, 3]))
            .unwrap();
        let df = ctx.table("cached_values").await.unwrap();

        let serializable = SerializableDataFrame::from_dataframe(df).unwrap();
        assert!(contains_bytes(&serializable.0, MEMTABLE_REF_MAGIC));
        assert!(!contains_bytes(&serializable.0, MEMTABLE_MAGIC));

        let restored = serializable.to_dataframe(&ctx).unwrap();
        let batches = restored.collect().await.unwrap();
        assert_eq!(batches[0].num_rows(), 3);

        let missing_ctx = SessionContext::new();
        let err = serializable.to_dataframe(&missing_ctx).unwrap_err();
        assert!(
            err.to_string().contains("cached_values"),
            "missing named table error should mention the table reference: {err}"
        );
    }

    #[tokio::test]
    async fn unnamed_memtable_still_serializes_inline() {
        let ctx = SessionContext::new();
        let batch = int_batch(vec![4, 5, 6]);
        let table = Arc::new(MemTable::try_new(batch.schema(), vec![vec![batch]]).unwrap());
        let df = ctx.read_table(table).unwrap();

        let serializable = SerializableDataFrame::from_dataframe(df).unwrap();
        assert!(contains_bytes(&serializable.0, MEMTABLE_MAGIC));
        assert!(!contains_bytes(&serializable.0, MEMTABLE_REF_MAGIC));

        let restored = serializable.to_dataframe(&SessionContext::new()).unwrap();
        let batches = restored.collect().await.unwrap();
        assert_eq!(batches[0].num_rows(), 3);
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }
}
