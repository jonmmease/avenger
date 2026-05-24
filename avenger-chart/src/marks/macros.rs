//! Compatibility re-export for the core mark-constructor macro.

pub use avenger_chart_core::impl_mark_base;

/// Macro to implement common Mark trait methods
/// Usage:
///   - impl_mark_trait_common!(MarkType) - Without compile method
///   - impl_mark_trait_common!(MarkType, RendererType) - With compile method
#[macro_export]
macro_rules! impl_mark_trait_common {
    // Pattern with RendererType - generates compile method
    ($mark_type:ident, $renderer_type:ident) => {
        fn state(&self) -> &$crate::marks::MarkState {
            self.mark_state()
        }

        fn state_mut(&mut self) -> &mut $crate::marks::MarkState {
            self.mark_state_mut()
        }

        fn data_context(&self) -> &$crate::marks::DataContext {
            self.get_data_context()
        }

        async fn compile(
            &self,
            compiled_state: $crate::marks::CompiledMarkState,
            _session_context: &datafusion::prelude::SessionContext,
        ) -> Result<std::sync::Arc<dyn $crate::marks::CompiledMark>, $crate::error::AvengerChartError> {
            Ok(std::sync::Arc::new($renderer_type {
                state: compiled_state,
            }))
        }
    };

    // Pattern without RendererType - no compile method generated
    ($mark_type:ident) => {
        fn state(&self) -> &$crate::marks::MarkState {
            self.mark_state()
        }

        fn state_mut(&mut self) -> &mut $crate::marks::MarkState {
            self.mark_state_mut()
        }

        fn data_context(&self) -> &$crate::marks::DataContext {
            self.get_data_context()
        }
    };
}

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
