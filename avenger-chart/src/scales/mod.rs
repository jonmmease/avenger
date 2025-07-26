pub mod udf;

use crate::error::AvengerChartError;
use avenger_scales::scales::{InferDomainFromDataMethod, ScaleImpl};
use datafusion::arrow::datatypes::DataType;
use datafusion::dataframe::DataFrame;
use datafusion::functions_array::expr_fn::make_array;
use datafusion::logical_expr::{Expr, ExprSchemable, lit, when};
use datafusion::prelude::named_struct;
use datafusion_common::{DFSchema, ScalarValue};
use palette::Srgba;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Scale {
    pub scale_impl: Arc<dyn ScaleImpl>,
    pub domain: ScaleDomain,
    pub range: ScaleRange,
    pub options: HashMap<String, Expr>,
    domain_explicit: bool,
    range_explicit: bool,
}

impl Scale {
    pub fn new<S: ScaleImpl>(scale_impl: S) -> Self {
        Self {
            scale_impl: Arc::new(scale_impl),
            domain: ScaleDomain::new_interval(lit(0.0), lit(1.0)),
            range: ScaleRange::new_interval(lit(0.0), lit(1.0)),
            options: HashMap::new(),
            domain_explicit: false,
            range_explicit: false,
        }
    }

    pub fn get_scale_impl(&self) -> &Arc<dyn ScaleImpl> {
        &self.scale_impl
    }

    // Domain builders
    pub fn domain<D: Into<ScaleDomain>>(self, domain: D) -> Self {
        Self {
            domain: domain.into(),
            domain_explicit: true,
            ..self
        }
    }

    pub fn get_domain(&self) -> &ScaleDomain {
        &self.domain
    }

    pub fn domain_interval<T: Into<Expr>>(self, start: T, end: T) -> Self {
        self.domain(ScaleDomain::new_interval(start, end))
    }

    pub fn domain_discrete<T: Into<Expr>>(mut self, values: Vec<T>) -> Self {
        let exprs: Vec<Expr> = values.into_iter().map(|v| v.into()).collect();

        // Switch to BandScale for discrete domains
        if self.scale_impl.scale_type() == "linear" {
            self.scale_impl = Arc::new(avenger_scales::scales::band::BandScale);

            // Set default band scale options if not already set
            if !self.options.contains_key("padding_inner") {
                self.options.insert("padding_inner".to_string(), lit(0.1));
            }
            if !self.options.contains_key("padding_outer") {
                self.options.insert("padding_outer".to_string(), lit(0.1));
            }
            if !self.options.contains_key("align") {
                self.options.insert("align".to_string(), lit(0.5));
            }
        }

        self.domain(ScaleDomain::new_discrete(exprs))
    }

    pub fn domain_data_field<S: Into<String>>(self, dataframe: Arc<DataFrame>, field: S) -> Self {
        self.domain(ScaleDomain::new_data_field(dataframe, field))
    }

    pub fn domain_data_fields<S: Into<String>>(self, fields: Vec<(Arc<DataFrame>, S)>) -> Self {
        self.domain(ScaleDomain::new_data_fields(fields))
    }

    pub fn raw_domain<E: Clone + Into<Expr>>(self, raw_domain: E) -> Self {
        let new_domain = self.domain.clone().with_raw(raw_domain.into());
        self.domain(new_domain)
    }

    // Range builders
    pub fn range(self, range: ScaleRange) -> Self {
        Self {
            range,
            range_explicit: true,
            ..self
        }
    }

    pub fn get_range(&self) -> &ScaleRange {
        &self.range
    }

    pub fn range_numeric<F: Into<Expr>>(self, start: F, end: F) -> Self {
        self.range(ScaleRange::new_interval(start, end))
    }

    pub fn range_color(self, colors: Vec<Srgba>) -> Self {
        self.range(ScaleRange::new_color(colors))
    }

    // Other builder methods
    pub fn option<K: Into<String>, V: Into<Expr>>(mut self, key: K, value: V) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    pub fn get_options(&self) -> &HashMap<String, Expr> {
        &self.options
    }

    // Check if domain/range were explicitly set
    pub fn has_explicit_domain(&self) -> bool {
        self.domain_explicit
    }

    pub fn has_explicit_range(&self) -> bool {
        self.range_explicit
    }

    /// Create a scale expression that transforms values using this scale
    pub fn to_expr(&self, values: Expr) -> Result<Expr, AvengerChartError> {
        let domain_expr = self.compile_domain()?;
        let range_expr = self.compile_range()?;
        let options_expr = self.compile_options()?;

        let domain_type = self.domain.data_type()?;
        let range_type = self.range.data_type()?;
        let options_type = options_expr.get_type(&DFSchema::empty())?;

        let udf = udf::create_scale_udf(
            self.scale_impl.clone(),
            domain_type,
            range_type,
            options_type,
        )?;

        Ok(udf.call(vec![domain_expr, range_expr, options_expr, values]))
    }

    /// Compile domain to an expression that evaluates to a list
    fn compile_domain(&self) -> Result<Expr, AvengerChartError> {
        self.domain
            .compile(self.scale_impl.infer_domain_from_data_method())
    }

    /// Compile range to an expression that evaluates to a list
    fn compile_range(&self) -> Result<Expr, AvengerChartError> {
        self.range.compile()
    }

