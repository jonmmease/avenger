pub mod color_defaults;
pub mod shape_defaults;
// pub mod defaults;
pub mod inference;
pub mod udf;

use crate::error::AvengerChartError;
use crate::utils::{DataFrameChartHelpers, eval_to_scalars};
use avenger_scales::scales::{InferDomainFromDataMethod, ScaleImpl};
use datafusion::arrow::array::{Array, AsArray};
use datafusion::arrow::datatypes::DataType;
use datafusion::dataframe::DataFrame;
use datafusion::functions_array::expr_fn::make_array;
use datafusion::logical_expr::{Expr, ExprSchemable, cast, col, lit, when};
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

        // // Create appropriate default domain based on scale type
        // let domain = match scale_type {
        //     "ordinal" | "band" | "point" => ScaleDomain::new_discrete(vec![]),
        //     _ => ScaleDomain::new_interval(lit(0.0), lit(1.0)),
        // };

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
        let old_type = self.scale_impl.scale_type();
        self.scale_impl = create_scale_impl(scale_type);
        let new_type = self.scale_impl.scale_type();

        // Clear existing options and apply new defaults
        if old_type != new_type {
            self.options.clear();
        }
        apply_scale_defaults(scale_type, &mut self.options);
        self
    }

    pub fn get_scale_impl(&self) -> &Arc<dyn ScaleImpl> {
        &self.scale_impl
    }

    /// Get the scale type name
    pub fn get_scale_type(&self) -> &str {
        self.scale_impl.scale_type()
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

    /// Get the cardinality of the domain for discrete scales
    pub fn get_domain_cardinality(&self) -> Option<usize> {
        match &self.domain.default_domain {
            ScaleDefaultDomain::Discrete(values) => Some(values.len()),
            _ => None,
        }
    }

    pub fn domain_interval<T: Into<Expr>>(self, start: T, end: T) -> Self {
        self.domain(ScaleDomain::new_interval(start, end))
    }

    pub fn domain_discrete<T: Into<Expr>>(self, values: Vec<T>) -> Self {
        let exprs: Vec<Expr> = values.into_iter().map(|v| v.into()).collect();
        self.domain(ScaleDomain::new_discrete(exprs))
    }

    pub fn domain_data_field(self, dataframe: Arc<DataFrame>, expr: Expr) -> Self {
        self.domain(ScaleDomain::new_data_field(dataframe, expr))
    }

    pub fn domain_data_field_with_radius(
        self,
        dataframe: Arc<DataFrame>,
        expr: Expr,
        radius: Expr,
    ) -> Self {
        self.domain(ScaleDomain::new_data_field_with_radius(
            dataframe, expr, radius,
        ))
    }

    pub fn domain_data_fields(self, fields: Vec<(Arc<DataFrame>, Expr)>) -> Self {
        self.domain(ScaleDomain::new_data_fields(fields))
    }

    /// Internal method to set domain for data fields without marking as explicit
    pub(crate) fn domain_data_fields_internal(
        mut self,
        fields: Vec<(Arc<DataFrame>, Expr)>,
    ) -> Self {
        self.domain = ScaleDomain::new_data_fields(fields);
        self
    }

    /// Internal method to set domain for data fields with radius without marking as explicit
    pub(crate) fn domain_data_fields_with_radius_internal(
        mut self,
        fields: Vec<(Arc<DataFrame>, Expr, Option<crate::marks::RadiusExpression>)>,
    ) -> Self {
        self.domain = ScaleDomain {
            default_domain: ScaleDefaultDomain::DomainExprs(
                fields
                    .into_iter()
                    .map(|(dataframe, expr, radius)| DomainExpr {
                        dataframe,
                        expr,
                        radius,
                    })
                    .collect(),
            ),
            raw_domain: None,
        };
        self
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

    // Clip padding builders - for backward compatibility, padding sets both lower and upper
    pub fn padding<E: Into<Expr>>(mut self, expr: E) -> Self {
        let padding_expr = expr.into();
        self.options
            .insert("clip_padding_lower".to_string(), padding_expr.clone());
        self.options
            .insert("clip_padding_upper".to_string(), padding_expr);
        self
    }

    pub fn clip_padding_lower<E: Into<Expr>>(mut self, expr: E) -> Self {
        self.options
            .insert("clip_padding_lower".to_string(), expr.into());
        self
    }

    pub fn clip_padding_upper<E: Into<Expr>>(mut self, expr: E) -> Self {
        self.options
            .insert("clip_padding_upper".to_string(), expr.into());
        self
    }

    pub fn padding_none(mut self) -> Self {
        self.options.remove("clip_padding_lower");
        self.options.remove("clip_padding_upper");
        self
    }

    pub fn has_explicit_padding(&self) -> bool {
        self.options.contains_key("clip_padding_lower")
            || self.options.contains_key("clip_padding_upper")
    }

    pub fn get_padding(&self) -> Option<&Expr> {
        // For backward compatibility, return lower padding if it exists
        self.options.get("clip_padding_lower")
    }

    // Nice option for numeric scales
    pub fn nice<E: Into<Expr>>(mut self, value: E) -> Self {
        self.options.insert("nice".to_string(), value.into());
        self
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
        let values = cast(values, domain_type.clone());

        let udf = udf::create_scale_udf(
            self.scale_impl.clone(),
            domain_type,
            range_type,
            options_type,
        )?;

        Ok(udf.call(vec![domain_expr, range_expr, options_expr, values]))
    }

    /// Infer domain from data fields and return a new scale with the inferred domain
    ///
    /// # Arguments
    /// * `range_hint` - Optional range to use for radius-aware padding calculations.
    ///   If not provided, padding calculations will be skipped.
    pub async fn infer_domain_from_data(
        mut self,
        range_hint: Option<(f64, f64)>,
    ) -> Result<Self, AvengerChartError> {
        use datafusion::prelude::SessionContext;

        // Use std::mem::replace to avoid partial move
        let default_domain = std::mem::replace(
            &mut self.domain.default_domain,
            ScaleDefaultDomain::Discrete(vec![]),
        );
        if let ScaleDefaultDomain::DomainExprs(mut data_fields) = default_domain {
            // Process DomainExprs with radius data first
            let mut i = 0;
            while i < data_fields.len() {
                let field = &data_fields[i];

                if let Some(radius_expr) = &field.radius {
                    if self.scale_impl.scale_type() == "linear" && range_hint.is_some() {
                        // Handle radius-aware domain calculation
                        let (range_min, range_max) = range_hint.unwrap();
                        let range_width = (range_max - range_min).abs();

                        let df = field.dataframe.clone();

                        // Select both position and radius expressions based on RadiusExpression type
                        let select_exprs = match radius_expr {
                            crate::marks::RadiusExpression::Symmetric(expr) => {
                                vec![
                                    field.expr.clone().alias("__position__"),
                                    expr.clone().alias("__radius_lower__"),
                                    expr.clone().alias("__radius_upper__"),
                                ]
                            }
                            crate::marks::RadiusExpression::Asymmetric { lower, upper } => {
                                vec![
                                    field.expr.clone().alias("__position__"),
                                    lower.clone().alias("__radius_lower__"),
                                    upper.clone().alias("__radius_upper__"),
                                ]
                            }
                        };

                        let df_with_exprs = df.as_ref().clone().select(select_exprs)?;
                        let batches = df_with_exprs.collect().await?;

                        if !batches.is_empty() && batches[0].num_rows() > 0 {
                            let batch = &batches[0];
                            let position_array = batch.column_by_name("__position__").unwrap();
                            let radius_lower_array =
                                batch.column_by_name("__radius_lower__").unwrap();
                            let radius_upper_array =
                                batch.column_by_name("__radius_upper__").unwrap();

                            // Cast to Float64 using arrow's cast function
                            use datafusion::arrow::compute::cast;
                            use datafusion::arrow::datatypes::DataType as ArrowDataType;

                            let position_f64 = cast(position_array, &ArrowDataType::Float64)?;
                            let radius_lower_f64 =
                                cast(radius_lower_array, &ArrowDataType::Float64)?;
                            let radius_upper_f64 =
                                cast(radius_upper_array, &ArrowDataType::Float64)?;

                            // Extract values as slices
                            use datafusion::arrow::array::AsArray;
                            use datafusion::arrow::datatypes::Float64Type;

                            let positions = position_f64.as_primitive::<Float64Type>();
                            let radius_lower = radius_lower_f64.as_primitive::<Float64Type>();
                            let radius_upper = radius_upper_f64.as_primitive::<Float64Type>();

                            // Convert to vectors, filtering out nulls
                            let position_vec: Vec<f64> = positions.iter().flatten().collect();
                            let radius_lower_vec: Vec<f64> =
                                radius_lower.iter().flatten().collect();
                            let radius_upper_vec: Vec<f64> =
                                radius_upper.iter().flatten().collect();

                            // Ensure matching lengths
                            let min_len = position_vec
                                .len()
                                .min(radius_lower_vec.len())
                                .min(radius_upper_vec.len());
                            let positions_slice = &position_vec[..min_len];
                            let radius_lower_slice = &radius_lower_vec[..min_len];
                            let radius_upper_slice = &radius_upper_vec[..min_len];

                            // Use the padding solver
                            use avenger_scales::scales::domain_solver::compute_domain_from_data_with_padding_linear;

                            let (d_min, d_max) = compute_domain_from_data_with_padding_linear(
                                positions_slice,
                                radius_lower_slice,
                                radius_upper_slice,
                                range_width,
                            )?;

                            // Create a new DataFrame with the computed domain
                            use datafusion::arrow::array::Float64Array;
                            use datafusion::arrow::datatypes::{DataType, Field, Schema};
                            use datafusion::arrow::record_batch::RecordBatch;

                            let domain_array = Float64Array::from(vec![d_min, d_max]);
                            let schema = Arc::new(Schema::new(vec![Field::new(
                                "__domain__",
                                DataType::Float64,
                                false,
                            )]));
                            let batch = RecordBatch::try_new(schema, vec![Arc::new(domain_array)])?;

                            let ctx = SessionContext::new();
                            let domain_df = Arc::new(ctx.read_batch(batch)?);

                            // Replace the DomainExpr with the computed domain
                            data_fields[i] = DomainExpr {
                                dataframe: domain_df,
                                expr: col("__domain__"),
                                radius: None,
                            };
                        }
                    }
                }
                i += 1;
            }

            // Now proceed with standard domain inference logic if we have remaining data fields
            if data_fields.is_empty() {
                // All fields were processed as radius-aware, nothing more to do
                return Ok(self);
            }

            let mut single_col_dfs: Vec<DataFrame> = Vec::new();

            for DomainExpr {
                dataframe,
                expr,
                radius: _,
            } in &data_fields
            {
                let df = dataframe.clone();
                // Select the expression and alias it to a consistent column name
                let df_with_expr = df
                    .as_ref()
                    .clone()
                    .select(vec![expr.clone().alias("__domain_col__")])?;
                single_col_dfs.push(df_with_expr);
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
            let schema = first_field.dataframe.schema();
            let df_schema = schema.clone();
            let _field_type = first_field.expr.get_type(&df_schema)?;

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
                let Some(list_array) = domain_array.as_list_opt::<i32>() else {
                    return Err(AvengerChartError::InternalError(
                        "Expected domain to be a ListArray".to_string(),
                    ));
                };
                if list_array.len() > 0 {
                    let inner_array = list_array.value(0);
                    if inner_array.len() >= 2 {
                        // Extract min and max values
                        let min_val = ScalarValue::try_from_array(&inner_array, 0)?;
                        let max_val =
                            ScalarValue::try_from_array(&inner_array, inner_array.len() - 1)?;
                        // Set domain without marking as explicit since this is inferred
                        self.domain = ScaleDomain::new_interval(lit(min_val), lit(max_val));
                    }
                }
            } else {
                let Some(list_array) = domain_array.as_list_opt::<i32>() else {
                    return Err(AvengerChartError::InternalError(
                        "Expected domain to be a ListArray".to_string(),
                    ));
                };
                if list_array.len() > 0 {
                    let inner_array = list_array.value(0);
                    let mut values = Vec::new();
                    for i in 0..inner_array.len() {
                        let val = datafusion_common::ScalarValue::try_from_array(&inner_array, i)?;
                        values.push(lit(val));
                    }
                    // Set domain without marking as explicit since this is inferred
                    self.domain = ScaleDomain::new_discrete(values);
                }
            }
        } else {
            // Restore the default domain if it wasn't DomainExprs
            self.domain.default_domain = default_domain;
        }

        Ok(self)
    }

    /// Apply normalization (zero, nice, padding) to the scale domain
    pub async fn normalize_domain(
        mut self,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Result<Self, AvengerChartError> {
        // Skip normalization for non-numeric ranges (e.g., color ranges)
        if !matches!(&self.range, ScaleRange::Numeric(_, _)) {
            return Ok(self);
        }

        // Skip normalization for non-numeric domains (e.g., ordinal scales)
        if !matches!(
            &self.domain.default_domain,
            ScaleDefaultDomain::Interval(_, _)
        ) {
            return Ok(self);
        }

        // Create a ConfiguredScale to apply normalization
        let configured_scale = self
            .create_configured_scale(plot_area_width, plot_area_height)
            .await?;

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
    pub async fn create_configured_scale(
        &self,
        _plot_area_width: f32,
        _plot_area_height: f32,
    ) -> Result<avenger_scales::scales::ConfiguredScale, AvengerChartError> {
        use crate::utils::ScalarValueHelpers;
        use avenger_scales::scales::{ConfiguredScale, ScaleConfig, ScaleContext};
        use datafusion::arrow::array::{Float32Array, StringArray};

        // Extract domain values as arrow array
        let domain = match &self.domain.default_domain {
            ScaleDefaultDomain::Interval(start, end) => {
                let scalars =
                    eval_to_scalars(vec![start.clone(), end.as_ref().clone()], None, None).await?;
                let [start_val, end_val] = scalars.as_slice() else {
                    return Err(AvengerChartError::InternalError(
                        "Expected two scalar values for interval domain".to_string(),
                    ));
                };

                // Convert to f32 values
                let start_f32 = start_val.as_f32()?;
                let end_f32 = end_val.as_f32()?;
                Arc::new(Float32Array::from(vec![start_f32, end_f32])) as arrow::array::ArrayRef
            }
            ScaleDefaultDomain::Discrete(values) => {
                // Extract string literals from the expressions
                let mut strings = Vec::new();
                let scalars = eval_to_scalars(values.clone(), None, None).await?;
                for scalar in scalars {
                    if let ScalarValue::Utf8(Some(s)) = scalar {
                        strings.push(s);
                    }
                }
                Arc::new(StringArray::from(strings)) as arrow::array::ArrayRef
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
                let scalars =
                    eval_to_scalars(vec![start.clone(), end.as_ref().clone()], None, None).await?;

                let [start_val, end_val] = scalars.as_slice() else {
                    return Err(AvengerChartError::InternalError(
                        "Expected two scalar values for numeric range".to_string(),
                    ));
                };

                let start_f32 = start_val.as_f32()?;
                let end_f32 = end_val.as_f32()?;
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
                // Convert the scalar values to an array
                ScalarValue::iter_to_array(values.iter().cloned())?
            }
        };

        // Eval scalars options values and convert them to avenger_scales::scalar::Scalar
        let (names, exprs): (Vec<_>, Vec<_>) = self.options.clone().into_iter().unzip();
        let scalars = eval_to_scalars(exprs, None, None)
            .await?
            .into_iter()
            .map(|s| s.as_scale_scalar())
            .collect::<Result<Vec<_>, _>>()?;
        let mut options = names.into_iter().zip(scalars).collect::<HashMap<_, _>>();

        // Add clip_padding options if specified
        if self.has_explicit_padding() {
            // Handle clip_padding_lower
            if let Some(expr) = self.options.get("clip_padding_lower") {
                let scalar_val = eval_to_scalars(vec![expr.clone()], None, None).await?;
                if let Some(val) = scalar_val.first() {
                    let padding_scalar = val.as_scale_scalar()?;
                    options.insert("clip_padding_lower".to_string(), padding_scalar);
                }
            } else if self.options.contains_key("clip_padding_upper") {
                // If upper is set but not lower, default lower to 0
                options.insert(
                    "clip_padding_lower".to_string(),
                    avenger_scales::scalar::Scalar::from_f32(0.0),
                );
            }

            // Handle clip_padding_upper
            if let Some(expr) = self.options.get("clip_padding_upper") {
                let scalar_val = eval_to_scalars(vec![expr.clone()], None, None).await?;
                if let Some(val) = scalar_val.first() {
                    let padding_scalar = val.as_scale_scalar()?;
                    options.insert("clip_padding_upper".to_string(), padding_scalar);
                }
            } else if self.options.contains_key("clip_padding_lower") {
                // If lower is set but not upper, default upper to 0
                options.insert(
                    "clip_padding_upper".to_string(),
                    avenger_scales::scalar::Scalar::from_f32(0.0),
                );
            }
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
        Ok(configured_scale)
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
    DomainExprs(Vec<DomainExpr>),
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
                use crate::utils::DataFrameChartHelpers;
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

#[derive(Debug, Clone)]
pub struct DomainExpr {
    pub dataframe: Arc<DataFrame>,
    pub expr: Expr,
    pub radius: Option<crate::marks::RadiusExpression>,
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
            options.entry("padding".to_string()).or_insert(lit(0.1));
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
