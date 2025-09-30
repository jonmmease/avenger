use crate::marks::ChannelValue;
use crate::serialization::{LogicalPlanNodeExt, SerializableDataFrame};
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

/// Stores a mark's data source and channel-to-expression mappings
/// (e.g., x -> col("price"), fill -> lit("blue")
#[serde_as]
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DataContext {
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    dataframe: Option<LogicalPlanNode>,
    channels: IndexMap<String, ChannelValue>,
}

impl DataContext {
    pub fn new(dataframe: DataFrame) -> Self {
        let plan = dataframe.logical_plan().clone();
        Self {
            dataframe: LogicalPlanNode::from_logical_plan(&plan).ok(),
            channels: IndexMap::new(),
        }
    }

    /// Get the DataFrame using the provided SessionContext
    pub fn dataframe_with_context(&self, ctx: &SessionContext) -> Option<DataFrame> {
        self.dataframe.as_ref().and_then(|node| {
            node.to_logical_plan(ctx)
                .ok()
                .map(|plan| DataFrame::new(ctx.state().clone(), plan))
        })
    }

    /// Legacy method - returns None since we no longer store DataFrames directly
    /// Use dataframe_with_context() instead
    pub fn dataframe(&self) -> Option<&DataFrame> {
        None
    }

    pub fn with_channel_value(mut self, channel: &str, value: ChannelValue) -> Self {
        self.channels.insert(channel.to_string(), value);
        self
    }

    pub fn channel(&self, channel: &str) -> Option<&ChannelValue> {
        self.channels.get(channel)
    }

    pub fn channels(&self) -> &IndexMap<String, ChannelValue> {
        &self.channels
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
