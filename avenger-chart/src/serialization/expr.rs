//! Serializable wrapper for DataFusion Expr
//!
//! This module provides SerializableExpr which stores Expr
//! as protobuf bytes for efficient binary serialization.

use super::LogicalExprNodeExt;
use crate::error::AvengerChartError;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use datafusion::logical_expr::Expr;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;
use prost::Message;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;
use std::convert::TryFrom;

/// A serializable wrapper for Expr that stores protobuf bytes
#[derive(Clone, Debug, PartialEq)]
pub struct SerializableExpr(pub Vec<u8>);

impl SerializableExpr {
    /// Create from an Expr, converting it to protobuf bytes
    pub fn from_expr(expr: Expr) -> Result<Self, AvengerChartError> {
        // Create a dummy context for conversion
        let ctx = SessionContext::new();
        let node = LogicalExprNode::from_expr(expr, &ctx)?;
        Ok(Self::from(node))
    }

    /// Convert back to an Expr using the provided SessionContext
    pub fn to_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
        let node: LogicalExprNode = self.clone().into();
        node.to_expr(ctx)
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

// Conversion implementations
impl From<LogicalExprNode> for SerializableExpr {
    fn from(node: LogicalExprNode) -> Self {
        // Encode protobuf to bytes
        let mut buf = Vec::new();
        node.encode(&mut buf).expect("Failed to encode LogicalExprNode");
        SerializableExpr(buf)
    }
}

// Required for serde_with FromInto
impl From<SerializableExpr> for LogicalExprNode {
    fn from(wrapper: SerializableExpr) -> Self {
        LogicalExprNode::decode(&wrapper.0[..])
            .expect("Failed to decode LogicalExprNode from SerializableExpr")
    }
}

// Custom serialization for better JSON support
impl Serialize for SerializableExpr {
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

impl<'de> Deserialize<'de> for SerializableExpr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            // For JSON and other text formats, expect base64
            let base64_str = String::deserialize(deserializer)?;
            let bytes = BASE64.decode(&base64_str)
                .map_err(|e| serde::de::Error::custom(format!("Failed to decode base64: {}", e)))?;
            Ok(SerializableExpr(bytes))
        } else {
            // For binary formats, expect raw bytes
            let bytes = Vec::<u8>::deserialize(deserializer)?;
            Ok(SerializableExpr(bytes))
        }
    }
}
