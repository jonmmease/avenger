//! Serializable wrapper for DataFusion DataFrames.
//!
//! The wrapper stores a `LogicalPlanNode` as protobuf bytes. It is intentionally
//! core-owned because shared scale-domain specs and compiled mark state both
//! need to serialize logical plans without depending on the top-level chart
//! facade.

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use datafusion::{dataframe::DataFrame, logical_expr::LogicalPlan, prelude::SessionContext};
use datafusion_proto::{
    logical_plan::{AsLogicalPlan, DefaultLogicalExtensionCodec},
    protobuf::LogicalPlanNode,
};
use prost::Message;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::AvengerChartError;

/// Extension trait for converting between logical plans and protobuf nodes.
pub trait LogicalPlanNodeExt: Sized {
    /// Create from a logical plan.
    fn from_logical_plan(plan: &LogicalPlan) -> Result<Self, AvengerChartError>;

    /// Convert to a logical plan using the provided session context.
    fn to_logical_plan(&self, ctx: &SessionContext) -> Result<LogicalPlan, AvengerChartError>;
}

impl LogicalPlanNodeExt for LogicalPlanNode {
    fn from_logical_plan(plan: &LogicalPlan) -> Result<Self, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        <Self as AsLogicalPlan>::try_from_logical_plan(plan, &codec).map_err(|err| {
            AvengerChartError::InternalError(format!("Failed to serialize logical plan: {}", err))
        })
    }

    fn to_logical_plan(&self, ctx: &SessionContext) -> Result<LogicalPlan, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        <Self as AsLogicalPlan>::try_into_logical_plan(self, ctx, &codec).map_err(|err| {
            AvengerChartError::InternalError(format!("Failed to parse logical plan: {}", err))
        })
    }
}

/// A serializable wrapper for DataFrames that stores protobuf logical-plan bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct SerializableDataFrame(pub Vec<u8>);

impl SerializableDataFrame {
    /// Create from a DataFrame by converting its logical plan to protobuf bytes.
    pub fn from_dataframe(df: DataFrame) -> Result<Self, AvengerChartError> {
        let plan = df.logical_plan().clone();
        let node = LogicalPlanNode::from_logical_plan(&plan)?;
        Ok(Self::from(node))
    }

    /// Convert to a DataFrame using the provided SessionContext.
    pub fn to_dataframe(&self, ctx: &SessionContext) -> Result<DataFrame, AvengerChartError> {
        let node: LogicalPlanNode = self.clone().into();
        let plan = node.to_logical_plan(ctx)?;
        Ok(DataFrame::new(ctx.state().clone(), plan))
    }
}

impl From<LogicalPlanNode> for SerializableDataFrame {
    fn from(node: LogicalPlanNode) -> Self {
        let mut buf = Vec::new();
        node.encode(&mut buf)
            .expect("Failed to encode LogicalPlanNode");
        SerializableDataFrame(buf)
    }
}

impl From<SerializableDataFrame> for LogicalPlanNode {
    fn from(wrapper: SerializableDataFrame) -> Self {
        LogicalPlanNode::decode(&wrapper.0[..])
            .expect("Failed to decode LogicalPlanNode from SerializableDataFrame")
    }
}

impl Serialize for SerializableDataFrame {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            BASE64.encode(&self.0).serialize(serializer)
        } else {
            self.0.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for SerializableDataFrame {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let base64_str = String::deserialize(deserializer)?;
            let bytes = BASE64.decode(&base64_str).map_err(|err| {
                serde::de::Error::custom(format!("Failed to decode base64: {}", err))
            })?;
            Ok(SerializableDataFrame(bytes))
        } else {
            let bytes = Vec::<u8>::deserialize(deserializer)?;
            Ok(SerializableDataFrame(bytes))
        }
    }
}
