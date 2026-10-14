pub mod band;
pub mod coerce;
pub mod domain_summary;
pub mod linear;
pub mod log;
pub mod nested_band;
pub mod ordinal;
pub mod point;
pub mod pow;
pub mod quantile;
pub mod quantize;
pub mod symlog;
pub mod threshold;
pub mod time;

use std::{collections::HashMap, fmt::Debug, sync::Arc};

use arrow::array::{Array, ArrayRef, AsArray, Float32Array, Float64Array};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Float32Type, Float64Type};
use avenger_color::{ColorOrGradient, GradientStop};
use avenger_common::{
    types::{
        AreaOrientation, FontStyle, FontWeight, ImageAlign, ImageBaseline, LinearScaleAdjustment,
        StrokeCap, StrokeJoin, TextAlign, TextBaseline,
    },
    value::ScalarOrArray,
};
use avenger_format::TickSpacing;

use crate::{
    color_interpolator::{ColorInterpolator, ColorInterpolatorConfig, SrgbaColorInterpolator},
    error::AvengerScaleError,
    scalar::Scalar,
    scales::coerce::{CastNumericCoercer, Coercer, ColorCoercer, CssColorCoercer, NumericCoercer},
    scales::domain_summary::{DomainOptions, DomainSummary, Extents},
};

/// Validation constraint for a scale option.
///
/// This enum defines various validation rules that can be applied to scale options.
/// Scale implementations use these constraints to validate their configuration options
/// before processing data.
///
/// # Example
/// ```ignore
/// // In a scale implementation:
/// fn validate_options(&self, config: &ScaleConfig) -> Result<(), AvengerScaleError> {
///     let definitions = vec![
///         OptionDefinition::optional("base", OptionConstraint::PositiveFloat),
///         OptionDefinition::optional("clamp", OptionConstraint::Boolean),
///         OptionDefinition::optional("round", OptionConstraint::Boolean),
///     ];
///     
///     OptionDefinition::validate_all(&definitions, &config.options)
/// }
/// ```
#[derive(Debug, Clone)]
pub enum OptionConstraint {
    /// Option must be a boolean
    Boolean,
    /// Option must be a float
    Float,
    /// Option must be an integer
    Integer,
    /// Option must be a string
    String,
    /// Option must be a float within a range (inclusive)
    FloatRange { min: f64, max: f64 },
    /// Option must be an integer within a range (inclusive)
    IntegerRange { min: i32, max: i32 },
    /// Option must be one of the specified string values
    StringEnum { values: Vec<String> },
    /// Option must be a positive float (> 0)
    PositiveFloat,
    /// Option must be a non-negative float (>= 0)
    NonNegativeFloat,
    /// Option must be a positive integer (> 0)
    PositiveInteger,
    /// Option must be a non-negative integer (>= 0)
    NonNegativeInteger,
    /// Custom validation function
    Custom {
        description: String,
        validator: fn(&Scalar) -> Result<(), String>,
    },
}

impl OptionConstraint {
    /// Constraint for base option in logarithmic scales (must be positive and not 1)
    pub fn log_base() -> Self {
        Self::Custom {
            description: "positive number not equal to 1".to_string(),
            validator: |value| match value.as_f64() {
                Ok(v) if v > 0.0 && v != 1.0 => Ok(()),
                Ok(v) => Err(format!("must be positive and not equal to 1 (got {v})")),
                Err(_) => Err("must be a numeric value".to_string()),
            },
        }
    }
}

/// Definition of a scale option with its name and validation constraints.
///
/// This struct represents a single configuration option for a scale, including
/// its name, validation constraint, and whether it's required. Scale implementations
/// use these definitions to automatically validate their configuration options.
///
/// # Example
/// ```ignore
/// let definitions = vec![
///     OptionDefinition::required("domain", OptionConstraint::Float),
///     OptionDefinition::optional("clamp", OptionConstraint::Boolean),
///     OptionDefinition::optional("round", OptionConstraint::Boolean),
/// ];
/// ```
#[derive(Debug, Clone)]
pub struct OptionDefinition {
    pub name: String,
    pub constraint: OptionConstraint,
    pub required: bool,
}

impl OptionDefinition {
    /// Create a new required option definition.
    ///
    /// # Arguments
    /// * `name` - The name of the option
    /// * `constraint` - The validation constraint to apply
    ///
    /// # Returns
    /// A new `OptionDefinition` with `required` set to `true`
    pub fn required(name: impl Into<String>, constraint: OptionConstraint) -> Self {
        Self {
            name: name.into(),
            constraint,
            required: true,
        }
    }

