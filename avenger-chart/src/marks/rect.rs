use crate::channel_configs::{ColorChannelConfig, OpacityChannelConfig, StrokeWidthChannelConfig};
use crate::coords::CoordinateSystem;
use crate::marks::MarkState;
use crate::{define_common_mark_channels, impl_mark_base};

pub struct Rect<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Rect);

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Rect {
        fill: {
            // Default now comes from theme
            with_config: ColorChannelConfig,
        },
        stroke: {
            // Default now comes from theme
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            // Default now comes from theme
            with_config: StrokeWidthChannelConfig,
        },
        opacity: {
            // Default now comes from theme
            with_config: OpacityChannelConfig,
        },
        corner_radius: {
            // Default now comes from theme
        },
    }
}
