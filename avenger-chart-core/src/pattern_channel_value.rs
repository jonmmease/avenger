use avenger_scenegraph::marks::pattern::PatternFill;
use datafusion::{logical_expr::Expr, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    ChannelExpr, ConditionalValue, CoordinationScope, DomainCoordination, DomainCoordinationGroup,
    IntoExpr, Legend, ScaleConfigSpec, SerializableExpr, channel::strip_trailing_numbers,
    channel_value::expr_to_string, serialization::DefaultLogicalExprNodeExt,
};

/// Represents a pattern-fill channel encoding value.
///
/// Pattern values are structured scenegraph style data, so literal patterns do
/// not travel through DataFusion. Scaled pattern channels still carry a normal
/// DataFusion input expression so scale-domain planning can reuse the ordinary
/// chart pipeline; the scaled output is later resolved through a typed pattern
/// range registry.
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PatternChannelValue {
    /// Expression that will be transformed through a pattern scale.
    Scaled {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        expr: LogicalExprNode,
        /// Optional custom scale name (defaults to channel name).
        scale_name: Option<String>,
        /// Optional scale configuration.
        scale_config: Option<Box<ScaleConfigSpec>>,
        /// Optional legend configuration.
        legend_config: Option<Box<Legend>>,
        /// Optional scale-domain coordination metadata.
        #[serde(default)]
        domain_coordination: Option<DomainCoordination>,
        /// Scope of the transform stage that produced this value, if any.
        #[serde(default)]
        transform_scope: Option<CoordinationScope>,
    },
    /// Conditional pattern encoding that is evaluated through a pattern scale.
    Conditional {
        /// List of (condition, value) pairs.
        #[serde_as(as = "Vec<(FromInto<SerializableExpr>, _)>")]
        conditions: Vec<(LogicalExprNode, ConditionalValue)>,
        /// Default value when no conditions match.
        otherwise: ConditionalValue,
        /// Optional scale configuration.
        scale_config: Option<Box<ScaleConfigSpec>>,
        /// Optional legend configuration.
        legend_config: Option<Box<Legend>>,
        /// Optional scale-domain coordination metadata.
        #[serde(default)]
        domain_coordination: Option<DomainCoordination>,
        /// Scope of the transform stage that produced this value, if any.
        #[serde(default)]
        transform_scope: Option<CoordinationScope>,
    },
    /// Literal pattern that bypasses scaling.
    Value {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<PatternFill>,
    },
}

impl PatternChannelValue {
    /// Create a scaled pattern channel from an expression.
    pub fn scaled(expr: impl IntoExpr) -> Self {
        Self::Scaled {
            expr: LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("Failed to serialize pattern channel expression"),
            scale_name: None,
            scale_config: None,
            legend_config: None,
            domain_coordination: None,
            transform_scope: None,
        }
    }

    /// Create a literal pattern channel.
    pub fn value(pattern: impl Into<Option<PatternFill>>) -> Self {
        Self::Value {
            pattern: pattern.into(),
        }
    }

    /// Get the expression for scaled values.
    pub fn expr(&self, ctx: &SessionContext) -> Option<Expr> {
        match self {
            Self::Scaled { expr, .. } => expr.to_default_expr(ctx).ok(),
            Self::Conditional { .. } => None,
            Self::Value { .. } => None,
        }
    }

    /// Get expression for scale domain collection.
    pub fn scale_input_expr(&self, ctx: &SessionContext) -> Option<Expr> {
        self.scaled_channel_surrogate()
            .and_then(|surrogate| surrogate.scale_input_expr(ctx))
    }