    /// Create a new optional option definition.
    ///
    /// # Arguments
    /// * `name` - The name of the option
    /// * `constraint` - The validation constraint to apply
    ///
    /// # Returns
    /// A new `OptionDefinition` with `required` set to `false`
    pub fn optional(name: impl Into<String>, constraint: OptionConstraint) -> Self {
        Self {
            name: name.into(),
            constraint,
            required: false,
        }
    }

    /// Validate a set of options against a list of option definitions.
    ///
    /// This method checks that:
    /// 1. All provided options are defined in the definitions list
    /// 2. All required options are present
    /// 3. All option values satisfy their constraints
    ///
    /// # Arguments
    /// * `definitions` - The list of valid option definitions
    /// * `options` - The options to validate
    ///
    /// # Returns
    /// * `Ok(())` if all validations pass
    /// * `Err(AvengerScaleError)` if any validation fails
    ///
    /// # Example
    /// ```ignore
    /// let definitions = vec![
    ///     OptionDefinition::optional("clamp", OptionConstraint::Boolean),
    ///     OptionDefinition::optional("round", OptionConstraint::Boolean),
    /// ];
    /// OptionDefinition::validate_all(&definitions, &config.options)?;
    /// ```
    pub fn validate_all(
        definitions: &[OptionDefinition],
        options: &HashMap<String, Scalar>,
    ) -> Result<(), AvengerScaleError> {
        // Check for unknown options
        for key in options.keys() {
            if !definitions.iter().any(|def| def.name == *key) {
                return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                    "Unknown option '{}'. Valid options are: {}",
                    key,
                    definitions
                        .iter()
                        .map(|d| d.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }

        // Validate each defined option
        for def in definitions {
            if let Some(value) = options.get(&def.name) {
                def.validate(value)?;
            } else if def.required {
                return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                    "Required option '{}' is missing",
                    def.name
                )));
            }
        }
        Ok(())
    }

    /// Validate a value against this option's constraint.
    ///
    /// # Arguments
    /// * `value` - The value to validate
    ///
    /// # Returns
    /// * `Ok(())` if the value satisfies the constraint
    /// * `Err(AvengerScaleError)` if the value doesn't satisfy the constraint
    pub fn validate(&self, value: &Scalar) -> Result<(), AvengerScaleError> {
        match &self.constraint {
            OptionConstraint::Boolean => value.as_boolean().map(|_| ()).map_err(|_| {
                AvengerScaleError::InvalidScalePropertyValue(format!(
                    "Option '{}' must be a boolean value",
                    self.name
                ))
            }),
            OptionConstraint::Float => value.as_f64().map(|_| ()).map_err(|_| {
                AvengerScaleError::InvalidScalePropertyValue(format!(
                    "Option '{}' must be a float value",
                    self.name
                ))
            }),
            OptionConstraint::Integer => value.as_i32().map(|_| ()).map_err(|_| {
                AvengerScaleError::InvalidScalePropertyValue(format!(
                    "Option '{}' must be an integer value",
                    self.name
                ))
            }),
            OptionConstraint::String => value.as_string().map(|_| ()).map_err(|_| {
                AvengerScaleError::InvalidScalePropertyValue(format!(
                    "Option '{}' must be a string value",
                    self.name
                ))
            }),
            OptionConstraint::FloatRange { min, max } => {
                let v = value.as_f64().map_err(|_| {
                    AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be a float value",
                        self.name
                    ))
                })?;
                if v < *min || v > *max {
                    Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be between {} and {} (got {})",
                        self.name, min, max, v
                    )))
                } else {
                    Ok(())
                }
            }
            OptionConstraint::IntegerRange { min, max } => {
                let v = value.as_i32().map_err(|_| {
                    AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be an integer value",
                        self.name
                    ))
                })?;
                if v < *min || v > *max {
                    Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be between {} and {} (got {})",
                        self.name, min, max, v
                    )))
                } else {
                    Ok(())
                }
            }
            OptionConstraint::StringEnum { values } => {
                let v = value.as_string().map_err(|_| {
                    AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be a string value",
                        self.name
                    ))
                })?;
                if !values.contains(&v) {
                    Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be one of: {} (got '{}')",
                        self.name,
                        values.join(", "),
                        v
                    )))
                } else {
                    Ok(())
                }
            }
            OptionConstraint::PositiveFloat => {
                let v = value.as_f64().map_err(|_| {
                    AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be a float value",
                        self.name
                    ))
                })?;
                if v <= 0.0 {
                    Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be positive (got {})",
                        self.name, v
                    )))
                } else {
                    Ok(())
                }
            }
            OptionConstraint::NonNegativeFloat => {
                let v = value.as_f64().map_err(|_| {
                    AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be a float value",
                        self.name
                    ))
                })?;
                if v < 0.0 {
                    Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be non-negative (got {})",
                        self.name, v
                    )))
                } else {
                    Ok(())
                }
            }
            OptionConstraint::PositiveInteger => {
                let v = value.as_i32().map_err(|_| {
                    AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be an integer value",
                        self.name
                    ))
                })?;
                if v <= 0 {
                    Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be positive (got {})",
                        self.name, v
                    )))
                } else {
                    Ok(())
                }
            }
            OptionConstraint::NonNegativeInteger => {
                let v = value.as_i32().map_err(|_| {
                    AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be an integer value",
                        self.name
                    ))
                })?;
                if v < 0 {
                    Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                        "Option '{}' must be non-negative (got {})",
                        self.name, v
                    )))
                } else {
                    Ok(())
                }
            }
            OptionConstraint::Custom {
                description,
                validator,
            } => validator(value).map_err(|err| {
                AvengerScaleError::InvalidScalePropertyValue(format!(
                    "Option '{}' validation failed ({}): {}",
                    self.name, description, err
                ))
            }),
        }
    }
}

