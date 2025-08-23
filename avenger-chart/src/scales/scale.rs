//! Type-safe scale system with compile-time method resolution

use crate::error::AvengerChartError;
use crate::scales::domain::{DomainExpr, ScaleDefaultDomain, ScaleDomain};
use crate::scales::domain_inference::DomainInferrer;
use crate::scales::range::ScaleRange;
use crate::scales::spec::*;
use crate::utils::{ScalarValueHelpers, eval_to_scalars};
use avenger_scales::scales::ScaleImpl;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
use palette::Srgba;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

/// Type-safe scale with compile-time method resolution
#[derive(Debug, Clone)]
pub struct Scale<S: ScaleSpec = Auto> {
    pub(crate) scale_impl: Arc<dyn ScaleImpl>,
    pub(crate) domain: ScaleDomain,
    pub(crate) range: ScaleRange,
    options: HashMap<String, Expr>,
    pub(crate) _phantom: PhantomData<S>,
}

// ===== Generic methods available on ALL scales =====
impl<S: ScaleSpec> Default for Scale<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: ScaleSpec> Scale<S> {
    /// Create a new scale of this type
    pub fn new() -> Self {
        let scale_impl = S::create_impl();

        // Use the scale's domain inference method to determine domain type
        // This is more extensible than hard-coding scale names
        use avenger_scales::scales::InferDomainFromDataMethod;
        let domain = match scale_impl.infer_domain_from_data_method() {
            InferDomainFromDataMethod::Unique => ScaleDomain::new_discrete(vec![]),
            InferDomainFromDataMethod::Interval => ScaleDomain::new_interval(lit(0.0), lit(1.0)),
            InferDomainFromDataMethod::All => ScaleDomain::new_discrete(vec![]), // Treat All as discrete
        };

        // Get default options from the scale implementation
        let default_options = scale_impl.default_options();
        let mut options = HashMap::new();

        // Convert Scalar values to Expr values
        for (key, scalar) in default_options {
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
            _phantom: PhantomData,
        }
    }

    /// Set the domain
    pub fn domain<D: Into<ScaleDomain>>(mut self, domain: D) -> Self {
        self.domain = domain.into();
        self
    }

    /// Set the domain as an interval
    pub fn domain_interval(mut self, min: impl Into<Expr>, max: impl Into<Expr>) -> Self {
        self.domain = ScaleDomain::new_interval(min.into(), max.into());
        self
    }

    /// Set the domain as discrete values
    pub fn domain_discrete(mut self, values: Vec<impl Into<Expr>>) -> Self {
        self.domain = ScaleDomain::new_discrete(values.into_iter().map(|v| v.into()).collect());
        self
    }

    /// Set domain from data field
    pub fn domain_data(mut self, dataframe: Arc<DataFrame>, expr: Expr) -> Self {
        self.domain.default_domain = ScaleDefaultDomain::DomainExprs(vec![DomainExpr {
            dataframe,
            expr,
            radius: None,
        }]);
        self
    }

    /// Set domain from data fields
    pub fn domain_data_fields(mut self, fields: Vec<(Arc<DataFrame>, Expr)>) -> Self {
        let exprs = fields
            .into_iter()
            .map(|(df, expr)| DomainExpr {
                dataframe: df,
                expr,
                radius: None,
            })
            .collect();
        self.domain.default_domain = ScaleDefaultDomain::DomainExprs(exprs);
        self
    }

    /// Set domain from data fields with radius expressions
    pub fn domain_data_fields_with_radius(
        mut self,
        fields: Vec<(Arc<DataFrame>, Expr, Option<crate::marks::RadiusExpression>)>,
    ) -> Self {
        let exprs = fields
            .into_iter()
            .map(|(df, expr, radius)| DomainExpr {
                dataframe: df,
                expr,
                radius,
            })
            .collect();
        self.domain.default_domain = ScaleDefaultDomain::DomainExprs(exprs);
        self
    }

    /// Set the range
    pub fn range(mut self, range: ScaleRange) -> Self {
        self.range = range;
        self
    }

    /// Set the range as an interval
    pub fn range_interval(mut self, min: impl Into<Expr>, max: impl Into<Expr>) -> Self {
        self.range = ScaleRange::new_interval(min.into(), max.into());
        self
    }

