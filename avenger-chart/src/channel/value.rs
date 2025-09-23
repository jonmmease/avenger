use crate::legend::Legend;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use datafusion::logical_expr::{Expr, lit};

/// Helper to format floats nicely (avoid unnecessary decimals)
fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e10 {
        format!("{:.0}", f)
    } else {
        f.to_string()
    }
}

/// Helper to convert an expression to a string for display
fn expr_to_string(expr: &Expr) -> String {
    expr_to_string_impl(expr, false)
}

/// Helper to convert an expression to a string, with option to quote strings for nested contexts
fn expr_to_string_impl(expr: &Expr, quote_strings: bool) -> String {
    use datafusion::scalar::ScalarValue;

    match expr {
        // Column reference
        Expr::Column(col) => col.name.clone(),

        // Literals - show value without type wrapper
        Expr::Literal(scalar, _) => match scalar {
            ScalarValue::Boolean(Some(b)) => b.to_string(),
            ScalarValue::Float32(Some(f)) => format_float(*f as f64),
            ScalarValue::Float64(Some(f)) => format_float(*f),
            ScalarValue::Int8(Some(i)) => i.to_string(),
            ScalarValue::Int16(Some(i)) => i.to_string(),
            ScalarValue::Int32(Some(i)) => i.to_string(),
            ScalarValue::Int64(Some(i)) => i.to_string(),
            ScalarValue::UInt8(Some(i)) => i.to_string(),
            ScalarValue::UInt16(Some(i)) => i.to_string(),
            ScalarValue::UInt32(Some(i)) => i.to_string(),
            ScalarValue::UInt64(Some(i)) => i.to_string(),
            ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                if quote_strings {
                    format!("'{}'", s)
                } else {
                    s.clone()
                }
            }
            ScalarValue::Null => "null".to_string(),
            _ => "?".to_string(),
        },

        // Function calls - recursively format arguments
        Expr::ScalarFunction(func) => {
            let args = func
                .args
                .iter()
                .map(|arg| expr_to_string_impl(arg, true)) // Quote strings in function args
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", func.func.name(), args)
        }
        Expr::AggregateFunction(agg) => {
            let args = agg
                .params
                .args
                .iter()
                .map(|arg| expr_to_string_impl(arg, true)) // Quote strings in function args
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", agg.func, args)
        }

        // Binary expressions
        Expr::BinaryExpr(binary) => {
            format!(
                "{} {} {}",
                expr_to_string_impl(&binary.left, quote_strings),
                binary.op,
                expr_to_string_impl(&binary.right, quote_strings)
            )
        }

        // For other expressions, fall back to the default Display implementation
        _ => expr.to_string(),
    }
}

/// Value for conditional encoding branches
#[derive(Clone, Debug, PartialEq)]
pub enum ConditionalValue {
    /// Value that gets scaled
    Scaled { expr: Expr },
    /// Literal value that bypasses scaling
    Value { expr: Expr },
}

impl ConditionalValue {
    /// Get the expression from this conditional value
    pub fn expr(&self) -> &Expr {
        match self {
            ConditionalValue::Scaled { expr } | ConditionalValue::Value { expr } => expr,
        }
    }

    /// Check if this is a field (scaled) value
    pub fn is_scaled(&self) -> bool {
        matches!(self, ConditionalValue::Scaled { .. })
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
        scale_config: Option<Scale<Auto>>,
        /// Optional legend configuration
        legend_config: Option<Legend>,
    },
    /// Expression that bypasses scaling (identity transformation)
    Value { expr: Expr },
    /// Conditional encoding with multiple branches
    Conditional {
        /// List of (condition, value) pairs
        conditions: Vec<(Expr, ConditionalValue)>,
        /// Default value when no conditions match
        otherwise: ConditionalValue,
        /// Optional scale configuration (applies to all Field branches)
        scale_config: Option<Scale<Auto>>,
        /// Optional legend configuration (applies to all Field branches)
        legend_config: Option<Legend>,
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
            ChannelValue::Value { expr } => f.debug_struct("Identity").field("expr", expr).finish(),
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
    pub fn get_scale_config(&self) -> Option<&Scale<Auto>> {
        match self {
            ChannelValue::Scaled { scale_config, .. }
            | ChannelValue::Conditional { scale_config, .. } => scale_config.as_ref(),
            _ => None,
        }
    }

    /// Get the legend configuration if present
    pub fn get_legend_config(&self) -> Option<&Legend> {
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
            ChannelValue::Value { expr } => Some(expr),
            ChannelValue::Conditional { .. } => None,
        }
    }

