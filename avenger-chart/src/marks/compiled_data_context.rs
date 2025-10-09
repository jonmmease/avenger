use crate::marks::ChannelValue;
use crate::serialization::{LogicalPlanNodeExt, SerializableDataFrame};
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

/// Compiled version of DataContext - stores serialized LogicalPlanNode
/// This is created during plot compilation and is immutable thereafter
#[serde_as]
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct CompiledDataContext {
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    logical_plan: Option<LogicalPlanNode>,
    channels: IndexMap<String, ChannelValue>,
}

impl CompiledDataContext {
    /// Create a new CompiledDataContext from a DataFrame
    pub fn new(dataframe: DataFrame, channels: IndexMap<String, ChannelValue>) -> Self {
        let plan = dataframe.logical_plan().clone();
        Self {
            logical_plan: LogicalPlanNode::from_logical_plan(&plan).ok(),
            channels,
        }
    }

    /// Create from an existing LogicalPlanNode (for backwards compatibility)
    pub fn from_logical_plan_node(
        logical_plan: Option<LogicalPlanNode>,
        channels: IndexMap<String, ChannelValue>,
    ) -> Self {
        Self {
            logical_plan,
            channels,
        }
    }

    /// Get the serialized LogicalPlanNode directly without deserialization
    pub fn logical_plan_node(&self) -> Option<&LogicalPlanNode> {
        self.logical_plan.as_ref()
    }

    /// Get the DataFrame using the provided SessionContext
    pub fn dataframe_with_context(&self, ctx: &SessionContext) -> Option<DataFrame> {
        self.logical_plan.as_ref().and_then(|node| {
            node.to_logical_plan(ctx)
                .ok()
                .map(|plan| DataFrame::new(ctx.state().clone(), plan))
        })
    }

    /// Get a reference to the channels
    pub fn channels(&self) -> &IndexMap<String, ChannelValue> {
        &self.channels
    }

    /// Get a specific channel value
    pub fn channel(&self, channel: &str) -> Option<&ChannelValue> {
        self.channels.get(channel)
    }

    // Compatibility methods for tests
    pub fn encoding(&self, channel: &str) -> Option<String> {
        let session_context = datafusion::prelude::SessionContext::new();
        self.channels
            .get(channel)
            .and_then(|v| v.as_column_name(&session_context))
    }

    pub fn encoding_expr_string(&self, channel: &str) -> Option<String> {
        let session_context = datafusion::prelude::SessionContext::new();
        self.channels
            .get(channel)
            .map(|v| format!("{:?}", v.expr(&session_context)))
    }
}
