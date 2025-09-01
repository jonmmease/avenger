/// Macro to define common channels shared across all coordinate systems
///
/// This macro generates:
/// 1. Basic setter methods for all channels (e.g., `fill()`, `size()`)
/// 2. Configuration methods with callbacks when `with_config` is specified
/// 3. Channel descriptors for the mark type
///
/// Each channel must specify `with_config` to get a `_with` method.
#[macro_export]
macro_rules! define_common_mark_channels {
    // Generate _with method using explicitly specified config type
    (@generate_with_method $mark:ident, $name:ident, with_config: $config_type:ty $(,$rest:tt)*) => {
        paste::paste! {
            pub fn [<$name _with>]<V, F>(self, value: V, f: F) -> Self
            where
                V: Into<$crate::marks::ChannelValue>,
                F: FnOnce($config_type) -> $config_type,
            {
                use $crate::channel_config_traits::ChannelConfig;
                let channel_value = value.into();
                let config = <$config_type>::new(channel_value);
                let configured = f(config);
                self.with_channel_value(stringify!($name), configured.into_inner())
            }
        }
    };

    // No with_config specified - no _with method generated
    (@generate_with_method $mark:ident, $name:ident $(,$rest:tt)*) => {
        // No _with method for this channel
    };

    // Main pattern - now accepts optional with_config and axis_config fields
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
        impl<C: $crate::coords::CoordinateSystem> $mark<C> {
            // Generate common encoding methods
            $(
                pub fn $name<V: Into<$crate::marks::ChannelValue>>(self, value: V) -> Self {
                    let channel_value = value.into();
                    self.with_channel_value(stringify!($name), channel_value)
                }

                // Generate _with configuration method based on optional config
                define_common_mark_channels!(@generate_with_method
                    $mark,
                    $name
                    $(, with_config: $config_type)?
                    $(, axis_config: $is_axis)?
                );
            )*

            /// Get common channel descriptors for this mark type
            pub fn common_channel_descriptors() -> Vec<$crate::marks::ChannelDescriptor> {
                vec![
                    $(
                        $crate::marks::ChannelDescriptor {
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
