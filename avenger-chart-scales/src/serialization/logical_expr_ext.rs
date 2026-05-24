//! Extension trait for LogicalExprNode to provide SessionContext-dependent conversions

use datafusion::{logical_expr::Expr, prelude::SessionContext};
use datafusion_proto::{
    logical_plan::{from_proto::parse_expr, to_proto::serialize_expr},
    protobuf::LogicalExprNode,
};

use avenger_chart_core::AvengerChartError;

use crate::AvengerChartExtensionCodec;

/// Extension trait for LogicalExprNode providing conversions with SessionContext
pub trait LogicalExprNodeExt: Sized {
    /// Create from an Expr
    fn from_expr(expr: Expr) -> Result<Self, AvengerChartError>;

    /// Convert to an Expr using the provided SessionContext
    fn to_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError>;
}

impl LogicalExprNodeExt for LogicalExprNode {
    fn from_expr(expr: Expr) -> Result<Self, AvengerChartError> {
        // Use our custom codec for serialization
        let codec = AvengerChartExtensionCodec::new();

        // Convert Expr to LogicalExprNode
        serialize_expr(&expr, &codec).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to serialize expr: {}", e))
        })
    }

    fn to_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
        // Use our custom codec for deserialization
        let codec = AvengerChartExtensionCodec::new();

        // Convert LogicalExprNode back to Expr
        parse_expr(self, ctx, &codec)
            .map_err(|e| AvengerChartError::InternalError(format!("Failed to parse expr: {}", e)))
    }
}