    /// Compile options to an expression that evaluates to a struct
    fn compile_options(&self) -> Result<Expr, AvengerChartError> {
        use datafusion::arrow::array::StructArray;

        if self.options.is_empty() {
            // Create an empty struct with 1 row for scalar
            let empty_struct = StructArray::new_empty_fields(1, None);
            Ok(lit(ScalarValue::Struct(Arc::new(empty_struct))))
        } else {
            let struct_args = self
                .options
                .iter()
                .flat_map(|(key, value)| vec![lit(key), value.clone()])
                .collect::<Vec<_>>();

            Ok(named_struct(struct_args))
        }
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
    Interval(Expr, Box<Expr>),
    // Discrete values
    Discrete(Vec<Expr>),

    // Domain derived from data
    DataField(DataField),
    DataFields(Vec<DataField>),
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

    pub fn new_data_field<S: Into<String>>(dataframe: Arc<DataFrame>, field: S) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::DataField(DataField {
                dataframe,
                field: field.into(),
            }),
            raw_domain: None,
        }
    }

    pub fn new_data_fields<S: Into<String>>(fields: Vec<(Arc<DataFrame>, S)>) -> Self {
        Self {
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
            ScaleDefaultDomain::DataField(DataField { dataframe, field }) => Ok(dataframe
                .schema()
                .field_with_name(None, field)?
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
                    .field_with_name(None, field)?
                    .data_type()
                    .clone())
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
                    return Err(AvengerChartError::InternalError(
                        "Scale does not support interval domain".to_string(),
                    ));
                }
                make_array(vec![start.clone(), end.as_ref().clone()])
            }
            ScaleDefaultDomain::Discrete(values) => make_array(values.clone()),
            ScaleDefaultDomain::DataField(_) | ScaleDefaultDomain::DataFields(_) => {
                // TODO: Implement data field domain inference
                // For now, return a placeholder
                return Err(AvengerChartError::InternalError(
                    "DataField domain inference not yet implemented for UDF scales".to_string(),
                ));
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

#[derive(Debug, Clone)]
pub struct DataField {
    pub dataframe: Arc<DataFrame>,
    pub field: String,
}

#[derive(Debug, Clone)]
pub enum ScaleRange {
    Numeric(Expr, Box<Expr>),
    Enum(Vec<ScalarValue>),
    Color(Vec<Srgba>),
}

impl ScaleRange {
    pub fn new_interval<E: Into<Expr>, F: Into<Expr>>(start: E, end: F) -> Self {
        Self::Numeric(start.into(), Box::new(end.into()))
    }

    pub fn new_color(colors: Vec<Srgba>) -> Self {
        Self::Color(colors)
    }

    pub fn new_enum<T: Into<ScalarValue>>(values: Vec<T>) -> Self {
        Self::Enum(values.into_iter().map(|v| v.into()).collect())
    }

    pub fn data_type(&self) -> Result<DataType, AvengerChartError> {
        match self {
            ScaleRange::Numeric(_, _) => Ok(DataType::Float32),
            ScaleRange::Enum(vals) => {
                vals.first()
                    .map(|v| v.data_type().clone())
                    .ok_or(AvengerChartError::InternalError(
                        "Enum range may not be empty".to_string(),
                    ))
            }
            ScaleRange::Color(_) => Ok(DataType::new_list(DataType::Float32, true)),
        }
    }

    /// Compile range to an expression that evaluates to a list
    pub fn compile(&self) -> Result<Expr, AvengerChartError> {
        match self {
            ScaleRange::Numeric(start, end) => {
                Ok(make_array(vec![start.clone(), end.as_ref().clone()]))
            }
            ScaleRange::Enum(values) => {
                let exprs = values.iter().map(|v| lit(v.clone())).collect::<Vec<_>>();
                Ok(make_array(exprs))
            }
            ScaleRange::Color(colors) => {
                // Convert colors to RGBA array expressions
                // TODO: Implement proper struct creation with the new datafusion API
                let color_exprs = colors
                    .iter()
                    .map(|c| {
                        // For now, create a simple array instead of struct
                        make_array(vec![lit(c.red), lit(c.green), lit(c.blue), lit(c.alpha)])
                    })
                    .collect::<Vec<_>>();
                Ok(make_array(color_exprs))
            }
        }
    }
}

/// Registry for accessing scales in a plot
#[derive(Debug, Clone, Default)]
pub struct ScaleRegistry {
    scales: HashMap<String, Scale>,
}

impl ScaleRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            scales: HashMap::new(),
        }
    }

    /// Add a scale to the registry
    pub fn add(&mut self, name: impl Into<String>, scale: Scale) -> &mut Self {
        self.scales.insert(name.into(), scale);
        self
    }

    /// Get a scale by name
    pub fn get(&self, name: &str) -> Option<&Scale> {
        self.scales.get(name)
    }

    /// Get all scales
    pub fn scales(&self) -> &HashMap<String, Scale> {
        &self.scales
    }

    /// Get scales for specific channels
    pub fn get_scales_for_channels<'a>(
        &'a self,
        channels: &'a [&'a str],
    ) -> Vec<(&'a str, &'a Scale)> {
        channels
            .iter()
            .filter_map(|&channel| self.scales.get(channel).map(|scale| (channel, scale)))
            .collect()
    }
}
