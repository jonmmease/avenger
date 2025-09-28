//! Extension trait for LogicalPlanNode to provide SessionContext-dependent conversions

use crate::error::AvengerChartError;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion_proto::logical_plan::AsLogicalPlan;
use datafusion_proto::protobuf::LogicalPlanNode;

/// Extension trait for LogicalPlanNode providing conversions with SessionContext
pub trait LogicalPlanNodeExt: Sized {
    /// Create from a LogicalPlan
    fn from_logical_plan(plan: &LogicalPlan) -> Result<Self, AvengerChartError>;

    /// Convert to a LogicalPlan using the provided SessionContext
    fn to_logical_plan(&self, ctx: &SessionContext) -> Result<LogicalPlan, AvengerChartError>;
}

impl LogicalPlanNodeExt for LogicalPlanNode {
    fn from_logical_plan(plan: &LogicalPlan) -> Result<Self, AvengerChartError> {
        // Use our custom codec for serialization
        let codec = crate::scales::AvengerChartExtensionCodec::new();

        // Convert LogicalPlan to LogicalPlanNode using AsLogicalPlan trait
        <Self as AsLogicalPlan>::try_from_logical_plan(plan, &codec).map_err(|e| {
            AvengerChartError::InternalError(format!("Failed to serialize logical plan: {}", e))
        })
    }

    fn to_logical_plan(&self, ctx: &SessionContext) -> Result<LogicalPlan, AvengerChartError> {
        // Use our custom codec for deserialization
        let codec = crate::scales::AvengerChartExtensionCodec::new();

        // Convert LogicalPlanNode back to LogicalPlan using AsLogicalPlan trait
        <Self as AsLogicalPlan>::try_into_logical_plan(self, ctx, &codec)
            .map_err(|e| AvengerChartError::InternalError(format!("Failed to parse logical plan: {}", e)))
    }
}