//! DataFusion logical-expression protobuf conversions using the default codec.

use datafusion::{logical_expr::Expr, prelude::SessionContext};
use datafusion_proto::{
    logical_plan::{
        DefaultLogicalExtensionCodec, from_proto::parse_expr, to_proto::serialize_expr,
    },
    protobuf::LogicalExprNode,
};

use crate::AvengerChartError;

/// Extension trait for logical-expression nodes that use DataFusion's default
/// extension codec.
pub trait DefaultLogicalExprNodeExt: Sized {
    /// Create a protobuf expression node from a DataFusion expression.
    fn from_default_expr(expr: Expr) -> Result<Self, AvengerChartError>;

    /// Convert a protobuf expression node to a DataFusion expression.
    fn to_default_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError>;

    /// Create a protobuf expression node from a DataFusion expression.
    ///
    /// This alias keeps core-owned helpers that predate the crate split readable
    /// while still documenting that they use the default DataFusion codec.
    fn from_expr(expr: Expr) -> Result<Self, AvengerChartError> {
        Self::from_default_expr(expr)
    }

    /// Convert a protobuf expression node to a DataFusion expression.
    ///
    /// This alias uses the default DataFusion codec.
    fn to_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
        self.to_default_expr(ctx)
    }
}

impl DefaultLogicalExprNodeExt for LogicalExprNode {
    fn from_default_expr(expr: Expr) -> Result<Self, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        serialize_expr(&expr, &codec)
            .map_err(|err| AvengerChartError::SerializationError(err.to_string()))
    }

    fn to_default_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        let task_ctx = ctx.task_ctx();
        parse_expr(self, &task_ctx, &codec)
            .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))
    }
}
