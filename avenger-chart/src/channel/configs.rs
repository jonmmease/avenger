use super::config_traits::{ChannelConfig, LegendableChannel};
use super::value::ChannelValue;
use crate::legend::{
    AngleLegendBuilder, ColorLegendBuilder, OpacityLegendBuilder, ShapeLegendBuilder,
    SizeLegendBuilder, StrokeDashLegendBuilder, StrokeWidthLegendBuilder,
};

// Channel config for color channels (fill, stroke, color)
pub struct ColorChannelConfig {
    value: ChannelValue,
}

impl ColorChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }
}

impl ChannelConfig for ColorChannelConfig {
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

impl LegendableChannel for ColorChannelConfig {
    type LegendBuilder = ColorLegendBuilder;
}

// Channel config for size channels
pub struct SizeChannelConfig {
    value: ChannelValue,
}

impl SizeChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }
}

impl ChannelConfig for SizeChannelConfig {
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

impl LegendableChannel for SizeChannelConfig {
    type LegendBuilder = SizeLegendBuilder;
}

// Channel config for shape channels
pub struct ShapeChannelConfig {
    value: ChannelValue,
}

impl ShapeChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }
}

impl ChannelConfig for ShapeChannelConfig {
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

impl LegendableChannel for ShapeChannelConfig {
    type LegendBuilder = ShapeLegendBuilder;
}

// Channel config for opacity channels
pub struct OpacityChannelConfig {
    value: ChannelValue,
}

impl OpacityChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }
}

impl ChannelConfig for OpacityChannelConfig {
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

impl LegendableChannel for OpacityChannelConfig {
    type LegendBuilder = OpacityLegendBuilder;
}

// Channel config for angle channels
pub struct AngleChannelConfig {
    value: ChannelValue,
}

impl AngleChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }
}

impl ChannelConfig for AngleChannelConfig {
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

impl LegendableChannel for AngleChannelConfig {
    type LegendBuilder = AngleLegendBuilder;
}

// Channel config for stroke width channels
pub struct StrokeWidthChannelConfig {
    value: ChannelValue,
}

impl StrokeWidthChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }
}

impl ChannelConfig for StrokeWidthChannelConfig {
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

impl LegendableChannel for StrokeWidthChannelConfig {
    type LegendBuilder = StrokeWidthLegendBuilder;
}

// Channel config for stroke dash channels
pub struct StrokeDashChannelConfig {
    value: ChannelValue,
}

impl StrokeDashChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }
}

impl ChannelConfig for StrokeDashChannelConfig {
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

impl LegendableChannel for StrokeDashChannelConfig {
    type LegendBuilder = StrokeDashLegendBuilder;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ConditionalValue;
    use crate::serialization::LogicalExprNodeExt;
    use datafusion::prelude::*;
    use datafusion_proto::protobuf::LogicalExprNode;

    // Helper function to check if two ChannelValues are structurally equal
    // (ignoring scale_config and legend_config which contain function pointers)
    fn assert_channel_value_eq(actual: &ChannelValue, expected: &ChannelValue) {
        match (actual, expected) {
            (
                ChannelValue::Scaled {
                    expr: e1,
                    scale_name: s1,
                    band: b1,
                    ..
                },
                ChannelValue::Scaled {
                    expr: e2,
                    scale_name: s2,
                    band: b2,
                    ..
                },
            ) => {
                assert_eq!(e1, e2, "Scaled expressions don't match");
                assert_eq!(s1, s2, "Scale names don't match");
                assert_eq!(b1, b2, "Band values don't match");
            }
            (ChannelValue::Value { expr: e1 }, ChannelValue::Value { expr: e2 }) => {
                assert_eq!(e1, e2, "Value expressions don't match");
            }
            (
                ChannelValue::Conditional {
                    conditions: c1,
                    otherwise: o1,
                    ..
                },
                ChannelValue::Conditional {
                    conditions: c2,
                    otherwise: o2,
                    ..
                },
            ) => {
                assert_eq!(c1, c2, "Conditions don't match");
                assert_eq!(o1, o2, "Otherwise values don't match");
            }
            _ => panic!(
                "Channel value types don't match: expected {:?}, got {:?}",
                expected, actual
            ),
        }
    }

