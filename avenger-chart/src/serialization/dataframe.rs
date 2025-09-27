//! Serializable wrapper for DataFusion DataFrames
//!
//! This module provides SerializableDataFrame which stores LogicalPlans
//! as protobuf bytes, avoiding the need to deserialize during serde operations.

use crate::error::AvengerChartError;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::Expr;
use datafusion::prelude::SessionContext;
use datafusion_proto::bytes::{
    logical_plan_from_bytes_with_extension_codec, logical_plan_to_bytes_with_extension_codec,
};
use serde::{Deserialize, Serialize};

/// A serializable wrapper for DataFrames that stores protobuf bytes
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableDataFrame {
    /// Base64-encoded protobuf representation of the LogicalPlan
    #[serde(rename = "plan_bytes")]
    plan_bytes_base64: String,
}

impl SerializableDataFrame {
    /// Create from a DataFrame, converting its LogicalPlan to protobuf bytes
    pub fn from_dataframe(df: DataFrame) -> Result<Self, AvengerChartError> {
        let plan = df.logical_plan().clone();

        // Use our custom codec for serialization
        let codec = crate::scales::AvengerChartExtensionCodec::new();

        // Convert LogicalPlan to protobuf bytes
        let bytes = logical_plan_to_bytes_with_extension_codec(&plan, &codec).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to serialize plan: {}", e))
        })?;

        // Encode as base64 for JSON compatibility
        let plan_bytes_base64 = BASE64.encode(&bytes);

        Ok(Self { plan_bytes_base64 })
    }

    /// Create from a DataFrame (legacy method for compatibility)
    /// Panics if serialization fails - use from_dataframe for proper error handling
    pub fn from_dataframe_unchecked(df: DataFrame) -> Self {
        Self::from_dataframe(df).expect("Failed to serialize DataFrame")
    }

    /// Convert to a DataFrame using the provided SessionContext
    pub fn to_dataframe(&self, ctx: &SessionContext) -> Result<DataFrame, AvengerChartError> {
        // Decode base64
        let bytes = BASE64.decode(&self.plan_bytes_base64).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to decode base64: {}", e))
        })?;

        // Use our custom codec for deserialization
        let codec = crate::scales::AvengerChartExtensionCodec::new();

        // Convert bytes back to LogicalPlan
        let plan =
            logical_plan_from_bytes_with_extension_codec(&bytes, ctx, &codec).map_err(|e| {
                AvengerChartError::InternalError(format!("Failed to deserialize plan: {}", e))
            })?;

        Ok(DataFrame::new(ctx.state().clone(), plan))
    }

    /// Get the raw protobuf bytes (for advanced use cases)
    pub fn to_protobuf_bytes(&self) -> Result<Vec<u8>, AvengerChartError> {
        BASE64.decode(&self.plan_bytes_base64).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to decode base64: {}", e))
        })
    }

    /// Create from raw protobuf bytes
    pub fn from_protobuf_bytes(bytes: &[u8]) -> Self {
        Self {
            plan_bytes_base64: BASE64.encode(bytes),
        }
    }

    /// Apply a select operation to the DataFrame
    pub fn select(
        &self,
        exprs: Vec<Expr>,
        ctx: &SessionContext,
    ) -> Result<DataFrame, AvengerChartError> {
        let df = self.to_dataframe(ctx)?;
        df.select(exprs)
            .map_err(|e| AvengerChartError::InternalError(format!("Failed to select: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Int32Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use std::sync::Arc;

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
