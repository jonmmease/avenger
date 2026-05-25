use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::SerializableExpr;

/// Expression for computing radius/padding requirements for marks
///
/// Used to determine how much space a mark needs beyond its base position,
/// accounting for visual properties like size, stroke width, etc.
#[serde_as]
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RadiusExpression {
    /// Same radius in all directions (e.g., circular symbols)
    Symmetric(#[serde_as(as = "FromInto<SerializableExpr>")] LogicalExprNode),
    /// Different radius for negative and positive directions (e.g., bars extending from baseline)
    Asymmetric {
        /// Radius in the negative direction
        #[serde_as(as = "FromInto<SerializableExpr>")]
        lower: LogicalExprNode,
        /// Radius in the positive direction
        #[serde_as(as = "FromInto<SerializableExpr>")]
        upper: LogicalExprNode,
    },
}
