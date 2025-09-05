use crate::channel_configs::{
    AngleChannelConfig, ColorChannelConfig, ShapeChannelConfig, SizeChannelConfig,
    StrokeWidthChannelConfig,
};
use crate::coords::CoordinateSystem;
use crate::marks::MarkState;
use crate::{define_common_mark_channels, impl_mark_base};

pub struct Symbol<C: CoordinateSystem> {
    pub(crate) state: MarkState<C>,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

// Implement MarkBase trait and Default
impl_mark_base!(Symbol);

// Define common channels using the macro with explicit config types
define_common_mark_channels! {
    Symbol {
        size: {
            // Default now comes from theme
            with_config: SizeChannelConfig,
        },
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
            allow_column: false,
            with_config: StrokeWidthChannelConfig,
        },
        shape: {
            // Default now comes from theme
            with_config: ShapeChannelConfig,
        },
        angle: {
            // Default now comes from theme
            with_config: AngleChannelConfig,
        },
    }
}
