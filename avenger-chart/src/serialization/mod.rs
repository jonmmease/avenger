//! Serialization helpers for DataFusion types

use datafusion::prelude::Expr;
use datafusion_proto::protobuf::LogicalExprNode;

mod dataframe;

pub use avenger_chart_core::serialization::{
    SerializableDataType, SerializableExpr, SerializableNestedScalarMap, SerializableScalar,
    SerializableScalarMap,
};
pub use avenger_chart_scales::serialization::{LogicalExprNodeExt, LogicalPlanNodeExt};
pub use dataframe::SerializableDataFrame;

pub(crate) fn serializable_expr_from_expr(expr: Expr, label: &str) -> SerializableExpr {
    let node = <LogicalExprNode as LogicalExprNodeExt>::from_expr(expr)
        .unwrap_or_else(|err| panic!("Failed to serialize {label}: {err}"));
    node.into()
}