    /// Get all DataFusion expressions referenced by this pattern channel.
    pub fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        self.scaled_channel_surrogate()
            .map(|surrogate| surrogate.all_exprs(ctx))
            .unwrap_or_default()
    }

    /// Get the scale name for this channel.
    pub fn get_scale_name(&self, channel_name: &str) -> Option<String> {
        match self {
            Self::Scaled { scale_name, .. } => scale_name
                .clone()
                .or_else(|| Some(strip_trailing_numbers(channel_name).to_string())),
            Self::Conditional { .. } => Some(strip_trailing_numbers(channel_name).to_string()),
            Self::Value { .. } => None,
        }
    }

    /// Extract a human-readable title from the scaled expression.
    pub fn as_column_name(&self, ctx: &SessionContext) -> Option<String> {
        self.expr(ctx).map(|expr| expr_to_string(&expr))
    }

    pub fn has_scale_config(&self) -> bool {
        self.get_scale_config().is_some()
    }

    pub fn has_legend_config(&self) -> bool {
        self.get_legend_config().is_some()
    }

    pub fn get_scale_config(&self) -> Option<&ScaleConfigSpec> {
        match self {
            Self::Scaled { scale_config, .. } => scale_config.as_deref(),
            Self::Conditional { scale_config, .. } => scale_config.as_deref(),
            Self::Value { .. } => None,
        }
    }

    pub fn get_legend_config(&self) -> Option<&Legend> {
        match self {
            Self::Scaled { legend_config, .. } => legend_config.as_deref(),
            Self::Conditional { legend_config, .. } => legend_config.as_deref(),
            Self::Value { .. } => None,
        }
    }

    pub fn get_domain_coordination(&self) -> Option<&DomainCoordination> {
        match self {
            Self::Scaled {
                domain_coordination,
                ..
            }
            | Self::Conditional {
                domain_coordination,
                ..
            } => domain_coordination.as_ref(),
            Self::Value { .. } => None,
        }
    }

    pub fn get_domain_scope(&self) -> Option<CoordinationScope> {
        self.get_domain_coordination()
            .map(|coordination| coordination.scope)
    }

    pub fn get_transform_scope(&self) -> Option<CoordinationScope> {
        match self {
            Self::Scaled {
                transform_scope, ..
            }
            | Self::Conditional {
                transform_scope, ..
            } => *transform_scope,
            Self::Value { .. } => None,
        }
    }

    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        match self {
            Self::Scaled {
                expr,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
                ..
            } => Self::Scaled {
                expr,
                scale_name: Some(name.into()),
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            },
            Self::Conditional { .. } => self,
            Self::Value { .. } => self,
        }
    }

    pub fn with_scale_config(self, scale_config: ScaleConfigSpec) -> Self {
        match self {
            Self::Scaled {
                expr,
                scale_name,
                legend_config,
                domain_coordination,
                transform_scope,
                ..
            } => Self::Scaled {
                expr,
                scale_name,
                scale_config: Some(Box::new(scale_config)),
                legend_config,
                domain_coordination,
                transform_scope,
            },
            Self::Conditional {
                conditions,
                otherwise,
                legend_config,
                domain_coordination,
                transform_scope,
                ..
            } => Self::Conditional {
                conditions,
                otherwise,
                scale_config: Some(Box::new(scale_config)),
                legend_config,
                domain_coordination,
                transform_scope,
            },
            Self::Value { .. } => self,
        }
    }

    pub fn with_legend_config(self, legend_config: Legend) -> Self {
        match self {
            Self::Scaled {
                expr,
                scale_name,
                scale_config,
                domain_coordination,
                transform_scope,
                ..
            } => Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config: Some(Box::new(legend_config)),
                domain_coordination,
                transform_scope,
            },
            Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                domain_coordination,
                transform_scope,
                ..
            } => Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config: Some(Box::new(legend_config)),
                domain_coordination,
                transform_scope,
            },
            Self::Value { .. } => self,
        }
    }

    pub fn with_transform_scope(self, scope: CoordinationScope) -> Self {
        let scope = scope.to_normalized();
        match self {
            Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination,
                ..
            } => Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination: domain_coordination
                    .or_else(|| Some(DomainCoordination::scale_name(scope))),
                transform_scope: Some(scope),
            },
            Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination,
                ..
            } => Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination: domain_coordination
                    .or_else(|| Some(DomainCoordination::scale_name(scope))),
                transform_scope: Some(scope),
            },
            Self::Value { .. } => self,
        }
    }

    pub fn with_domain_scope(self, scope: CoordinationScope) -> Self {
        match self {
            Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            } => Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination: Some(
                    domain_coordination.unwrap_or_default().with_scope(scope),
                ),
                transform_scope,
            },
            Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            } => Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination: Some(
                    domain_coordination.unwrap_or_default().with_scope(scope),
                ),
                transform_scope,
            },
            Self::Value { .. } => self,
        }
    }

    pub fn with_domain_group(self, group: impl Into<String>) -> Self {
        let group = DomainCoordinationGroup::Named(group.into());
        match self {
            Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            } => Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination: Some(
                    domain_coordination.unwrap_or_default().with_group(group),
                ),
                transform_scope,
            },
            Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            } => Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination: Some(
                    domain_coordination.unwrap_or_default().with_group(group),
                ),
                transform_scope,
            },
            Self::Value { .. } => self,
        }
    }

    pub fn with_domain_coordination(self, coordination: DomainCoordination) -> Self {
        let scope = coordination.scope;
        let coordination = coordination.with_scope(scope);
        match self {
            Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                transform_scope,
                ..
            } => Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination: Some(coordination),
                transform_scope,
            },
            Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                transform_scope,
                ..
            } => Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination: Some(coordination),
                transform_scope,
            },
            Self::Value { .. } => self,
        }
    }

    /// Return the ordinary scalar channel surrogate used for scale/data prep.
    pub fn scaled_channel_surrogate(&self) -> Option<crate::ChannelValue> {
        match self {
            Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            } => Some(crate::ChannelValue::Scaled {
                expr: expr.clone(),
                scale_name: scale_name.clone(),
                position_boundary: None,
                scale_config: scale_config.clone(),
                nested_band_config: None,
                legend_config: legend_config.clone(),
                axis_config: None,
                domain_coordination: domain_coordination.clone(),
                transform_scope: *transform_scope,
            }),
            Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            } => Some(crate::ChannelValue::Conditional {
                conditions: conditions.clone(),
                otherwise: otherwise.clone(),
                scale_config: scale_config.clone(),
                nested_band_config: None,
                legend_config: legend_config.clone(),
                axis_config: None,
                domain_coordination: domain_coordination.clone(),
                transform_scope: *transform_scope,
            }),
            Self::Value { .. } => None,
        }
    }
}

