//! Serializable wrapper for DataFusion Expr
//!
//! This module provides SerializableExpr which stores Expr
//! as protobuf bytes for efficient binary serialization.

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use datafusion_proto::protobuf::LogicalExprNode;
use prost::Message;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A serializable wrapper for Expr that stores protobuf bytes
#[derive(Clone, Debug, PartialEq)]
pub struct SerializableExpr(pub Vec<u8>);

// Conversion implementations
impl From<LogicalExprNode> for SerializableExpr {
    fn from(node: LogicalExprNode) -> Self {
        // Encode protobuf to bytes
        let mut buf = Vec::new();
        node.encode(&mut buf)
            .expect("Failed to encode LogicalExprNode");
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

// Conversion from Expr to SerializableExpr via LogicalExprNode
impl From<datafusion::prelude::Expr> for SerializableExpr {
    fn from(expr: datafusion::prelude::Expr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        // Convert Expr to LogicalExprNode
        let node =
            LogicalExprNode::from_expr(expr).expect("Failed to convert Expr to LogicalExprNode");
        // Convert LogicalExprNode to SerializableExpr
        node.into()
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
            let bytes = BASE64
                .decode(&base64_str)
                .map_err(|e| serde::de::Error::custom(format!("Failed to decode base64: {}", e)))?;
            Ok(SerializableExpr(bytes))
        } else {
            // For binary formats, expect raw bytes
            let bytes = Vec::<u8>::deserialize(deserializer)?;
            Ok(SerializableExpr(bytes))
        }
    }
}
