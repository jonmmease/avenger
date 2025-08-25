use crate::legend::Legend;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use datafusion::logical_expr::{Expr, lit};
use std::sync::Arc;

/// Type alias for scale configuration function
pub type ScaleConfig = Arc<dyn Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync>;

/// Type alias for legend configuration function  
pub type LegendConfig = Arc<dyn Fn(Legend) -> Legend + Send + Sync>;

/// Value for conditional encoding branches
#[derive(Clone, Debug)]
pub enum ConditionalValue {
    /// Field value that gets scaled
    Field { expr: Expr },
    /// Literal value that bypasses scaling
    Value { expr: Expr },
}

impl ConditionalValue {
    /// Get the expression from this conditional value
    pub fn expr(&self) -> &Expr {
        match self {
            ConditionalValue::Field { expr } | ConditionalValue::Value { expr } => expr,
        }
    }

    /// Check if this is a field (scaled) value
    pub fn is_field(&self) -> bool {
        matches!(self, ConditionalValue::Field { .. })
    }
}

/// Represents a channel encoding value
#[derive(Clone)]
pub enum ChannelValue {
    /// Expression that will be transformed through a scale
    Scaled {
        expr: Expr,
        /// Optional custom scale name (defaults to channel name)
        scale_name: Option<String>,
        /// Band parameter for band scales (0.0 = start of band, 1.0 = end of band)
        band: Option<f64>,
        /// Optional scale configuration
        scale_config: Option<ScaleConfig>,
        /// Optional legend configuration
        legend_config: Option<LegendConfig>,
    },
    /// Expression that bypasses scaling (identity transformation)
    Identity { expr: Expr },
    /// Conditional encoding with multiple branches
    Conditional {
        /// List of (condition, value) pairs
        conditions: Vec<(Expr, ConditionalValue)>,
        /// Default value when no conditions match
        otherwise: ConditionalValue,
        /// Optional scale configuration (applies to all Field branches)
        scale_config: Option<ScaleConfig>,
        /// Optional legend configuration (applies to all Field branches)
        legend_config: Option<LegendConfig>,
    },
}

impl std::fmt::Debug for ChannelValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                ..
            } => f
                .debug_struct("Scaled")
                .field("expr", expr)
                .field("scale_name", scale_name)
                .field("band", band)
                .field("has_scale_config", &self.has_scale_config())
                .field("has_legend_config", &self.has_legend_config())
                .finish(),
            ChannelValue::Identity { expr } => {
                f.debug_struct("Identity").field("expr", expr).finish()
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => f
                .debug_struct("Conditional")
                .field("conditions", &conditions.len())
                .field("otherwise", otherwise)
                .field("has_scale_config", &self.has_scale_config())
                .field("has_legend_config", &self.has_legend_config())
                .finish(),
        }
    }
}

impl ChannelValue {
    /// Check if this channel has scale configuration
    pub fn has_scale_config(&self) -> bool {
        match self {
            ChannelValue::Scaled { scale_config, .. }
            | ChannelValue::Conditional { scale_config, .. } => scale_config.is_some(),
            _ => false,
        }
    }

    /// Check if this channel has legend configuration
    pub fn has_legend_config(&self) -> bool {
        match self {
            ChannelValue::Scaled { legend_config, .. }
            | ChannelValue::Conditional { legend_config, .. } => legend_config.is_some(),
            _ => false,
        }
    }

    /// Get the scale configuration if present
    pub fn get_scale_config(&self) -> Option<&ScaleConfig> {
        match self {
            ChannelValue::Scaled { scale_config, .. }
            | ChannelValue::Conditional { scale_config, .. } => scale_config.as_ref(),
            _ => None,
        }
    }

    /// Get the legend configuration if present
    pub fn get_legend_config(&self) -> Option<&LegendConfig> {
        match self {
            ChannelValue::Scaled { legend_config, .. }
            | ChannelValue::Conditional { legend_config, .. } => legend_config.as_ref(),
            _ => None,
        }
    }
}

impl ChannelValue {
    /// Get the expression (for non-conditional values)
    /// For conditional values, returns None since there are multiple expressions
    pub fn expr(&self) -> Option<&Expr> {
        match self {
            ChannelValue::Scaled { expr, .. } => Some(expr),
            ChannelValue::Identity { expr } => Some(expr),
            ChannelValue::Conditional { .. } => None,
        }
    }