impl From<PatternFill> for PatternChannelValue {
    fn from(pattern: PatternFill) -> Self {
        Self::value(Some(pattern))
    }
}

impl From<Option<PatternFill>> for PatternChannelValue {
    fn from(pattern: Option<PatternFill>) -> Self {
        Self::value(pattern)
    }
}

impl From<Expr> for PatternChannelValue {
    fn from(expr: Expr) -> Self {
        Self::scaled(expr)
    }
}

impl From<ChannelExpr> for PatternChannelValue {
    fn from(value: ChannelExpr) -> Self {
        let channel_value = value.into_channel_value();
        match channel_value {
            crate::ChannelValue::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
                ..
            } => Self::Scaled {
                expr,
                scale_name,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            },
            crate::ChannelValue::Value { expr } => Self::Scaled {
                expr,
                scale_name: None,
                scale_config: None,
                legend_config: None,
                domain_coordination: None,
                transform_scope: None,
            },
            crate::ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
                ..
            } => Self::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                domain_coordination,
                transform_scope,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use avenger_scenegraph::marks::pattern::{PatternFill, PatternLayer, StripePatternLayer};
    use datafusion::prelude::{SessionContext, col, lit};

    use super::*;
    use crate::ChannelValue;

    fn expr_node(expr: Expr) -> LogicalExprNode {
        LogicalExprNode::from_default_expr(expr).expect("serialize test expression")
    }

    #[test]
    fn literal_pattern_channel_serializes_structured_pattern() {
        let value = PatternChannelValue::from(PatternFill {
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(
                45.0, 16.0, 1.25,
            ))],
            ..Default::default()
        });

        let json = serde_json::to_value(&value).expect("serialize pattern channel");
        assert_eq!(json["Value"]["pattern"]["layers"][0]["type"], "stripe");

        let decoded: PatternChannelValue =
            serde_json::from_value(json).expect("deserialize pattern channel");
        assert!(matches!(
            decoded,
            PatternChannelValue::Value { pattern: Some(_) }
        ));
    }

    #[test]
    fn scaled_pattern_channel_uses_default_scale_name() {
        let value = PatternChannelValue::from(col("category"));
        assert_eq!(
            value.get_scale_name("fill_pattern").as_deref(),
            Some("fill_pattern")
        );
    }

    #[test]
    fn scaled_pattern_channel_preserves_custom_scale_name() {
        let value = PatternChannelValue::from(col("category")).with_scale_name("hatch");
        assert_eq!(
            value.get_scale_name("fill_pattern").as_deref(),
            Some("hatch")
        );

        let Some(surrogate) = value.scaled_channel_surrogate() else {
            panic!("scaled pattern channels should provide a scalar surrogate");
        };
        assert_eq!(
            surrogate.get_scale_name("fill_pattern").as_deref(),
            Some("hatch")
        );
    }

    #[test]
    fn conditional_channel_expr_preserves_pattern_scale_surrogate() {
        let value = PatternChannelValue::from(ChannelExpr::new(
            col("series"),
            ChannelValue::Conditional {
                conditions: vec![(
                    expr_node(col("flag").eq(lit(true))),
                    ConditionalValue::Scaled {
                        expr: expr_node(col("series")),
                    },
                )],
                otherwise: ConditionalValue::Scaled {
                    expr: expr_node(lit("fallback")),
                },
                scale_config: None,
                nested_band_config: None,
                legend_config: None,
                axis_config: None,
                domain_coordination: None,
                transform_scope: None,
            },
        ));

        assert!(matches!(value, PatternChannelValue::Conditional { .. }));
        assert_eq!(
            value.get_scale_name("fill_pattern").as_deref(),
            Some("fill_pattern")
        );

        let Some(surrogate) = value.scaled_channel_surrogate() else {
            panic!("conditional pattern channels should provide a scalar surrogate");
        };
        assert!(matches!(surrogate, ChannelValue::Conditional { .. }));

        let ctx = SessionContext::new();
        assert!(value.scale_input_expr(&ctx).is_some());
        assert_eq!(value.all_exprs(&ctx).len(), 3);
    }
}
