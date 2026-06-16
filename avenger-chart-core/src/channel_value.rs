use datafusion::{
    common::ScalarValue,
    logical_expr::{Case, Expr, lit},
    prelude::SessionContext,
};
use datafusion_proto::{
    logical_plan::{
        DefaultLogicalExtensionCodec, from_proto::parse_expr, to_proto::serialize_expr,
    },
    protobuf::LogicalExprNode,
};
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, Axis, CoordinationScope, DomainCoordination, DomainCoordinationGroup,
    Legend, NestedBandSpec, PositionBoundary, ScaleConfigSpec, SerializableExpr,
    channel::strip_trailing_numbers,
};

trait LogicalExprNodeExt: Sized {
    fn from_expr(expr: Expr) -> Result<Self, AvengerChartError>;
    fn to_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError>;
}

impl LogicalExprNodeExt for LogicalExprNode {
    fn from_expr(expr: Expr) -> Result<Self, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        serialize_expr(&expr, &codec)
            .map_err(|err| AvengerChartError::SerializationError(err.to_string()))
    }

    fn to_expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
        let codec = DefaultLogicalExtensionCodec {};
        parse_expr(self, ctx, &codec)
            .map_err(|err| AvengerChartError::DeserializationError(err.to_string()))
    }
}

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
    pub fn expr(&self, ctx: &SessionContext) -> Result<Expr, AvengerChartError> {
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
        /// Boundary request for banded position scales.
        position_boundary: Option<PositionBoundary>,
        /// Optional scale configuration
        scale_config: Option<Box<ScaleConfigSpec>>,
        /// Optional nested-band level configuration.
        #[serde(default)]
        nested_band_config: Option<Box<NestedBandSpec>>,
        /// Optional legend configuration
        legend_config: Option<Box<Legend>>,
        /// Optional axis configuration applied when this value is used on a position channel
        #[serde(default)]
        axis_config: Option<Box<dyn Axis>>,
        /// Optional scale-domain coordination metadata.
        #[serde(default)]
        domain_coordination: Option<DomainCoordination>,
        /// Scope of the transform stage that produced this value, if any.
        #[serde(default)]
        transform_scope: Option<CoordinationScope>,
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
        scale_config: Option<Box<ScaleConfigSpec>>,
        /// Optional nested-band level configuration.
        #[serde(default)]
        nested_band_config: Option<Box<NestedBandSpec>>,
        /// Optional legend configuration (applies to all Field branches)
        legend_config: Option<Box<Legend>>,
        /// Optional axis configuration applied when this value is used on a position channel
        #[serde(default)]
        axis_config: Option<Box<dyn Axis>>,
        /// Optional scale-domain coordination metadata.
        #[serde(default)]
        domain_coordination: Option<DomainCoordination>,
        /// Scope of the transform stage that produced this value, if any.
        #[serde(default)]
        transform_scope: Option<CoordinationScope>,
    },
}

/// Authoring-time handle for a data expression with default channel encoding metadata.
///
/// This is useful for transform output handles. When used as a transform input,
/// only the data expression is consumed. When used as a mark encoding, the full
/// `ChannelValue` is consumed, preserving scale, axis, legend, and sharing
/// defaults carried by the transform output.
#[derive(Clone, Debug)]
pub struct ChannelExpr {
    expr: Expr,
    channel_value: ChannelValue,
}

impl ChannelExpr {
    /// Create a handle from a data expression and its corresponding channel value.
    pub fn new(expr: Expr, channel_value: ChannelValue) -> Self {
        Self {
            expr,
            channel_value,
        }
    }

    /// Create a scaled channel expression from a DataFusion expression.
    pub fn scaled(expr: Expr) -> Self {
        let channel_value = ChannelValue::from(expr.clone());
        Self {
            expr,
            channel_value,
        }
    }

