//! Trait-based channel configuration system
//!
//! This module provides traits that reduce duplication across channel configs
//! by implementing common behavior once and allowing configs to opt into capabilities.

use crate::channel::{ChannelValue, ConditionalValue};
use crate::legend::{Legend, LegendBuilder};
use crate::scales::{Auto, Scale, ScaleSpec};
use datafusion::logical_expr::Expr;

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

    /// Configure the scale for this channel
    fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        // Create a Scale and apply the configuration
        let scale_changes = f(Scale::new());
        let new_value = apply_scale_config(self.get_value().clone(), scale_changes);
        self.set_value(new_value);
        self
    }

    /// Configure the scale with explicit type
    fn scale_with<S: ScaleSpec + Default>(
        self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        // Use Fn instead of FnOnce so it can be called multiple times
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            f(typed_scale).into_auto()
        })
    }

    /// Disable scaling for this channel
    fn no_scale(mut self) -> Self {
        let new_value = self.get_value().clone().no_scale();
        self.set_value(new_value);
        self
    }

    /// Configure whether this channel's scale is shared across facets or free per facet
    ///
    /// Supports full ScaleSharing enum: Shared, Free, SharedInRow, SharedInColumn
    fn with_scale_sharing(mut self, mode: ScaleSharing) -> Self {
        let mut value = self.get_value().clone();
        value = match value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config,
                share_mode: Some(mode),
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                share_mode: Some(mode),
            },
            other => other,
        };
        self.set_value(value);
        self
    }

    /// Share this channel's scale across all facets (convenience method)
    fn share_scale(self) -> Self {
        self.with_scale_sharing(ScaleSharing::Shared)
    }

    /// Make this channel's scale independent for each facet (convenience method)
    fn free_scale(self) -> Self {
        self.with_scale_sharing(ScaleSharing::Free)
    }

}

/// Facet scale sharing modes for a channel
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleSharing {
    /// Share scales across all facets (one domain for all subplots)
    /// Equivalent to Level(u8::MAX)
    Shared,
    /// Independent scales per facet (each subplot has its own domain)
    /// Equivalent to Level(0)
    Free,
    /// Hierarchical level-based scale sharing for nested facets
    /// Level(0) = Free (independent per cell)
    /// Level(1) = Share with immediate parent facet
    /// Level(N) = Share N levels up in the hierarchy
    /// Level(u8::MAX) = Shared (global across all facets)
    #[serde(rename = "level")]
    Level(u8),
}

impl From<bool> for ScaleSharing {
    fn from(v: bool) -> Self {
        if v {
            ScaleSharing::Shared
        } else {
            ScaleSharing::Free
        }
    }
}

impl ScaleSharing {
    /// Convert this sharing mode to a level value
    ///
    /// - Free → 0
    /// - Level(n) → n
    /// - Shared → u8::MAX
    pub fn to_level(self) -> u8 {
        match self {
            ScaleSharing::Free => 0,
            ScaleSharing::Level(n) => n,
            ScaleSharing::Shared => u8::MAX,
        }
    }

    /// Create a ScaleSharing from a level value
    ///
    /// - 0 → Free
    /// - u8::MAX → Shared
    /// - n → Level(n)
    pub fn from_level(level: u8) -> Self {
        match level {
            0 => ScaleSharing::Free,
            u8::MAX => ScaleSharing::Shared,
            n => ScaleSharing::Level(n),
        }
    }

    /// Check if this sharing mode shares with the parent facet
    ///
    /// Returns true for Level(1+), Shared
    /// Returns false for Free, Level(0)
    pub fn should_share_with_parent(self) -> bool {
        self.to_level() > 0
    }

    /// Check if this is fully shared (global across all facets)
    ///
    /// Returns true for Shared and Level(u8::MAX)
    pub fn is_fully_shared(self) -> bool {
        matches!(self, ScaleSharing::Shared | ScaleSharing::Level(u8::MAX))
    }

