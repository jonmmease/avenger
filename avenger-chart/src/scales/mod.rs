pub mod defaults;
pub mod inference;
pub mod udf;

use crate::error::AvengerChartError;
use crate::utils::DataFrameChartUtils;
use avenger_scales::scales::{InferDomainFromDataMethod, ScaleImpl};
use datafusion::arrow::array::{Array, AsArray};
use datafusion::arrow::datatypes::DataType;
use datafusion::dataframe::DataFrame;
use datafusion::functions_array::expr_fn::make_array;
use datafusion::logical_expr::{Expr, ExprSchemable, lit, when};
use datafusion::prelude::named_struct;
use datafusion_common::{DFSchema, ScalarValue};
use palette::Srgba;
use std::collections::HashMap;
use std::sync::Arc;

// Import all scale types
use avenger_scales::scales::{
    band::BandScale, linear::LinearScale, log::LogScale, ordinal::OrdinalScale, point::PointScale,
    pow::PowScale, quantile::QuantileScale, quantize::QuantizeScale, symlog::SymlogScale,
    threshold::ThresholdScale, time::TimeScale,
};

/// Helper function to evaluate constant expressions to scalar values
fn eval_to_scalar(expr: &Expr) -> Result<ScalarValue, AvengerChartError> {
    match expr {
        Expr::Literal(scalar, _) => Ok(scalar.clone()),
        _ => Err(AvengerChartError::InternalError(
            "Cannot evaluate non-literal expression to scalar value".to_string(),
        )),
    }
}

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
        let scale_type = scale_impl.scale_type();
        let mut scale = Self {
            scale_impl: Arc::new(scale_impl),
            domain: ScaleDomain::new_interval(lit(0.0), lit(1.0)),
            range: ScaleRange::new_interval(lit(0.0), lit(1.0)),
            options: HashMap::new(),
            domain_explicit: false,
            range_explicit: false,
        };
        apply_scale_defaults(scale_type, &mut scale.options);
        scale
    }

    /// Create a scale with a specific type
    pub fn with_type(scale_type: &str) -> Self {
        let scale_impl = create_scale_impl(scale_type);
        let mut scale = Self {
            scale_impl,
            domain: ScaleDomain::new_interval(lit(0.0), lit(1.0)),
            range: ScaleRange::new_interval(lit(0.0), lit(1.0)),
            options: HashMap::new(),
            domain_explicit: false,
            range_explicit: false,
        };
        apply_scale_defaults(scale_type, &mut scale.options);
        scale
    }

    /// Set the scale type, replacing the current implementation
    pub fn scale_type(mut self, scale_type: &str) -> Self {
        self.scale_impl = create_scale_impl(scale_type);
        // Clear existing options and apply new defaults
        self.options.clear();
        apply_scale_defaults(scale_type, &mut self.options);
        self
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

    pub fn domain_discrete<T: Into<Expr>>(self, values: Vec<T>) -> Self {
        let exprs: Vec<Expr> = values.into_iter().map(|v| v.into()).collect();
        self.domain(ScaleDomain::new_discrete(exprs))
    }

    pub fn domain_data_field<S: Into<String>>(self, dataframe: Arc<DataFrame>, field: S) -> Self {
        self.domain(ScaleDomain::new_data_field(dataframe, field))
    }

    pub fn domain_data_fields<S: Into<String>>(self, fields: Vec<(Arc<DataFrame>, S)>) -> Self {
        self.domain(ScaleDomain::new_data_fields(fields))
    }

    pub fn domain_expressions(self, expressions: Vec<Expr>) -> Self {
        self.domain(ScaleDomain::new_expressions(expressions))
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

    pub fn range_interval<T: Into<Expr>>(self, start: T, end: T) -> Self {
        self.range(ScaleRange::new_interval(start, end))
    }

    pub fn range_discrete<T: Into<Expr>>(self, values: Vec<T>) -> Self {
        let scalars: Vec<ScalarValue> = values
            .into_iter()
            .map(|v| {
                let expr = v.into();
                match expr {
                    Expr::Literal(scalar, _) => scalar,
                    _ => ScalarValue::Null,
                }
            })
            .collect();
        self.range(ScaleRange::new_enum(scalars))
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

        // Cast values to match the domain type if needed
        let values = match &domain_type {
            DataType::Float32 => {
                // For numeric scales with Float32 domain, cast values to Float32
                use datafusion::logical_expr::cast;
                cast(values, DataType::Float32)
            }
            DataType::Float64 => {
                // For Float64 domains, cast to Float32 for scale compatibility
                use datafusion::logical_expr::cast;
                cast(values, DataType::Float32)
            }
            _ => values,
        };

        let udf = udf::create_scale_udf(
            self.scale_impl.clone(),
            domain_type,
            range_type,
            options_type,
        )?;

        Ok(udf.call(vec![domain_expr, range_expr, options_expr, values]))
    }

    /// Infer domain from data fields and return a new scale with the inferred domain
    pub async fn infer_domain_from_data(mut self) -> Result<Self, AvengerChartError> {
        use datafusion::prelude::SessionContext;

        if let ScaleDefaultDomain::DataFields(data_fields) = &self.domain.default_domain {
            let mut single_col_dfs: Vec<DataFrame> = Vec::new();

            for DataField { dataframe, field } in data_fields {
                let df = dataframe.clone();
                let df_with_field = df.as_ref().clone().select_columns(&[field])?;
                single_col_dfs.push(df_with_field);
            }

            // Union all of the single column dataframes
            let union_df = if single_col_dfs.len() > 1 {
                single_col_dfs
                    .iter()
                    .skip(1)
                    .fold(single_col_dfs[0].clone(), |acc, df| {
                        acc.union(df.clone()).unwrap()
                    })
            } else {
                single_col_dfs[0].clone()
            };

            // Determine the data type by checking the first field
            let first_field = &data_fields[0];
            let field_type = first_field
                .dataframe
                .schema()
                .field_with_name(None, &first_field.field)?
                .data_type();

            // Convert to band scale if we have string data and current scale is linear
            if field_type == &DataType::Utf8 && self.scale_impl.scale_type() == "linear" {
                // Convert to band scale for string data
                let data_fields_for_scale: Vec<(Arc<DataFrame>, String)> = data_fields
                    .iter()
                    .map(|field| (field.dataframe.clone(), field.field.clone()))
                    .collect();

                self = Scale::new(avenger_scales::scales::band::BandScale)
                    .domain_data_fields(data_fields_for_scale);

                // Set default band scale options if not already set
                if !self.options.contains_key("padding_inner") {
                    self = self.option("padding_inner", lit(0.1));
                }
                if !self.options.contains_key("padding_outer") {
                    self = self.option("padding_outer", lit(0.1));
                }
                if !self.options.contains_key("align") {
                    self = self.option("align", lit(0.5));
                }
            }

            // Determine the appropriate method based on scale type
            let method = self.scale_impl.infer_domain_from_data_method();

            // Use DataFrameChartUtils methods to get domain expression
            let domain_expr = match method {
                InferDomainFromDataMethod::Interval => union_df.span()?,
                InferDomainFromDataMethod::Unique => union_df.unique_values()?,
                InferDomainFromDataMethod::All => union_df.all_values()?,
            };

            // Now evaluate the domain expression
            let ctx = SessionContext::new();
            let empty_df = ctx.read_empty()?;
            let result_df = empty_df.select(vec![domain_expr.alias("domain")])?;
            let batches = result_df.collect().await?;
            let domain_array = batches[0].column_by_name("domain").unwrap().clone();

            // Convert the array to expressions based on the scale type
            if method == InferDomainFromDataMethod::Interval {
                // For interval domains, we expect a list array with a 2-element inner array
                if let Some(list_array) = domain_array.as_list_opt::<i32>() {
                    if list_array.len() > 0 {
                        let inner_array = list_array.value(0);
                        if inner_array.len() >= 2 {
                            // Extract min and max values
                            let min_val =
                                datafusion_common::ScalarValue::try_from_array(&inner_array, 0)?;
                            let max_val = datafusion_common::ScalarValue::try_from_array(
                                &inner_array,
                                inner_array.len() - 1,
                            )?;
                            self = self.domain_interval(lit(min_val), lit(max_val));
                        }
                    }
                }
            } else {
                // For unique/all domains, array_agg returns a ListArray
                if let Some(list_array) = domain_array.as_list_opt::<i32>() {
                    if list_array.len() > 0 {
                        let inner_array = list_array.value(0);
                        let mut values = Vec::new();
                        for i in 0..inner_array.len() {
                            let val =
                                datafusion_common::ScalarValue::try_from_array(&inner_array, i)?;
                            values.push(lit(val));
                        }
                        self = self.domain_discrete(values);
                    }
                } else {
                    // Fallback: treat as a simple array
                    let mut values = Vec::new();
                    for i in 0..domain_array.len() {
                        let val = datafusion_common::ScalarValue::try_from_array(&domain_array, i)?;
                        values.push(lit(val));
                    }
                    self = self.domain_discrete(values);
                }
            }
        }

        Ok(self)
    }

    /// Apply normalization (zero, nice, padding) to the scale domain
    pub fn normalize_domain(
        mut self,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Result<Self, AvengerChartError> {
        // Skip normalization for non-numeric ranges (e.g., color ranges)
        if !matches!(&self.range, ScaleRange::Numeric(_, _)) {
            return Ok(self);
        }

        // Create a ConfiguredScale to apply normalization
        let configured_scale = self.create_configured_scale(plot_area_width, plot_area_height)?;

        // Convert back to avenger-chart Scale with the normalized domain
        if let ScaleDefaultDomain::Interval(_start, _end) = &self.domain.default_domain {
            // Get the normalized domain from ConfiguredScale
            if let Ok((min, max)) = configured_scale.numeric_interval_domain() {
                self = self.domain_interval(lit(min as f64), lit(max as f64));
            }
        }

        Ok(self)
    }

    /// Create a ConfiguredScale from this Scale
    pub fn create_configured_scale(
        &self,
        plot_area_width: f32,
        _plot_area_height: f32,
    ) -> Result<avenger_scales::scales::ConfiguredScale, AvengerChartError> {
        use avenger_scales::scales::{ConfiguredScale, ScaleConfig, ScaleContext};
        use datafusion::arrow::array::{Float32Array, StringArray};

        // Extract domain values as arrow array
        let domain = match &self.domain.default_domain {
            ScaleDefaultDomain::Interval(start, end) => {
                let start_val = eval_to_scalar(start)?;
                let end_val = eval_to_scalar(end)?;

                // Convert to f32 values
                let start_f32 = match start_val {
                    ScalarValue::Float64(Some(v)) => v as f32,
                    ScalarValue::Float32(Some(v)) => v,
                    ScalarValue::Int64(Some(v)) => v as f32,
                    ScalarValue::Int32(Some(v)) => v as f32,
                    _ => {
                        return Err(AvengerChartError::InternalError(
                            "Scale domain must be numeric literals".to_string(),
                        ));
                    }
                };
                let end_f32 = match end_val {
                    ScalarValue::Float64(Some(v)) => v as f32,
                    ScalarValue::Float32(Some(v)) => v,
                    ScalarValue::Int64(Some(v)) => v as f32,
                    ScalarValue::Int32(Some(v)) => v as f32,
                    _ => {
                        return Err(AvengerChartError::InternalError(
                            "Scale domain must be numeric literals".to_string(),
                        ));
                    }
                };
                Arc::new(Float32Array::from(vec![start_f32, end_f32]))
                    as datafusion::arrow::array::ArrayRef
            }
            ScaleDefaultDomain::Discrete(values) => {
                // Extract string literals from the expressions
                let mut strings = Vec::new();
                for expr in values {
                    let scalar = eval_to_scalar(expr)?;
                    if let ScalarValue::Utf8(Some(s)) = scalar {
                        strings.push(s);
                    }
                }
                Arc::new(StringArray::from(strings)) as datafusion::arrow::array::ArrayRef
            }
            _ => {
                return Err(AvengerChartError::InternalError(
                    "Scale domain must be explicitly set".to_string(),
                ));
            }
        };

        // Extract range values as arrow array
        let range = match &self.range {
            ScaleRange::Numeric(start, end) => {
                let start_val = eval_to_scalar(start)?;
                let end_val = eval_to_scalar(end)?;

                let start_f32 = match start_val {
                    ScalarValue::Float64(Some(v)) => v as f32,
                    ScalarValue::Float32(Some(v)) => v,
                    ScalarValue::Int64(Some(v)) => v as f32,
                    ScalarValue::Int32(Some(v)) => v as f32,
                    _ => 0.0,
                };
                let end_f32 = match end_val {
                    ScalarValue::Float64(Some(v)) => v as f32,
                    ScalarValue::Float32(Some(v)) => v,
                    ScalarValue::Int64(Some(v)) => v as f32,
                    ScalarValue::Int32(Some(v)) => v as f32,
                    _ => plot_area_width,
                };
                Arc::new(Float32Array::from(vec![start_f32, end_f32]))
                    as datafusion::arrow::array::ArrayRef
            }
            ScaleRange::Color(colors) => {
                // Convert Vec<Srgba> to a list array of [f32; 4] arrays
                let color_arrays: Vec<datafusion::arrow::array::ArrayRef> = colors
                    .iter()
                    .map(|color| {
                        let rgba = [color.red, color.green, color.blue, color.alpha];
                        Arc::new(Float32Array::from(Vec::from(rgba)))
                            as datafusion::arrow::array::ArrayRef
                    })
                    .collect();

                // Create a ListArray from the color arrays
                avenger_scales::scalar::Scalar::arrays_into_list_array(color_arrays)?
            }
            ScaleRange::Enum(values) => {
                // Create a string array for enum values
                let strings: Vec<Option<&str>> = values
                    .iter()
                    .map(|v| match v {
                        ScalarValue::Utf8(Some(s)) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                Arc::new(StringArray::from(strings)) as datafusion::arrow::array::ArrayRef
            }
        };

        // Convert options to avenger_scales::scalar::Scalar
        let mut options = HashMap::new();
        for (key, expr) in &self.options {
            let scalar_val = eval_to_scalar(expr)?;
            let scalar = match scalar_val {
                ScalarValue::Float64(Some(v)) => avenger_scales::scalar::Scalar::from_f32(v as f32),
                ScalarValue::Float32(Some(v)) => avenger_scales::scalar::Scalar::from_f32(v),
                ScalarValue::Int64(Some(v)) => avenger_scales::scalar::Scalar::from_f32(v as f32),
                ScalarValue::Int32(Some(v)) => avenger_scales::scalar::Scalar::from_f32(v as f32),
                ScalarValue::Boolean(Some(v)) => avenger_scales::scalar::Scalar::from_bool(v),
                _ => continue,
            };
            options.insert(key.clone(), scalar);
        }

        let config = ScaleConfig {
            domain,
            range,
            options,
            context: ScaleContext::default(),
        };

        let configured_scale = ConfiguredScale {
            scale_impl: self.scale_impl.clone(),
            config,
        };

        // Normalize the scale to apply zero and nice transformations
        Ok(configured_scale.normalize()?)
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
    // Domain derived from encoding expressions
    Expressions(Vec<Expr>),
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

    pub fn new_expressions(expressions: Vec<Expr>) -> Self {
        Self {
            default_domain: ScaleDefaultDomain::Expressions(expressions),
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
            ScaleDefaultDomain::Expressions(exprs) => {
                if exprs.is_empty() {
                    return Err(AvengerChartError::InternalError(
                        "Expressions domain may not be empty".to_string(),
                    ));
                }
                Ok(exprs[0].get_type(&schema)?)
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
            ScaleDefaultDomain::DataField(DataField { dataframe, field }) => {
                use crate::utils::DataFrameChartUtils;
                let df_with_field = dataframe.as_ref().clone().select_columns(&[field])?;

                match method {
                    InferDomainFromDataMethod::Interval => df_with_field.span()?,
                    InferDomainFromDataMethod::Unique => df_with_field.unique_values()?,
                    InferDomainFromDataMethod::All => df_with_field.all_values()?,
                }
            }
            ScaleDefaultDomain::DataFields(data_fields) => {
                use crate::utils::DataFrameChartUtils;
                let mut single_col_dfs: Vec<DataFrame> = Vec::new();

                for DataField { dataframe, field } in data_fields {
                    let df = dataframe.clone();
                    let df_with_field = df.as_ref().clone().select_columns(&[field])?;
                    single_col_dfs.push(df_with_field);
                }

                // Union all of the single column dataframes
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
            ScaleDefaultDomain::Expressions(_expressions) => {
                // For expressions, we can't create a DataFrame here since we don't have
                // the actual data. Instead, we'll return a placeholder that will be
                // evaluated at runtime when we have the actual DataFrames.
                // This is a limitation we'll need to address by passing DataFrames
                // to the compile method or deferring compilation.
                return Err(AvengerChartError::InternalError(
                    "Expression-based domain inference requires runtime evaluation".to_string(),
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

/// Create a scale implementation based on scale type name
fn create_scale_impl(scale_type: &str) -> Arc<dyn ScaleImpl> {
    match scale_type {
        "linear" => Arc::new(LinearScale),
        "log" | "logarithmic" => Arc::new(LogScale),
        "pow" | "power" => Arc::new(PowScale),
        "sqrt" => {
            // sqrt is pow with exponent 0.5, but PowScale will handle this via options
            Arc::new(PowScale)
        }
        "symlog" => Arc::new(SymlogScale),
        "time" | "temporal" => Arc::new(TimeScale),
        "band" => Arc::new(BandScale),
        "point" => Arc::new(PointScale),
        "ordinal" => Arc::new(OrdinalScale),
        "threshold" => Arc::new(ThresholdScale),
        "quantile" => Arc::new(QuantileScale),
        "quantize" => Arc::new(QuantizeScale),
        _ => {
            eprintln!("Unknown scale type '{}', defaulting to linear", scale_type);
            Arc::new(LinearScale)
        }
    }
}

/// Apply default options for each scale type
fn apply_scale_defaults(scale_type: &str, options: &mut HashMap<String, Expr>) {
    match scale_type {
        "band" => {
            options
                .entry("padding_inner".to_string())
                .or_insert(lit(0.1));
            options
                .entry("padding_outer".to_string())
                .or_insert(lit(0.1));
            options.entry("align".to_string()).or_insert(lit(0.5));
        }
        "point" => {
            options.entry("padding".to_string()).or_insert(lit(0.5));
            options.entry("align".to_string()).or_insert(lit(0.5));
        }
        "log" | "logarithmic" => {
            options.entry("base".to_string()).or_insert(lit(10.0));
        }
        "pow" | "power" => {
            options.entry("exponent".to_string()).or_insert(lit(1.0));
        }
        "sqrt" => {
            // sqrt is pow with exponent 0.5
            options.insert("exponent".to_string(), lit(0.5));
        }
        "symlog" => {
            options.entry("constant".to_string()).or_insert(lit(1.0));
        }
        "linear" => {
            // Linear scales might have nice=true by default
            options.entry("nice".to_string()).or_insert(lit(true));
        }
        _ => {
            // No specific defaults for other scale types
        }
    }
}