/// Macro to generate scale_to_X trait methods that return a default error implementation
#[macro_export]
macro_rules! declare_enum_scale_method {
    ($type_name:ident) => {
        paste::paste! {
            fn [<scale_to_ $type_name:snake>](
                &self,
                config: &ScaleConfig,
                values: &ArrayRef,
            ) -> Result<ScalarOrArray<$type_name>, AvengerScaleError> {
                let scaled = self.scale(config, values)?;
                let coercer = Coercer::default();
                coercer.[<to_ $type_name:snake>](&scaled)
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct ScaleConfig {
    pub domain: ArrayRef,
    pub range: ArrayRef,
    pub options: HashMap<String, Scalar>,
    pub context: ScaleContext,
}

#[derive(Debug, Clone)]
pub struct ScaleContext {
    pub color_interpolator: Arc<dyn ColorInterpolator>,
    pub color_coercer: Arc<dyn ColorCoercer>,
    pub numeric_coercer: Arc<dyn NumericCoercer>,
}

impl Default for ScaleContext {
    fn default() -> Self {
        Self {
            color_interpolator: Arc::new(SrgbaColorInterpolator),
            color_coercer: Arc::new(CssColorCoercer),
            numeric_coercer: Arc::new(CastNumericCoercer),
        }
    }
}

impl ScaleConfig {
    pub fn empty() -> Self {
        Self {
            domain: Arc::new(Float64Array::from(Vec::<f64>::new())) as ArrayRef,
            range: Arc::new(Float64Array::from(Vec::<f64>::new())) as ArrayRef,
            options: HashMap::new(),
            context: ScaleContext::default(),
        }
    }

    pub fn numeric_interval_domain(&self) -> Result<(f64, f64), AvengerScaleError> {
        if self.domain.len() != 2 {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "numeric_interval_domain".to_string(),
            ));
        }
        let domain = cast(self.domain.as_ref(), &DataType::Float64)?;
        let domain = domain.as_primitive::<Float64Type>();
        Ok((domain.value(0), domain.value(1)))
    }

    pub fn numeric_interval_range(&self) -> Result<(f64, f64), AvengerScaleError> {
        if self.range.len() != 2 {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "numeric_interval_range".to_string(),
            ));
        }
        let range = cast(self.range.as_ref(), &DataType::Float64)?;
        let range = range.as_primitive::<Float64Type>();
        Ok((range.value(0), range.value(1)))
    }

    pub fn color_range(&self) -> Result<Vec<[f32; 4]>, AvengerScaleError> {
        let coercer = CssColorCoercer;
        let range_colors = coercer.coerce(&self.range, None)?;
        let range_colors_vec: Vec<_> = range_colors
            .as_iter(range_colors.len(), None)
            .map(|c| c.color_or_transparent())
            .collect();
        Ok(range_colors_vec)
    }

    pub fn option_f64(&self, key: &str, default: f64) -> f64 {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(Scalar::from(default))
            .as_f64()
            .unwrap_or(default)
    }

    pub fn option_boolean(&self, key: &str, default: bool) -> bool {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(Scalar::from(default))
            .as_boolean()
            .unwrap_or(default)
    }

    pub fn option_i32(&self, key: &str, default: i32) -> i32 {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(Scalar::from(default))
            .as_i32()
            .unwrap_or(default)
    }

    pub fn option_string(&self, key: &str, default: &str) -> String {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(Scalar::from(default))
            .as_string()
            .unwrap_or(default.to_string())
    }
}

