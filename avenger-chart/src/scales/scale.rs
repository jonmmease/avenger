use crate::error::AvengerChartError;
use crate::scales::domain::{DomainExpr, ScaleDefaultDomain, ScaleDomain};
use crate::scales::domain_inference::DomainInferrer;
use crate::scales::range::ScaleRange;
use crate::utils::{ScalarValueHelpers, eval_to_scalars};
use avenger_scales::scales::ScaleImpl;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
use palette::Srgba;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Scale {
    pub scale_impl: Arc<dyn ScaleImpl>,
    pub domain: ScaleDomain,
    pub range: ScaleRange,
    pub options: HashMap<String, Expr>,
}

impl Scale {
    pub fn new<S: ScaleImpl>(scale_impl: S) -> Self {
        let scale_type = scale_impl.scale_type();

        // Create appropriate default domain based on scale type
        let domain = match scale_type {
            "band" | "point" | "ordinal" => ScaleDomain::new_discrete(vec![]),
            _ => ScaleDomain::new_interval(lit(0.0), lit(1.0)),
        };

        // Get default options from the scale implementation
        let default_options = scale_impl.default_options();
        let mut options = HashMap::new();
        
        // Convert Scalar values to Expr values
        for (key, scalar) in default_options {
            // Convert the avenger_scales::scalar::Scalar to a datafusion ScalarValue
            let scalar_value = if let Ok(b) = scalar.as_boolean() {
                ScalarValue::Boolean(Some(b))
            } else if let Ok(f) = scalar.as_f32() {
                ScalarValue::Float32(Some(f))
            } else if let Ok(i) = scalar.as_i32() {
                ScalarValue::Int32(Some(i))
            } else if let Ok(s) = scalar.as_string() {
                ScalarValue::Utf8(Some(s))
            } else {
                ScalarValue::Null
            };
            options.insert(key, lit(scalar_value));
        }

        Self {
            scale_impl: Arc::new(scale_impl),
            domain,
            range: ScaleRange::new_interval(lit(0.0), lit(1.0)),
            options,
        }
    }

    /// Create a scale with default linear type
    pub fn default() -> Self {
        use avenger_scales::scales::linear::LinearScale;
        Self::new(LinearScale)
    }

    /// Create a scale with an Arc<dyn ScaleImpl>
    pub fn with_impl(scale_impl: Arc<dyn ScaleImpl>) -> Self {
        let scale_type = scale_impl.scale_type();

        // Create appropriate default domain based on scale type
        let domain = match scale_type {
            "band" | "point" | "ordinal" => ScaleDomain::new_discrete(vec![]),
            _ => ScaleDomain::new_interval(lit(0.0), lit(1.0)),
        };

        // Get default options from the scale implementation
        let default_options = scale_impl.default_options();
        let mut options = HashMap::new();
        
        // Convert Scalar values to Expr values
        for (key, scalar) in default_options {
            // Convert the avenger_scales::scalar::Scalar to a datafusion ScalarValue
            let scalar_value = if let Ok(b) = scalar.as_boolean() {
                ScalarValue::Boolean(Some(b))
            } else if let Ok(f) = scalar.as_f32() {
                ScalarValue::Float32(Some(f))
            } else if let Ok(i) = scalar.as_i32() {
                ScalarValue::Int32(Some(i))
            } else if let Ok(s) = scalar.as_string() {
                ScalarValue::Utf8(Some(s))
            } else {
                ScalarValue::Null
            };
            options.insert(key, lit(scalar_value));
        }

        Self {
            scale_impl,
            domain,
            range: ScaleRange::new_interval(lit(0.0), lit(1.0)),
            options,
        }
    }

