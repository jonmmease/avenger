use indexmap::IndexMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_with::{FromInto, serde_as};

use datafusion::{dataframe::DataFrame, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalPlanNode;

use crate::{
    ChannelValue, DataTransformStage, LogicalPlanNodeExt, PatternChannelValue,
    SerializableDataFrame, StoreData,
};

/// Compiled version of DataContext - stores serialized LogicalPlanNode
/// This is created during plot compilation and is immutable thereafter
#[serde_as]
#[derive(Clone, Default, Deserialize)]
pub struct CompiledDataContext {
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    logical_plan: Option<LogicalPlanNode>,
    store_data: Option<StoreData>,
    #[serde(default)]
    transforms: Vec<DataTransformStage>,
    channels: IndexMap<String, ChannelValue>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pattern_channels: IndexMap<String, PatternChannelValue>,
}

impl Serialize for CompiledDataContext {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() && self.pattern_channels.is_empty() {
            #[serde_as]
            #[derive(Serialize)]
            struct HumanReadableCompiledDataContext {
                #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
                logical_plan: Option<LogicalPlanNode>,
                store_data: Option<StoreData>,
                #[serde(default)]
                transforms: Vec<DataTransformStage>,
                channels: IndexMap<String, ChannelValue>,
            }

            HumanReadableCompiledDataContext {
                logical_plan: self.logical_plan.clone(),
                store_data: self.store_data.clone(),
                transforms: self.transforms.clone(),
                channels: self.channels.clone(),
            }
            .serialize(serializer)
        } else {
            #[serde_as]
            #[derive(Serialize)]
            struct FullCompiledDataContext {
                #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
                logical_plan: Option<LogicalPlanNode>,
                store_data: Option<StoreData>,
                #[serde(default)]
                transforms: Vec<DataTransformStage>,
                channels: IndexMap<String, ChannelValue>,
                #[serde(default)]
                pattern_channels: IndexMap<String, PatternChannelValue>,
            }

            FullCompiledDataContext {
                logical_plan: self.logical_plan.clone(),
                store_data: self.store_data.clone(),
                transforms: self.transforms.clone(),
                channels: self.channels.clone(),
                pattern_channels: self.pattern_channels.clone(),
            }
            .serialize(serializer)
        }
    }
}

impl CompiledDataContext {
    /// Create a new CompiledDataContext from a DataFrame
    pub fn new(
        dataframe: Option<DataFrame>,
        transforms: Vec<DataTransformStage>,
        channels: IndexMap<String, ChannelValue>,
    ) -> Self {
        let logical_plan = if let Some(df) = dataframe {
            let plan = df.logical_plan().clone();
            LogicalPlanNode::from_logical_plan(&plan).ok()
        } else {
            None
        };
        Self {
            logical_plan,
            store_data: None,
            transforms,
            channels,
            pattern_channels: IndexMap::new(),
        }
    }

    pub fn new_with_pattern_channels(
        dataframe: Option<DataFrame>,
        transforms: Vec<DataTransformStage>,
        channels: IndexMap<String, ChannelValue>,
        pattern_channels: IndexMap<String, PatternChannelValue>,
    ) -> Self {
        let logical_plan = if let Some(df) = dataframe {
            let plan = df.logical_plan().clone();
            LogicalPlanNode::from_logical_plan(&plan).ok()
        } else {
            None
        };
        Self {
            logical_plan,
            store_data: None,
            transforms,
            channels,
            pattern_channels,
        }
    }

    pub fn new_store_data(
        store_data: StoreData,
        transforms: Vec<DataTransformStage>,
        channels: IndexMap<String, ChannelValue>,
    ) -> Self {
        Self {
            logical_plan: None,
            store_data: Some(store_data),
            transforms,
            channels,
            pattern_channels: IndexMap::new(),
        }
    }

    pub fn new_store_data_with_pattern_channels(
        store_data: StoreData,
        transforms: Vec<DataTransformStage>,
        channels: IndexMap<String, ChannelValue>,
        pattern_channels: IndexMap<String, PatternChannelValue>,
    ) -> Self {
        Self {
            logical_plan: None,
            store_data: Some(store_data),
            transforms,
            channels,
            pattern_channels,
        }
    }