/// The kind of data a scale expects in its domain
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainKind {
    /// Numeric values (floats, integers)
    /// Used by linear, log, pow, sqrt, symlog, threshold, quantize, quantile scales
    Numeric,
    /// Temporal values (dates, timestamps)
    /// Used by time, utc scales
    Temporal,
    /// Categorical values (strings, discrete categories)
    /// Used by ordinal, band, point scales
    Categorical,
    /// Struct-valued categorical paths.
    /// Used by nested band scales.
    NestedCategorical,
}

/// A kind of scale: how it maps values from its domain to its range, configured by a
/// [`ScaleConfig`].
///
/// Scales serialize through [typetag](https://docs.rs/typetag), so that a serialized scale decodes
/// without registration, including scales defined in other crates. An implementation derives
/// `Serialize` and `Deserialize`, marks its `impl` block `#[typetag::serde(name = "...")]` with its
/// [`scale_type`](Self::scale_type), and depends on `serde` and `typetag` itself.
#[typetag::serde(tag = "type", content = "config")]
pub trait ScaleImpl: Debug + Send + Sync + 'static {
    /// Return the scale type name for this scale implementation
    fn scale_type(&self) -> &'static str;

    /// The kind of data this scale expects in its domain
    fn domain_kind(&self) -> DomainKind;

    /// Return the option definitions for this scale.
    ///
    /// This method should return a slice of `OptionDefinition` structs that describe
    /// all valid options for this scale type. The default implementation returns
    /// an empty slice, indicating that the scale has no configurable options.
    ///
    /// Scale implementations should override this method to define their supported
    /// options. The `validate_options` method will automatically use these definitions
    /// to validate the scale configuration.
    ///
    /// # Example
    /// ```ignore
    /// fn option_definitions(&self) -> &[OptionDefinition] {
    ///     static DEFINITIONS: &[OptionDefinition] = &[
    ///         OptionDefinition::optional("clamp", OptionConstraint::Boolean),
    ///         OptionDefinition::optional("round", OptionConstraint::Boolean),
    ///     ];
    ///     DEFINITIONS
    /// }
    /// ```
    fn option_definitions(&self) -> &[OptionDefinition] {
        &[]
    }

    /// Validate scale options.
    ///
    /// The default implementation automatically validates options against the
    /// definitions returned by `option_definitions()`. Scale implementations
    /// typically don't need to override this method unless they have special
    /// validation requirements beyond what `OptionDefinition` provides.
    ///
    /// This method is automatically called before scaling operations.
    ///
    /// # Implementation Note
    /// If you need custom validation beyond what `OptionDefinition` provides,
    /// you can override this method, but make sure to call the default
    /// implementation first:
    /// ```ignore
    /// fn validate_options(&self, config: &ScaleConfig) -> Result<(), AvengerScaleError> {
    ///     // First run the standard validation
    ///     OptionDefinition::validate_all(self.option_definitions(), &config.options)?;
    ///     
    ///     // Then add custom validation
    ///     if some_custom_condition {
    ///         return Err(AvengerScaleError::InvalidScalePropertyValue(
    ///             "Custom validation failed".to_string()
    ///         ));
    ///     }
    ///     Ok(())
    /// }
    /// ```
    fn validate_options(&self, config: &ScaleConfig) -> Result<(), AvengerScaleError> {
        OptionDefinition::validate_all(self.option_definitions(), &config.options)
    }

    fn scale(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError>;

    /// Summarizes `values`, with the pixel extents of the marks drawn at them, into a summary for
    /// fitting this scale's domain. It reads only the scale's options, never its domain or range,
    /// so a summary can be built before either is known. Summaries combine with
    /// [`DomainSummary::merge`]. The default reports that this scale's domain cannot be inferred
    /// from data.
    fn summarize(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
        _extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "summarize".to_string(),
        ))
    }

    /// `config` with its domain fitted to `summary` for its range. Band, point, and nested band
    /// scales also set the clip padding that keeps every mark in the range. Other options are
    /// unchanged.
    fn fit_domain(
        &self,
        _config: &ScaleConfig,
        _options: &DomainOptions,
        _summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "fit_domain".to_string(),
        ))
    }

    /// Invert an array of numeric values from range to domain.
    ///
    /// This method provides array-based inverse transformation that parallels
    /// the `scale()` method. It accepts an ArrayRef of numeric values in the range
    /// and returns an ArrayRef of the corresponding domain values.
    fn invert(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert".to_string(),
        ))
    }

    /// Scale to numeric values
    fn scale_to_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // Validate options before scaling
        self.validate_options(config)?;

        let scaled = self.scale(config, values)?;
        let coercer = &config.context.numeric_coercer;
        let default = config.option_f64("default", f64::NAN) as f32;
        let t = coercer.coerce(&scaled, Some(default))?;
        Ok(t)
    }

    fn scale_scalar_to_numeric(
        &self,
        config: &ScaleConfig,
        value: &Scalar,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        let array = value.to_array();
        Ok(self
            .scale_to_numeric(config, &array)?
            .to_scalar_if_len_one())
    }

    fn invert_from_numeric(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
    ) -> Result<ScalarOrArray<f64>, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_from_numeric".to_string(),
        ))
    }

    fn invert_scalar(&self, config: &ScaleConfig, value: f64) -> Result<f64, AvengerScaleError> {
        let value_array = self.invert_from_numeric(
            config,
            &(Arc::new(Float64Array::from(vec![value])) as ArrayRef),
        )?;
        Ok(value_array.as_vec(1, None)[0])
    }

    /// Invert a range interval to a subset of the domain
    fn invert_range_interval(
        &self,
        _config: &ScaleConfig,
        _range: (f64, f64),
    ) -> Result<ArrayRef, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_range_interval".to_string(),
        ))
    }

    /// Get the domain values for ticks for the scale
    /// These can be scaled to number for position, and scaled to string for labels.
    fn ticks(
        &self,
        _config: &ScaleConfig,
        _count: Option<f64>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "ticks".to_string(),
        ))
    }

    /// How the scale's ticks are spaced, which decides whether their labels share one precision.
    fn tick_spacing(&self) -> TickSpacing {
        TickSpacing::Uniform
    }

    /// The ticks to label, among the `ticks` that [`ScaleImpl::ticks`] made for `count`. Every
    /// tick has a label unless the scale thins them, as log scales thin their minor ticks.
    fn label_ticks(
        &self,
        _config: &ScaleConfig,
        ticks: &ArrayRef,
        _count: Option<f64>,
    ) -> ArrayRef {
        ticks.clone()
    }

    /// Scale to color values
    fn scale_to_color(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        // Validate options before scaling
        self.validate_options(config)?;

        let scaled = self.scale(config, values)?;
        let coercer = &config.context.color_coercer;
        let default = config
            .options
            .get("default")
            .and_then(|v| Some(ColorOrGradient::Color(v.as_rgba().ok()?)))
            .unwrap_or(ColorOrGradient::transparent());

        coercer.coerce(&scaled, Some(default))
    }

    fn scale_scalar_to_color(
        &self,
        config: &ScaleConfig,
        value: &Scalar,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        let array = value.to_array();
        Ok(self.scale_to_color(config, &array)?.to_scalar_if_len_one())
    }

    /// Scale to string values, such as an ordinal scale's string range. Other outputs convert
    /// to text, and nulls use the `default` option.
    fn scale_to_string(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        // Validate options before scaling
        self.validate_options(config)?;

        let scaled = self.scale(config, values)?;
        to_text(&scaled, &config.option_string("default", ""))
    }

    fn scale_scalar_to_string(
        &self,
        config: &ScaleConfig,
        value: &Scalar,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        let array = value.to_array();
        Ok(self.scale_to_string(config, &array)?.to_scalar_if_len_one())
    }

    // Pan/zoom operations
    fn pan(&self, _config: &ScaleConfig, _delta: f64) -> Result<ScaleConfig, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "pan".to_string(),
        ))
    }

    fn zoom(
        &self,
        _config: &ScaleConfig,
        _anchor: f64,
        _scale_factor: f64,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "zoom".to_string(),
        ))
    }

    fn adjust(
        &self,
        _from_config: &ScaleConfig,
        _to_config: &ScaleConfig,
    ) -> Result<LinearScaleAdjustment, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "adjust".to_string(),
        ))
    }

    // Scale to enums
    declare_enum_scale_method!(StrokeCap);
    declare_enum_scale_method!(StrokeJoin);
    declare_enum_scale_method!(ImageAlign);
    declare_enum_scale_method!(ImageBaseline);
    declare_enum_scale_method!(AreaOrientation);
    declare_enum_scale_method!(TextAlign);
    declare_enum_scale_method!(TextBaseline);
    declare_enum_scale_method!(FontStyle);

    /// Scale to font weights, which are numbers.
    fn scale_to_font_weight(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<FontWeight>, AvengerScaleError> {
        Ok(self
            .scale_to_numeric(config, values)?
            .map(|weight| FontWeight::from(*weight)))
    }
}

