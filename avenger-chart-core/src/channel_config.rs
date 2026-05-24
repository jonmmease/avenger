//! Trait-based channel configuration system
//!
//! This module provides traits that reduce duplication across channel configs
//! by implementing common behavior once and allowing configs to opt into capabilities.

use datafusion::logical_expr::Expr;
use datafusion_proto::{
    logical_plan::{DefaultLogicalExtensionCodec, to_proto::serialize_expr},
    protobuf::LogicalExprNode,
};

use crate::{AvengerChartError, ChannelValue, ConditionalValue};

trait LogicalExprNodeExt: Sized {
    fn from_expr(expr: Expr) -> Result<Self, AvengerChartError>;
}

impl LogicalExprNodeExt for LogicalExprNode {
    fn from_expr(expr: Expr) -> Result<Self, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        serialize_expr(&expr, &codec)
            .map_err(|err| AvengerChartError::SerializationError(err.to_string()))
    }
}

/// Base trait that all channel configs implement (includes scaling and conditionals)
pub trait ChannelConfig: Sized {
    /// Get the channel value
    fn get_value(&self) -> &ChannelValue;

    /// Set the channel value
    fn set_value(&mut self, value: ChannelValue);

    /// Consume self and return the inner channel value
    fn into_inner(self) -> ChannelValue;

    /// Add a conditional branch where the value will be scaled
    ///
    /// Conditions are evaluated in the order they are added. The first condition
    /// that evaluates to true will be used, and subsequent conditions will not be evaluated.
    ///
    /// # Example
    /// ```ignore
    /// channel.when_scaled(col("error"), col("error_value"))   // If error, use error_value (scaled)
    ///        .when_scaled(col("warning"), col("warn_value"))  // Else if warning, use warn_value
    /// // Otherwise use the default channel value
    /// ```
    fn when_scaled<C, V>(mut self, condition: C, value: V) -> Self
    where
        C: Into<Expr>,
        V: Into<Expr>,
    {
        let current_value = self.get_value().clone();
        let new_value = add_scaled_condition(current_value, condition.into(), value.into());
        self.set_value(new_value);
        self
    }

    /// Add a conditional branch with a literal value (bypasses scale)
    ///
    /// Conditions are evaluated in the order they are added. The first condition
    /// that evaluates to true will be used, and subsequent conditions will not be evaluated.
    ///
    /// # Example
    /// ```ignore
    /// channel.when_value(col("error"), lit("red"))      // If error, use red
    ///        .when_value(col("warning"), lit("orange"))  // Else if warning, use orange
    /// // Otherwise use the default channel value
    /// ```
    fn when_value<C, V>(mut self, condition: C, value: V) -> Self
    where
        C: Into<Expr>,
        V: Into<Expr>,
    {
        let current_value = self.get_value().clone();
        let new_value = add_value_condition(current_value, condition.into(), value.into());
        self.set_value(new_value);
        self
    }

    /// Disable scaling for this channel
    fn no_scale(mut self) -> Self {
        let new_value = self.get_value().clone().no_scale();
        self.set_value(new_value);
        self
    }
}

// Helper functions

/// Add a scaled condition to a ChannelValue
fn add_scaled_condition(current: ChannelValue, condition: Expr, value: Expr) -> ChannelValue {
    let value_node = LogicalExprNode::from_expr(value).expect("Failed to serialize expr");
    let new_branch = ConditionalValue::Scaled { expr: value_node };
    let condition_node = LogicalExprNode::from_expr(condition).expect("Failed to serialize expr");

    match current {
        ChannelValue::Conditional {
            mut conditions,
            otherwise,
            scale_config,
            legend_config,
            share_mode,
        } => {
            conditions.push((condition_node, new_branch));
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                share_mode,
            }
        }
        ChannelValue::Scaled {
            expr,
            scale_config,
            legend_config,
            scale_name: _,
            band: _,
            share_mode,
        } => ChannelValue::Conditional {
            conditions: vec![(condition_node, new_branch)],
            otherwise: ConditionalValue::Scaled { expr },
            scale_config,
            legend_config,
            share_mode,
        },
        ChannelValue::Value { expr } => ChannelValue::Conditional {
            conditions: vec![(condition_node, new_branch)],
            otherwise: ConditionalValue::Value { expr },
            scale_config: None,
            legend_config: None,
            share_mode: None,
        },
    }
}

