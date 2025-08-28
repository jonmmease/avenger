/// Macro to define position-specific channels for marks in different coordinate systems
///
/// This macro generates:
/// 1. Basic setter methods for all position channels (e.g., `x()`, `y()`, `r()`, `theta()`)
/// 2. Configuration methods with position configs when specified (e.g., `x_with()`, `y_with()`)
/// 3. Position channel descriptors for the mark type
/// 4. Axis configuration storage when using config types that support it
///
/// Example usage:
/// ```rust,ignore
/// define_position_channels! {
///     Symbol<Cartesian> {
///         x: {
///             type: ChannelType::Numeric,
///             with_config: CartesianPositionConfig,
///             has_axis: true
///         },
///         y: {
///             type: ChannelType::Numeric,
///             with_config: CartesianPositionConfig,
///             has_axis: true
///         }
///     }
/// }
/// ```
///
/// For polar coordinates:
/// ```rust,ignore
/// define_position_channels! {
///     Symbol<Polar> {
///         r: {
///             type: ChannelType::Numeric,
///             with_config: PolarPositionConfig,
///             has_axis: false
///         },
///         theta: {
///             type: ChannelType::Numeric,
///             with_config: PolarPositionConfig,
///             has_axis: false
///         }
///     }
/// }
/// ```
#[macro_export]
macro_rules! define_position_channels {
    // Single implementation - all position configs must implement PositionConfig trait
    (@generate_with_method $mark:ident, $coord:ty, $name:ident, with_config: $config_type:ty) => {
        paste::paste! {
            pub fn [<$name _with>]<V, F>(self, value: V, f: F) -> Self
            where
                V: Into<$crate::marks::ChannelValue>,
                F: FnOnce($config_type) -> $config_type,
                $config_type: $crate::cartesian::channels::PositionConfig<Axis = <$coord as $crate::coords::CoordinateSystem>::Axis>,
            {
                use $crate::cartesian::channels::PositionConfig;

                let channel_value = value.into();
                let config = <$config_type>::new(channel_value);
                let configured = f(config);

                // Always extract axis config (returns None for systems without axes)
                let (channel_value, axis_config) = configured.take_axis_config();

                // Store channel value
                let mut mark = self.with_channel_value(stringify!($name), channel_value);

                // Store axis config if present (will be None for Polar, etc.)
                if let Some(axis_config) = axis_config {
                    // Types match due to the trait bound above
                    mark.state_mut()
                        .axis_configs
                        .insert(stringify!($name).to_string(), axis_config);
                }

                mark
            }
        }
    };

    // Helper pattern when no config is specified - no _with method
    (@generate_with_method $mark:ident, $coord:ty, $name:ident) => {
        // No _with method generated
    };

    // Main pattern
    (
        $mark:ident<$coord:ty> {
            $(
                $name:ident: {
                    type: $channel_type:expr
                    $(, default: $default:expr)?
                    $(, required: $required:expr)?
                    $(, with_config: $config_type:ty)?
                }
            ),* $(,)?
        }
    ) => {
        impl $mark<$coord> {
            // Generate basic setter methods for all position channels
            $(
                pub fn $name<V: Into<$crate::marks::ChannelValue>>(self, value: V) -> Self {
                    self.with_channel_value(stringify!($name), value.into())
                }

                // Generate _with configuration method if config type is specified
                define_position_channels!(@generate_with_method
                    $mark,
                    $coord,
                    $name
                    $(, with_config: $config_type)?
                );
            )*

            /// Get position channel descriptors for this mark type
            pub fn position_channel_descriptors() -> Vec<$crate::marks::ChannelDescriptor> {
                vec![
                    $(
                        $crate::marks::ChannelDescriptor {
                            name: stringify!($name),
                            required: false $(|| $required)?,
                            channel_type: $channel_type,
                            default_value: None $(.or(Some($default)))?,
                            allow_column_ref: true,
                        },
                    )*
                ]
            }

            /// Get all channel descriptors (common + position)
            pub fn all_channel_descriptors() -> Vec<$crate::marks::ChannelDescriptor> {
                let mut descriptors = Self::common_channel_descriptors();
                descriptors.extend(Self::position_channel_descriptors());
                descriptors
            }
        }
    };
}
