/// Macro to define common channels shared across all coordinate systems
#[macro_export]
macro_rules! define_common_mark_channels {
    (
        $mark:ident {
            $(
                $name:ident: {
                    type: $channel_type:expr
                    $(, default: $default:expr)?
                    $(, allow_column: $allow_column:expr)?
                    $(, required: $required:expr)?
                }
            ),* $(,)?
        }
    ) => {
        impl<C: $crate::coords::CoordinateSystem> $mark<C> {
            // Generate common encoding methods
            $(
                pub fn $name<V: Into<$crate::marks::ChannelValue>>(mut self, value: V) -> Self {
                    let channel_value = value.into();
                    self.config.data = self.config.data.with_channel_value(stringify!($name), channel_value);
                    self
                }
            )*

            /// Get common channel descriptors for this mark type
            pub fn common_channel_descriptors() -> Vec<$crate::marks::ChannelDescriptor> {
                vec![
                    $(
                        $crate::marks::ChannelDescriptor {
                            name: stringify!($name),
                            required: false $(|| $required)?,
                            channel_type: $channel_type,
                            default_value: None $(.or(Some($crate::marks::ChannelDefault::Scalar($default))))?,
                            allow_column_ref: true $(&& $allow_column)?,
                        },
                    )*
                ]
            }
        }
    };
}

/// Macro to define position channels specific to a coordinate system
#[macro_export]
macro_rules! define_position_mark_channels {
    (
        $mark:ident<$coord:ty> {
            $(
                $name:ident: {
                    type: $channel_type:expr
                    $(, default: $default:expr)?
                    $(, allow_column: $allow_column:expr)?
                    $(, required: $required:expr)?
                }
            ),* $(,)?
        }
    ) => {
        impl $mark<$coord> {
            // Generate position-specific encoding methods
            $(
                pub fn $name<V: Into<$crate::marks::ChannelValue>>(mut self, value: V) -> Self {
                    let channel_value = value.into();
                    self.config.data = self.config.data.with_channel_value(stringify!($name), channel_value);
                    self
                }
            )*

            /// Get position channel descriptors for this coordinate system
            pub fn position_channel_descriptors() -> Vec<$crate::marks::ChannelDescriptor> {
                vec![
                    $(
                        $crate::marks::ChannelDescriptor {
                            name: stringify!($name),
                            required: false $(|| $required)?,
                            channel_type: $channel_type,
                            default_value: None $(.or(Some($crate::marks::ChannelDefault::Scalar($default))))?,
                            allow_column_ref: true $(&& $allow_column)?,
                        },
                    )*
                ]
            }

            /// Get all channel descriptors for this mark in this coordinate system
            pub fn all_channel_descriptors() -> Vec<$crate::marks::ChannelDescriptor> {
                let mut descriptors = Self::common_channel_descriptors();
                descriptors.extend(Self::position_channel_descriptors());
                descriptors
            }
        }
    };
}

