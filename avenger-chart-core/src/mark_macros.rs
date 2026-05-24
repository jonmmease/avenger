/// Macro to implement base mark functionality.
///
/// This lives in core because it only depends on core mark state/data contracts.
/// Coordinate-specific `Mark<C>` implementations still live with the owning
/// coordinate package.
#[macro_export]
macro_rules! impl_mark_base {
    ($mark_type:ident) => {
        impl<C> Default for $mark_type<C>
        where
            $mark_type<C>: Sized,
        {
            fn default() -> Self {
                Self {
                    state: $crate::MarkState {
                        data: $crate::DataContext::default(),
                        facet_strategy: $crate::FacetStrategy::Filter,
                        details: None,
                        zindex: None,
                        axis_configs: std::collections::HashMap::new(),
                    },
                    _phantom: std::marker::PhantomData,
                }
            }
        }

        impl<C> $mark_type<C>
        where
            $mark_type<C>: Sized,
        {
            /// Create a new mark with default settings.
            pub fn new() -> Self {
                Self::default()
            }

            /// Set explicit data for this mark.
            pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
                self.state.data = $crate::DataContext::new(dataframe);
                self
            }

            /// Control faceting behavior.
            pub fn facet_strategy(mut self, strategy: $crate::FacetStrategy) -> Self {
                self.state.facet_strategy = strategy;
                self
            }

            /// Make this mark appear in all facets.
            pub fn broadcast_to_facets(mut self) -> Self {
                self.state.facet_strategy = $crate::FacetStrategy::Broadcast;
                self
            }

            /// Set detail channels for tooltips/interactions.
            pub fn details(mut self, details: Vec<String>) -> Self {
                self.state.details = Some(details);
                self
            }

            /// Set rendering order.
            pub fn zindex(mut self, zindex: i32) -> Self {
                self.state.zindex = Some(zindex);
                self
            }

            /// Get the mark state.
            #[inline]
            pub fn state(&self) -> &$crate::MarkState {
                &self.state
            }

            /// Get mutable mark state.
            #[inline]
            pub fn state_mut(&mut self) -> &mut $crate::MarkState {
                &mut self.state
            }

            /// Get the mark state without colliding with trait method names.
            #[doc(hidden)]
            #[inline]
            pub fn mark_state(&self) -> &$crate::MarkState {
                &self.state
            }

            /// Get mutable mark state without colliding with trait method names.
            #[doc(hidden)]
            #[inline]
            pub fn mark_state_mut(&mut self) -> &mut $crate::MarkState {
                &mut self.state
            }

            /// Get the z-index for rendering.
            #[inline]
            pub fn get_zindex(&self) -> Option<i32> {
                self.state.zindex
            }

            /// Get the data context.
            #[inline]
            pub fn get_data_context(&self) -> &$crate::DataContext {
                &self.state.data
            }

            /// Set a channel value.
            #[doc(hidden)]
            pub fn with_channel_value(mut self, name: &str, value: $crate::ChannelValue) -> Self {
                self.state.data = self.state.data.with_channel_value(name, value);
                self
            }
        }
    };
}

/// Macro to define common channels shared across coordinate systems.
///
/// Scale and legend fluent methods are supplied by their owning extension
/// traits; this macro only needs core channel configuration contracts.
#[macro_export]
macro_rules! define_common_mark_channels {
    (@generate_with_method $mark:ident, $name:ident, with_config: $config_type:ty $(,$rest:tt)*) => {
        $crate::__private::paste::paste! {
            pub fn [<$name _with>]<V, F>(self, value: V, f: F) -> Self
            where
                V: Into<$crate::ChannelValue>,
                F: FnOnce($config_type) -> $config_type,
            {
                use $crate::ChannelConfig;

                let channel_value = value.into();
                let config = <$config_type>::new(channel_value);
                let configured = f(config);
                self.with_channel_value(stringify!($name), configured.into_inner())
            }
        }
    };

    (@generate_with_method $mark:ident, $name:ident $(,$rest:tt)*) => {
        // No `_with` method for this channel.
    };

    (
        $mark:ident {
            $(
                $name:ident: {
                    $(default: $default:expr,)?
                    $(allow_column: $allow_column:expr,)?
                    $(required: $required:expr,)?
                    $(with_config: $config_type:ty,)?
                    $(axis_config: $is_axis:expr,)?
                }
            ),* $(,)?
        }
    ) => {
        impl<C> $mark<C>
        where
            $mark<C>: Sized,
        {
            $(
                pub fn $name<V: Into<$crate::ChannelValue>>(self, value: V) -> Self {
                    let channel_value = value.into();
                    self.with_channel_value(stringify!($name), channel_value)
                }

                $crate::define_common_mark_channels!(@generate_with_method
                    $mark,
                    $name
                    $(, with_config: $config_type)?
                    $(, axis_config: $is_axis)?
                );
            )*

            /// Get common channel descriptors for this mark type.
            pub fn common_channel_descriptors() -> Vec<$crate::ChannelDescriptor> {
                vec![
                    $(
                        $crate::ChannelDescriptor {
                            name: stringify!($name),
                            required: false $(|| $required)?,
                            default_value: None $(.or(Some($default)))?,
                            allow_column_ref: true $(&& $allow_column)?,
                        },
                    )*
                ]
            }
        }
    };
}