    /// Check if this is free (independent per facet cell)
    ///
    /// Returns true for Free and Level(0)
    pub fn is_free(self) -> bool {
        self.to_level() == 0
    }
}

/// Trait for channels that support legends
pub trait LegendableChannel: ChannelConfig {
    /// The type of legend builder for this channel
    type LegendBuilder: LegendBuilder + Default;

    /// Configure the legend for this channel
    fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(Self::LegendBuilder) -> Self::LegendBuilder,
    {
        let builder = Self::LegendBuilder::default();
        let configured = f(builder);
        let legend = configured.build();

        let new_value = apply_legend_config(self.get_value().clone(), legend);
        self.set_value(new_value);
        self
    }

    /// Disable legend for this channel
    fn no_legend(self) -> Self {
        self.legend(|l| l.visible(false))
    }
}

// Helper functions

/// Add a scaled condition to a ChannelValue
fn add_scaled_condition(current: ChannelValue, condition: Expr, value: Expr) -> ChannelValue {
    use crate::serialization::LogicalExprNodeExt;
    use datafusion_proto::protobuf::LogicalExprNode;
    let value_node = LogicalExprNode::from_expr(value).expect("Failed to serialize expr");
    let new_branch = ConditionalValue::Scaled { expr: value_node };
    let condition_node = LogicalExprNode::from_expr(condition).expect("Failed to serialize expr");

    match current {
        ChannelValue::Conditional {
            mut conditions,
            otherwise,
            scale_config,
            legend_config,
            ..
        } => {
            conditions.push((condition_node, new_branch));
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                share_mode: None,
            }
        }
        ChannelValue::Scaled {
            expr,
            scale_config,
            legend_config,
            scale_name: _,
            band: _,
            ..
        } => ChannelValue::Conditional {
            conditions: vec![(condition_node, new_branch)],
            otherwise: ConditionalValue::Scaled { expr },
            scale_config,
            legend_config,
            share_mode: None,
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
    use crate::serialization::LogicalExprNodeExt;
    use datafusion_proto::protobuf::LogicalExprNode;
    let value_node = LogicalExprNode::from_expr(value).expect("Failed to serialize expr");
    let new_branch = ConditionalValue::Value { expr: value_node };
    let condition_node = LogicalExprNode::from_expr(condition).expect("Failed to serialize expr");

    match current {
        ChannelValue::Conditional {
            mut conditions,
            otherwise,
            scale_config,
            legend_config,
            ..
        } => {
            conditions.push((condition_node, new_branch));
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                share_mode: None,
            }
        }
        ChannelValue::Scaled {
            expr,
            scale_config,
            legend_config,
            scale_name: _,
            band: _,
            ..
        } => ChannelValue::Conditional {
            conditions: vec![(condition_node, new_branch)],
            otherwise: ConditionalValue::Scaled { expr },
            scale_config,
            legend_config,
            share_mode: None,
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

/// Apply scale configuration to a ChannelValue
fn apply_scale_config(value: ChannelValue, scale_config: Scale<Auto>) -> ChannelValue {
    match value {
        ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            legend_config,
            ..
        } => ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            scale_config: Some(scale_config),
            legend_config,
            share_mode: None,
        },
        ChannelValue::Conditional {
            conditions,
            otherwise,
            legend_config,
            ..
        } => ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config: Some(scale_config),
            legend_config,
            share_mode: None,
        },
        ChannelValue::Value { .. } => {
            // Identity values don't support scale config
            value
        }
    }
}

/// Apply legend configuration to a ChannelValue
fn apply_legend_config(value: ChannelValue, legend_config: Legend) -> ChannelValue {
    match value {
        ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            scale_config,
            ..
        } => ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            scale_config,
            legend_config: Some(legend_config),
            share_mode: None,
        },
        ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config,
            ..
        } => ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config,
            legend_config: Some(legend_config),
            share_mode: None,
        },
        ChannelValue::Value { .. } => {
            // Identity values don't support legend config
            value
        }
    }
}
