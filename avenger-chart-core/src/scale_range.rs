use avenger_scenegraph::marks::pattern::PatternFill;
use datafusion::logical_expr::Expr;
use datafusion_common::ScalarValue;
use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, to_proto::serialize_expr};
use datafusion_proto::protobuf::LogicalExprNode;
use palette::Srgba;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{SerializableExpr, SerializableScalar};

#[serde_as]
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScaleRange {
    Numeric(
        #[serde_as(as = "FromInto<SerializableExpr>")] LogicalExprNode,
        #[serde_as(as = "Box<FromInto<SerializableExpr>>")] Box<LogicalExprNode>,
    ),
    Discrete(Vec<SerializableScalar>),
    Color(Vec<[f32; 4]>),
    Pattern(Vec<Option<PatternFill>>),
}

impl ScaleRange {
    pub fn new_interval<E: Into<Expr>, F: Into<Expr>>(start: E, end: F) -> Self {
        let codec = DefaultLogicalExtensionCodec {};
        let start_node =
            serialize_expr(&start.into(), &codec).expect("Failed to serialize start expr");
        let end_node = serialize_expr(&end.into(), &codec).expect("Failed to serialize end expr");
        Self::Numeric(start_node, Box::new(end_node))
    }

    pub fn new_color(colors: Vec<Srgba>) -> Self {
        Self::Color(
            colors
                .into_iter()
                .map(|c| [c.red, c.green, c.blue, c.alpha])
                .collect(),
        )
    }

    pub fn new_pattern(patterns: Vec<Option<PatternFill>>) -> Self {
        Self::Pattern(patterns)
    }

    pub fn new_discrete<T: Into<ScalarValue>>(values: Vec<T>) -> Self {
        Self::Discrete(
            values
                .into_iter()
                .map(|v| SerializableScalar::new(v.into()))
                .collect(),
        )
    }

    /// Create a discrete range with `num` values linearly spaced between `start` and `end`.
    pub fn new_linspace_discrete(start: f32, end: f32, num: usize) -> Self {
        let step = if num > 1 {
            (end - start) / (num - 1) as f32
        } else {
            0.0
        };
        let values: Vec<SerializableScalar> = (0..num)
            .map(|i| SerializableScalar::new(ScalarValue::Float32(Some(start + i as f32 * step))))
            .collect();
        Self::Discrete(values)
    }
}