    /// Get all expressions from this channel value
    pub fn all_exprs(&self) -> Vec<&Expr> {
        match self {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Identity { expr } => vec![expr],
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                let mut exprs = Vec::new();
                for (cond, val) in conditions {
                    exprs.push(cond);
                    exprs.push(val.expr());
                }
                exprs.push(otherwise.expr());
                exprs
            }
        }
    }

    /// Get the scale name for this channel
    pub fn get_scale_name(&self, channel_name: &str) -> Option<String> {
        match self {
            ChannelValue::Scaled { scale_name, .. } => {
                scale_name.clone().or_else(|| {
                    // Use channel name with trailing numbers removed
                    Some(strip_trailing_numbers(channel_name).to_string())
                })
            }
            ChannelValue::Conditional { .. } => {
                // Conditional values with Field branches use scales
                Some(strip_trailing_numbers(channel_name).to_string())
            }
            ChannelValue::Identity { .. } => None,
        }
    }

    /// Set the band parameter for this channel (only for scaled values)
    pub fn band(self, band: f64) -> Self {
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band: Some(band),
                scale_config,
                legend_config,
            },
            other => other, // No-op for identity and conditional values
        }
    }

    /// Set a custom scale name (only for scaled values)
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        match self {
            ChannelValue::Scaled {
                expr,
                band,
                scale_config,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name: Some(name.into()),
                band,
                scale_config,
                legend_config,
            },
            ChannelValue::Identity { expr } => {
                // Convert to scaled with custom scale
                ChannelValue::Scaled {
                    expr,
                    scale_name: Some(name.into()),
                    band: None,
                    scale_config: None,
                    legend_config: None,
                }
            }
            ChannelValue::Conditional { .. } => {
                // Conditional values already have implicit scale names
                self
            }
        }
    }

    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        match self {
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
                scale_config: Some(Arc::new(f)),
                legend_config,
            },
            ChannelValue::Identity { expr } => {
                // Convert to scaled with scale config
                ChannelValue::Scaled {
                    expr,
                    scale_name: None,
                    band: None,
                    scale_config: Some(Arc::new(f)),
                    legend_config: None,
                }
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                legend_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config: Some(Arc::new(f)),
                legend_config,
            },
        }
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec, F>(self, f: F) -> Self
    where
        F: Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    {
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            f(typed_scale).into_auto()
        })
    }

    /// Configure the legend for this channel
    pub fn legend<F>(self, f: F) -> Self
    where
        F: Fn(Legend) -> Legend + Send + Sync + 'static,
    {
        match self {
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
                legend_config: Some(Arc::new(f)),
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
                legend_config: Some(Arc::new(f)),
            },
            ChannelValue::Identity { .. } => {
                // No-op for identity values - they don't have legends
                self
            }
        }
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        self.legend(|_| Legend::new().visible(false))
    }

    /// Disable scaling and use raw values
    pub fn no_scale(self) -> Self {
        match self {
            ChannelValue::Scaled { expr, .. } => ChannelValue::Identity { expr },
            other => other,
        }
    }

    /// Extract column name if this is a simple column reference
    pub fn as_column_name(&self) -> Option<String> {
        // Check if expr is a simple column identifier
        if let Some(Expr::Column(col)) = self.expr() {
            Some(col.name.clone())
        } else {
            None
        }
    }

    /// Create a conditional encoding
    pub fn when(condition: Expr, field: Expr) -> ConditionalBuilder {
        ConditionalBuilder {
            conditions: vec![(condition, ConditionalValue::Field { expr: field })],
            otherwise: None,
        }
    }

    /// Create a conditional encoding with a literal value
    pub fn when_value(condition: Expr, value: impl Into<ChannelValue>) -> ConditionalBuilder {
        let value = value.into();
        let cond_value = match value {
            ChannelValue::Identity { expr } => ConditionalValue::Value { expr },
            ChannelValue::Scaled { expr, .. } => ConditionalValue::Field { expr },
            _ => panic!("Cannot use conditional value in when_value"),
        };
        ConditionalBuilder {
            conditions: vec![(condition, cond_value)],
            otherwise: None,
        }
    }
}

/// Builder for conditional channel values
pub struct ConditionalBuilder {
    conditions: Vec<(Expr, ConditionalValue)>,
    otherwise: Option<ConditionalValue>,
}

impl ConditionalBuilder {
    /// Add another condition with a field
    pub fn when(mut self, condition: Expr, field: Expr) -> Self {
        self.conditions
            .push((condition, ConditionalValue::Field { expr: field }));
        self
    }

    /// Add another condition with a literal value  
    pub fn when_value(mut self, condition: Expr, value: impl Into<ChannelValue>) -> Self {
        let value = value.into();
        let cond_value = match value {
            ChannelValue::Identity { expr } => ConditionalValue::Value { expr },
            ChannelValue::Scaled { expr, .. } => ConditionalValue::Field { expr },
            _ => panic!("Cannot use conditional value in when_value"),
        };
        self.conditions.push((condition, cond_value));
        self
    }

    /// Set the default field value
    pub fn otherwise(mut self, field: Expr) -> Self {
        self.otherwise = Some(ConditionalValue::Field { expr: field });
        self
    }

