use crate::channel::config_traits::ScaleSharing;
use crate::legend::Legend;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use crate::serialization::{LogicalExprNodeExt, SerializableExpr};
use datafusion::logical_expr::{Case, Expr, lit};
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

/// Helper to format floats nicely (avoid unnecessary decimals)
fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e10 {
        format!("{:.0}", f)
    } else {
        f.to_string()
    }
}

/// Helper to convert an expression to a string for display
pub fn expr_to_string(expr: &Expr) -> String {
    expr_to_string_impl(expr, false)
}

/// Helper to convert an expression to a string, with option to quote strings for nested contexts
fn expr_to_string_impl(expr: &Expr, quote_strings: bool) -> String {
    use datafusion::scalar::ScalarValue;

    match expr {
        // Column reference - strip table qualifier for cleaner display
        Expr::Column(col) => {
            // Remove table qualifiers like "?table?." from column names
            col.name.replace("?table?.", "")
        }

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
#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConditionalValue {
    /// Value that gets scaled
    Scaled {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        expr: LogicalExprNode,
    },
    /// Literal value that bypasses scaling
    Value {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        expr: LogicalExprNode,
    },
}

impl ConditionalValue {
    /// Get the expression from this conditional value
    pub fn expr(&self, ctx: &SessionContext) -> Result<Expr, crate::error::AvengerChartError> {
        match self {
            ConditionalValue::Scaled { expr } | ConditionalValue::Value { expr } => {
                expr.to_expr(ctx)
            }
        }
    }

    /// Check if this is a field (scaled) value
    pub fn is_scaled(&self) -> bool {
        matches!(self, ConditionalValue::Scaled { .. })
    }
}

/// Represents a channel encoding value
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub enum ChannelValue {
    /// Expression that will be transformed through a scale
    Scaled {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        expr: LogicalExprNode,
        /// Optional custom scale name (defaults to channel name)
        scale_name: Option<String>,
        /// Band parameter for band scales (0.0 = start of band, 1.0 = end of band)
        band: Option<f64>,
        /// Optional scale configuration
        scale_config: Option<Scale<Auto>>,
        /// Optional legend configuration
        legend_config: Option<Legend>,
        /// Share this channel's scale across facets using ScaleSharing enum
        #[serde(default)]
        share_mode: Option<ScaleSharing>,
    },
    /// Expression that bypasses scaling (identity transformation)
    Value {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        expr: LogicalExprNode,
    },
    /// Conditional encoding with multiple branches
    Conditional {
        /// List of (condition, value) pairs
        #[serde_as(as = "Vec<(FromInto<SerializableExpr>, _)>")]
        conditions: Vec<(LogicalExprNode, ConditionalValue)>,
        /// Default value when no conditions match
        otherwise: ConditionalValue,
        /// Optional scale configuration (applies to all Field branches)
        scale_config: Option<Scale<Auto>>,
        /// Optional legend configuration (applies to all Field branches)
        legend_config: Option<Legend>,
        /// Share this channel's scale across facets using ScaleSharing enum
        #[serde(default)]
        share_mode: Option<ScaleSharing>,
    },
}

impl std::fmt::Debug for ChannelValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelValue::Scaled {
                scale_name, band, ..
            } => f
                .debug_struct("Scaled")
                .field("expr", &format!("<SerializableExpr>"))
                .field("scale_name", scale_name)
                .field("band", band)
                .field("has_scale_config", &self.has_scale_config())
                .field("has_legend_config", &self.has_legend_config())
                .finish(),
            ChannelValue::Value { expr: _ } => f
                .debug_struct("Identity")
                .field("expr", &format!("<SerializableExpr>"))
                .finish(),
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

    /// Get per-channel facet sharing mode
    pub fn get_share_mode(&self) -> Option<ScaleSharing> {
        match self {
            ChannelValue::Scaled { share_mode, .. }
            | ChannelValue::Conditional { share_mode, .. } => *share_mode,
            _ => None,
        }
    }
}

