//! Serializable wrapper for DataFusion DataFrames
//!
//! This module provides SerializableDataFrame which stores LogicalPlanNode
//! as protobuf bytes for efficient binary serialization.

use super::LogicalPlanNodeExt;
use crate::error::AvengerChartError;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use prost::Message;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A serializable wrapper for DataFrames that stores protobuf bytes
#[derive(Clone, Debug, PartialEq)]
pub struct SerializableDataFrame(pub Vec<u8>);

impl SerializableDataFrame {
    /// Create from a DataFrame, converting its LogicalPlan to protobuf bytes
    pub fn from_dataframe(df: DataFrame) -> Result<Self, AvengerChartError> {
        let plan = df.logical_plan().clone();
        let node = LogicalPlanNode::from_logical_plan(&plan)?;
        Ok(Self::from(node))
    }

    /// Convert to a DataFrame using the provided SessionContext
    pub fn to_dataframe(&self, ctx: &SessionContext) -> Result<DataFrame, AvengerChartError> {
        let node: LogicalPlanNode = self.clone().into();
        let plan = node.to_logical_plan(ctx)?;
        Ok(DataFrame::new(ctx.state().clone(), plan))
    }
}

// Conversion implementations
impl From<LogicalPlanNode> for SerializableDataFrame {
    fn from(node: LogicalPlanNode) -> Self {
        // Encode protobuf to bytes
        let mut buf = Vec::new();
        node.encode(&mut buf)
            .expect("Failed to encode LogicalPlanNode");
        SerializableDataFrame(buf)
    }
}

// Required for serde_with FromInto
impl From<SerializableDataFrame> for LogicalPlanNode {
    fn from(wrapper: SerializableDataFrame) -> Self {
        LogicalPlanNode::decode(&wrapper.0[..])
            .expect("Failed to decode LogicalPlanNode from SerializableDataFrame")
    }
}

// Custom serialization for better JSON support
impl Serialize for SerializableDataFrame {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // For JSON and other text formats, use base64
            let base64_str = BASE64.encode(&self.0);
            base64_str.serialize(serializer)
        } else {
            // For binary formats, use raw bytes
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
            // For JSON and other text formats, expect base64
            let base64_str = String::deserialize(deserializer)?;
            let bytes = BASE64
                .decode(&base64_str)
                .map_err(|e| serde::de::Error::custom(format!("Failed to decode base64: {}", e)))?;
            Ok(SerializableDataFrame(bytes))
        } else {
            // For binary formats, expect raw bytes
            let bytes = Vec::<u8>::deserialize(deserializer)?;
            Ok(SerializableDataFrame(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_serializable_dataframe_roundtrip() {
        // Create a test DataFrame using SQL which creates a serializable plan
        let ctx = SessionContext::new();

        // Use VALUES to create a simple DataFrame that can be serialized
        let df = ctx
            .sql("SELECT * FROM (VALUES (1, 'a'), (2, 'b'), (3, 'c')) AS t(id, name)")
            .await
            .unwrap();

        // Create SerializableDataFrame
        let serializable = SerializableDataFrame::from_dataframe(df).unwrap();

        // Serialize to JSON
        let json = serde_json::to_string(&serializable).unwrap();

        // Deserialize back
        let deserialized: SerializableDataFrame = serde_json::from_str(&json).unwrap();

        // Convert back to DataFrame
        let new_ctx = SessionContext::new();
        let restored_df = deserialized.to_dataframe(&new_ctx);

        // Verify the plan is preserved
        assert!(restored_df.unwrap().logical_plan().schema().fields().len() == 2);
    }

    #[tokio::test]
    async fn test_option_serializable_dataframe() {
        // Test Option<SerializableDataFrame> with None
        let none_df: Option<SerializableDataFrame> = None;
        let json = serde_json::to_string(&none_df).unwrap();
        assert_eq!(json, "null");

        let deserialized: Option<SerializableDataFrame> = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_none());

        // Test with Some
        let ctx = SessionContext::new();
        let df = ctx.sql("SELECT 1 as id").await.unwrap();
        let some_df = Some(SerializableDataFrame::from_dataframe(df).unwrap());

        let json = serde_json::to_string(&some_df).unwrap();
        let deserialized: Option<SerializableDataFrame> = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_some());
    }
}
