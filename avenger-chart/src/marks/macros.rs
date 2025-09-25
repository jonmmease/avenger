/// Macro to implement base mark functionality
/// Provides constructor and builder methods for marks
#[macro_export]
macro_rules! impl_mark_base {
    ($mark_type:ident) => {
        impl<C: CoordinateSystem> Default for $mark_type<C> {
            fn default() -> Self {
                Self {
                    state: $crate::marks::MarkState {
                        data: $crate::marks::DataContext::default(),
                        facet_strategy: $crate::marks::FacetStrategy::Filter,
                        details: None,
                        zindex: None,
                        axis_configs: std::collections::HashMap::new(),
                    },
                    _phantom: std::marker::PhantomData,
                }
            }
        }

        impl<C: CoordinateSystem> $mark_type<C> {
            /// Create a new mark with default settings
            pub fn new() -> Self {
                Self::default()
            }

            /// Set explicit data for this mark
            pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
                self.state.data = $crate::marks::DataContext::new(dataframe);
                self
            }

            /// Control faceting behavior
            pub fn facet_strategy(mut self, strategy: $crate::marks::FacetStrategy) -> Self {
                self.state.facet_strategy = strategy;
                self
            }

            /// Make this mark appear in all facets
            pub fn broadcast_to_facets(mut self) -> Self {
                self.state.facet_strategy = $crate::marks::FacetStrategy::Broadcast;
                self
            }

            /// Set detail channels for tooltips/interactions
            pub fn details(mut self, details: Vec<String>) -> Self {
                self.state.details = Some(details);
                self
            }

            /// Set rendering order
            pub fn zindex(mut self, zindex: i32) -> Self {
                self.state.zindex = Some(zindex);
                self
            }

            // Accessor methods for use by coordinate-specific implementations

            /// Get the z-index for rendering
            #[inline]
            pub fn get_zindex(&self) -> Option<i32> {
                self.state.zindex
            }

            /// Get the data context
            #[inline]
            pub fn get_data_context(&self) -> &$crate::marks::DataContext {
                &self.state.data
            }

            /// Set a channel value (for use by macros)
            #[doc(hidden)]
            pub fn with_channel_value(
                mut self,
                name: &str,
                value: $crate::marks::ChannelValue,
            ) -> Self {
                self.state.data = self.state.data.with_channel_value(name, value);
                self
            }

            /// Get mutable access to state (for use by coordinate-specific implementations)
            #[doc(hidden)]
            pub fn state_mut(&mut self) -> &mut $crate::marks::MarkState {
                &mut self.state
            }
        }
    };
}

/// Macro to implement common Mark trait methods
/// Usage:
///   - impl_mark_trait_common!(MarkType) - Without build method
///   - impl_mark_trait_common!(MarkType, RendererType) - With build method
#[macro_export]
macro_rules! impl_mark_trait_common {
    // Pattern with RendererType - generates build method
    ($mark_type:ident, $renderer_type:ident) => {
        fn state(&self) -> &$crate::marks::MarkState {
            &self.state
        }

        fn state_mut(&mut self) -> &mut $crate::marks::MarkState {
            &mut self.state
        }

        fn data_context(&self) -> &$crate::marks::DataContext {
            self.get_data_context()
        }

        fn build(&self) -> std::sync::Arc<dyn $crate::marks::MarkRenderer> {
            std::sync::Arc::new($renderer_type {
                state: self.state.clone(),
            })
        }
    };

    // Pattern without RendererType - no build method generated
    ($mark_type:ident) => {
        fn state(&self) -> &$crate::marks::MarkState {
            &self.state
        }

        fn state_mut(&mut self) -> &mut $crate::marks::MarkState {
            &mut self.state
        }

        fn data_context(&self) -> &$crate::marks::DataContext {
            self.get_data_context()
        }
    };
}

/// Macro to generate supported_channels method for MarkRenderer implementations
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