    /// Set the scale type, replacing the current implementation
    pub fn scale_type<S: ScaleImpl>(mut self, scale_impl: S) -> Self {
        let old_type = self.scale_impl.scale_type();
        let new_type = scale_impl.scale_type();

        // Clear existing options and apply new defaults if type changed
        if old_type != new_type {
            self.options.clear();
            
            // Get default options from the scale implementation
            let default_options = scale_impl.default_options();
            
            // Convert Scalar values to Expr values
            for (key, scalar) in default_options {
                // Convert the avenger_scales::scalar::Scalar to a datafusion ScalarValue
                let scalar_value = if let Ok(b) = scalar.as_boolean() {
                    ScalarValue::Boolean(Some(b))
                } else if let Ok(f) = scalar.as_f32() {
                    ScalarValue::Float32(Some(f))
                } else if let Ok(i) = scalar.as_i32() {
                    ScalarValue::Int32(Some(i))
                } else if let Ok(s) = scalar.as_string() {
                    ScalarValue::Utf8(Some(s))
                } else {
                    ScalarValue::Null
                };
                self.options.insert(key, lit(scalar_value));
            }
        }
        
        self.scale_impl = Arc::new(scale_impl);
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

    /// Set domain from data fields with optional radius expressions for padding calculations
    ///
    /// **Note**: This is a low-level method for staged processing where radius information
    /// from non-positional scales needs to be incorporated into positional scale domains.
    #[doc(hidden)]
    pub fn domain_data_fields_with_radius(
        self,
        fields: Vec<(Arc<DataFrame>, Expr, Option<crate::marks::RadiusExpression>)>,
    ) -> Self {
        let domain = ScaleDomain {
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
        self.domain(domain)
    }

    pub fn raw_domain<E: Clone + Into<Expr>>(self, raw_domain: E) -> Self {
        let new_domain = self.domain.clone().with_raw(raw_domain.into());
        self.domain(new_domain)
    }

    // Range builders
    pub fn range(self, range: ScaleRange) -> Self {
        Self { range, ..self }
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

    /// Infer domain from data fields and return a new scale with the inferred domain
    ///
    /// **Note**: This is a low-level method. Most users should use `PlotRenderer::build_scale_with_context()` instead.
    /// This method is exposed for advanced staged processing scenarios and testing.
    ///
    /// # Arguments
    /// * `range_hint` - Optional range to use for radius-aware padding calculations.
    ///   If not provided, padding calculations will be skipped.
    #[doc(hidden)]
    pub async fn infer_domain_from_data(
        mut self,
        range_hint: Option<(f64, f64)>,
    ) -> Result<Self, AvengerChartError> {
        // Use the DomainInferrer to handle the complex logic
        self.domain = DomainInferrer::infer(&self.scale_impl, self.domain, range_hint).await?;
        Ok(self)
    }

    /// Apply normalization (zero, nice, padding) to the scale domain
    ///
    /// **Note**: This is a low-level method. Most users should use `PlotRenderer::build_scale_with_context()` instead.
    /// This method is exposed for advanced staged processing scenarios and testing.
    #[doc(hidden)]
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
    ///
    /// **Note**: This is a low-level method. Most users should use `PlotRenderer::build_scale_with_context()` instead.
    /// This method is exposed for advanced staged processing scenarios and testing.
    #[doc(hidden)]
    pub async fn create_configured_scale(
        &self,
        _plot_area_width: f32,
        _plot_area_height: f32,
    ) -> Result<avenger_scales::scales::ConfiguredScale, AvengerChartError> {
        use avenger_scales::scales::{ConfiguredScale, ScaleConfig, ScaleContext};
        use datafusion::arrow::array::{ArrayRef, Float32Array, StringArray};

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
                Arc::new(Float32Array::from(vec![start_f32, end_f32])) as ArrayRef
            }
            ScaleDefaultDomain::Discrete(values) => {
                // For threshold scales, we need numeric values
                // For other discrete scales (ordinal, band, point), we need strings
                let scalars = eval_to_scalars(values.clone(), None, None).await?;

                if self.scale_impl.scale_type() == "threshold" {
                    // Threshold scales need numeric domain values
                    let mut floats = Vec::new();
                    for scalar in scalars {
                        let f = scalar.as_f32()?;
                        floats.push(f);
                    }
                    Arc::new(Float32Array::from(floats)) as ArrayRef
                } else {
                    // Other discrete scales need string values
                    let mut strings = Vec::new();
                    for scalar in scalars {
                        if let ScalarValue::Utf8(Some(s)) = scalar {
                            strings.push(s);
                        } else {
                            // Convert non-strings to strings
                            strings.push(format!("{:?}", scalar));
                        }
                    }
                    Arc::new(StringArray::from(strings)) as ArrayRef
                }
            }
            _ => {
                return Err(AvengerChartError::InternalError(format!(
                    "Scale domain must be explicitly set. Domain type: {:?}",
                    self.domain.default_domain
                )));
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
                Arc::new(Float32Array::from(vec![start_f32, end_f32])) as ArrayRef
            }
            ScaleRange::Color(colors) => {
                // Convert Vec<Srgba> to a list array of [f32; 4] arrays
                let color_arrays: Vec<ArrayRef> = colors
                    .iter()
                    .map(|color| {
                        let rgba = [color.red, color.green, color.blue, color.alpha];
                        Arc::new(Float32Array::from(Vec::from(rgba))) as ArrayRef
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
}
