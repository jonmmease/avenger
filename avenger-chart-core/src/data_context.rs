use indexmap::IndexMap;

use datafusion::{dataframe::DataFrame, prelude::SessionContext};

use crate::{
    ChannelValue, CompiledDataTransform, CoordinationScope, DataTransformStage, StoreData,
};

/// Stores a mark's data source and channel-to-expression mappings during construction
/// This is the uncompiled version that holds a live DataFrame that can be transformed
/// (e.g., x -> col("price"), fill -> lit("blue"))
#[derive(Clone)]
pub struct DataContext {
    dataframe: Option<DataFrame>,
    store_data: Option<StoreData>,
    transforms: Vec<DataTransformStage>,
    channels: IndexMap<String, ChannelValue>,
}

impl Default for DataContext {
    fn default() -> Self {
        Self {
            dataframe: None,
            store_data: None,
            transforms: Vec::new(),
            channels: IndexMap::new(),
        }
    }
}

impl DataContext {
    pub fn new(dataframe: DataFrame) -> Self {
        Self {
            dataframe: Some(dataframe),
            store_data: None,
            transforms: Vec::new(),
            channels: IndexMap::new(),
        }
    }

    pub fn store_data(data: StoreData) -> Self {
        Self {
            dataframe: None,
            store_data: Some(data),
            transforms: Vec::new(),
            channels: IndexMap::new(),
        }
    }

    /// Get the DataFrame if present
    pub fn dataframe(&self) -> Option<&DataFrame> {
        self.dataframe.as_ref()
    }

    /// Get a mutable reference to the DataFrame if present
    pub fn dataframe_mut(&mut self) -> Option<&mut DataFrame> {
        self.dataframe.as_mut()
    }

    /// Take ownership of the DataFrame, leaving None in its place
    pub fn take_dataframe(&mut self) -> Option<DataFrame> {
        self.dataframe.take()
    }

    pub fn store_data_ref(&self) -> Option<&StoreData> {
        self.store_data.as_ref()
    }

    pub fn with_channel_value(mut self, channel: &str, value: ChannelValue) -> Self {
        self.channels.insert(channel.to_string(), value);
        self
    }

    pub fn with_transform(mut self, transform: Box<dyn CompiledDataTransform>) -> Self {
        self.transforms
            .push(DataTransformStage::new(CoordinationScope::Free, transform));
        self
    }

    pub fn with_transform_stage(
        mut self,
        scope: CoordinationScope,
        transform: Box<dyn CompiledDataTransform>,
    ) -> Self {
        self.transforms
            .push(DataTransformStage::new(scope, transform));
        self
    }

    pub fn channel(&self, channel: &str) -> Option<&ChannelValue> {
        self.channels.get(channel)
    }

    pub fn channels(&self) -> &IndexMap<String, ChannelValue> {
        &self.channels
    }

    pub fn transforms(&self) -> &[DataTransformStage] {
        &self.transforms
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