    /// Set the default literal value
    pub fn otherwise_value(mut self, value: impl Into<ChannelValue>) -> Self {
        let value = value.into();
        let cond_value = match value {
            ChannelValue::Identity { expr } => ConditionalValue::Value { expr },
            ChannelValue::Scaled { expr, .. } => ConditionalValue::Field { expr },
            _ => panic!("Cannot use conditional value in otherwise_value"),
        };
        self.otherwise = Some(cond_value);
        self
    }
}

impl From<ConditionalBuilder> for ChannelValue {
    fn from(builder: ConditionalBuilder) -> Self {
        ChannelValue::Conditional {
            conditions: builder.conditions,
            otherwise: builder
                .otherwise
                .expect("Conditional encoding must have an otherwise clause"),
            scale_config: None,
            legend_config: None,
        }
    }
}

/// Remove trailing numbers from a channel name to get the base scale name
/// e.g., "x1" -> "x", "color2" -> "color", "x" -> "x"
pub(crate) fn strip_trailing_numbers(name: &str) -> &str {
    name.trim_end_matches(char::is_numeric)
}

// Smart conversion for &str - always literals, identity by default
impl From<&str> for ChannelValue {
    fn from(s: &str) -> Self {
        // Always treat strings as literals - identity by default
        ChannelValue::Identity { expr: lit(s) }
    }
}

// Expressions default to scaled
impl From<Expr> for ChannelValue {
    fn from(expr: Expr) -> Self {
        ChannelValue::Scaled {
            expr,
            scale_name: None,
            band: None,
            scale_config: None,
            legend_config: None,
        }
    }
}

// Numeric literals default to identity
impl From<f64> for ChannelValue {
    fn from(v: f64) -> Self {
        ChannelValue::Identity { expr: lit(v) }
    }
}

impl From<f32> for ChannelValue {
    fn from(v: f32) -> Self {
        ChannelValue::Identity { expr: lit(v) }
    }
}

impl From<i32> for ChannelValue {
    fn from(v: i32) -> Self {
        ChannelValue::Identity { expr: lit(v) }
    }
}

impl From<i64> for ChannelValue {
    fn from(v: i64) -> Self {
        ChannelValue::Identity { expr: lit(v) }
    }
}

impl From<bool> for ChannelValue {
    fn from(v: bool) -> Self {
        ChannelValue::Identity { expr: lit(v) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::prelude::col;

    #[test]
    fn test_strip_trailing_numbers() {
        assert_eq!(strip_trailing_numbers("x"), "x");
        assert_eq!(strip_trailing_numbers("x1"), "x");
        assert_eq!(strip_trailing_numbers("x123"), "x");
        assert_eq!(strip_trailing_numbers("color2"), "color");
        assert_eq!(strip_trailing_numbers("foo"), "foo");
    }

    #[test]
    fn test_channel_expr_trait() {
        // Test default conversion to scaled
        let cv: ChannelValue = lit(5).into();
        assert!(matches!(cv, ChannelValue::Scaled { .. }));

        // Test identity
        let cv = ChannelValue::from(5);
        assert!(matches!(cv, ChannelValue::Identity { .. }));

        // Test band - convert to ChannelValue first, then apply band
        let cv: ChannelValue = col("x").into();
        let cv = cv.band(0.5);
        assert!(matches!(
            cv,
            ChannelValue::Scaled {
                band: Some(0.5),
                ..
            }
        ));

        // Test band on ChannelValue
        let cv1: ChannelValue = col("x").into();
        let cv1 = cv1.band(1.0);
        let cv2: ChannelValue = col("x").into();
        let cv2 = cv2.band(1.0);
        match (cv1, cv2) {
            (ChannelValue::Scaled { band: b1, .. }, ChannelValue::Scaled { band: b2, .. }) => {
                assert_eq!(b1, b2);
            }
            _ => panic!("Expected both to be scaled"),
        }
    }

    #[test]
    fn test_smart_string_conversion() {
        // All strings should be identity literals
        let cv: ChannelValue = "red".into();
        assert!(matches!(cv, ChannelValue::Identity { .. }));

        // Hex colors should be identity
        let cv: ChannelValue = "#ff0000".into();
        assert!(matches!(cv, ChannelValue::Identity { .. }));

        // Even column-like names should be identity literals
        let cv: ChannelValue = "category".into();
        assert!(matches!(cv, ChannelValue::Identity { .. }));

        // Check the expression is a literal
        if let ChannelValue::Identity { expr } = cv {
            assert!(matches!(expr, Expr::Literal(..)));
        }
    }

    #[test]
    fn test_numeric_defaults() {
        // Numbers default to identity
        let cv: ChannelValue = 42.0.into();
        assert!(matches!(cv, ChannelValue::Identity { .. }));

        let cv: ChannelValue = 42.into();
        assert!(matches!(cv, ChannelValue::Identity { .. }));
    }
}
