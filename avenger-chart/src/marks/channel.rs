use datafusion::logical_expr::{Expr, ident, lit};

/// Represents a channel encoding value
#[derive(Debug, Clone)]
pub enum ChannelValue {
    /// Expression that will be transformed through a scale
    Scaled {
        expr: Expr,
        /// Optional custom scale name (defaults to channel name)
        scale_name: Option<String>,
        /// Band parameter for band scales (0.0 = start of band, 1.0 = end of band)
        band: Option<f64>,
    },
    /// Expression that bypasses scaling (identity transformation)
    Identity { expr: Expr },
}

/// Trait for types that can be converted to channel values
pub trait ChannelExpr: Sized {
    /// Scale this value (default for expressions)
    fn scaled(self) -> ChannelValue;

    /// Use this value as-is without scaling (identity)
    fn identity(self) -> ChannelValue;
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
    pub fn scale_name(&self, channel_name: &str) -> Option<String> {
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
    pub fn with_band(self, band: f64) -> Self {
        match self {
            ChannelValue::Scaled {
                expr, scale_name, ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band: Some(band),
            },
            identity => identity, // No-op for identity values
        }
    }

    /// Set a custom scale name (only for scaled values)
    pub fn with_scale(self, name: impl Into<String>) -> Self {
        match self {
            ChannelValue::Scaled { expr, band, .. } => ChannelValue::Scaled {
                expr,
                scale_name: Some(name.into()),
                band,
            },
            ChannelValue::Identity { expr } => {
                // Convert to scaled with custom scale
                ChannelValue::Scaled {
                    expr,
                    scale_name: Some(name.into()),
                    band: None,
                }
            }
        }
    }

    /// Helper for creating column references
    pub fn column(column: impl Into<String>) -> Self {
        ident(column.into()).scaled()
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
    fn scaled(self) -> ChannelValue {
        ChannelValue::Scaled {
            expr: self,
            scale_name: None,
            band: None,
        }
    }

    fn identity(self) -> ChannelValue {
        ChannelValue::Identity { expr: self }
    }
}

// Implement ChannelExpr for &str
impl ChannelExpr for &str {
    fn scaled(self) -> ChannelValue {
        // Always treat strings as literals
        lit(self).scaled()
    }

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
        expr.scaled()
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
        // Test scaled
        let cv = lit(5).scaled();
        assert!(matches!(cv, ChannelValue::Scaled { .. }));

        // Test identity
        let cv = lit(5).identity();
        assert!(matches!(cv, ChannelValue::Identity { .. }));
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
