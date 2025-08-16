use crate::error::AvengerChartError;
use crate::utils::DataFrameChartHelpers;
use avenger_scales::scales::InferDomainFromDataMethod;
use datafusion::arrow::datatypes::DataType;
use datafusion::dataframe::DataFrame;
use datafusion::functions_array::expr_fn::make_array;
use datafusion::logical_expr::{Expr, ExprSchemable, lit, when};
use datafusion_common::{DFSchema, ScalarValue};
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

    pub fn with_raw(self, raw_domain: Expr) -> Self {
        Self {
            default_domain: self.default_domain,
            raw_domain: Some(raw_domain),
        }
    }

    pub fn data_type(&self) -> Result<DataType, AvengerChartError> {
        let schema = DFSchema::empty();
        match &self.default_domain {
            ScaleDefaultDomain::Interval(expr, _) => Ok(expr.get_type(&schema)?),
            ScaleDefaultDomain::Discrete(exprs) => {
                if exprs.is_empty() {
                    // Default to string type for empty discrete domains
                    Ok(DataType::Utf8)
                } else {
                    Ok(exprs[0].get_type(&schema)?)
                }
            }
            ScaleDefaultDomain::DomainExprs(fields) => {
                let DomainExpr {
                    dataframe,
                    expr,
                    radius: _,
                } = fields.first().ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "Domain data fields may not be empty".to_string(),
                    )
                })?;
                // Use the expression's data type
                let schema = dataframe.schema();
                let df_schema = schema.clone();
                Ok(expr.get_type(&df_schema)?)
            }
        }
    }

    /// Compile domain to an expression that evaluates to a list
    pub fn compile(&self, method: InferDomainFromDataMethod) -> Result<Expr, AvengerChartError> {
        // If raw domain is provided, use it when not null
        let raw_expr = if let Some(raw) = &self.raw_domain {
            raw.clone()
        } else {
            lit(ScalarValue::Null)
        };

        // Compile default domain based on type
        let default_expr = match &self.default_domain {
            ScaleDefaultDomain::Interval(start, end) => {
                if method != InferDomainFromDataMethod::Interval {
                    return Err(AvengerChartError::InternalError(format!(
                        "Scale does not support interval domain: {self:?}"
                    )));
                }
                make_array(vec![start.clone(), end.as_ref().clone()])
            }
            ScaleDefaultDomain::Discrete(values) => make_array(values.clone()),
            ScaleDefaultDomain::DomainExprs(data_fields) => {
                let mut single_col_dfs: Vec<DataFrame> = Vec::new();

                for DomainExpr {
                    dataframe,
                    expr,
                    radius: _,
                } in data_fields
                {
                    let df = dataframe.clone();
                    // Select the expression and alias it to a consistent column name
                    let df_with_expr = df
                        .as_ref()
                        .clone()
                        .select(vec![expr.clone().alias("__domain_col__")])?;
                    single_col_dfs.push(df_with_expr);
                }

                // Union all the single column dataframes
                let union_df = single_col_dfs
                    .iter()
                    .skip(1)
                    .fold(single_col_dfs[0].clone(), |acc, df| {
                        acc.union(df.clone()).unwrap()
                    });

                match method {
                    InferDomainFromDataMethod::Interval => union_df.span()?,
                    InferDomainFromDataMethod::Unique => union_df.unique_values()?,
                    InferDomainFromDataMethod::All => union_df.all_values()?,
                }
            }
        };

        // Use raw domain if not null, otherwise use default
        Ok(when(raw_expr.clone().is_not_null(), raw_expr).otherwise(default_expr)?)
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