/// Add a value condition to a ChannelValue
fn add_value_condition(current: ChannelValue, condition: Expr, value: Expr) -> ChannelValue {
    let value_node = LogicalExprNode::from_expr(value).expect("Failed to serialize expr");
    let new_branch = ConditionalValue::Value { expr: value_node };
    let condition_node = LogicalExprNode::from_expr(condition).expect("Failed to serialize expr");

    match current {
        ChannelValue::Conditional {
            mut conditions,
            otherwise,
            scale_config,
            legend_config,
            share_mode,
        } => {
            conditions.push((condition_node, new_branch));
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                share_mode,
            }
        }
        ChannelValue::Scaled {
            expr,
            scale_config,
            legend_config,
            scale_name: _,
            band: _,
            share_mode,
        } => ChannelValue::Conditional {
            conditions: vec![(condition_node, new_branch)],
            otherwise: ConditionalValue::Scaled { expr },
            scale_config,
            legend_config,
            share_mode,
        },
        ChannelValue::Value { expr } => ChannelValue::Conditional {
            conditions: vec![(condition_node, new_branch)],
            otherwise: ConditionalValue::Value { expr },
            scale_config: None,
            legend_config: None,
            share_mode: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::ScaleSharing;

    #[test]
    fn test_scale_sharing_from_bool() {
        assert_eq!(ScaleSharing::from(true), ScaleSharing::Shared);
        assert_eq!(ScaleSharing::from(false), ScaleSharing::Free);
    }

    #[test]
    fn test_scale_sharing_serde() {
        // Test serialization
        let shared = ScaleSharing::Shared;
        let json = serde_json::to_string(&shared).unwrap();
        assert_eq!(json, "\"shared\"");

        let free = ScaleSharing::Free;
        let json = serde_json::to_string(&free).unwrap();
        assert_eq!(json, "\"free\"");

        // Test deserialization
        let shared: ScaleSharing = serde_json::from_str("\"shared\"").unwrap();
        assert_eq!(shared, ScaleSharing::Shared);

        let free: ScaleSharing = serde_json::from_str("\"free\"").unwrap();
        assert_eq!(free, ScaleSharing::Free);
    }

    #[test]
    fn test_scale_sharing_to_level() {
        // Free => 0
        assert_eq!(ScaleSharing::Free.to_level(), 0);

        // Level(n) => n for various values
        assert_eq!(ScaleSharing::Level(0).to_level(), 0);
        assert_eq!(ScaleSharing::Level(1).to_level(), 1);
        assert_eq!(ScaleSharing::Level(2).to_level(), 2);
        assert_eq!(ScaleSharing::Level(10).to_level(), 10);
        assert_eq!(ScaleSharing::Level(u8::MAX).to_level(), u8::MAX);

        // Shared => u8::MAX
        assert_eq!(ScaleSharing::Shared.to_level(), u8::MAX);
    }

    #[test]
    fn test_scale_sharing_from_level() {
        // from_level now returns normalized Level values
        assert_eq!(ScaleSharing::from_level(0), ScaleSharing::Level(0));
        assert_eq!(
            ScaleSharing::from_level(u8::MAX),
            ScaleSharing::Level(u8::MAX)
        );
        assert_eq!(ScaleSharing::from_level(1), ScaleSharing::Level(1));
        assert_eq!(ScaleSharing::from_level(2), ScaleSharing::Level(2));
        assert_eq!(ScaleSharing::from_level(127), ScaleSharing::Level(127));
        assert_eq!(ScaleSharing::from_level(254), ScaleSharing::Level(254));
    }

    #[test]
    fn test_scale_sharing_level_round_trip() {
        // Test that from_level(to_level(x)) preserves level semantics
        // Note: Free and Shared are converted to their Level equivalents

        // Free -> 0 -> Level(0) (normalized)
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Free.to_level()),
            ScaleSharing::Level(0)
        );

        // Shared -> u8::MAX -> Level(255) (normalized)
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Shared.to_level()),
            ScaleSharing::Level(u8::MAX)
        );

        // Level(n) for intermediate values
        for n in [1u8, 2, 10, 100, 200, 254] {
            assert_eq!(
                ScaleSharing::from_level(ScaleSharing::Level(n).to_level()),
                ScaleSharing::Level(n)
            );
        }

        // Level(0) stays as Level(0) (normalized form)
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Level(0).to_level()),
            ScaleSharing::Level(0)
        );

        // Level(u8::MAX) stays as Level(255) (normalized form)
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Level(u8::MAX).to_level()),
            ScaleSharing::Level(u8::MAX)
        );
    }

    #[test]
    fn test_scale_sharing_should_share_with_parent() {
        // Free and Level(0) should NOT share with parent
        assert!(!ScaleSharing::Free.should_share_with_parent());
        assert!(!ScaleSharing::Level(0).should_share_with_parent());

        // Level(1+) should share with parent
        assert!(ScaleSharing::Level(1).should_share_with_parent());
        assert!(ScaleSharing::Level(2).should_share_with_parent());
        assert!(ScaleSharing::Level(10).should_share_with_parent());

        // Shared and Level(u8::MAX) should share
        assert!(ScaleSharing::Shared.should_share_with_parent());
        assert!(ScaleSharing::Level(u8::MAX).should_share_with_parent());
    }

    #[test]
    fn test_scale_sharing_is_fully_shared() {
        // Only Shared and Level(u8::MAX) are fully shared
        assert!(ScaleSharing::Shared.is_fully_shared());
        assert!(ScaleSharing::Level(u8::MAX).is_fully_shared());

        // Free and Level(0..254) are NOT fully shared
        assert!(!ScaleSharing::Free.is_fully_shared());
        assert!(!ScaleSharing::Level(0).is_fully_shared());
        assert!(!ScaleSharing::Level(1).is_fully_shared());
        assert!(!ScaleSharing::Level(100).is_fully_shared());
        assert!(!ScaleSharing::Level(254).is_fully_shared());
    }

    #[test]
    fn test_scale_sharing_is_free() {
        // Only Free and Level(0) are free
        assert!(ScaleSharing::Free.is_free());
        assert!(ScaleSharing::Level(0).is_free());

        // Everything else is not free
        assert!(!ScaleSharing::Level(1).is_free());
        assert!(!ScaleSharing::Level(100).is_free());
        assert!(!ScaleSharing::Level(u8::MAX).is_free());
        assert!(!ScaleSharing::Shared.is_free());
    }

    #[test]
    fn test_scale_sharing_to_normalized() {
        // Free normalizes to Level(0)
        assert_eq!(ScaleSharing::Free.to_normalized(), ScaleSharing::Level(0));

        // Shared normalizes to Level(255)
        assert_eq!(
            ScaleSharing::Shared.to_normalized(),
            ScaleSharing::Level(u8::MAX)
        );

        // Level values are unchanged
        assert_eq!(
            ScaleSharing::Level(0).to_normalized(),
            ScaleSharing::Level(0)
        );
        assert_eq!(
            ScaleSharing::Level(1).to_normalized(),
            ScaleSharing::Level(1)
        );
        assert_eq!(
            ScaleSharing::Level(100).to_normalized(),
            ScaleSharing::Level(100)
        );
        assert_eq!(
            ScaleSharing::Level(u8::MAX).to_normalized(),
            ScaleSharing::Level(u8::MAX)
        );
    }

    #[test]
    fn test_scale_sharing_level_serde() {
        // Test Level variant serialization
        let level1 = ScaleSharing::Level(1);
        let json = serde_json::to_string(&level1).unwrap();
        assert_eq!(json, "{\"level\":1}");

        let level42 = ScaleSharing::Level(42);
        let json = serde_json::to_string(&level42).unwrap();
        assert_eq!(json, "{\"level\":42}");

        let level_max = ScaleSharing::Level(u8::MAX);
        let json = serde_json::to_string(&level_max).unwrap();
        assert_eq!(json, "{\"level\":255}");

        // Test Level variant deserialization
        let level1: ScaleSharing = serde_json::from_str("{\"level\":1}").unwrap();
        assert_eq!(level1, ScaleSharing::Level(1));

        let level42: ScaleSharing = serde_json::from_str("{\"level\":42}").unwrap();
        assert_eq!(level42, ScaleSharing::Level(42));

        let level_max: ScaleSharing = serde_json::from_str("{\"level\":255}").unwrap();
        assert_eq!(level_max, ScaleSharing::Level(255));

        // Level(0) serializes as level:0, distinct from "free"
        let level0 = ScaleSharing::Level(0);
        let json = serde_json::to_string(&level0).unwrap();
        assert_eq!(json, "{\"level\":0}");

        let level0: ScaleSharing = serde_json::from_str("{\"level\":0}").unwrap();
        assert_eq!(level0, ScaleSharing::Level(0));
    }
}
