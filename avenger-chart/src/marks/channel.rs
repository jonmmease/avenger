use crate::legend::Legend;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use datafusion::logical_expr::{Expr, ident, lit};
use std::sync::Arc;

/// Type alias for scale configuration function
pub type ScaleConfig = Arc<dyn Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync>;

/// Type alias for legend configuration function  
pub type LegendConfig = Arc<dyn Fn(Legend) -> Legend + Send + Sync>;

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
        }
    }
}

impl ChannelValue {
    /// Check if this channel has scale configuration
    pub fn has_scale_config(&self) -> bool {
        match self {
            ChannelValue::Scaled { scale_config, .. } => scale_config.is_some(),
            _ => false,
        }
    }

    /// Check if this channel has legend configuration
    pub fn has_legend_config(&self) -> bool {
        match self {
            ChannelValue::Scaled { legend_config, .. } => legend_config.is_some(),
            _ => false,
        }
    }

    /// Get the scale configuration if present
    pub fn get_scale_config(&self) -> Option<&ScaleConfig> {
        match self {
            ChannelValue::Scaled { scale_config, .. } => scale_config.as_ref(),
            _ => None,
        }
    }

    /// Get the legend configuration if present
    pub fn get_legend_config(&self) -> Option<&LegendConfig> {
        match self {
            ChannelValue::Scaled { legend_config, .. } => legend_config.as_ref(),
            _ => None,
        }
    }
}

/// Trait for types that can be converted to channel values
pub trait ChannelExpr: Sized {
    /// Use this value as-is without scaling (identity)
    fn identity(self) -> ChannelValue;

    /// Set the band parameter for this channel
    fn band(self, band: f64) -> ChannelValue
    where
        Self: Into<ChannelValue>,
    {
        let channel_value: ChannelValue = self.into();
        channel_value.band(band)
    }

    /// Configure the scale for this value
    fn scale<F>(self, f: F) -> ChannelValue
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
        Self: Into<ChannelValue>,
    {
        let channel_value: ChannelValue = self.into();
        channel_value.scale(f)
    }

    /// Configure the scale with explicit type
    fn scale_with<S: ScaleTypeSpec, F>(self, f: F) -> ChannelValue
    where
        F: Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
        Self: Into<ChannelValue>,
    {
        let channel_value: ChannelValue = self.into();
        channel_value.scale_with(f)
    }

    /// Configure the legend for this value
    fn legend<F>(self, f: F) -> ChannelValue
    where
        F: Fn(Legend) -> Legend + Send + Sync + 'static,
        Self: Into<ChannelValue>,
    {
        let channel_value: ChannelValue = self.into();
        channel_value.legend(f)
    }

    /// Disable legend for this value
    fn no_legend(self) -> ChannelValue
    where
        Self: Into<ChannelValue>,
    {
        let channel_value: ChannelValue = self.into();
        channel_value.no_legend()
    }
}

impl ChannelValue {
    /// Get the expression
    pub fn expr(&self) -> &Expr {
        match self {
            ChannelValue::Scaled { expr, .. } => expr,
            ChannelValue::Identity { expr } => expr,
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
            identity => identity, // No-op for identity values
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

    /// Helper for creating column references
    pub fn column(column: impl Into<String>) -> Self {
        ident(column.into()).into()
    }

    /// Helper for creating unscaled values (backwards compat)
    pub fn no_scale(expr: Expr) -> Self {
        expr.identity()
    }

    /// Extract column name if this is a simple column reference
    pub fn as_column_name(&self) -> Option<String> {
        // Check if expr is a simple column identifier
        if let Expr::Column(col) = self.expr() {
            Some(col.name.clone())
        } else {
            None
        }
    }
}

/// Remove trailing numbers from a channel name to get the base scale name
/// e.g., "x1" -> "x", "color2" -> "color", "x" -> "x"
pub(crate) fn strip_trailing_numbers(name: &str) -> &str {
    name.trim_end_matches(char::is_numeric)
}

// Implement ChannelExpr for Expr
impl ChannelExpr for Expr {
    fn identity(self) -> ChannelValue {
        ChannelValue::Identity { expr: self }
    }
}

// Implement ChannelExpr for &str
impl ChannelExpr for &str {
    fn identity(self) -> ChannelValue {
        lit(self).identity()
    }
}

// Smart conversion for &str - always literals, identity by default
impl From<&str> for ChannelValue {
    fn from(s: &str) -> Self {
        // Always treat strings as literals - identity by default
        lit(s).identity()
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
        lit(v).identity()
    }
}

impl From<f32> for ChannelValue {
    fn from(v: f32) -> Self {
        lit(v as f64).identity()
    }
}

impl From<i32> for ChannelValue {
    fn from(v: i32) -> Self {
        lit(v).identity()
    }
}

impl From<i64> for ChannelValue {
    fn from(v: i64) -> Self {
        lit(v).identity()
    }
}

impl From<bool> for ChannelValue {
    fn from(v: bool) -> Self {
        lit(v).identity()
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
        let cv = lit(5).identity();
        assert!(matches!(cv, ChannelValue::Identity { .. }));

        // Test band
        let cv = col("x").band(0.5);
        assert!(matches!(
            cv,
            ChannelValue::Scaled {
                band: Some(0.5),
                ..
            }
        ));

        // Test band on expression creates scaled with band
        let cv1 = col("x").band(1.0);
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
