use std::sync::Arc;

use datafusion::{
    dataframe::DataFrame,
    logical_expr::{Expr, lit},
};
use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, to_proto::serialize_expr};
use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, LogicalPlanNodeExt, RadiusExpression, ResolvedDomain, SerializableDataFrame,
    SerializableExpr,
};

fn logical_expr_node_from_expr(expr: Expr, label: &str) -> LogicalExprNode {
    let codec = DefaultLogicalExtensionCodec {};
    serialize_expr(&expr, &codec).unwrap_or_else(|err| panic!("Failed to serialize {label}: {err}"))
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleDomain {
    pub default_domain: ScaleDefaultDomain,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub raw_domain: Option<LogicalExprNode>,
}

#[serde_as]
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScaleDefaultDomain {
    // Intervals
    Interval(
        #[serde_as(as = "FromInto<SerializableExpr>")] LogicalExprNode,
        #[serde_as(as = "Box<FromInto<SerializableExpr>>")] Box<LogicalExprNode>,
    ),
    // Discrete values
    Discrete(#[serde_as(as = "Vec<FromInto<SerializableExpr>>")] Vec<LogicalExprNode>),
    // Domain derived from data
    DomainExprs(Vec<DomainExpr>),
    // No default domain, must be provided explicitly
    NoDefault,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainExpr {
    #[serde_as(as = "Arc<FromInto<SerializableDataFrame>>")]
    pub dataframe: Arc<LogicalPlanNode>,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    pub radius: Option<RadiusExpression>,
}

impl ScaleDomain {
    pub fn new_interval<E: Into<Expr>>(start: E, end: E) -> Self {
        let start_node = logical_expr_node_from_expr(start.into(), "start expr");
        let end_node = logical_expr_node_from_expr(end.into(), "end expr");
        Self {
            default_domain: ScaleDefaultDomain::Interval(start_node, Box::new(end_node)),
            raw_domain: None,
        }
    }

    pub fn new_discrete(values: Vec<Expr>) -> Self {
        let serialized_values: Vec<LogicalExprNode> = values
            .into_iter()
            .map(|expr| logical_expr_node_from_expr(expr, "expr"))
            .collect();
        Self {
            default_domain: ScaleDefaultDomain::Discrete(serialized_values),
            raw_domain: None,
        }
    }

    pub fn new_data_field(dataframe: Arc<DataFrame>, expr: Expr) -> Self {
        let plan = dataframe.logical_plan().clone();
        let plan_node = Arc::new(
            LogicalPlanNode::from_logical_plan(&plan).expect("Failed to serialize logical plan"),
        );
        let expr_node = logical_expr_node_from_expr(expr, "expr");
        Self {
            default_domain: ScaleDefaultDomain::DomainExprs(vec![DomainExpr {
                dataframe: plan_node,
                expr: expr_node,
                radius: None,
            }]),
            raw_domain: None,
        }
    }

    pub fn new_data_fields(fields: Vec<(Arc<DataFrame>, Expr)>) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::DomainExprs(
                fields
                    .into_iter()
                    .map(|(dataframe, expr)| {
                        let plan = dataframe.logical_plan().clone();
                        let plan_node = Arc::new(
                            LogicalPlanNode::from_logical_plan(&plan)
                                .expect("Failed to serialize logical plan"),
                        );
                        let expr_node = logical_expr_node_from_expr(expr, "expr");
                        DomainExpr {
                            dataframe: plan_node,
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
        let plan = dataframe.logical_plan().clone();
        let plan_node = Arc::new(
            LogicalPlanNode::from_logical_plan(&plan).expect("Failed to serialize logical plan"),
        );
        let expr_node = logical_expr_node_from_expr(expr, "expr");
        let radius_node = logical_expr_node_from_expr(radius, "radius expr");
        Self {
            default_domain: ScaleDefaultDomain::DomainExprs(vec![DomainExpr {
                dataframe: plan_node,
                expr: expr_node,
                radius: Some(RadiusExpression::Symmetric(radius_node)),
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
