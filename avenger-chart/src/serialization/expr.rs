//! Serializable wrapper for DataFusion Expr
//!
//! This module provides SerializableExpr which stores Expr
//! as protobuf bytes, avoiding the need to deserialize during serde operations.

use crate::error::AvengerChartError;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use datafusion::logical_expr::Expr;
use datafusion::prelude::SessionContext;
use datafusion_proto::logical_plan::{
    from_proto::parse_expr, to_proto::serialize_expr,
};
use datafusion_proto::protobuf::LogicalExprNode;
use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A serializable wrapper for Expr that stores protobuf representation
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SerializableExpr {
    /// Base64-encoded protobuf representation of the Expr
    #[serde(rename = "expr_bytes")]
    expr_bytes_base64: String,
}

impl SerializableExpr {
    /// Create from an Expr, converting it to protobuf bytes
    pub fn from_expr(expr: Expr) -> Result<Self, AvengerChartError> {
        // Use our custom codec for serialization
        let codec = crate::scales::AvengerChartExtensionCodec::new();

        // Convert Expr to protobuf
        let expr_node = serialize_expr(&expr, &codec).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to serialize expr: {}", e))
        })?;

        // Encode protobuf to bytes
        let mut buf = Vec::new();
        expr_node.encode(&mut buf).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to encode expr protobuf: {}", e))
        })?;

        // Encode to base64
        let expr_bytes_base64 = BASE64.encode(&buf);

        Ok(Self { expr_bytes_base64 })
    }

    /// Convert back to an Expr using the provided SessionContext
    pub fn to_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
        // Decode from base64
        let bytes = BASE64.decode(&self.expr_bytes_base64).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to decode base64: {}", e))
        })?;

        // Decode protobuf
        let expr_node = LogicalExprNode::decode(&bytes[..]).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to decode expr protobuf: {}", e))
        })?;

        // Use our custom codec for deserialization
        let codec = crate::scales::AvengerChartExtensionCodec::new();

        // Convert protobuf back to Expr
        parse_expr(&expr_node, ctx, &codec)
            .map_err(|e| AvengerChartError::InternalError(format!("Failed to parse expr: {}", e)))
    }

    /// Add an alias to the expression
    pub fn alias(&self, name: &str, ctx: &SessionContext) -> Result<Self, AvengerChartError> {
        // We need to deserialize, add alias, and re-serialize
        // This requires a context to parse UDFs, but we want to preserve the serialized form
        let expr = self.to_expr(ctx)?;
        Self::from_expr(expr.alias(name))
    }

    /// Get column references from the expression
    pub fn column_refs(&self, ctx: &SessionContext) -> Result<HashSet<String>, AvengerChartError> {
        let expr = self.to_expr(ctx)?;
        Ok(expr
            .column_refs()
            .into_iter()
            .map(|c| c.name.clone())
            .collect())
    }
}
