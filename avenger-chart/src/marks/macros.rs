//! Compatibility re-exports for core mark-constructor macros.

pub use avenger_chart_core::{impl_mark_base, impl_mark_trait_common};

/// Macro to generate supported_channels method for CompiledMark implementations
///
/// This macro generates a Vec<ChannelDescriptor> from lists of common and position channels.
///
/// Usage:
/// ```rust,ignore
/// impl_supported_channels! {
///     common: [size, fill, stroke, stroke_width, shape, angle],
///     position: [x, y]
/// }
/// ```
#[macro_export]
macro_rules! impl_supported_channels {
    (
        common: [$($common:ident),* $(,)?],
        position: [$($position:ident),* $(,)?]
    ) => {
        fn supported_channels(&self) -> Vec<$crate::channel::ChannelDescriptor> {
            vec![
                // Position channels
                $(
                    $crate::channel::ChannelDescriptor {
                        name: stringify!($position),
                        required: false,
                        default_value: None,
                        allow_column_ref: true,
                    },
                )*
                // Common channels
                $(
                    $crate::channel::ChannelDescriptor {
                        name: stringify!($common),
                        required: false,
                        default_value: None,
                        allow_column_ref: true,
                    },
                )*
            ]
        }
    };
}
