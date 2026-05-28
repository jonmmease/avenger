use datafusion_common::ScalarValue;

use avenger_chart_core::{MarkState, define_common_mark_channels, impl_mark_base};

pub struct Image<C> {
    pub(crate) state: MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(Image);

define_common_mark_channels! {
    Image {
        image: {
            allow_column: true,
        },
        width: {
            allow_column: true,
        },
        height: {
            allow_column: true,
        },
        align: {
            allow_column: true,
        },
        baseline: {
            allow_column: true,
        },
        aspect: {
            allow_column: false,
        },
        smooth: {
            allow_column: false,
        },
    }
}

/// Get default values for Image mark channels.
pub fn image_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "width" => Some(ScalarValue::Float32(Some(0.0))),
        "height" => Some(ScalarValue::Float32(Some(0.0))),
        "align" => Some(ScalarValue::Utf8(Some("left".to_string()))),
        "baseline" => Some(ScalarValue::Utf8(Some("top".to_string()))),
        "aspect" => Some(ScalarValue::Boolean(Some(true))),
        "smooth" => Some(ScalarValue::Boolean(Some(true))),
        _ => None,
    }
}
