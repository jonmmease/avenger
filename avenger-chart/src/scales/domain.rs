use crate::error::AvengerChartError;
use crate::scales::ScaleRange;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, lit};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ScaleDomain {
    pub default_domain: ScaleDefaultDomain,
    pub raw_domain: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum ScaleDefaultDomain {
    // Intervals
    Interval(Expr, Box<Expr>),
    // Discrete values
    Discrete(Vec<Expr>),
    // Domain derived from data
    DomainExprs(Vec<DomainExpr>),
    // No default domain, must be provided explicitly
    NoDefault,
}

#[derive(Debug, Clone)]
pub struct DomainExpr {
    pub dataframe: Arc<DataFrame>,
    pub expr: Expr,
    pub radius: Option<crate::marks::RadiusExpression>,
}

impl ScaleDomain {
    pub fn new_interval<E: Into<Expr>>(start: E, end: E) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::Interval(start.into(), Box::new(end.into())),
            raw_domain: None,
        }
    }

    pub fn new_discrete(values: Vec<Expr>) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::Discrete(values),
            raw_domain: None,
        }
    }

    pub fn new_data_field(dataframe: Arc<DataFrame>, expr: Expr) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::DomainExprs(vec![DomainExpr {
                dataframe,
                expr,
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
                    .map(|(dataframe, expr)| DomainExpr {
                        dataframe,
                        expr,
                        radius: None,
                    })
                    .collect(),
            ),
            raw_domain: None,
        }
    }

    pub fn new_data_field_with_radius(dataframe: Arc<DataFrame>, expr: Expr, radius: Expr) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::DomainExprs(vec![DomainExpr {
                dataframe,
                expr,
                radius: Some(crate::marks::RadiusExpression::Symmetric(radius)),
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