    #[test]
    fn test_color_config_when_value_on_scaled() {
        let config = ColorChannelConfig::new(col("temperature").into())
            .when_value(col("selected"), lit("red"));

        let expected = ChannelValue::Conditional {
            conditions: vec![(
                LogicalExprNode::from_expr(col("selected")).expect("Failed to serialize expr"),
                ConditionalValue::Value {
                    expr: LogicalExprNode::from_expr(lit("red")).expect("Failed to serialize expr"),
                },
            )],
            otherwise: ConditionalValue::Scaled {
                expr: LogicalExprNode::from_expr(col("temperature"))
                    .expect("Failed to serialize expr"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
            share_across_facets: None,
        };

        assert_channel_value_eq(&config.into_inner(), &expected);
    }

    #[test]
    fn test_color_config_when_scaled_on_identity() {
        let config = ColorChannelConfig::new(ChannelValue::Value {
            expr: LogicalExprNode::from_expr(lit("blue")).expect("Failed to serialize expr"),
        })
        .when_scaled(col("important"), col("importance_score"));

        let expected = ChannelValue::Conditional {
            conditions: vec![(
                LogicalExprNode::from_expr(col("important")).expect("Failed to serialize expr"),
                ConditionalValue::Scaled {
                    expr: LogicalExprNode::from_expr(col("importance_score"))
                        .expect("Failed to serialize expr"),
                },
            )],
            otherwise: ConditionalValue::Value {
                expr: LogicalExprNode::from_expr(lit("blue")).expect("Failed to serialize expr"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
            share_across_facets: None,
        };

        assert_channel_value_eq(&config.into_inner(), &expected);
    }

    #[test]
    fn test_color_config_multiple_conditions() {
        let config = ColorChannelConfig::new(col("default").into())
            .when_value(col("error"), lit("red"))
            .when_value(col("warning"), lit("orange"))
            .when_scaled(col("important"), col("score"));

        let expected = ChannelValue::Conditional {
            conditions: vec![
                // Conditions now stored in order of addition
                (
                    LogicalExprNode::from_expr(col("error")).expect("Failed to serialize expr"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("red"))
                            .expect("Failed to serialize expr"),
                    },
                ),
                (
                    LogicalExprNode::from_expr(col("warning")).expect("Failed to serialize expr"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("orange"))
                            .expect("Failed to serialize expr"),
                    },
                ),
                (
                    LogicalExprNode::from_expr(col("important")).expect("Failed to serialize expr"),
                    ConditionalValue::Scaled {
                        expr: LogicalExprNode::from_expr(col("score"))
                            .expect("Failed to serialize expr"),
                    },
                ),
            ],
            otherwise: ConditionalValue::Scaled {
                expr: LogicalExprNode::from_expr(col("default")).expect("Failed to serialize expr"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
            share_across_facets: None,
        };

        assert_channel_value_eq(&config.into_inner(), &expected);
    }

    #[test]
    fn test_color_config_preserves_scale_config() {
        let config = ColorChannelConfig::new(col("temperature").into())
            .scale(|s| s) // Simple identity function for testing
            .when_value(col("selected"), lit("#00ff00"));

        let value = config.into_inner();
        // We can't check the full equality due to the function pointer,
        // but we can verify the structure and that scale_config exists
        assert!(value.has_scale_config(), "Scale config should be preserved");

        // Verify the rest of the structure
        if let ChannelValue::Conditional {
            conditions,
            otherwise,
            ..
        } = value
        {
            assert_eq!(
                conditions,
                vec![(
                    LogicalExprNode::from_expr(col("selected")).expect("Failed to serialize expr"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("#00ff00"))
                            .expect("Failed to serialize expr")
                    }
                )]
            );
            assert_eq!(
                otherwise,
                ConditionalValue::Scaled {
                    expr: LogicalExprNode::from_expr(col("temperature"))
                        .expect("Failed to serialize expr")
                }
            );
        } else {
            panic!("Expected Conditional");
        }
    }

    #[test]
    fn test_color_config_preserves_legend_config() {
        let config = ColorChannelConfig::new(col("temperature").into())
            .legend(|l| l.title("Temperature"))
            .when_value(col("selected"), lit("red"));

        let value = config.into_inner();
        // We can't check the full equality due to the function pointer,
        // but we can verify the structure and that legend_config exists
        assert!(
            value.has_legend_config(),
            "Legend config should be preserved"
        );

        // Verify the rest of the structure
        if let ChannelValue::Conditional {
            conditions,
            otherwise,
            ..
        } = value
        {
            assert_eq!(
                conditions,
                vec![(
                    LogicalExprNode::from_expr(col("selected")).expect("Failed to serialize expr"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("red"))
                            .expect("Failed to serialize expr")
                    }
                )]
            );
            assert_eq!(
                otherwise,
                ConditionalValue::Scaled {
                    expr: LogicalExprNode::from_expr(col("temperature"))
                        .expect("Failed to serialize expr")
                }
            );
        } else {
            panic!("Expected Conditional");
        }
    }

    #[test]
    fn test_color_config_chaining_order() {
        // Test that conditions are evaluated in order of addition (first added has highest priority)
        let config = ColorChannelConfig::new(col("base").into())
            .when_value(col("a"), lit("red")) // Added first, evaluated first (highest priority)
            .when_value(col("b"), lit("blue")) // Added second, evaluated second
            .when_value(col("c"), lit("green")); // Added last, evaluated last (lowest priority)

        let expected = ChannelValue::Conditional {
            conditions: vec![
                // Conditions stored in order of addition
                (
                    LogicalExprNode::from_expr(col("a")).expect("Failed to serialize expr"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("red"))
                            .expect("Failed to serialize expr"),
                    },
                ),
                (
                    LogicalExprNode::from_expr(col("b")).expect("Failed to serialize expr"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("blue"))
                            .expect("Failed to serialize expr"),
                    },
                ),
                (
                    LogicalExprNode::from_expr(col("c")).expect("Failed to serialize expr"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("green"))
                            .expect("Failed to serialize expr"),
                    },
                ),
            ],
            otherwise: ConditionalValue::Scaled {
                expr: LogicalExprNode::from_expr(col("base")).expect("Failed to serialize expr"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
            share_across_facets: None,
        };

        assert_channel_value_eq(&config.into_inner(), &expected);
    }
}