/// Macro to generate scale_to_X trait methods that return a default error implementation
#[macro_export]
macro_rules! declare_enum_configured_scale_method {
    ($type_name:ident) => {
        paste::paste! {
            pub fn [<scale_to_ $type_name:snake>](
                &self,
                values: &ArrayRef,
            ) -> Result<ScalarOrArray<$type_name>, AvengerScaleError> {
                self.scale_impl.[<scale_to_ $type_name:snake>](&self.config, values)

            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct ConfiguredScale {
    pub scale_impl: Arc<dyn ScaleImpl>,
    pub config: ScaleConfig,
}

// Builder methods
impl ConfiguredScale {
    pub fn with_scale_impl(self, scale_impl: Arc<dyn ScaleImpl>) -> ConfiguredScale {
        ConfiguredScale { scale_impl, ..self }
    }

    pub fn with_domain(self, domain: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            config: ScaleConfig {
                domain,
                ..self.config
            },
            ..self
        }
    }

    pub fn with_domain_interval(self, domain: (f64, f64)) -> ConfiguredScale {
        ConfiguredScale {
            config: ScaleConfig {
                domain: Arc::new(Float64Array::from(vec![domain.0, domain.1])),
                ..self.config
            },
            ..self
        }
    }

    pub fn with_range_interval(self, range: (f64, f64)) -> ConfiguredScale {
        ConfiguredScale {
            config: ScaleConfig {
                range: Arc::new(Float64Array::from(vec![range.0, range.1])),
                ..self.config
            },
            ..self
        }
    }

    pub fn with_range_colors(
        self,
        range_colors: Vec<[f32; 4]>,
    ) -> Result<ConfiguredScale, AvengerScaleError> {
        let arrays = range_colors
            .into_iter()
            .map(|clr| Arc::new(Float32Array::from(Vec::from(clr))) as ArrayRef)
            .collect::<Vec<_>>();

        let range = Scalar::arrays_into_list_array(arrays)?;
        Ok(self.with_range(range))
    }

    pub fn with_range(self, range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            config: ScaleConfig {
                range,
                ..self.config
            },
            ..self
        }
    }

    pub fn with_config(self, config: ScaleConfig) -> ConfiguredScale {
        ConfiguredScale { config, ..self }
    }

    pub fn with_option<S: Into<String>, V: Into<Scalar>>(
        mut self,
        key: S,
        value: V,
    ) -> ConfiguredScale {
        self.config.options.insert(key.into(), value.into());
        self
    }

    pub fn with_color_interpolator(
        self,
        color_interpolator: Arc<dyn ColorInterpolator>,
    ) -> ConfiguredScale {
        ConfiguredScale {
            config: ScaleConfig {
                context: ScaleContext {
                    color_interpolator,
                    ..self.config.context
                },
                ..self.config
            },
            ..self
        }
    }
}

// Pan / zoom methods
impl ConfiguredScale {
    pub fn pan(self, delta: f64) -> Result<ConfiguredScale, AvengerScaleError> {
        let config = self.scale_impl.pan(&self.config, delta)?;
        Ok(self.with_config(config))
    }

    pub fn zoom(
        self,
        anchor: f64,
        scale_factor: f64,
    ) -> Result<ConfiguredScale, AvengerScaleError> {
        let config = self.scale_impl.zoom(&self.config, anchor, scale_factor)?;
        Ok(self.with_config(config))
    }

    pub fn adjust(
        &self,
        to_scale: &ConfiguredScale,
    ) -> Result<LinearScaleAdjustment, AvengerScaleError> {
        self.scale_impl.adjust(&self.config, &to_scale.config)
    }
}

// Pass through methods
impl ConfiguredScale {
    /// Summarizes `values` and the extents of the marks drawn at them for fitting this scale's
    /// domain. See [`ScaleImpl::summarize`].
    pub fn summarize(
        &self,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        self.scale_impl.summarize(&self.config, values, extents)
    }

    /// This scale with its domain fitted to `summary`. See [`ScaleImpl::fit_domain`].
    pub fn fit_domain(
        self,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ConfiguredScale, AvengerScaleError> {
        let config = self.scale_impl.fit_domain(&self.config, options, summary)?;
        Ok(self.with_config(config))
    }

    pub fn scale(&self, values: &ArrayRef) -> Result<ArrayRef, AvengerScaleError> {
        // Validate options before scaling
        self.scale_impl.validate_options(&self.config)?;
        self.scale_impl.scale(&self.config, values)
    }

    pub fn scale_scalar<S: Into<Scalar> + Clone>(
        &self,
        value: &S,
    ) -> Result<Scalar, AvengerScaleError> {
        let scaled = self.scale(&Scalar::iter_to_array(vec![value.clone().into()])?)?;
        Scalar::try_from_array(scaled.as_ref(), 0)
    }

    /// Scale to numeric values
    pub fn scale_to_numeric(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        self.scale_impl.scale_to_numeric(&self.config, values)
    }

    pub fn scale_scalar_to_numeric(
        &self,
        value: &Scalar,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        self.scale_impl.scale_scalar_to_numeric(&self.config, value)
    }

    pub fn invert_from_numeric(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f64>, AvengerScaleError> {
        self.scale_impl.invert_from_numeric(&self.config, values)
    }

    /// Invert an array of numeric values from range to domain.
    ///
    /// This method provides array-based inverse transformation that matches the pattern
    /// of the `scale()` method. It accepts an ArrayRef of numeric values in the range
    /// and returns an ArrayRef of the corresponding domain values.
    ///
    /// # Arguments
    /// * `values` - ArrayRef containing numeric values to invert
    ///
    /// # Returns
    /// * `Result<ArrayRef, AvengerScaleError>` - Array of inverted domain values
    ///
    /// # Example
    /// ```no_run
    /// # use arrow::array::{ArrayRef, Float32Array};
    /// # use std::sync::Arc;
    /// # use avenger_scales::scales::linear::LinearScale;
    /// let scale = LinearScale::configured((0.0, 100.0), (0.0, 1.0));
    /// let range_values = Arc::new(Float32Array::from(vec![0.0, 0.5, 1.0])) as ArrayRef;
    /// let domain_values = scale.invert(&range_values)?;
    /// // domain_values will contain [0.0, 50.0, 100.0]
    /// # Ok::<(), avenger_scales::error::AvengerScaleError>(())
    /// ```
    pub fn invert(&self, values: &ArrayRef) -> Result<ArrayRef, AvengerScaleError> {
        self.scale_impl.invert(&self.config, values)
    }

    pub fn invert_scalar(&self, value: f64) -> Result<f64, AvengerScaleError> {
        self.scale_impl.invert_scalar(&self.config, value)
    }

    /// Invert a range interval to a subset of the domain
    pub fn invert_range_interval(&self, range: (f64, f64)) -> Result<ArrayRef, AvengerScaleError> {
        self.scale_impl.invert_range_interval(&self.config, range)
    }

    /// Get the domain values for ticks for the scale
    /// These can be scaled to number for position, and scaled to string for labels.
    pub fn ticks(&self, count: Option<f64>) -> Result<ArrayRef, AvengerScaleError> {
        self.scale_impl.ticks(&self.config, count)
    }

    /// How the scale's ticks are spaced, which decides whether their labels share one precision.
    pub fn tick_spacing(&self) -> TickSpacing {
        self.scale_impl.tick_spacing()
    }

    /// The ticks to label, among the `ticks` that [`Self::ticks`] made for `count`.
    pub fn label_ticks(&self, ticks: &ArrayRef, count: Option<f64>) -> ArrayRef {
        self.scale_impl.label_ticks(&self.config, ticks, count)
    }

    /// Ticks at `start + k * step` for whole numbers `k`, across the numeric domain, in place of
    /// [`Self::ticks`].
    pub fn start_step_ticks(&self, start: f64, step: f64) -> Result<ArrayRef, AvengerScaleError> {
        let (a, b) = self.numeric_interval_domain()?;
        let ticks = crate::array::start_step_ticks(a.min(b), a.max(b), start, step)?;
        Ok(crate::array::tick_array(ticks))
    }

    /// Ticks at a start instant and whole multiples of a calendar step before and after it,
    /// across the temporal domain, in place of [`Self::ticks`]. The step has one
    /// part: `months`, `days`, or `nanos`, a whole number of milliseconds. Steps of days and
    /// months are in the scale's civil time, and shorter steps are elapsed time.
    pub fn temporal_start_step_ticks(
        &self,
        start_millis: i64,
        months: i32,
        days: i32,
        nanos: i64,
    ) -> Result<ArrayRef, AvengerScaleError> {
        time::start_step_ticks(&self.config, start_millis, months, days, nanos)
    }

    /// Scale to color values
    pub fn scale_to_color(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        self.scale_impl.scale_to_color(&self.config, values)
    }

    pub fn scale_scalar_to_color(
        &self,
        value: &Scalar,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerScaleError> {
        self.scale_impl.scale_scalar_to_color(&self.config, value)
    }

    /// Scale to string values
    pub fn scale_to_string(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        self.scale_impl.scale_to_string(&self.config, values)
    }

    pub fn scale_scalar_to_string(
        &self,
        value: &Scalar,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        self.scale_impl.scale_scalar_to_string(&self.config, value)
    }

    // Enums
    declare_enum_configured_scale_method!(StrokeCap);
    declare_enum_configured_scale_method!(StrokeJoin);
    declare_enum_configured_scale_method!(ImageAlign);
    declare_enum_configured_scale_method!(ImageBaseline);
    declare_enum_configured_scale_method!(AreaOrientation);
    declare_enum_configured_scale_method!(TextAlign);
    declare_enum_configured_scale_method!(TextBaseline);
    declare_enum_configured_scale_method!(FontWeight);
    declare_enum_configured_scale_method!(FontStyle);
}

// ScaleConfig pass through methods
impl ConfiguredScale {
    pub fn domain(&self) -> &ArrayRef {
        &self.config.domain
    }

    pub fn numeric_interval_domain(&self) -> Result<(f64, f64), AvengerScaleError> {
        self.config.numeric_interval_domain()
    }

    pub fn range(&self) -> &ArrayRef {
        &self.config.range
    }

    pub fn numeric_interval_range(&self) -> Result<(f64, f64), AvengerScaleError> {
        self.config.numeric_interval_range()
    }

    pub fn color_range(&self) -> Result<Vec<[f32; 4]>, AvengerScaleError> {
        self.config.color_range()
    }

    pub fn color_range_as_gradient_stops(
        &self,
        num_segments: usize,
    ) -> Result<Vec<GradientStop>, AvengerScaleError> {
        let fractions = (0..=num_segments)
            .map(|i| i as f32 / num_segments as f32)
            .collect::<Vec<f32>>();

        let colors = self.config.color_range()?;
        let interpolator_config = ColorInterpolatorConfig { colors };
        let color_result = self
            .config
            .context
            .color_interpolator
            .interpolate(&interpolator_config, &fractions)?;

        // Convert the interpolated colors back to ColorOrGradient
        let list_array = color_result.as_list::<i32>();
        let color_or_gradients: Vec<ColorOrGradient> = list_array
            .iter()
            .map(|color_opt| {
                let color = color_opt.expect("Color should not be null");
                let values = color.as_primitive::<Float32Type>();
                ColorOrGradient::Color([
                    values.value(0),
                    values.value(1),
                    values.value(2),
                    values.value(3),
                ])
            })
            .collect();

        Ok(fractions
            .iter()
            .zip(color_or_gradients)
            .map(|(f, c)| GradientStop {
                offset: *f,
                color: c.color_or_transparent(),
            })
            .collect())
    }

    pub fn option(&self, key: &str) -> Option<Scalar> {
        self.config.options.get(key).cloned()
    }

    pub fn option_f64(&self, key: &str, default: f64) -> f64 {
        self.config.option_f64(key, default)
    }

    pub fn option_boolean(&self, key: &str, default: bool) -> bool {
        self.config.option_boolean(key, default)
    }

    pub fn option_i32(&self, key: &str, default: i32) -> i32 {
        self.config.option_i32(key, default)
    }

    pub fn option_string(&self, key: &str, default: &str) -> String {
        self.config.option_string(key, default)
    }
}

/// ColorInterpolator pass through methods
impl ConfiguredScale {
    pub fn interpolate_colors(
        &self,
        colors: Vec<[f32; 4]>,
        values: &[f32],
    ) -> Result<ArrayRef, AvengerScaleError> {
        let interpolator_config = ColorInterpolatorConfig {
            colors: colors.to_vec(),
        };
        self.config
            .context
            .color_interpolator
            .as_ref()
            .interpolate(&interpolator_config, values)
    }
}

/// The domain of a scale that has none yet.
pub(crate) fn no_domain() -> ArrayRef {
    Arc::new(Float64Array::from(Vec::<f64>::new()))
}

/// Convert values to text with Arrow's cast, writing `default` for nulls.
pub fn to_text(
    values: &ArrayRef,
    default: &str,
) -> Result<ScalarOrArray<String>, AvengerScaleError> {
    let values = cast(values, &DataType::Utf8)?;
    Ok(ScalarOrArray::new_array(
        values
            .as_string::<i32>()
            .iter()
            .map(|value| value.unwrap_or(default).to_string())
            .collect(),
    ))
}
