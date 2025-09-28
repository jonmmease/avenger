use crate::error::AvengerChartError;
use crate::scales::ScaleRange;
use crate::serialization::{SerializableExpr, LogicalExprNodeExt};
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, lit};
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, FromInto};
use std::sync::Arc;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleDomain {
    pub default_domain: ScaleDefaultDomain,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub raw_domain: Option<LogicalExprNode>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScaleDefaultDomain {
    // Intervals
    Interval(
        #[serde_as(as = "FromInto<SerializableExpr>")]
        LogicalExprNode,
        #[serde_as(as = "Box<FromInto<SerializableExpr>>")]
        Box<LogicalExprNode>
    ),
    // Discrete values
    Discrete(
        #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
        Vec<LogicalExprNode>
    ),
    // Domain derived from data
    DomainExprs(Vec<DomainExpr>),
    // No default domain, must be provided explicitly
    NoDefault,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainExpr {
    pub dataframe: Arc<crate::serialization::SerializableDataFrame>,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    pub radius: Option<crate::marks::RadiusExpression>,
}

impl ScaleDomain {
    pub fn new_interval<E: Into<Expr>>(start: E, end: E) -> Self {
        use crate::serialization::context::create_context_with_udfs;
        let ctx = create_context_with_udfs();
        let start_node =
            LogicalExprNode::from_expr(start.into(), &ctx).expect("Failed to serialize start expr");
        let end_node =
            LogicalExprNode::from_expr(end.into(), &ctx).expect("Failed to serialize end expr");
        Self {
            default_domain: ScaleDefaultDomain::Interval(start_node, Box::new(end_node)),
            raw_domain: None,
        }
    }

    pub fn new_discrete(values: Vec<Expr>) -> Self {
        use crate::serialization::context::create_context_with_udfs;
        let ctx = create_context_with_udfs();
        let serialized_values: Vec<LogicalExprNode> = values
            .into_iter()
            .map(|e| LogicalExprNode::from_expr(e, &ctx).expect("Failed to serialize expr"))
            .collect();
        Self {
            default_domain: ScaleDefaultDomain::Discrete(serialized_values),
            raw_domain: None,
        }
    }

    pub fn new_data_field(dataframe: Arc<DataFrame>, expr: Expr) -> Self {
        use crate::serialization::{SerializableDataFrame, context::create_context_with_udfs};
        let ctx = create_context_with_udfs();
        let df_ser = Arc::new(
            SerializableDataFrame::from_dataframe((*dataframe).clone())
                .expect("Failed to serialize dataframe"),
        );
        let expr_node = LogicalExprNode::from_expr(expr, &ctx).expect("Failed to serialize expr");
        Self {
            default_domain: ScaleDefaultDomain::DomainExprs(vec![DomainExpr {
                dataframe: df_ser,
                expr: expr_node,
                radius: None,
            }]),
            raw_domain: None,
        }
    }

    pub fn new_data_fields(fields: Vec<(Arc<DataFrame>, Expr)>) -> Self {
        use crate::serialization::{SerializableDataFrame, context::create_context_with_udfs};
        let ctx = create_context_with_udfs();
        Self {
            default_domain: ScaleDefaultDomain::DomainExprs(
                fields
                    .into_iter()
                    .map(|(dataframe, expr)| {
                        let df_ser = Arc::new(
                            SerializableDataFrame::from_dataframe((*dataframe).clone())
                                .expect("Failed to serialize dataframe"),
                        );
                        let expr_node =
                            LogicalExprNode::from_expr(expr, &ctx).expect("Failed to serialize expr");
                        DomainExpr {
                            dataframe: df_ser,
                            expr: expr_node,
                            radius: None,
                        }
                    })
                    .collect(),
            ),
            raw_domain: None,
        }
    }

    pub fn new_data_field_with_radius(dataframe: Arc<DataFrame>, expr: Expr, radius: Expr) -> Self {
        use crate::serialization::{SerializableDataFrame, context::create_context_with_udfs};
        let ctx = create_context_with_udfs();
        let df_ser = Arc::new(
            SerializableDataFrame::from_dataframe((*dataframe).clone())
                .expect("Failed to serialize dataframe"),
        );
        let expr_node = LogicalExprNode::from_expr(expr, &ctx).expect("Failed to serialize expr");
        let radius_node =
            LogicalExprNode::from_expr(radius, &ctx).expect("Failed to serialize radius expr");
        Self {
            default_domain: ScaleDefaultDomain::DomainExprs(vec![DomainExpr {
                dataframe: df_ser,
                expr: expr_node,
                radius: Some(crate::marks::RadiusExpression::Symmetric(radius_node)),
            }]),
            raw_domain: None,
        }
    }
}

impl From<(f32, f32)> for ScaleDomain {
    fn from(interval: (f32, f32)) -> Self {
        ScaleDomain::new_interval(lit(interval.0), lit(interval.1))
    }
}

impl From<(f64, f64)> for ScaleDomain {
    fn from(interval: (f64, f64)) -> Self {
        ScaleDomain::new_interval(lit(interval.0), lit(interval.1))
    }
}

impl From<(Expr, Expr)> for ScaleDomain {
    fn from(interval: (Expr, Expr)) -> Self {
        ScaleDomain::new_interval(interval.0, interval.1)
    }
}

impl From<Vec<Expr>> for ScaleDomain {
    fn from(values: Vec<Expr>) -> Self {
        ScaleDomain::new_discrete(values)
    }
}

/// Resolved domain type for marks to make range decisions
///
/// This enum indicates whether a domain has been resolved to discrete
/// values or a continuous interval, without needing to evaluate expressions.
#[derive(Debug, Clone, Copy)]
pub enum ResolvedDomain {
    /// Discrete domain with the number of unique values
    Discrete(usize),
    /// Continuous interval domain
    Interval,
}

impl ResolvedDomain {
    /// Create a numeric interval or linear spaced ScaleRange
    pub fn make_interval_or_linspaced_range(&self, start: f32, end: f32) -> ScaleRange {
        match self {
            ResolvedDomain::Interval => ScaleRange::new_interval(lit(start), lit(end)),
            ResolvedDomain::Discrete(count) => {
                ScaleRange::new_linspace_discrete(start, end, *count)
            }
        }
    }
}

impl ScaleDomain {
    /// Convert to ResolvedDomain if the domain has been resolved
    ///
    /// Returns an error if the domain is still DomainExprs or NoDefault
    pub fn to_resolved(&self) -> Result<ResolvedDomain, AvengerChartError> {
        match &self.default_domain {
            ScaleDefaultDomain::Discrete(values) => Ok(ResolvedDomain::Discrete(values.len())),
            ScaleDefaultDomain::Interval(_, _) => Ok(ResolvedDomain::Interval),
            ScaleDefaultDomain::DomainExprs(_) => Err(AvengerChartError::InternalError(
                "Cannot resolve domain: DomainExprs not yet evaluated".to_string(),
            )),
            ScaleDefaultDomain::NoDefault => Err(AvengerChartError::InternalError(
                "Cannot resolve domain: NoDefault domain type".to_string(),
            )),
        }
    }
}
