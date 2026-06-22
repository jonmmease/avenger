use indexmap::IndexMap;

use datafusion::{dataframe::DataFrame, prelude::SessionContext};

use crate::{
    AvengerChartError, ChannelValue, CompiledDataTransform, CoordinationScope, DataTransformStage,
    RepeatContext, StoreData, resolve_repeat_channel_value, resolve_repeat_placeholders,
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

    /// Whether this context starts from an explicit dataframe or mutable store
    /// instead of inheriting data from its parent plot or group.
    pub fn has_explicit_data_source(&self) -> bool {
        self.dataframe.is_some() || self.store_data.is_some()
    }

    pub fn with_channel_value(mut self, channel: &str, value: ChannelValue) -> Self {
        self.channels.insert(channel.to_string(), value);
        self
    }

    #[doc(hidden)]
    pub fn with_channels(mut self, channels: IndexMap<String, ChannelValue>) -> Self {
        self.channels = channels;
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

    pub fn resolve_repeat(&self, ctx: &RepeatContext) -> Result<Self, AvengerChartError> {
        Ok(Self {
            dataframe: self.dataframe.clone(),
            store_data: self.store_data.clone(),
            transforms: self
                .transforms
                .iter()
                .map(|stage| stage.map_exprs(&mut |expr| resolve_repeat_placeholders(expr, ctx)))
                .collect::<Result<_, AvengerChartError>>()?,
            channels: self
                .channels
                .iter()
                .map(|(channel, value)| {
                    Ok((
                        channel.clone(),
                        resolve_repeat_channel_value(value.clone(), ctx)?,
                    ))
                })
                .collect::<Result<_, AvengerChartError>>()?,
        })
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
