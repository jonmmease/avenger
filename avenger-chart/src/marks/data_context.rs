use crate::marks::ChannelValue;
use datafusion::dataframe::DataFrame;
use indexmap::IndexMap;

/// Stores a mark's data source and channel-to-expression mappings 
/// (e.g., x -> col("price"), fill -> lit("blue")
#[derive(Clone, Default)]
pub struct DataContext {
    dataframe: Option<DataFrame>,
    channels: IndexMap<String, ChannelValue>,
}

impl DataContext {
    pub fn new(dataframe: DataFrame) -> Self {
        Self {
            dataframe: Some(dataframe),
            channels: IndexMap::new(),
        }
    }

    pub fn dataframe(&self) -> Option<&DataFrame> {
        self.dataframe.as_ref()
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
        self.channels.get(channel).and_then(|v| v.as_column_name())
    }

    pub fn encoding_expr_string(&self, channel: &str) -> Option<String> {
        self.channels
            .get(channel)
            .map(|v| format!("{:?}", v.expr()))
    }
}
