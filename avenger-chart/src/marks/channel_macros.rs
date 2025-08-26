/// Macro to define common channels shared across all coordinate systems
#[macro_export]
macro_rules! define_common_mark_channels {
    // Helper pattern to generate _with methods based on channel type
    (@generate_with_method $mark:ident, fill, $channel_type:expr) => {
        paste::paste! {
            pub fn [<fill _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::ColorChannel) -> $crate::marks::typed_channels::ColorChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::ColorChannel(channel_value));
                self.with_channel_value("fill", channel.into())
            }
        }
    };
    (@generate_with_method $mark:ident, stroke, $channel_type:expr) => {
        paste::paste! {
            pub fn [<stroke _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::ColorChannel) -> $crate::marks::typed_channels::ColorChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::ColorChannel(channel_value));
                self.with_channel_value("stroke", channel.into())
            }
        }
    };
    (@generate_with_method $mark:ident, size, $channel_type:expr) => {
        paste::paste! {
            pub fn [<size _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::SizeChannel) -> $crate::marks::typed_channels::SizeChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::SizeChannel(channel_value));
                self.with_channel_value("size", channel.into())
            }
        }
    };
    (@generate_with_method $mark:ident, stroke_width, $channel_type:expr) => {
        paste::paste! {
            pub fn [<stroke_width _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::SizeChannel) -> $crate::marks::typed_channels::SizeChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::SizeChannel(channel_value));
                self.with_channel_value("stroke_width", channel.into())
            }
        }
    };
    (@generate_with_method $mark:ident, shape, $channel_type:expr) => {
        paste::paste! {
            pub fn [<shape _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::ShapeChannel) -> $crate::marks::typed_channels::ShapeChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::ShapeChannel(channel_value));
                self.with_channel_value("shape", channel.into())
            }
        }
    };
    (@generate_with_method $mark:ident, angle, $channel_type:expr) => {
        paste::paste! {
            pub fn [<angle _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::AngleChannel) -> $crate::marks::typed_channels::AngleChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::AngleChannel(channel_value));
                self.with_channel_value("angle", channel.into())
            }
        }
    };
    (@generate_with_method $mark:ident, stroke_dash, $channel_type:expr) => {
        paste::paste! {
            pub fn [<stroke_dash _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::StrokeDashChannel) -> $crate::marks::typed_channels::StrokeDashChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::StrokeDashChannel(channel_value));
                self.with_channel_value("stroke_dash", channel.into())
            }
        }
    };
    // Default case for channels without typed wrappers - do nothing
    (@generate_with_method $mark:ident, $name:ident, $channel_type:expr) => {
        // No _with method for this channel
    };

    // Main pattern
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
                pub fn $name<V: Into<$crate::marks::ChannelValue>>(self, value: V) -> Self {
                    let channel_value = value.into();
                    self.with_channel_value(stringify!($name), channel_value)
                }

                // Generate _with configuration method based on channel type
                define_common_mark_channels!(@generate_with_method $mark, $name, $channel_type);
            )*

            /// Get common channel descriptors for this mark type
            pub fn common_channel_descriptors() -> Vec<$crate::marks::ChannelDescriptor> {
                vec![
                    $(
                        $crate::marks::ChannelDescriptor {
                            name: stringify!($name),
                            required: false $(|| $required)?,
                            channel_type: $channel_type,
                            default_value: None $(.or(Some($default)))?,
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
    // Helper patterns for position channel _with methods
    (@generate_with_method x) => {
        paste::paste! {
            pub fn [<x _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::PositionChannel) -> $crate::marks::typed_channels::PositionChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::PositionChannel(channel_value));
                self.with_channel_value("x", channel.into())
            }
        }
    };
    (@generate_with_method y) => {
        paste::paste! {
            pub fn [<y _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::PositionChannel) -> $crate::marks::typed_channels::PositionChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::PositionChannel(channel_value));
                self.with_channel_value("y", channel.into())
            }
        }
    };
    (@generate_with_method x2) => {
        paste::paste! {
            pub fn [<x2 _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::PositionChannel) -> $crate::marks::typed_channels::PositionChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::PositionChannel(channel_value));
                self.with_channel_value("x2", channel.into())
            }
        }
    };
    (@generate_with_method y2) => {
        paste::paste! {
            pub fn [<y2 _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::PositionChannel) -> $crate::marks::typed_channels::PositionChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::PositionChannel(channel_value));
                self.with_channel_value("y2", channel.into())
            }
        }
    };
    (@generate_with_method theta) => {
        paste::paste! {
            pub fn [<theta _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::PositionChannel) -> $crate::marks::typed_channels::PositionChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::PositionChannel(channel_value));
                self.with_channel_value("theta", channel.into())
            }
        }
    };
    (@generate_with_method radius) => {
        paste::paste! {
            pub fn [<radius _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::PositionChannel) -> $crate::marks::typed_channels::PositionChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::PositionChannel(channel_value));
                self.with_channel_value("radius", channel.into())
            }
        }
    };
    (@generate_with_method r) => {
        paste::paste! {
            pub fn [<r _with>]<F>(self, value: impl Into<$crate::marks::ChannelValue>, f: F) -> Self
            where
                F: FnOnce($crate::marks::typed_channels::PositionChannel) -> $crate::marks::typed_channels::PositionChannel,
            {
                let channel_value: $crate::marks::ChannelValue = value.into();
                let channel = f($crate::marks::typed_channels::PositionChannel(channel_value));
                self.with_channel_value("r", channel.into())
            }
        }
    };
    // Default case for other channels
    (@generate_with_method $name:ident) => {
        // No _with method for this channel
    };

    // Main pattern
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
                pub fn $name<V: Into<$crate::marks::ChannelValue>>(self, value: V) -> Self {
                    let channel_value = value.into();
                    self.with_channel_value(stringify!($name), channel_value)
                }

                // Generate _with configuration method for position channels
                define_position_mark_channels!(@generate_with_method $name);
            )*

            /// Get position channel descriptors for this coordinate system
            pub fn position_channel_descriptors() -> Vec<$crate::marks::ChannelDescriptor> {
                vec![
                    $(
                        $crate::marks::ChannelDescriptor {
                            name: stringify!($name),
                            required: false $(|| $required)?,
                            channel_type: $channel_type,
                            default_value: None $(.or(Some($default)))?,
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
