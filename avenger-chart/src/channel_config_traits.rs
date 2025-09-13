//! Trait-based channel configuration system
//!
//! This module provides traits that reduce duplication across channel configs
//! by implementing common behavior once and allowing configs to opt into capabilities.

use crate::legend::LegendBuilder;
use crate::marks::channel::{ChannelValue, ConditionalValue, LegendConfig, ScaleConfig};
use crate::scales::{Auto, Scale, ScaleSpec};
use datafusion::logical_expr::Expr;
use std::sync::Arc;

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
        let scale_config: ScaleConfig = Arc::new(f);
        let new_value = apply_scale_config(self.get_value().clone(), scale_config);
        self.set_value(new_value);
        self
    }

    /// Configure the scale with explicit type
    fn scale_with<S: ScaleSpec>(
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

        let legend_config: LegendConfig = Arc::new(move |_| legend.clone());
        let new_value = apply_legend_config(self.get_value().clone(), legend_config);
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
    let new_branch = ConditionalValue::Scaled { expr: value };

    match current {
        ChannelValue::Conditional {
            mut conditions,
            otherwise,
            scale_config,
            legend_config,
        } => {
            conditions.push((condition, new_branch));
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
            }
        }
        ChannelValue::Scaled {
            expr,
            scale_config,
            legend_config,
            scale_name: _,
            band: _,
        } => ChannelValue::Conditional {
            conditions: vec![(condition, new_branch)],
            otherwise: ConditionalValue::Scaled { expr },
            scale_config,
            legend_config,
        },
        ChannelValue::Value { expr } => ChannelValue::Conditional {
            conditions: vec![(condition, new_branch)],
            otherwise: ConditionalValue::Value { expr },
            scale_config: None,
            legend_config: None,
        },
    }
}

/// Add a value condition to a ChannelValue
fn add_value_condition(current: ChannelValue, condition: Expr, value: Expr) -> ChannelValue {
    let new_branch = ConditionalValue::Value { expr: value };

    match current {
        ChannelValue::Conditional {
            mut conditions,
            otherwise,
            scale_config,
            legend_config,
        } => {
            conditions.push((condition, new_branch));
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
            }
        }
        ChannelValue::Scaled {
            expr,
            scale_config,
            legend_config,
            scale_name: _,
            band: _,
        } => ChannelValue::Conditional {
            conditions: vec![(condition, new_branch)],
            otherwise: ConditionalValue::Scaled { expr },
            scale_config,
            legend_config,
        },
        ChannelValue::Value { expr } => ChannelValue::Conditional {
            conditions: vec![(condition, new_branch)],
            otherwise: ConditionalValue::Value { expr },
            scale_config: None,
            legend_config: None,
        },
    }
}

/// Apply scale configuration to a ChannelValue
fn apply_scale_config(value: ChannelValue, scale_config: ScaleConfig) -> ChannelValue {
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
        },
        ChannelValue::Value { .. } => {
            // Identity values don't support scale config
            value
        }
    }
}

/// Apply legend configuration to a ChannelValue
fn apply_legend_config(value: ChannelValue, legend_config: LegendConfig) -> ChannelValue {
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
        },
        ChannelValue::Value { .. } => {
            // Identity values don't support legend config
            value
        }
    }
}
