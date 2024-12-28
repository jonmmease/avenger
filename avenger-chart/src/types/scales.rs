use std::{collections::HashMap, sync::Arc};

use arrow::datatypes::{DataType, Field};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use datafusion::prelude::SessionContext;
use datafusion::{
    common::{DFSchema, ExprSchema},
    logical_expr::ExprSchemable,
    prelude::{lit, DataFrame, Expr},
    scalar::ScalarValue,
};
use palette::{Hsla, Laba, Srgba};

use crate::error::AvengerChartError;
use crate::runtime::scale::{compile_domain, EvaluatedScale};

#[derive(Debug, Clone)]
pub struct Scale {
    pub scale_impl: Arc<dyn ScaleImpl>,
    pub domain: ScaleDomain,
    pub range: ScaleRange,
    pub options: HashMap<String, Expr>,
}

impl Scale {
    pub fn new<S: ScaleImpl>(scale_impl: S) -> Self {
        Self {
            scale_impl: Arc::new(scale_impl),
            domain: ScaleDomain::new_interval(lit(0.0), lit(1.0)),
            range: ScaleRange::new_interval(lit(0.0), lit(1.0)),
            options: HashMap::new(),
        }
    }

    pub fn get_scale_impl(&self) -> &Arc<dyn ScaleImpl> {
        &self.scale_impl
    }

    // Domain builders
    pub fn domain(self, domain: ScaleDomain) -> Self {
        Self { domain, ..self }
    }

    pub fn get_domain(&self) -> &ScaleDomain {
        &self.domain
    }

    pub fn domain_interval<T: Into<Expr>>(self, start: T, end: T) -> Self {
        Self {
            domain: ScaleDomain {
                default_domain: ScaleDefaultDomain::Interval(start.into(), end.into()),
                raw_domain: None,
            },
            ..self
        }
    }

    pub fn domain_discrete<T: Into<Expr>>(self, values: Vec<T>) -> Self {
        Self {
            domain: ScaleDomain {
                default_domain: ScaleDefaultDomain::Discrete(
                    values.into_iter().map(|v| v.into()).collect(),
                ),
                raw_domain: None,
            },
            ..self
        }
    }

    pub fn domain_data_field<S: Into<String>>(self, dataframe: Arc<DataFrame>, field: S) -> Self {
        Self {
            domain: ScaleDomain {
                default_domain: ScaleDefaultDomain::DataField(DataField {
                    dataframe,
                    field: field.into(),
                }),
                raw_domain: None,
            },
            ..self
        }
    }

    pub fn domain_data_fields<S: Into<String>>(self, fields: Vec<(Arc<DataFrame>, S)>) -> Self {
        Self {
            domain: ScaleDomain {
                default_domain: ScaleDefaultDomain::DataFields(
                    fields
                        .into_iter()
                        .map(|(dataframe, field)| DataField {
                            dataframe,
                            field: field.into(),
                        })
                        .collect(),
                ),
                raw_domain: None,
            },
            ..self
        }
    }

    pub fn raw_domain<E: Clone + Into<Expr>>(self, raw_domain: E) -> Self {
        Self {
            domain: self.domain.with_raw(raw_domain.into()),
            ..self
        }
    }

    // Range builders
    pub fn range(self, range: ScaleRange) -> Self {
        Self { range, ..self }
    }

    pub fn get_range(&self) -> &ScaleRange {
        &self.range
    }

    pub fn range_numeric<F: Into<Expr>>(self, start: F, end: F) -> Self {
        Self {
            range: ScaleRange::Numeric(start.into(), end.into()),
            ..self
        }
    }

    pub fn range_color(self, colors: Vec<Srgba>) -> Self {
        Self {
            range: ScaleRange::Color(colors),
            ..self
        }
    }

    // Other builder methods
    pub fn option(mut self, key: String, value: Expr) -> Self {
        self.options.insert(key, value);
        self
    }

    pub fn get_options(&self) -> &HashMap<String, Expr> {
        &self.options
    }
}

#[derive(Debug, Clone)]
pub struct ScaleDomain {
    pub default_domain: ScaleDefaultDomain,
    pub raw_domain: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum ScaleDefaultDomain {
    // Intervals
    Interval(Expr, Expr),
    // Discrete values
    Discrete(Vec<Expr>),

    // Domain derived from data
    DataField(DataField),
    DataFields(Vec<DataField>),
}

impl ScaleDomain {
    pub fn new_interval<E: Into<Expr>>(start: E, end: E) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::Interval(start.into(), end.into()),
            raw_domain: None,
        }
    }

    pub fn new_discrete(values: Vec<Expr>) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::Discrete(values),
            raw_domain: None,
        }
    }

    pub fn new_data_field<S: Into<String>>(self, dataframe: Arc<DataFrame>, field: S) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::DataField(DataField {
                dataframe,
                field: field.into(),
            }),
            raw_domain: None,
        }
    }

    pub fn new_data_fields<S: Into<String>>(self, fields: Vec<(Arc<DataFrame>, S)>) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::DataFields(
                fields
                    .into_iter()
                    .map(|(dataframe, field)| DataField {
                        dataframe: dataframe,
                        field: field.into(),
                    })
                    .collect(),
            ),
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
            ScaleDefaultDomain::Discrete(exprs) => Ok(exprs[0].get_type(&schema)?),
            // TODO: change these to hold DataFrame references
            ScaleDefaultDomain::DataField(DataField { dataframe, field }) => Ok(dataframe
                .schema()
                .field_with_name(None, &field)?
                .data_type()
                .clone()),
            ScaleDefaultDomain::DataFields(fields) => {
                let DataField { dataframe, field } = fields.first().ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "Domain data fields may not be empty".to_string(),
                    )
                })?;
                Ok(dataframe
                    .schema()
                    .field_with_name(None, &field)?
                    .data_type()
                    .clone())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataField {
    pub dataframe: Arc<DataFrame>,
    pub field: String,
}

#[derive(Debug, Clone)]
pub enum ScaleRange {
    Numeric(Expr, Expr),
    Enum(Vec<String>),
    Color(Vec<Srgba>),
}

impl ScaleRange {
    pub fn new_interval<E: Into<Expr>, F: Into<Expr>>(start: E, end: F) -> Self {
        Self::Numeric(start.into(), end.into())
    }

    pub fn new_color(colors: Vec<Srgba>) -> Self {
        Self::Color(colors)
    }

    pub fn data_type(&self) -> Result<DataType, AvengerChartError> {
        match self {
            ScaleRange::Numeric(_, _) => Ok(DataType::Float32),
            ScaleRange::Enum(_) => Ok(DataType::Utf8),
            ScaleRange::Color(_) => Ok(DataType::new_list(DataType::Float32, true)),
        }
    }
}