    /// Set the range as discrete values from ScalarValues
    pub fn range_discrete(mut self, values: Vec<impl Into<ScalarValue>>) -> Self {
        self.range = ScaleRange::new_discrete(values.into_iter().map(|v| v.into()).collect());
        self
    }

    /// Set the range as colors
    pub fn range_colors(mut self, colors: Vec<Srgba>) -> Self {
        self.range = ScaleRange::new_color(colors);
        self
    }

    /// Change the scale type by providing a new implementation
    pub fn scale_type(mut self, scale_impl: impl ScaleImpl + 'static) -> Self {
        self.scale_impl = Arc::new(scale_impl);
        self
    }

    pub fn get_scale_type(&self) -> &str {
        self.scale_impl.scale_type()
    }

    /// Internal method for setting options
    /// This is intentionally undocumented and prefixed with _ to discourage direct use.
    /// External crates implementing custom scales can use this for their typed methods.
    /// Regular users should use the typed methods or Scale<Auto>::option()
    #[doc(hidden)]
    pub fn _option(mut self, key: impl Into<String>, value: impl Into<Expr>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    /// Convert to a different scale type
    pub fn into_type<T: ScaleSpec>(self) -> Scale<T> {
        // Check if we're changing scale type before moving values
        let changing_type =
            T::name() != "auto" && self.scale_impl.scale_type() != T::create_impl().scale_type();

        // For Auto, preserve the existing scale implementation
        // For other types, create the appropriate implementation
        let scale_impl = if T::name() == "auto" {
            // Preserve the existing scale implementation for Auto
            self.scale_impl
        } else {
            // Create new implementation for specific types
            T::create_impl()
        };

        // When changing scale types, only preserve options that are valid for the new scale
        // This prevents issues like LinearScale's "nice" option being passed to ThresholdScale
        let options = if !changing_type {
            // Same scale type, keep all options
            self.options
        } else {
            // Different scale type, start with new scale's default options
            // User can override these with the builder methods
            let default_options = scale_impl.default_options();
            let mut new_options = HashMap::new();

            // Convert Scalar values to Expr values
            for (key, scalar) in default_options {
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
                new_options.insert(key, lit(scalar_value));
            }
            new_options
        };

        Scale {
            scale_impl,
            domain: self.domain,
            range: self.range,
            options,
            _phantom: PhantomData,
        }
    }

    /// Convert to Auto type for storage
    pub fn into_auto(self) -> Scale<Auto> {
        Scale {
            scale_impl: self.scale_impl,
            domain: self.domain,
            range: self.range,
            options: self.options,
            _phantom: PhantomData,
        }
    }

    // ===== Getters =====

    pub fn get_scale_impl(&self) -> &Arc<dyn ScaleImpl> {
        &self.scale_impl
    }

    pub fn get_domain(&self) -> &ScaleDomain {
        &self.domain
    }

    pub fn get_range(&self) -> &ScaleRange {
        &self.range
    }

    pub fn get_options(&self) -> &HashMap<String, Expr> {
        &self.options
    }

    // ===== Domain inference and configuration =====

    /// Infer domain from data fields
    pub async fn infer_domain_from_data(
        mut self,
        _plot_area_width: f32,
        _plot_area_height: f32,
    ) -> Result<Self, AvengerChartError> {
        // Calculate range hint for radius-aware padding
        let range_hint = match &self.range {
            ScaleRange::Numeric(start, end) => {
                let scalars =
                    eval_to_scalars(vec![start.clone(), end.as_ref().clone()], None, None).await?;
                if scalars.len() == 2 {
                    Some((scalars[0].as_f64()?, scalars[1].as_f64()?))
                } else {
                    None
                }
            }
            _ => None,
        };

        // Infer domain using DomainInferrer
        self.domain = DomainInferrer::infer(&self.scale_impl, self.domain, range_hint).await?;
        Ok(self)
    }

    /// Apply normalization (zero, nice, padding) to domain
    pub async fn normalize_domain(
        mut self,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Result<Self, AvengerChartError> {
        // Skip for non-numeric ranges
        if !matches!(&self.range, ScaleRange::Numeric(_, _)) {
            return Ok(self);
        }

        // Skip for discrete domains - normalization doesn't apply to categorical data
        if matches!(&self.domain.default_domain, ScaleDefaultDomain::Discrete(_)) {
            return Ok(self);
        }

        // Create ConfiguredScale to apply normalization
        let configured = self
            .create_configured_scale(plot_area_width, plot_area_height)
            .await?;

        // Update domain with normalized values
        // This will only succeed for scales with numeric interval domains
        // Discrete scales will return Err and keep their original domain
        if let Ok((min, max)) = configured.numeric_interval_domain() {
            self = self.domain_interval(lit(min as f64), lit(max as f64));
        }

        Ok(self)
    }

    /// Create a ConfiguredScale for rendering
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
                    let mut float_values = Vec::new();
                    for scalar in scalars {
                        float_values.push(scalar.as_f32()?);
                    }
                    Arc::new(Float32Array::from(float_values)) as ArrayRef
                } else {
                    // Other discrete scales need string values
                    let mut string_values = Vec::new();
                    for scalar in scalars {
                        string_values.push(scalar.as_scalar_string()?);
                    }
                    Arc::new(StringArray::from(string_values)) as ArrayRef
                }
            }
            ScaleDefaultDomain::DomainExprs(_) => {
                return Err(AvengerChartError::InternalError(
                    "Domain must be resolved before creating ConfiguredScale".to_string(),
                ));
            }
        };

        // Extract range values
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
            ScaleRange::Enum(values) => {
                // Check if all values are numeric - if so, keep as Float32Array
                // This handles cases like stroke_width which uses ordinal scale with numeric range
                let all_numeric = values.iter().all(|v| v.as_f32().is_ok());

                if all_numeric {
                    let mut float_values = Vec::new();
                    for scalar in values {
                        float_values.push(scalar.as_f32()?);
                    }
                    Arc::new(Float32Array::from(float_values)) as ArrayRef
                } else {
                    // For non-numeric discrete values, convert to strings
                    let mut string_values = Vec::new();
                    for scalar in values {
                        string_values.push(scalar.as_scalar_string()?);
                    }
                    Arc::new(StringArray::from(string_values)) as ArrayRef
                }
            }
            ScaleRange::Color(colors) => {
                // Convert colors to a list array of RGBA values
                use datafusion::arrow::array::Float32Builder;
                use datafusion::arrow::array::ListBuilder;

                let mut list_builder = ListBuilder::new(Float32Builder::new());

                for color in colors {
                    // Append a new list entry for this color
                    let values_builder = list_builder.values();
                    values_builder.append_value(color.red);
                    values_builder.append_value(color.green);
                    values_builder.append_value(color.blue);
                    values_builder.append_value(color.alpha);
                    list_builder.append(true);
                }

                Arc::new(list_builder.finish()) as ArrayRef
            }
        };

        // Extract options as HashMap<String, Scalar>
        let mut scalar_options = HashMap::new();
        for (key, value_expr) in &self.options {
            // Evaluate the expression to get a scalar value
            let scalars = eval_to_scalars(vec![value_expr.clone()], None, None).await?;
            if let Some(scalar_value) = scalars.first() {
                // Convert to avenger_scales::scalar::Scalar
                let scalar = scalar_value.as_scale_scalar()?;
                scalar_options.insert(key.clone(), scalar);
            }
        }

        // Convert padding to clip_padding_lower and clip_padding_upper for linear/log/pow/symlog scales
        if scalar_options.contains_key("padding") {
            let scale_type = self.scale_impl.scale_type();
            if scale_type == "linear"
                || scale_type == "log"
                || scale_type == "pow"
                || scale_type == "symlog"
            {
                if let Some(padding_value) = scalar_options.get("padding").cloned() {
                    // For continuous scales, padding becomes clip_padding
                    if !scalar_options.contains_key("clip_padding_lower") {
                        scalar_options
                            .insert("clip_padding_lower".to_string(), padding_value.clone());
                    }
                    if !scalar_options.contains_key("clip_padding_upper") {
                        scalar_options.insert("clip_padding_upper".to_string(), padding_value);
                    }
                    // Remove the padding option as it's been converted
                    scalar_options.remove("padding");
                }
            }
            // For band and point scales, padding is valid and should be kept as-is
            // For other scales (ordinal, threshold, etc.), padding is not used
        }

        // Create scale context using default
        let context = ScaleContext::default();

        // Create scale config
        let config = ScaleConfig {
            domain,
            range,
            options: scalar_options,
            context,
        };

        // Create configured scale
        Ok(ConfiguredScale {
            scale_impl: self.scale_impl.clone(),
            config,
        })
    }
}

