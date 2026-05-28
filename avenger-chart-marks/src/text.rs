use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AngleChannelConfig, ChannelValue, ColorChannelConfig, MarkState, OpacityChannelConfig,
    SizeChannelConfig, define_common_mark_channels, impl_mark_base,
};

pub struct Text<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(Text);

define_common_mark_channels! {
    Text {
        color: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        angle: {
            allow_column: true,
            with_config: AngleChannelConfig,
        },
        font_size: {
            allow_column: true,
            with_config: SizeChannelConfig,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
    }
}

impl<C> Text<C>
where
    Text<C>: Sized,
{
    pub fn text<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("text", value.into().no_scale())
    }

    pub fn align<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("align", value.into().no_scale())
    }

    pub fn baseline<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("baseline", value.into().no_scale())
    }

    pub fn font<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font", value.into().no_scale())
    }

    pub fn font_weight<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font_weight", value.into().no_scale())
    }

    pub fn font_style<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font_style", value.into().no_scale())
    }

    pub fn limit<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("limit", value.into().no_scale())
    }
}

/// Get default values for Text mark channels.
pub fn text_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "text" => Some(ScalarValue::Utf8(Some(String::new()))),
        "color" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "align" => Some(ScalarValue::Utf8(Some("left".to_string()))),
        "baseline" => Some(ScalarValue::Utf8(Some("alphabetic".to_string()))),
        "angle" => Some(ScalarValue::Float32(Some(0.0))),
        "font" => Some(ScalarValue::Utf8(Some("sans-serif".to_string()))),
        "font_size" => Some(ScalarValue::Float32(Some(10.0))),
        "font_weight" => Some(ScalarValue::Utf8(Some("normal".to_string()))),
        "font_style" => Some(ScalarValue::Utf8(Some("normal".to_string()))),
        "limit" => Some(ScalarValue::Float32(Some(0.0))),
        _ => None,
    }
}
