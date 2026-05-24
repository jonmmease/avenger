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
}

impl DefaultLogicalExprNodeExt for LogicalExprNode {
    fn from_default_expr(expr: Expr) -> Result<Self, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        serialize_expr(&expr, &codec)
            .map_err(|err| AvengerChartError::SerializationError(err.to_string()))
    }

    fn to_default_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        parse_expr(self, ctx, &codec)
            .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))
    }
}