    /// Create from an existing LogicalPlanNode (for backwards compatibility)
    pub fn from_logical_plan_node(
        logical_plan: Option<LogicalPlanNode>,
        transforms: Vec<DataTransformStage>,
        channels: IndexMap<String, ChannelValue>,
    ) -> Self {
        Self {
            logical_plan,
            store_data: None,
            transforms,
            channels,
            pattern_channels: IndexMap::new(),
        }
    }

    pub fn from_logical_plan_node_with_pattern_channels(
        logical_plan: Option<LogicalPlanNode>,
        transforms: Vec<DataTransformStage>,
        channels: IndexMap<String, ChannelValue>,
        pattern_channels: IndexMap<String, PatternChannelValue>,
    ) -> Self {
        Self {
            logical_plan,
            store_data: None,
            transforms,
            channels,
            pattern_channels,
        }
    }

    /// Get the serialized LogicalPlanNode directly without deserialization
    pub fn logical_plan_node(&self) -> Option<&LogicalPlanNode> {
        self.logical_plan.as_ref()
    }

    pub fn store_data(&self) -> Option<&StoreData> {
        self.store_data.as_ref()
    }

    /// Whether this context starts from an explicit logical plan or mutable
    /// store instead of inheriting data from its parent plot or group.
    pub fn has_explicit_data_source(&self) -> bool {
        self.logical_plan.is_some() || self.store_data.is_some()
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

    pub fn pattern_channels(&self) -> &IndexMap<String, PatternChannelValue> {
        &self.pattern_channels
    }

    pub fn transforms(&self) -> &[DataTransformStage] {
        &self.transforms
    }

    /// Get a specific channel value
    pub fn channel(&self, channel: &str) -> Option<&ChannelValue> {
        self.channels.get(channel)
    }

    pub fn pattern_channel(&self, channel: &str) -> Option<&PatternChannelValue> {
        self.pattern_channels.get(channel)
    }

    // Compatibility methods for tests
    pub fn encoding(&self, channel: &str) -> Option<String> {
        let session_context = SessionContext::new();
        self.channels
            .get(channel)
            .and_then(|v| v.as_column_name(&session_context))
    }

    pub fn encoding_expr_string(&self, channel: &str) -> Option<String> {
        let session_context = SessionContext::new();
        self.channels
            .get(channel)
            .map(|v| format!("{:?}", v.expr(&session_context)))
    }
}

#[cfg(test)]
mod tests {
    use avenger_scenegraph::marks::pattern::{PatternFill, PatternLayer, StripePatternLayer};

    use super::*;

    fn stripe_pattern() -> PatternFill {
        PatternFill {
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(
                45.0, 16.0, 1.25,
            ))],
            ..Default::default()
        }
    }

    #[test]
    fn empty_pattern_channels_are_skipped_when_serializing() {
        let value = serde_json::to_value(CompiledDataContext::default())
            .expect("serialize compiled data context");

        assert!(value.get("pattern_channels").is_none());
    }

    #[test]
    fn missing_pattern_channels_deserializes_as_empty() {
        let mut value = serde_json::to_value(CompiledDataContext::default())
            .expect("serialize compiled data context");
        value
            .as_object_mut()
            .expect("compiled data context json object")
            .remove("pattern_channels");

        let decoded: CompiledDataContext =
            serde_json::from_value(value).expect("deserialize compiled data context");

        assert!(decoded.pattern_channels().is_empty());
    }

    #[test]
    fn empty_pattern_channels_round_trip_through_bincode() {
        let context = CompiledDataContext::default();

        let bytes = bincode::serialize(&context).expect("serialize compiled data context");
        let decoded: CompiledDataContext =
            bincode::deserialize(&bytes).expect("deserialize compiled data context");

        assert!(decoded.pattern_channels().is_empty());
    }

    #[test]
    fn non_empty_pattern_channels_are_serialized() {
        let mut pattern_channels = IndexMap::new();
        pattern_channels.insert(
            "fill_pattern".to_string(),
            PatternChannelValue::from(stripe_pattern()),
        );
        let context = CompiledDataContext::new_with_pattern_channels(
            None,
            Vec::new(),
            IndexMap::new(),
            pattern_channels,
        );

        let value =
            serde_json::to_value(context).expect("serialize compiled data context with pattern");

        assert_eq!(
            value["pattern_channels"]["fill_pattern"]["Value"]["pattern"]["layers"][0]["type"],
            "stripe"
        );
    }
}
