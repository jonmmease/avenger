use crate::{ChannelConfig, ChannelValue};

macro_rules! define_channel_config {
    ($name:ident) => {
        pub struct $name {
            value: ChannelValue,
        }

        impl $name {
            pub fn new(value: ChannelValue) -> Self {
                Self { value }
            }
        }

        impl ChannelConfig for $name {
            fn get_value(&self) -> &ChannelValue {
                &self.value
            }

            fn set_value(&mut self, value: ChannelValue) {
                self.value = value;
            }

            fn into_inner(self) -> ChannelValue {
                self.value
            }
        }
    };
}

// Channel config for color channels (fill, stroke, color)
define_channel_config!(ColorChannelConfig);

// Channel config for size channels
define_channel_config!(SizeChannelConfig);

// Channel config for shape channels
define_channel_config!(ShapeChannelConfig);

// Channel config for SVG path data channels
define_channel_config!(PathChannelConfig);

// Channel config for SVG path transform channels
define_channel_config!(TransformChannelConfig);

// Channel config for opacity channels
define_channel_config!(OpacityChannelConfig);

// Channel config for angle channels
define_channel_config!(AngleChannelConfig);

// Channel config for stroke width channels
define_channel_config!(StrokeWidthChannelConfig);

// Channel config for stroke dash channels
define_channel_config!(StrokeDashChannelConfig);

#[cfg(test)]
mod tests {
    use datafusion::prelude::{col, lit};

    use crate::{
        ChannelConfig, ChannelValue, ColorChannelConfig, ConditionalValue, PathChannelConfig,
        StrokeDashChannelConfig, TransformChannelConfig,
    };

    #[test]
    fn all_common_configs_implement_channel_config() {
        fn assert_channel_config<T: ChannelConfig>() {}

        assert_channel_config::<crate::ColorChannelConfig>();
        assert_channel_config::<crate::SizeChannelConfig>();
        assert_channel_config::<crate::ShapeChannelConfig>();
        assert_channel_config::<PathChannelConfig>();
        assert_channel_config::<TransformChannelConfig>();
        assert_channel_config::<crate::OpacityChannelConfig>();
        assert_channel_config::<crate::AngleChannelConfig>();
        assert_channel_config::<crate::StrokeWidthChannelConfig>();
        assert_channel_config::<StrokeDashChannelConfig>();
    }

    #[test]
    fn no_scale_converts_scaled_channel_to_value() {
        let value = ColorChannelConfig::new(col("category").into())
            .no_scale()
            .into_inner();

        assert!(matches!(value, ChannelValue::Value { .. }));
    }

    #[test]
    fn conditionals_keep_scaled_otherwise_branch() {
        let value = ColorChannelConfig::new(col("temperature").into())
            .when_value(col("selected"), lit("red"))
            .into_inner();

        let ChannelValue::Conditional {
            conditions,
            otherwise,
            share_mode,
            ..
        } = value
        else {
            panic!("expected conditional channel value");
        };

        assert_eq!(conditions.len(), 1);
        assert!(matches!(conditions[0].1, ConditionalValue::Value { .. }));
        assert!(matches!(otherwise, ConditionalValue::Scaled { .. }));
        assert_eq!(share_mode, None);
    }
}