// ===== Linear scale specific methods =====
impl Scale<Linear> {
    /// Set whether to include zero in the domain
    pub fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }

    /// Set padding as a fraction of the domain
    pub fn padding(self, value: f32) -> Self {
        self._option("padding", lit(value))
    }
}

// ===== Log scale specific methods =====
impl Scale<Log> {
    /// Set the logarithm base
    pub fn base(self, value: f32) -> Self {
        self._option("base", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Pow scale specific methods =====
impl Scale<Pow> {
    /// Set the exponent
    pub fn exponent(self, value: f32) -> Self {
        self._option("exponent", lit(value))
    }

    /// Set whether to include zero in the domain
    pub fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Sqrt scale specific methods =====
impl Scale<Sqrt> {
    /// Set whether to include zero in the domain
    pub fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Symlog scale specific methods =====
impl Scale<Symlog> {
    /// Set the constant parameter
    pub fn constant(self, value: f32) -> Self {
        self._option("constant", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Band scale specific methods =====
impl Scale<Band> {
    /// Set the inner padding (between bands)
    pub fn padding_inner(self, value: f32) -> Self {
        self._option("padding_inner", lit(value))
    }

    /// Set the outer padding (before first and after last band)
    pub fn padding_outer(self, value: f32) -> Self {
        self._option("padding_outer", lit(value))
    }

    /// Set the alignment (0 = left, 0.5 = center, 1 = right)
    pub fn align(self, value: f32) -> Self {
        self._option("align", lit(value))
    }

    /// Set whether to round positions to pixel boundaries
    pub fn round(self, value: bool) -> Self {
        self._option("round", lit(value))
    }
}

// ===== Point scale specific methods =====
impl Scale<Point> {
    /// Set the padding (as a fraction of the step)
    pub fn padding(self, value: f32) -> Self {
        self._option("padding", lit(value))
    }

    /// Set the alignment (0 = left, 0.5 = center, 1 = right)
    pub fn align(self, value: f32) -> Self {
        self._option("align", lit(value))
    }

    /// Set whether to round positions to pixel boundaries
    pub fn round(self, value: bool) -> Self {
        self._option("round", lit(value))
    }
}

// ===== Ordinal scale specific methods =====
impl Scale<Ordinal> {
    /// Set the value to use for unknown inputs
    pub fn unknown(self, value: impl Into<Expr>) -> Self {
        self._option("unknown", value.into())
    }
}

// ===== Time scale specific methods =====
impl Scale<Time> {
    /// Set whether to nice the domain to time intervals
    pub fn nice(mut self, value: bool) -> Self {
        self.options.insert("nice".to_string(), lit(value));
        self
        // self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Auto scale for dynamic construction =====
impl Scale<Auto> {
    /// Set a generic option
    ///
    /// This method allows dynamic configuration when the scale type isn't known at compile time.
    /// For typed scales (like Scale<Linear>), use the specific methods instead (e.g., `.nice()`, `.zero()`)
    ///
    /// # Example
    /// ```
    /// use avenger_chart::scales::{Scale, Auto, Linear};
    /// use datafusion::prelude::lit;
    /// use std::sync::Arc;
    ///
    /// // Create an Auto scale from a Linear implementation
    /// let scale = Scale::<Linear>::new()
    ///     .into_auto()
    ///     .option("nice", lit(true))
    ///     .option("clamp", lit(false));
    /// ```
    pub fn option(self, key: impl Into<String>, value: impl Into<Expr>) -> Self {
        self._option(key, value)
    }

    /// Create a scale from a dynamic ScaleImpl (used when type is not known at compile time)
    pub fn from_impl(scale_impl: Arc<dyn ScaleImpl>) -> Self {
        // Use the scale's domain inference method to determine domain type
        // This is more extensible than hard-coding scale names
        use avenger_scales::scales::InferDomainFromDataMethod;
        let domain = match scale_impl.infer_domain_from_data_method() {
            InferDomainFromDataMethod::Unique => ScaleDomain::new_discrete(vec![]),
            InferDomainFromDataMethod::Interval => ScaleDomain::new_interval(lit(0.0), lit(1.0)),
            InferDomainFromDataMethod::All => ScaleDomain::new_discrete(vec![]), // Treat All as discrete
        };

        // Get default options from the scale implementation
        let default_options = scale_impl.default_options();
        let mut options = HashMap::new();

        // Convert Scalar values to Expr values
        for (key, scalar) in default_options {
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
            _phantom: PhantomData,
        }
    }
}