    /// Create an unscaled channel expression from a DataFusion expression.
    pub fn value(expr: Expr) -> Self {
        let channel_value = ChannelValue::from(expr.clone()).no_scale();
        Self {
            expr,
            channel_value,
        }
    }

    /// Borrow the data expression represented by this handle.
    pub fn data_expr(&self) -> &Expr {
        &self.expr
    }

    /// Consume this handle and return only the data expression.
    pub fn into_data_expr(self) -> Expr {
        self.expr
    }

    /// Borrow the channel value represented by this handle.
    pub fn channel_value(&self) -> &ChannelValue {
        &self.channel_value
    }

    /// Check if the underlying channel value has scale configuration.
    pub fn has_scale_config(&self) -> bool {
        self.channel_value.has_scale_config()
    }

    /// Check if the underlying channel value has legend configuration.
    pub fn has_legend_config(&self) -> bool {
        self.channel_value.has_legend_config()
    }

    /// Check if the underlying channel value has axis configuration defaults.
    pub fn has_axis_config(&self) -> bool {
        self.channel_value.has_axis_config()
    }

    /// Get the underlying channel value's axis configuration.
    pub fn get_axis_config(&self) -> Option<&dyn Axis> {
        self.channel_value.get_axis_config()
    }

    /// Get the underlying channel value's scale configuration.
    pub fn get_scale_config(&self) -> Option<&ScaleConfigSpec> {
        self.channel_value.get_scale_config()
    }

    /// Get the underlying channel value's legend configuration.
    pub fn get_legend_config(&self) -> Option<&Legend> {
        self.channel_value.get_legend_config()
    }

    /// Get the underlying channel value's explicit domain coordination scope.
    pub fn get_domain_scope(&self) -> Option<CoordinationScope> {
        self.channel_value.get_domain_scope()
    }

    /// Get the underlying channel value's explicit domain coordination target.
    pub fn get_domain_coordination(&self) -> Option<&DomainCoordination> {
        self.channel_value.get_domain_coordination()
    }

    /// Get the transform scope that produced this channel expression, if any.
    pub fn get_transform_scope(&self) -> Option<CoordinationScope> {
        self.channel_value.get_transform_scope()
    }

    /// Consume this handle and return the full channel value.
    pub fn into_channel_value(self) -> ChannelValue {
        self.channel_value
    }

    /// Update the channel metadata while preserving the data expression.
    pub fn map_channel_value(self, f: impl FnOnce(ChannelValue) -> ChannelValue) -> Self {
        Self {
            expr: self.expr,
            channel_value: f(self.channel_value),
        }
    }

    /// Disable scaling for this channel value while preserving the data expression.
    pub fn no_scale(self) -> Self {
        self.map_channel_value(ChannelValue::no_scale)
    }

    /// Set the band parameter for this channel value.
    pub fn band(self, band: f64) -> Self {
        self.map_channel_value(|value| value.band(band))
    }

    /// Set the boundary for a specific nested-band level.
    pub fn level_band(self, level: usize, band: f64) -> Self {
        self.map_channel_value(|value| value.level_band(level, band))
    }

    /// Set a custom scale name for this channel value.
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        self.map_channel_value(|value| value.with_scale_name(name))
    }

    /// Attach transform-scope metadata and use it as default scale sharing.
    pub fn with_transform_scope(self, scope: CoordinationScope) -> Self {
        self.map_channel_value(|value| value.with_transform_scope(scope))
    }

    /// Set the domain coordination owner scope for this channel value.
    pub fn with_domain_scope(self, scope: CoordinationScope) -> Self {
        self.map_channel_value(|value| value.with_domain_scope(scope))
    }

    /// Set the named domain coordination group for this channel value.
    pub fn with_domain_group(self, group: impl Into<String>) -> Self {
        self.map_channel_value(|value| value.with_domain_group(group))
    }

    /// Set the full domain coordination target for this channel value.
    pub fn with_domain_coordination(self, coordination: DomainCoordination) -> Self {
        self.map_channel_value(|value| value.with_domain_coordination(coordination))
    }

    /// Attach default axis configuration to this channel value.
    pub fn with_axis_config<A: Axis + 'static>(self, axis_config: A) -> Self {
        self.map_channel_value(|value| value.with_axis_config(axis_config))
    }

    /// Attach boxed default axis configuration to this channel value.
    pub fn with_boxed_axis_config(self, axis_config: Box<dyn Axis>) -> Self {
        self.map_channel_value(|value| value.with_boxed_axis_config(axis_config))
    }
}