impl ChannelValue {
    /// Get the expression (for non-conditional values)
    /// For conditional values, returns None since there are multiple expressions
    pub fn expr(&self, ctx: &SessionContext) -> Option<Expr> {
        match self {
            ChannelValue::Scaled { expr, .. } => expr.to_expr(ctx).ok(),
            ChannelValue::Value { expr } => expr.to_expr(ctx).ok(),
            ChannelValue::Conditional { .. } => None,
        }
    }

    /// Get expression for domain inference (includes all branches).
    ///
    /// For conditional values, builds a CASE expression that combines all branches,
    /// allowing type inference to consider all possible output values.
    ///
    /// Note: For scale domain collection, use `scale_input_expr()` instead, which
    /// excludes literal Value branches that bypass the scale.
    pub fn expr_for_domain(&self, ctx: &SessionContext) -> Option<Expr> {
        match self {
            ChannelValue::Scaled { expr, .. } => expr.to_expr(ctx).ok(),
            ChannelValue::Value { expr } => expr.to_expr(ctx).ok(),
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                // Build CASE WHEN cond1 THEN val1 WHEN cond2 THEN val2 ... ELSE otherwise END
                let when_then_exprs: Vec<(Box<Expr>, Box<Expr>)> = conditions
                    .iter()
                    .filter_map(|(cond, val)| {
                        let cond_expr = cond.to_expr(ctx).ok()?;
                        let val_expr = val.expr(ctx).ok()?;
                        Some((Box::new(cond_expr), Box::new(val_expr)))
                    })
                    .collect();

                let else_expr = otherwise.expr(ctx).ok().map(Box::new);

                // Return None if we couldn't build any when/then pairs and no else
                if when_then_exprs.is_empty() && else_expr.is_none() {
                    return None;
                }

                Some(Expr::Case(Case::new(None, when_then_exprs, else_expr)))
            }
        }
    }

    /// Get expression for scale domain collection.
    ///
    /// Returns an expression representing only values that pass through the scale.
    /// Literal `Value` branches that bypass the scale are replaced with NULL so they
    /// don't affect domain computation (min/max, distinct values).
    ///
    /// - `Scaled` → returns the expression
    /// - `Value` → returns None (literals don't use scales)
    /// - `Conditional` → CASE expression with NULL for Value branches, expression for Scaled
    ///
    /// Returns None if there are no scaled values (pure literal conditional or Value variant).
    pub fn scale_input_expr(&self, ctx: &SessionContext) -> Option<Expr> {
        match self {
            ChannelValue::Scaled { expr, .. } => expr.to_expr(ctx).ok(),
            ChannelValue::Value { .. } => None, // Literals bypass scale, no domain needed
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                // Check if there are any Scaled branches
                let has_scaled = conditions
                    .iter()
                    .any(|(_, val)| matches!(val, ConditionalValue::Scaled { .. }))
                    || matches!(otherwise, ConditionalValue::Scaled { .. });

                if !has_scaled {
                    // All branches are literals - no scale input
                    return None;
                }

                // Build CASE with NULL for Value branches, expression for Scaled branches
                let when_then_exprs: Vec<(Box<Expr>, Box<Expr>)> = conditions
                    .iter()
                    .filter_map(|(cond, val)| {
                        let cond_expr = cond.to_expr(ctx).ok()?;
                        let val_expr = match val {
                            ConditionalValue::Scaled { expr } => expr.to_expr(ctx).ok()?,
                            ConditionalValue::Value { .. } => {
                                // Use NULL for literal values - they bypass the scale
                                lit(datafusion::scalar::ScalarValue::Null)
                            }
                        };
                        Some((Box::new(cond_expr), Box::new(val_expr)))
                    })
                    .collect();

                let else_expr = match otherwise {
                    ConditionalValue::Scaled { expr } => expr.to_expr(ctx).ok().map(Box::new),
                    ConditionalValue::Value { .. } => {
                        // Use NULL for literal otherwise value
                        Some(Box::new(lit(datafusion::scalar::ScalarValue::Null)))
                    }
                };

                Some(Expr::Case(Case::new(None, when_then_exprs, else_expr)))
            }
        }
    }

    /// Get all expressions from this channel value
    pub fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        match self {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                expr.to_expr(ctx).ok().into_iter().collect()
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                let mut exprs = Vec::new();
                for (cond, val) in conditions {
                    if let Ok(e) = cond.to_expr(ctx) {
                        exprs.push(e);
                    }
                    if let Ok(e) = val.expr(ctx) {
                        exprs.push(e);
                    }
                }
                if let Ok(e) = otherwise.expr(ctx) {
                    exprs.push(e);
                }
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
                share_mode,
                ..
            } => ChannelValue::Scaled {
                expr: expr.clone(),
                scale_name: scale_name.clone(),
                band: Some(band),
                scale_config: scale_config.clone(),
                legend_config: legend_config.clone(),
                share_mode,
            },
            other => other, // No-op for identity and conditional values
        }
    }

    /// Update the expression while preserving scale configuration
    /// This is useful when replacing expressions after aggregation
    pub fn with_expr(self, new_expr: LogicalExprNode) -> Self {
        match self {
            ChannelValue::Scaled {
                scale_name,
                band,
                scale_config,
                legend_config,
                share_mode,
                ..
            } => ChannelValue::Scaled {
                expr: new_expr,
                scale_name,
                band,
                scale_config,
                legend_config,
                share_mode,
            },
            ChannelValue::Value { .. } => ChannelValue::Value { expr: new_expr },
            ChannelValue::Conditional { .. } => {
                // For conditional, we can't easily update - just create a new scaled value
                ChannelValue::Scaled {
                    expr: new_expr,
                    scale_name: None,
                    band: None,
                    scale_config: None,
                    legend_config: None,
                    share_mode: None,
                }
            }
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
                expr: expr.clone(),
                scale_name: Some(name.into()),
                band,
                scale_config: scale_config.clone(),
                legend_config: legend_config.clone(),
                share_mode: None,
            },
            ChannelValue::Value { expr } => {
                // Convert to scaled with custom scale
                ChannelValue::Scaled {
                    expr,
                    scale_name: Some(name.into()),
                    band: None,
                    scale_config: None,
                    legend_config: None,
                    share_mode: None,
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
                share_mode: None,
            },
            ChannelValue::Value { expr } => {
                // Convert to scaled with scale config
                ChannelValue::Scaled {
                    expr,
                    scale_name: None,
                    band: None,
                    scale_config: Some(scale_changes),
                    legend_config: None,
                    share_mode: None,
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
                share_mode: None,
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
                legend_config: Some(legend),
                share_mode: None,
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
    pub fn as_column_name(&self, ctx: &SessionContext) -> Option<String> {
        self.expr(ctx).map(|e| expr_to_string(&e))
    }

    /// Get the data type of this channel value.
    /// For conditional values, uses the 'otherwise' expression for type inference.
    /// Returns an error if the expression cannot be deserialized or type cannot be determined.
    pub fn get_data_type(
        &self,
        schema: &datafusion::common::DFSchema,
        ctx: &SessionContext,
    ) -> Result<datafusion::arrow::datatypes::DataType, datafusion::error::DataFusionError> {
        use datafusion::logical_expr::ExprSchemable;

        let expr = match self {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                expr.to_expr(ctx)?
            }
            ChannelValue::Conditional { otherwise, .. } => {
                // For conditional channels, use the 'otherwise' expression for type inference
                otherwise.expr(ctx)?
            }
        };

        // Skip channel references - they need to be resolved first
        if let Expr::Column(c) = &expr {
            if c.name.starts_with(':') {
                return Err(datafusion::error::DataFusionError::Plan(format!(
                    "Cannot get data type for channel reference: {}",
                    c.name
                )));
            }
        }

        // Get the data type from the expression
        expr.get_type(schema)
    }
}

/// Remove trailing numbers from a channel name to get the base scale name
/// e.g., "x1" -> "x", "color2" -> "color", "x" -> "x"
pub(crate) fn strip_trailing_numbers(name: &str) -> &str {
    name.trim_end_matches(char::is_numeric)
}

/// Channel name with trailing numbers stripped (y2 -> y, x10 -> x).
///
/// This newtype ensures that scales are stored and retrieved using consistent
/// base names, preventing runtime errors from mismatched lookups (e.g., looking
/// up "y2" when scale is stored under "y").
///
/// # Example
/// ```
/// use avenger_chart::channel::value::BaseChannelName;
///
/// let base = BaseChannelName::from_raw("y2");
/// assert_eq!(base.as_str(), "y");
///
/// let base = BaseChannelName::from_raw("color");
/// assert_eq!(base.as_str(), "color");
/// ```
#[derive(Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BaseChannelName(String);

impl BaseChannelName {
    /// Create a BaseChannelName by stripping trailing numbers from the raw name.
    pub fn from_raw(name: &str) -> Self {
        Self(strip_trailing_numbers(name).to_string())
    }

    /// Return the base channel name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BaseChannelName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Debug for BaseChannelName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BaseChannelName({})", self.0)
    }
}

impl From<&str> for BaseChannelName {
    fn from(name: &str) -> Self {
        Self::from_raw(name)
    }
}

impl From<String> for BaseChannelName {
    fn from(name: String) -> Self {
        Self::from_raw(&name)
    }
}

impl AsRef<str> for BaseChannelName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// Smart conversion for &str - always literals, identity by default
impl From<&str> for ChannelValue {
    fn from(s: &str) -> Self {
        // Always treat strings as literals - identity by default
        ChannelValue::Value {
            expr: LogicalExprNode::from_expr(lit(s)).expect("Failed to serialize expr"),
        }
    }
}

// Expressions default to scaled
impl From<Expr> for ChannelValue {
    fn from(expr: Expr) -> Self {
        ChannelValue::Scaled {
            expr: LogicalExprNode::from_expr(expr).expect("Failed to serialize expression"),
            scale_name: None,
            band: None,
            scale_config: None,
            legend_config: None,
            share_mode: None,
        }
    }
}

// Numeric literals default to identity
impl From<f64> for ChannelValue {
    fn from(v: f64) -> Self {
        ChannelValue::Value {
            expr: LogicalExprNode::from_expr(lit(v)).expect("Failed to serialize expr"),
        }
    }
}

impl From<f32> for ChannelValue {
    fn from(v: f32) -> Self {
        ChannelValue::Value {
            expr: LogicalExprNode::from_expr(lit(v)).expect("Failed to serialize expr"),
        }
    }
}

impl From<i32> for ChannelValue {
    fn from(v: i32) -> Self {
        ChannelValue::Value {
            expr: LogicalExprNode::from_expr(lit(v)).expect("Failed to serialize expr"),
        }
    }
}

impl From<i64> for ChannelValue {
    fn from(v: i64) -> Self {
        ChannelValue::Value {
            expr: LogicalExprNode::from_expr(lit(v)).expect("Failed to serialize expr"),
        }
    }
}

impl From<bool> for ChannelValue {
    fn from(v: bool) -> Self {
        ChannelValue::Value {
            expr: LogicalExprNode::from_expr(lit(v)).expect("Failed to serialize expr"),
        }
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
    fn test_base_channel_name_from_raw() {
        // Basic stripping
        assert_eq!(BaseChannelName::from_raw("y").as_str(), "y");
        assert_eq!(BaseChannelName::from_raw("y2").as_str(), "y");
        assert_eq!(BaseChannelName::from_raw("y10").as_str(), "y");

        // Multiple characters
        assert_eq!(BaseChannelName::from_raw("color").as_str(), "color");
        assert_eq!(BaseChannelName::from_raw("color2").as_str(), "color");

        // Edge cases
        assert_eq!(BaseChannelName::from_raw("x123").as_str(), "x");
    }

    #[test]
    fn test_base_channel_name_equality() {
        let y1 = BaseChannelName::from_raw("y");
        let y2 = BaseChannelName::from_raw("y2");
        let y10 = BaseChannelName::from_raw("y10");

        // All variants of y should be equal
        assert_eq!(y1, y2);
        assert_eq!(y2, y10);
        assert_eq!(y1, y10);

        // Different base names should not be equal
        let x = BaseChannelName::from_raw("x");
        assert_ne!(y1, x);
    }

    #[test]
    fn test_base_channel_name_hash() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        set.insert(BaseChannelName::from_raw("y"));
        set.insert(BaseChannelName::from_raw("y2"));
        set.insert(BaseChannelName::from_raw("y10"));

        // All should resolve to the same base name, so set should have 1 element
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_base_channel_name_display() {
        let base = BaseChannelName::from_raw("y2");
        assert_eq!(format!("{}", base), "y");
    }

    #[test]
    fn test_base_channel_name_debug() {
        let base = BaseChannelName::from_raw("y2");
        assert_eq!(format!("{:?}", base), "BaseChannelName(y)");
    }

    #[test]
    fn test_base_channel_name_from_traits() {
        // From &str
        let base: BaseChannelName = "y2".into();
        assert_eq!(base.as_str(), "y");

        // From String
        let base: BaseChannelName = "color2".to_string().into();
        assert_eq!(base.as_str(), "color");
    }

    #[test]
    fn test_base_channel_name_serialization_roundtrip() {
        // Test that serialization and deserialization work correctly
        let original = BaseChannelName::from_raw("y2");

        // Serialize to JSON
        let json = serde_json::to_string(&original).expect("serialize");
        assert_eq!(json, "\"y\""); // Should serialize as just the base name

        // Deserialize from JSON
        let deserialized: BaseChannelName = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, deserialized);
        assert_eq!(deserialized.as_str(), "y");
    }

    #[test]
    fn test_base_channel_name_hashmap_lookup() {
        use std::collections::HashMap;

        // This test validates the primary use case: storing scales by base name
        // and looking them up with either the base or numbered variant
        let mut scales: HashMap<BaseChannelName, &str> = HashMap::new();

        // Store scale under "y"
        scales.insert(BaseChannelName::from_raw("y"), "y_scale");

        // Lookup should work with y, y2, y10
        assert_eq!(
            scales.get(&BaseChannelName::from_raw("y")),
            Some(&"y_scale")
        );
        assert_eq!(
            scales.get(&BaseChannelName::from_raw("y2")),
            Some(&"y_scale")
        );
        assert_eq!(
            scales.get(&BaseChannelName::from_raw("y10")),
            Some(&"y_scale")
        );

        // x should not find anything
        assert_eq!(scales.get(&BaseChannelName::from_raw("x")), None);
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
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();
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
            // Convert SerializableExpr back to Expr to check if it's a literal
            let datafusion_expr = expr.to_expr(&ctx).unwrap();
            assert!(matches!(
                datafusion_expr,
                datafusion::logical_expr::Expr::Literal(..),
            ));
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
        use datafusion::prelude::SessionContext;

        let ctx = SessionContext::new();

        // Test column reference
        let cv: ChannelValue = col("my_column").into();
        assert_eq!(cv.as_column_name(&ctx), Some("my_column".to_string()));

        // Test integer literals
        let cv: ChannelValue = 42.into();
        assert_eq!(cv.as_column_name(&ctx), Some("42".to_string()));

        let cv: ChannelValue = (-100i32).into();
        assert_eq!(cv.as_column_name(&ctx), Some("-100".to_string()));

        // Test float literals
        let cv: ChannelValue = 3.5.into();
        assert_eq!(cv.as_column_name(&ctx), Some("3.5".to_string()));

        let cv: ChannelValue = 5.0.into();
        assert_eq!(cv.as_column_name(&ctx), Some("5".to_string())); // Should format as integer

        let cv: ChannelValue = 1000.0.into();
        assert_eq!(cv.as_column_name(&ctx), Some("1000".to_string()));

        // Test string literals
        let cv: ChannelValue = "hello".into();
        assert_eq!(cv.as_column_name(&ctx), Some("hello".to_string()));

        let cv: ChannelValue = "#ff0000".into();
        assert_eq!(cv.as_column_name(&ctx), Some("#ff0000".to_string()));

        // Test boolean literals
        let cv: ChannelValue = true.into();
        assert_eq!(cv.as_column_name(&ctx), Some("true".to_string()));

        let cv: ChannelValue = false.into();
        assert_eq!(cv.as_column_name(&ctx), Some("false".to_string()));

        // Test null literal
        let cv: ChannelValue = ChannelValue::Value {
            expr: LogicalExprNode::from_expr(lit(datafusion::scalar::ScalarValue::Null))
                .expect("Failed to serialize expr"),
        };
        assert_eq!(cv.as_column_name(&ctx), Some("null".to_string()));

        // Test function call with arguments
        let cv: ChannelValue = sqrt(col("x")).into();
        assert_eq!(cv.as_column_name(&ctx), Some("sqrt(x)".to_string()));

        // Test function with literal argument
        let cv: ChannelValue = sqrt(lit(16.0)).into();
        assert_eq!(cv.as_column_name(&ctx), Some("sqrt(16)".to_string()));

        // Test nested function calls
        use datafusion::functions::expr_fn::abs;
        let cv: ChannelValue = sqrt(abs(col("x"))).into();
        assert_eq!(cv.as_column_name(&ctx), Some("sqrt(abs(x))".to_string()));

        // Test function with multiple arguments (using pow as example)
        use datafusion::functions::expr_fn::power;
        let cv: ChannelValue = power(col("x"), lit(2)).into();
        assert_eq!(cv.as_column_name(&ctx), Some("power(x, 2)".to_string()));

        // Test function with string arguments (should be quoted in function context)
        use datafusion::functions::expr_fn::concat;
        let cv: ChannelValue = concat(vec![lit("hello"), lit("world")]).into();
        assert_eq!(
            cv.as_column_name(&ctx),
            Some("concat('hello', 'world')".to_string())
        );

        // Test complex expression (now returns the expression string)
        let cv: ChannelValue = (col("x") + col("y")).into();
        assert_eq!(cv.as_column_name(&ctx), Some("x + y".to_string()));

        // Test conditional value (should return None - no single name)
        let cv = ChannelValue::Conditional {
            conditions: vec![(
                LogicalExprNode::from_expr(col("category").eq(lit("A")))
                    .expect("Failed to serialize expr"),
                ConditionalValue::Value {
                    expr: LogicalExprNode::from_expr(lit("red")).expect("Failed to serialize expr"),
                },
            )],
            otherwise: ConditionalValue::Scaled {
                expr: LogicalExprNode::from_expr(col("color")).expect("Failed to serialize expr"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
        };
        // Conditional values don't have a single column name
        assert_eq!(cv.as_column_name(&ctx), None);
    }

    #[test]
    fn test_expr_for_domain_scaled() {
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Scaled variant should return same as expr()
        let cv: ChannelValue = col("x").into();
        let expr = cv.expr(&ctx);
        let domain_expr = cv.expr_for_domain(&ctx);
        assert!(expr.is_some());
        assert!(domain_expr.is_some());
        assert_eq!(expr.unwrap().to_string(), domain_expr.unwrap().to_string());
    }

    #[test]
    fn test_expr_for_domain_value() {
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Value variant should return same as expr()
        let cv: ChannelValue = 42.into();
        let expr = cv.expr(&ctx);
        let domain_expr = cv.expr_for_domain(&ctx);
        assert!(expr.is_some());
        assert!(domain_expr.is_some());
        assert_eq!(expr.unwrap().to_string(), domain_expr.unwrap().to_string());
    }

    #[test]
    fn test_expr_for_domain_conditional() {
        use super::ConditionalValue;
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Build a conditional: if category == "A" then "red" else "blue"
        let cv = ChannelValue::Conditional {
            conditions: vec![(
                LogicalExprNode::from_expr(col("category").eq(lit("A")))
                    .expect("Failed to serialize condition"),
                ConditionalValue::Value {
                    expr: LogicalExprNode::from_expr(lit("red"))
                        .expect("Failed to serialize value"),
                },
            )],
            otherwise: ConditionalValue::Value {
                expr: LogicalExprNode::from_expr(lit("blue"))
                    .expect("Failed to serialize otherwise"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
        };

        // expr() returns None for conditional
        assert!(cv.expr(&ctx).is_none());

        // expr_for_domain() returns a CASE expression
        let domain_expr = cv.expr_for_domain(&ctx);
        assert!(domain_expr.is_some());
        let expr_str = domain_expr.unwrap().to_string();
        // CASE expression should contain WHEN, THEN, ELSE
        assert!(
            expr_str.contains("WHEN"),
            "Expected CASE expression, got: {}",
            expr_str
        );
        assert!(
            expr_str.contains("THEN"),
            "Expected CASE expression, got: {}",
            expr_str
        );
        assert!(
            expr_str.contains("ELSE"),
            "Expected CASE expression, got: {}",
            expr_str
        );
    }

    #[test]
    fn test_expr_for_domain_conditional_multiple_branches() {
        use super::ConditionalValue;
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Build a conditional with multiple branches:
        // if category == "A" then "red" elif category == "B" then "green" else "blue"
        let cv = ChannelValue::Conditional {
            conditions: vec![
                (
                    LogicalExprNode::from_expr(col("category").eq(lit("A")))
                        .expect("Failed to serialize condition"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("red"))
                            .expect("Failed to serialize value"),
                    },
                ),
                (
                    LogicalExprNode::from_expr(col("category").eq(lit("B")))
                        .expect("Failed to serialize condition"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("green"))
                            .expect("Failed to serialize value"),
                    },
                ),
            ],
            otherwise: ConditionalValue::Value {
                expr: LogicalExprNode::from_expr(lit("blue"))
                    .expect("Failed to serialize otherwise"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
        };

        let domain_expr = cv.expr_for_domain(&ctx);
        assert!(domain_expr.is_some());
        let expr_str = domain_expr.unwrap().to_string();
        // Should have multiple WHEN clauses
        let when_count = expr_str.matches("WHEN").count();
        assert_eq!(
            when_count, 2,
            "Expected 2 WHEN clauses, got {}: {}",
            when_count, expr_str
        );
    }

    #[test]
    fn test_scale_input_expr_scaled() {
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Scaled variant should return same as expr()
        let cv: ChannelValue = col("x").into();
        let expr = cv.expr(&ctx);
        let scale_expr = cv.scale_input_expr(&ctx);
        assert!(expr.is_some());
        assert!(scale_expr.is_some());
        assert_eq!(expr.unwrap().to_string(), scale_expr.unwrap().to_string());
    }

    #[test]
    fn test_scale_input_expr_value() {
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Value variant should return None (literals bypass scale)
        let cv: ChannelValue = 42.into();
        let scale_expr = cv.scale_input_expr(&ctx);
        assert!(scale_expr.is_none());
    }

    #[test]
    fn test_scale_input_expr_conditional_mixed() {
        use super::ConditionalValue;
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Build a conditional with mixed branches:
        // if highlight then "red" (literal) else value (scaled)
        let cv = ChannelValue::Conditional {
            conditions: vec![(
                LogicalExprNode::from_expr(col("highlight"))
                    .expect("Failed to serialize condition"),
                ConditionalValue::Value {
                    expr: LogicalExprNode::from_expr(lit("red"))
                        .expect("Failed to serialize value"),
                },
            )],
            otherwise: ConditionalValue::Scaled {
                expr: LogicalExprNode::from_expr(col("value"))
                    .expect("Failed to serialize otherwise"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
        };

        // scale_input_expr() should return CASE with NULL for literal branches
        let scale_expr = cv.scale_input_expr(&ctx);
        assert!(scale_expr.is_some());
        let expr_str = scale_expr.unwrap().to_string();
        // Should be a CASE expression
        assert!(
            expr_str.contains("WHEN"),
            "Expected CASE expression, got: {}",
            expr_str
        );
        // Should contain NULL for the literal branch
        assert!(
            expr_str.contains("NULL"),
            "Expected NULL for literal branch, got: {}",
            expr_str
        );
        // Should contain the column reference for scaled branch
        assert!(
            expr_str.contains("value"),
            "Expected 'value' column in expression, got: {}",
            expr_str
        );
    }

    #[test]
    fn test_scale_input_expr_conditional_all_literals() {
        use super::ConditionalValue;
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Build a conditional with all literal branches:
        // if category == "A" then "red" else "blue"
        let cv = ChannelValue::Conditional {
            conditions: vec![(
                LogicalExprNode::from_expr(col("category").eq(lit("A")))
                    .expect("Failed to serialize condition"),
                ConditionalValue::Value {
                    expr: LogicalExprNode::from_expr(lit("red"))
                        .expect("Failed to serialize value"),
                },
            )],
            otherwise: ConditionalValue::Value {
                expr: LogicalExprNode::from_expr(lit("blue"))
                    .expect("Failed to serialize otherwise"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
        };

        // scale_input_expr() should return None when all branches are literals
        let scale_expr = cv.scale_input_expr(&ctx);
        assert!(
            scale_expr.is_none(),
            "Expected None for all-literal conditional"
        );
    }

    #[test]
    fn test_scale_input_expr_conditional_all_scaled() {
        use super::ConditionalValue;
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();

        // Build a conditional with all scaled branches:
        // if is_large then big_value else small_value
        let cv = ChannelValue::Conditional {
            conditions: vec![(
                LogicalExprNode::from_expr(col("is_large")).expect("Failed to serialize condition"),
                ConditionalValue::Scaled {
                    expr: LogicalExprNode::from_expr(col("big_value"))
                        .expect("Failed to serialize value"),
                },
            )],
            otherwise: ConditionalValue::Scaled {
                expr: LogicalExprNode::from_expr(col("small_value"))
                    .expect("Failed to serialize otherwise"),
            },
            scale_config: None,
            legend_config: None,
            share_mode: None,
        };

        // scale_input_expr() should return CASE without NULL (all branches use scale)
        let scale_expr = cv.scale_input_expr(&ctx);
        assert!(scale_expr.is_some());
        let expr_str = scale_expr.unwrap().to_string();
        // Should be a CASE expression
        assert!(
            expr_str.contains("WHEN"),
            "Expected CASE expression, got: {}",
            expr_str
        );
        // Should NOT contain NULL since all branches are scaled
        assert!(
            !expr_str.contains("NULL"),
            "Did not expect NULL in all-scaled conditional, got: {}",
            expr_str
        );
        // Should contain both column references
        assert!(
            expr_str.contains("big_value"),
            "Expected 'big_value' column, got: {}",
            expr_str
        );
        assert!(
            expr_str.contains("small_value"),
            "Expected 'small_value' column, got: {}",
            expr_str
        );
    }
}