    /// Get all expressions from this channel value
    pub fn all_exprs(&self) -> Vec<&Expr> {
        match self {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => vec![expr],
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
            ChannelValue::Value { .. } => None,
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
            ChannelValue::Value { expr } => {
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
        // Create a Scale and apply the configuration
        let scale_changes = f(Scale::new());

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
                scale_config: Some(scale_changes),
                legend_config,
            },
            ChannelValue::Value { expr } => {
                // Convert to scaled with scale config
                ChannelValue::Scaled {
                    expr,
                    scale_name: None,
                    band: None,
                    scale_config: Some(scale_changes),
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
                scale_config: Some(scale_changes),
                legend_config,
            },
        }
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec + Default>(
        self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            f(typed_scale).into_auto()
        })
    }

    /// Configure the legend for this channel
    pub fn legend(self, legend: Legend) -> Self {
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
                legend_config: Some(legend),
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
                legend_config: Some(legend),
            },
            ChannelValue::Value { .. } => {
                // No-op for identity values - they don't have legends
                self
            }
        }
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        self.legend(Legend::new().visible(false))
    }

    /// Disable scaling and use raw values
    pub fn no_scale(self) -> Self {
        match self {
            ChannelValue::Scaled { expr, .. } => ChannelValue::Value { expr },
            other => other,
        }
    }

    /// Extract a human-readable name from the expression.
    /// Returns column names directly, literal values without type wrapper,
    /// function names for function calls, or the expression's string representation.
    pub fn as_column_name(&self) -> Option<String> {
        self.expr().map(expr_to_string)
    }

    /// Get the data type of this channel value.
    /// For conditional values, uses the 'otherwise' expression for type inference.
    /// Returns None if the expression is a channel reference or type cannot be determined.
    pub fn get_data_type(
        &self,
        schema: &datafusion::common::DFSchema,
    ) -> Option<datafusion::arrow::datatypes::DataType> {
        use datafusion::logical_expr::ExprSchemable;

        let expr = match self {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => expr,
            ChannelValue::Conditional { otherwise, .. } => {
                // For conditional channels, use the 'otherwise' expression for type inference
                otherwise.expr()
            }
        };

        // Skip channel references - they need to be resolved first
        if let Expr::Column(c) = expr {
            if c.name.starts_with(':') {
                return None;
            }
        }

        // Try to get the data type from the expression
        expr.get_type(schema).ok()
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
        ChannelValue::Value { expr: lit(s) }
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
        ChannelValue::Value { expr: lit(v) }
    }
}

impl From<f32> for ChannelValue {
    fn from(v: f32) -> Self {
        ChannelValue::Value { expr: lit(v) }
    }
}

impl From<i32> for ChannelValue {
    fn from(v: i32) -> Self {
        ChannelValue::Value { expr: lit(v) }
    }
}

impl From<i64> for ChannelValue {
    fn from(v: i64) -> Self {
        ChannelValue::Value { expr: lit(v) }
    }
}

impl From<bool> for ChannelValue {
    fn from(v: bool) -> Self {
        ChannelValue::Value { expr: lit(v) }
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
        assert!(matches!(cv, ChannelValue::Value { .. }));

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
        assert!(matches!(cv, ChannelValue::Value { .. }));

        // Hex colors should be identity
        let cv: ChannelValue = "#ff0000".into();
        assert!(matches!(cv, ChannelValue::Value { .. }));

        // Even column-like names should be identity literals
        let cv: ChannelValue = "category".into();
        assert!(matches!(cv, ChannelValue::Value { .. }));

        // Check the expression is a literal
        if let ChannelValue::Value { expr } = cv {
            assert!(matches!(expr, Expr::Literal(..)));
        }
    }