impl From<Expr> for ChannelExpr {
    fn from(expr: Expr) -> Self {
        ChannelExpr::scaled(expr)
    }
}

impl From<ChannelExpr> for ChannelValue {
    fn from(value: ChannelExpr) -> Self {
        value.into_channel_value()
    }
}

impl From<&ChannelExpr> for ChannelValue {
    fn from(value: &ChannelExpr) -> Self {
        value.channel_value.clone()
    }
}

impl From<ChannelExpr> for Expr {
    fn from(value: ChannelExpr) -> Self {
        value.into_data_expr()
    }
}

impl From<&ChannelExpr> for Expr {
    fn from(value: &ChannelExpr) -> Self {
        value.expr.clone()
    }
}

impl crate::IntoExpr for ChannelExpr {
    fn into_expr(self) -> Expr {
        self.into_data_expr()
    }
}

impl crate::IntoExpr for &ChannelExpr {
    fn into_expr(self) -> Expr {
        self.data_expr().clone()
    }
}

impl std::fmt::Debug for ChannelValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelValue::Scaled {
                scale_name,
                position_boundary,
                transform_scope,
                ..
            } => f
                .debug_struct("Scaled")
                .field("expr", &"<SerializableExpr>".to_string())
                .field("scale_name", scale_name)
                .field("position_boundary", position_boundary)
                .field("has_scale_config", &self.has_scale_config())
                .field("has_legend_config", &self.has_legend_config())
                .field("has_axis_config", &self.has_axis_config())
                .field("transform_scope", transform_scope)
                .finish(),
            ChannelValue::Value { expr: _ } => f
                .debug_struct("Identity")
                .field("expr", &"<SerializableExpr>".to_string())
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
                .field("has_axis_config", &self.has_axis_config())
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
            ChannelValue::Value { .. } => false,
        }
    }

    /// Check if this channel has axis configuration defaults
    pub fn has_axis_config(&self) -> bool {
        self.get_axis_config().is_some()
    }

    /// Get the axis configuration if present
    pub fn get_axis_config(&self) -> Option<&dyn Axis> {
        match self {
            ChannelValue::Scaled { axis_config, .. }
            | ChannelValue::Conditional { axis_config, .. } => axis_config.as_deref(),
            ChannelValue::Value { .. } => None,
        }
    }

    /// Get the scale configuration if present
    pub fn get_scale_config(&self) -> Option<&ScaleConfigSpec> {
        match self {
            ChannelValue::Scaled { scale_config, .. }
            | ChannelValue::Conditional { scale_config, .. } => scale_config.as_deref(),
            _ => None,
        }
    }

    /// Get the legend configuration if present
    pub fn get_legend_config(&self) -> Option<&Legend> {
        match self {
            ChannelValue::Scaled { legend_config, .. }
            | ChannelValue::Conditional { legend_config, .. } => legend_config.as_deref(),
            ChannelValue::Value { .. } => None,
        }
    }

    /// Get the explicit per-channel domain coordination target, if one was set.
    pub fn get_domain_coordination(&self) -> Option<&DomainCoordination> {
        match self {
            ChannelValue::Scaled {
                domain_coordination,
                ..
            }
            | ChannelValue::Conditional {
                domain_coordination,
                ..
            } => domain_coordination.as_ref(),
            ChannelValue::Value { .. } => None,
        }
    }

    /// Get per-channel domain coordination scope.
    pub fn get_domain_scope(&self) -> Option<CoordinationScope> {
        self.get_domain_coordination()
            .map(|coordination| coordination.scope)
    }

    /// Get the transform scope that produced this channel value, if any.
    pub fn get_transform_scope(&self) -> Option<CoordinationScope> {
        match self {
            ChannelValue::Scaled {
                transform_scope, ..
            }
            | ChannelValue::Conditional {
                transform_scope, ..
            } => *transform_scope,
            ChannelValue::Value { .. } => None,
        }
    }

    /// Attach transform-scope metadata and use it as default scale sharing.
    pub fn with_transform_scope(self, scope: CoordinationScope) -> Self {
        let scope = scope.to_normalized();
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination: domain_coordination
                    .or_else(|| Some(DomainCoordination::scale_name(scope))),
                transform_scope: Some(scope),
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination: domain_coordination
                    .or_else(|| Some(DomainCoordination::scale_name(scope))),
                transform_scope: Some(scope),
            },
            ChannelValue::Value { expr } => ChannelValue::Value { expr },
        }
    }

    /// Set the domain coordination owner scope, preserving any explicit group.
    pub fn with_domain_scope(self, scope: CoordinationScope) -> Self {
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination: Some(
                    domain_coordination.unwrap_or_default().with_scope(scope),
                ),
                transform_scope,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination: Some(
                    domain_coordination.unwrap_or_default().with_scope(scope),
                ),
                transform_scope,
            },
            ChannelValue::Value { expr } => ChannelValue::Value { expr },
        }
    }

    /// Set an explicit named domain coordination group, preserving any explicit scope.
    pub fn with_domain_group(self, group: impl Into<String>) -> Self {
        let group = DomainCoordinationGroup::Named(group.into());
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination: Some(
                    domain_coordination.unwrap_or_default().with_group(group),
                ),
                transform_scope,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination: Some(
                    domain_coordination.unwrap_or_default().with_group(group),
                ),
                transform_scope,
            },
            ChannelValue::Value { expr } => ChannelValue::Value { expr },
        }
    }

    /// Set the full domain coordination target.
    pub fn with_domain_coordination(self, coordination: DomainCoordination) -> Self {
        let scope = coordination.scope;
        let coordination = coordination.with_scope(scope);
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                transform_scope,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination: Some(coordination),
                transform_scope,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                transform_scope,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination: Some(coordination),
                transform_scope,
            },
            ChannelValue::Value { expr } => ChannelValue::Value { expr },
        }
    }

    /// Attach default axis configuration to this channel value.
    ///
    /// The defaults apply only when this value is used on a coordinate position
    /// channel whose guide supports the axis type. Explicit mark-level position
    /// configuration, such as `.x_with(..., |c| c.axis(...))`, is merged on top.
    pub fn with_axis_config<A: Axis + 'static>(self, axis_config: A) -> Self {
        self.with_boxed_axis_config(Box::new(axis_config))
    }

    /// Attach a boxed default axis configuration to this channel value.
    pub fn with_boxed_axis_config(self, axis_config: Box<dyn Axis>) -> Self {
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                domain_coordination,
                transform_scope,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config: Some(axis_config),
                domain_coordination,
                transform_scope,
            },
            ChannelValue::Value { expr } => ChannelValue::Value { expr },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                domain_coordination,
                transform_scope,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config: Some(axis_config),
                domain_coordination,
                transform_scope,
            },
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
                                lit(ScalarValue::Null)
                            }
                        };
                        Some((Box::new(cond_expr), Box::new(val_expr)))
                    })
                    .collect();

                let else_expr = match otherwise {
                    ConditionalValue::Scaled { expr } => expr.to_expr(ctx).ok().map(Box::new),
                    ConditionalValue::Value { .. } => {
                        // Use NULL for literal otherwise value
                        Some(Box::new(lit(ScalarValue::Null)))
                    }
                };

                Some(Expr::Case(Case::new(None, when_then_exprs, else_expr)))
            }
        }
    }

    /// Get all expressions from this channel value
    pub fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        match self {
            ChannelValue::Scaled {
                expr,
                nested_band_config,
                ..
            } => {
                let mut exprs = expr.to_expr(ctx).ok().into_iter().collect::<Vec<_>>();
                if let Some(config) = nested_band_config {
                    exprs.extend(config.all_exprs(ctx));
                }
                exprs
            }
            ChannelValue::Value { expr } => expr.to_expr(ctx).ok().into_iter().collect(),
            ChannelValue::Conditional {
                conditions,
                otherwise,
                nested_band_config,
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
                if let Some(config) = nested_band_config {
                    exprs.extend(config.all_exprs(ctx));
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
        self.with_position_boundary(PositionBoundary::band(band))
    }

    /// Set the boundary for a specific nested-band level.
    pub fn level_band(self, level: usize, band: f64) -> Self {
        self.with_position_boundary(PositionBoundary::level_band(level, band))
    }

    /// Set the position boundary for this channel.
    pub fn with_position_boundary(self, position_boundary: PositionBoundary) -> Self {
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary: Some(position_boundary),
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            },
            other => other,
        }
    }

    /// Get the position boundary configured for this channel.
    pub fn get_position_boundary(&self) -> Option<PositionBoundary> {
        match self {
            ChannelValue::Scaled {
                position_boundary, ..
            } => *position_boundary,
            ChannelValue::Conditional { .. } | ChannelValue::Value { .. } => None,
        }
    }

    /// Get the nested-band configuration if present.
    pub fn get_nested_band_config(&self) -> Option<&NestedBandSpec> {
        match self {
            ChannelValue::Scaled {
                nested_band_config, ..
            }
            | ChannelValue::Conditional {
                nested_band_config, ..
            } => nested_band_config.as_deref(),
            ChannelValue::Value { .. } => None,
        }
    }

    /// Replace the nested-band configuration on this scaled channel.
    pub fn with_nested_band_config(self, nested_band_config: NestedBandSpec) -> Self {
        match self {
            ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config: Some(Box::new(nested_band_config)),
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                nested_band_config: Some(Box::new(nested_band_config)),
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            },
            other => other,
        }
    }

    /// Update the expression while preserving scale configuration
    /// This is useful when replacing expressions after aggregation
    pub fn with_expr(self, new_expr: LogicalExprNode) -> Self {
        match self {
            ChannelValue::Scaled {
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                ..
            } => ChannelValue::Scaled {
                expr: new_expr,
                scale_name,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            },
            ChannelValue::Value { .. } => ChannelValue::Value { expr: new_expr },
            ChannelValue::Conditional { .. } => {
                // For conditional, we can't easily update - just create a new scaled value
                ChannelValue::Scaled {
                    expr: new_expr,
                    scale_name: None,
                    position_boundary: None,
                    scale_config: None,
                    nested_band_config: None,
                    legend_config: None,
                    axis_config: None,
                    domain_coordination: None,
                    transform_scope: None,
                }
            }
        }
    }

    /// Set a custom scale name (only for scaled values)
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        match self {
            ChannelValue::Scaled {
                expr,
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name: Some(name.into()),
                position_boundary,
                scale_config,
                nested_band_config,
                legend_config,
                axis_config,
                domain_coordination,
                transform_scope,
            },
            ChannelValue::Value { expr } => {
                // Convert to scaled with custom scale
                ChannelValue::Scaled {
                    expr,
                    scale_name: Some(name.into()),
                    position_boundary: None,
                    scale_config: None,
                    nested_band_config: None,
                    legend_config: None,
                    axis_config: None,
                    domain_coordination: None,
                    transform_scope: None,
                }
            }
            ChannelValue::Conditional { .. } => {
                // Conditional values already have implicit scale names
                self
            }
        }
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
        if let Expr::Column(c) = &expr
            && c.name.starts_with(':')
        {
            return Err(datafusion::error::DataFusionError::Plan(format!(
                "Cannot get data type for channel reference: {}",
                c.name
            )));
        }

        // Get the data type from the expression
        expr.get_type(schema)
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
            position_boundary: None,
            scale_config: None,
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
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
    use crate::{Maybe, NestScope, ScaleOrderingSpec, channel::BaseChannelName};
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
                position_boundary: Some(PositionBoundary::Band { band: 0.5 }),
                ..
            }
        ));

        // Test band on ChannelValue
        let cv1: ChannelValue = col("x").into();
        let cv1 = cv1.band(1.0);
        let cv2: ChannelValue = col("x").into();
        let cv2 = cv2.band(1.0);
        match (cv1, cv2) {
            (
                ChannelValue::Scaled {
                    position_boundary: b1,
                    ..
                },
                ChannelValue::Scaled {
                    position_boundary: b2,
                    ..
                },
            ) => {
                assert_eq!(b1, b2);
            }
            _ => panic!("Expected both to be scaled"),
        }
    }

    #[test]
    fn nested_band_channel_value_level_band_stores_position_boundary() {
        let cv: ChannelValue = col("x").into();
        let cv = cv.level_band(1, 0.75);
        assert_eq!(
            cv.get_position_boundary(),
            Some(PositionBoundary::LevelBand {
                level: 1,
                band: 0.75
            })
        );
    }

    #[test]
    fn nested_band_channel_value_band_stores_leaf_boundary() {
        let cv: ChannelValue = col("x").into();
        let cv = cv.band(1.0);
        assert_eq!(
            cv.get_position_boundary(),
            Some(PositionBoundary::Band { band: 1.0 })
        );
    }

    #[test]
    fn nested_band_channel_expr_level_band_stores_position_boundary() {
        let expr = ChannelExpr::scaled(col("x")).level_band(0, 1.0);
        assert_eq!(
            expr.channel_value().get_position_boundary(),
            Some(PositionBoundary::LevelBand {
                level: 0,
                band: 1.0
            })
        );
    }

    #[test]
    fn nested_band_config_expressions_are_collected() {
        let ctx = SessionContext::new();
        let mut nested = NestedBandSpec::default();
        nested.level_mut(1).ordering = Maybe::Set(ScaleOrderingSpec {
            order_expr: Some(
                LogicalExprNode::from_expr(col("series_sort")).expect("serialize order expr"),
            ),
            order_descending: Some(true),
        });
        let cv: ChannelValue = col("category").into();
        let cv = cv.with_nested_band_config(nested);

        let rendered = cv
            .all_exprs(&ctx)
            .into_iter()
            .map(|expr| expr.to_string())
            .collect::<Vec<_>>();
        assert_eq!(rendered, vec!["category", "series_sort"]);
    }

    #[test]
    fn scaled_channel_value_bincode_round_trips() {
        let value = ChannelValue::from(col("category")).band(0.0);

        let serialized = bincode::serialize(&value).expect("serialize");
        let restored: ChannelValue = bincode::deserialize(&serialized).expect("deserialize");

        assert_eq!(
            restored.get_position_boundary(),
            Some(PositionBoundary::Band { band: 0.0 })
        );
    }

    #[test]
    fn nested_band_col_channel_value_bincode_round_trips() {
        let mut nested = NestedBandSpec::default();
        nested.level_mut(1).nest_scope = Some(NestScope::Shared);
        let value = ChannelValue::from(col("category"))
            .with_nested_band_config(nested)
            .band(0.0);

        let serialized = bincode::serialize(&value).expect("serialize");
        let restored: ChannelValue = bincode::deserialize(&serialized).expect("deserialize");

        assert_eq!(
            restored
                .get_nested_band_config()
                .and_then(|nested| nested.level(1))
                .and_then(|level| level.nest_scope),
            Some(NestScope::Shared)
        );
    }

    #[test]
    fn nested_band_named_struct_channel_value_bincode_round_trips() {
        use datafusion::prelude::{lit, named_struct};

        let mut nested = NestedBandSpec::default();
        nested.level_mut(1).nest_scope = Some(NestScope::Shared);
        nested.level_mut(1).padding_inner = Some(0.05);
        let value = ChannelValue::from(named_struct(vec![
            lit("group"),
            col("group"),
            lit("member"),
            col("member"),
        ]))
        .with_nested_band_config(nested)
        .band(0.0);

        let serialized = bincode::serialize(&value).expect("serialize");
        let restored: ChannelValue = bincode::deserialize(&serialized).expect("deserialize");

        let nested = restored.get_nested_band_config().expect("nested config");
        assert_eq!(nested.level(1).unwrap().nest_scope, Some(NestScope::Shared));
        assert_eq!(nested.level(1).unwrap().padding_inner, Some(0.05));
    }

    #[test]
    fn test_smart_string_conversion() {
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
        if let ChannelValue::Value { expr, .. } = cv {
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
        use datafusion::{functions::expr_fn::sqrt, logical_expr::col, prelude::SessionContext};

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
            expr: LogicalExprNode::from_expr(lit(ScalarValue::Null))
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
        {
            use datafusion::functions::expr_fn::abs;

            let cv: ChannelValue = sqrt(abs(col("x"))).into();
            assert_eq!(cv.as_column_name(&ctx), Some("sqrt(abs(x))".to_string()));
        }

        // Test function with multiple arguments (using pow as example)
        {
            use datafusion::functions::expr_fn::power;

            let cv: ChannelValue = power(col("x"), lit(2)).into();
            assert_eq!(cv.as_column_name(&ctx), Some("power(x, 2)".to_string()));
        }

        // Test function with string arguments (should be quoted in function context)
        {
            use datafusion::functions::expr_fn::concat;

            let cv: ChannelValue = concat(vec![lit("hello"), lit("world")]).into();
            assert_eq!(
                cv.as_column_name(&ctx),
                Some("concat('hello', 'world')".to_string())
            );
        }

        // Test complex expression (now returns the expression string)
        let cv: ChannelValue = (col("x") + col("y")).into();
        assert_eq!(cv.as_column_name(&ctx), Some("x + y".to_string()));

        // Test conditional value (should return None - no single name)
        {
            use super::ConditionalValue;

            let cv = ChannelValue::Conditional {
                conditions: vec![(
                    LogicalExprNode::from_expr(col("category").eq(lit("A")))
                        .expect("Failed to serialize expr"),
                    ConditionalValue::Value {
                        expr: LogicalExprNode::from_expr(lit("red"))
                            .expect("Failed to serialize expr"),
                    },
                )],
                otherwise: ConditionalValue::Scaled {
                    expr: LogicalExprNode::from_expr(col("color"))
                        .expect("Failed to serialize expr"),
                },
                scale_config: None,
                nested_band_config: None,
                legend_config: None,
                axis_config: None,
                domain_coordination: None,
                transform_scope: None,
            };
            // Conditional values don't have a single column name
            assert_eq!(cv.as_column_name(&ctx), None);
        }
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
        use datafusion::prelude::SessionContext;

        use super::ConditionalValue;

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
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
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
        use datafusion::prelude::SessionContext;

        use super::ConditionalValue;

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
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
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
        use datafusion::prelude::SessionContext;

        use super::ConditionalValue;

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
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
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
        use datafusion::prelude::SessionContext;

        use super::ConditionalValue;

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
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
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
        use datafusion::prelude::SessionContext;

        use super::ConditionalValue;

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
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
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