    #[test]
    fn test_numeric_defaults() {
        // Numbers default to identity
        let cv: ChannelValue = 42.0.into();
        assert!(matches!(cv, ChannelValue::Value { .. }));

        let cv: ChannelValue = 42.into();
        assert!(matches!(cv, ChannelValue::Value { .. }));
    }

    #[test]
    fn test_as_column_name() {
        use super::ConditionalValue;
        use datafusion::functions::expr_fn::sqrt;
        use datafusion::logical_expr::col;

        // Test column reference
        let cv: ChannelValue = col("my_column").into();
        assert_eq!(cv.as_column_name(), Some("my_column".to_string()));

        // Test integer literals
        let cv: ChannelValue = 42.into();
        assert_eq!(cv.as_column_name(), Some("42".to_string()));

        let cv: ChannelValue = (-100i32).into();
        assert_eq!(cv.as_column_name(), Some("-100".to_string()));

        // Test float literals
        let cv: ChannelValue = 3.5.into();
        assert_eq!(cv.as_column_name(), Some("3.5".to_string()));

        let cv: ChannelValue = 5.0.into();
        assert_eq!(cv.as_column_name(), Some("5".to_string())); // Should format as integer

        let cv: ChannelValue = 1000.0.into();
        assert_eq!(cv.as_column_name(), Some("1000".to_string()));

        // Test string literals
        let cv: ChannelValue = "hello".into();
        assert_eq!(cv.as_column_name(), Some("hello".to_string()));

        let cv: ChannelValue = "#ff0000".into();
        assert_eq!(cv.as_column_name(), Some("#ff0000".to_string()));

        // Test boolean literals
        let cv: ChannelValue = true.into();
        assert_eq!(cv.as_column_name(), Some("true".to_string()));

        let cv: ChannelValue = false.into();
        assert_eq!(cv.as_column_name(), Some("false".to_string()));

        // Test null literal
        let cv: ChannelValue = ChannelValue::Value {
            expr: lit(datafusion::scalar::ScalarValue::Null),
        };
        assert_eq!(cv.as_column_name(), Some("null".to_string()));

        // Test function call with arguments
        let cv: ChannelValue = sqrt(col("x")).into();
        assert_eq!(cv.as_column_name(), Some("sqrt(x)".to_string()));

        // Test function with literal argument
        let cv: ChannelValue = sqrt(lit(16.0)).into();
        assert_eq!(cv.as_column_name(), Some("sqrt(16)".to_string()));

        // Test nested function calls
        use datafusion::functions::expr_fn::abs;
        let cv: ChannelValue = sqrt(abs(col("x"))).into();
        assert_eq!(cv.as_column_name(), Some("sqrt(abs(x))".to_string()));

        // Test function with multiple arguments (using pow as example)
        use datafusion::functions::expr_fn::power;
        let cv: ChannelValue = power(col("x"), lit(2)).into();
        assert_eq!(cv.as_column_name(), Some("power(x, 2)".to_string()));

        // Test function with string arguments (should be quoted in function context)
        use datafusion::functions::expr_fn::concat;
        let cv: ChannelValue = concat(vec![lit("hello"), lit("world")]).into();
        assert_eq!(
            cv.as_column_name(),
            Some("concat('hello', 'world')".to_string())
        );

        // Test complex expression (now returns the expression string)
        let cv: ChannelValue = (col("x") + col("y")).into();
        assert_eq!(cv.as_column_name(), Some("x + y".to_string()));

        // Test conditional value (should return None - no single name)
        let cv = ChannelValue::Conditional {
            conditions: vec![(
                col("category").eq(lit("A")),
                ConditionalValue::Value { expr: lit("red") },
            )],
            otherwise: ConditionalValue::Scaled { expr: col("color") },
            scale_config: None,
            legend_config: None,
        };
        // Conditional values don't have a single column name
        assert_eq!(cv.as_column_name(), None);
    }
}
