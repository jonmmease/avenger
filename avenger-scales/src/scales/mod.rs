pub mod band;
pub mod coerce;
pub mod linear;
pub use linear::NormalizationConfig;
pub mod log;
pub mod ordinal;
pub mod point;
pub mod pow;
pub use pow::PowNormalizationConfig;
pub mod quantile;
pub mod quantize;
pub mod symlog;
pub use symlog::SymlogNormalizationConfig;
pub mod threshold;
pub mod time;

use std::{collections::HashMap, fmt::Debug, sync::Arc};

use crate::{color_interpolator::ColorInterpolator, error::AvengerScaleError, scalar::Scalar};
use crate::{color_interpolator::ColorInterpolatorConfig, formatter::Formatters};
use crate::{
    color_interpolator::SrgbaColorInterpolator,
    scales::coerce::{ColorCoercer, CssColorCoercer},
};
use arrow::array::Array;
use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::cast,
    datatypes::{DataType, Float32Type},
};
use avenger_common::{
    types::{
        AreaOrientation, ColorOrGradient, GradientStop, ImageAlign, ImageBaseline,
        LinearScaleAdjustment, StrokeCap, StrokeJoin,
    },
    value::ScalarOrArray,
};
use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use coerce::{CastNumericCoercer, Coercer, NumericCoercer};

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
///         OptionDefinition::optional("nice", OptionConstraint::nice()),
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
    FloatRange { min: f32, max: f32 },
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
    /// Constraint for 'nice' option that accepts boolean or numeric values
    pub fn nice() -> Self {
        Self::Custom {
            description: "boolean or number".to_string(),
            validator: |value| {
                if value.as_boolean().is_ok() || value.as_f32().is_ok() {
                    Ok(())
                } else {
                    Err("must be a boolean or numeric value".to_string())
                }
            },
        }
    }

    /// Constraint for base option in logarithmic scales (must be positive and not 1)
    pub fn log_base() -> Self {
        Self::Custom {
            description: "positive number not equal to 1".to_string(),
            validator: |value| match value.as_f32() {
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
///     OptionDefinition::optional("nice", OptionConstraint::nice()),
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
            OptionConstraint::Float => value.as_f32().map(|_| ()).map_err(|_| {
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
                let v = value.as_f32().map_err(|_| {
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
                let v = value.as_f32().map_err(|_| {
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
                let v = value.as_f32().map_err(|_| {
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
    pub formatters: Formatters,
    pub color_coercer: Arc<dyn ColorCoercer>,
    pub numeric_coercer: Arc<dyn NumericCoercer>,
}

impl Default for ScaleContext {
    fn default() -> Self {
        Self {
            color_interpolator: Arc::new(SrgbaColorInterpolator),
            formatters: Formatters::default(),
            color_coercer: Arc::new(CssColorCoercer),
            numeric_coercer: Arc::new(CastNumericCoercer),
        }
    }
}

impl ScaleConfig {
    pub fn empty() -> Self {
        Self {
            domain: Arc::new(Float32Array::from(Vec::<f32>::new())) as ArrayRef,
            range: Arc::new(Float32Array::from(Vec::<f32>::new())) as ArrayRef,
            options: HashMap::new(),
            context: ScaleContext::default(),
        }
    }

    pub fn numeric_interval_domain(&self) -> Result<(f32, f32), AvengerScaleError> {
        if self.domain.len() != 2 {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "numeric_interval_domain".to_string(),
            ));
        }
        let domain = cast(self.domain.as_ref(), &DataType::Float32)?;
        let domain = domain.as_primitive::<Float32Type>();
        Ok((domain.value(0), domain.value(1)))
    }

    pub fn numeric_interval_range(&self) -> Result<(f32, f32), AvengerScaleError> {
        if self.range.len() != 2 {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "numeric_interval_range".to_string(),
            ));
        }
        let range = cast(self.range.as_ref(), &DataType::Float32)?;
        let range = range.as_primitive::<Float32Type>();
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

    pub fn option_f32(&self, key: &str, default: f32) -> f32 {
        self.options
            .get(key)
            .cloned()
            .unwrap_or(Scalar::from(default))
            .as_f32()
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

/// Method that should be used to infer a scale's domain from the data that it will scale
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferDomainFromDataMethod {
    /// Use the min and max values of the data
    /// In this case the domain will be a two element array
    Interval,
    /// Use the unique values of the data
    /// In this case the domain will be an array of unique values
    Unique,
    /// Use all values of the data
    /// In this case the domain will be an array of all values
    All,
}

pub trait ScaleImpl: Debug + Send + Sync + 'static {
    /// Return the scale type name for this scale implementation
    fn scale_type(&self) -> &'static str;

    /// Method that should be used to infer a scale's domain from the data that it will scale
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod;

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
    ///         OptionDefinition::optional("nice", OptionConstraint::nice()),
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
        let default = config.option_f32("default", f32::NAN);
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
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_from_numeric".to_string(),
        ))
    }

    fn invert_scalar(&self, config: &ScaleConfig, value: f32) -> Result<f32, AvengerScaleError> {
        let value_array = self.invert_from_numeric(
            config,
            &(Arc::new(Float32Array::from(vec![value])) as ArrayRef),
        )?;
        Ok(value_array.as_vec(1, None)[0])
    }

    /// Invert a range interval to a subset of the domain
    fn invert_range_interval(
        &self,
        _config: &ScaleConfig,
        _range: (f32, f32),
    ) -> Result<ArrayRef, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "invert_range_interval".to_string(),
        ))
    }

    /// Get the domain values for ticks for the scale
    /// These can be scaled to number for position, and scaled to string for labels
    fn ticks(
        &self,
        _config: &ScaleConfig,
        _count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "ticks".to_string(),
        ))
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

    /// Scale to string values
    fn scale_to_string(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        // Validate options before scaling
        self.validate_options(config)?;

        let scaled = self.scale(config, values)?;
        let formatter = &config.context.formatters;
        let default = config.option_string("default", "");
        let t = formatter.format(&scaled, Some(&default))?;
        Ok(t)
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
    fn pan(&self, _config: &ScaleConfig, _delta: f32) -> Result<ScaleConfig, AvengerScaleError> {
        Err(AvengerScaleError::ScaleOperationNotSupported(
            "pan".to_string(),
        ))
    }

    fn zoom(
        &self,
        _config: &ScaleConfig,
        _anchor: f32,
        _scale_factor: f32,
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

    /// Compute the normalized domain for this scale given the current configuration
    /// This handles zero, nice, and clip_padding options as appropriate for the scale type
    /// For scales that don't support normalization, this returns the original domain
    fn compute_normalized_domain(
        &self,
        config: &ScaleConfig,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Default implementation returns the original domain for scales that don't support normalization
        Ok(config.domain.clone())
    }

    // Scale to enums
    declare_enum_scale_method!(StrokeCap);
    declare_enum_scale_method!(StrokeJoin);
    declare_enum_scale_method!(ImageAlign);
    declare_enum_scale_method!(ImageBaseline);
    declare_enum_scale_method!(AreaOrientation);
    declare_enum_scale_method!(TextAlign);
    declare_enum_scale_method!(TextBaseline);
    declare_enum_scale_method!(FontWeight);
    declare_enum_scale_method!(FontStyle);
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

    pub fn with_domain_interval(self, domain: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain.0, domain.1])),
                ..self.config
            },
            ..self
        }
    }

    pub fn with_range_interval(self, range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            config: ScaleConfig {
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
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
    pub fn pan(self, delta: f32) -> Result<ConfiguredScale, AvengerScaleError> {
        let config = self.scale_impl.pan(&self.config, delta)?;
        Ok(self.with_config(config))
    }

    pub fn zoom(
        self,
        anchor: f32,
        scale_factor: f32,
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

    /// Internal method to get the normalized scale configuration
    fn get_normalized_config(&self) -> Result<ScaleConfig, AvengerScaleError> {
        if !self.domain().data_type().is_numeric() {
            // Only scales with numeric domain can be normalized
            return Ok(self.config.clone());
        }

        let normalized_domain = self.scale_impl.compute_normalized_domain(&self.config)?;
        let mut new_options = self.config.options.clone();

        // Only set normalization options that are supported by this scale type
        let option_definitions = self.scale_impl.option_definitions();
        let supported_options: std::collections::HashSet<&str> = option_definitions
            .iter()
            .map(|def| def.name.as_str())
            .collect();

        // Only set these options if they're supported by the scale
        if supported_options.contains("zero") {
            new_options.insert("zero".to_string(), false.into());
        }
        if supported_options.contains("nice") {
            new_options.insert("nice".to_string(), false.into());
        }
        if supported_options.contains("clip_padding_lower") {
            new_options.insert("clip_padding_lower".to_string(), 0.0.into());
        }
        if supported_options.contains("clip_padding_upper") {
            new_options.insert("clip_padding_upper".to_string(), 0.0.into());
        }

        Ok(ScaleConfig {
            domain: normalized_domain,
            range: self.config.range.clone(),
            options: new_options,
            context: self.config.context.clone(),
        })
    }

    /// Check if the scale needs normalization based on current options
    fn needs_normalization(&self) -> bool {
        if !self.domain().data_type().is_numeric() {
            return false;
        }

        let zero = self.config.option_boolean("zero", false);
        let nice = self
            .config
            .options
            .get("nice")
            .and_then(|v| {
                v.as_boolean()
                    .ok()
                    .or_else(|| v.as_f32().ok().map(|n| n != 0.0))
            })
            .unwrap_or(false);
        let clip_padding_lower = self.config.option_f32("clip_padding_lower", 0.0) != 0.0;
        let clip_padding_upper = self.config.option_f32("clip_padding_upper", 0.0) != 0.0;

        zero || nice || clip_padding_lower || clip_padding_upper
    }
}

// Pass through methods
impl ConfiguredScale {
    /// Method that should be used to infer a scale's domain from the data that it will scale
    pub fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        self.scale_impl.infer_domain_from_data_method()
    }

    pub fn scale(&self, values: &ArrayRef) -> Result<ArrayRef, AvengerScaleError> {
        // Auto-normalize if needed
        let config = if self.needs_normalization() {
            self.get_normalized_config()?
        } else {
            self.config.clone()
        };

        // Validate options before scaling
        self.scale_impl.validate_options(&config)?;
        self.scale_impl.scale(&config, values)
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
        // Auto-normalize if needed
        let config = if self.needs_normalization() {
            self.get_normalized_config()?
        } else {
            self.config.clone()
        };

        self.scale_impl.scale_to_numeric(&config, values)
    }

    pub fn scale_scalar_to_numeric(
        &self,
        value: &Scalar,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // Auto-normalize if needed
        let config = if self.needs_normalization() {
            self.get_normalized_config()?
        } else {
            self.config.clone()
        };

        self.scale_impl.scale_scalar_to_numeric(&config, value)
    }

    pub fn invert_from_numeric(
        &self,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // Auto-normalize if needed
        let config = if self.needs_normalization() {
            self.get_normalized_config()?
        } else {
            self.config.clone()
        };

        self.scale_impl.invert_from_numeric(&config, values)
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
        // Auto-normalize if needed
        let config = if self.needs_normalization() {
            self.get_normalized_config()?
        } else {
            self.config.clone()
        };

        self.scale_impl.invert(&config, values)
    }

    pub fn invert_scalar(&self, value: f32) -> Result<f32, AvengerScaleError> {
        // Auto-normalize if needed
        let config = if self.needs_normalization() {
            self.get_normalized_config()?
        } else {
            self.config.clone()
        };

        self.scale_impl.invert_scalar(&config, value)
    }

    /// Invert a range interval to a subset of the domain
    pub fn invert_range_interval(&self, range: (f32, f32)) -> Result<ArrayRef, AvengerScaleError> {
        // Auto-normalize if needed
        let config = if self.needs_normalization() {
            self.get_normalized_config()?
        } else {
            self.config.clone()
        };

        self.scale_impl.invert_range_interval(&config, range)
    }

    /// Get the domain values for ticks for the scale
    /// These can be scaled to number for position, and scaled to string for labels
    pub fn ticks(&self, count: Option<f32>) -> Result<ArrayRef, AvengerScaleError> {
        // Auto-normalize if needed for ticks
        let config = if self.needs_normalization() {
            self.get_normalized_config()?
        } else {
            self.config.clone()
        };

        self.scale_impl.ticks(&config, count)
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

    /// Returns the normalized domain that will be used for scaling operations.
    ///
    /// This includes any domain expansions from:
    /// - `zero` option: ensures domain includes zero
    /// - `nice` option: rounds domain to nice values
    /// - `clip_padding_lower/upper` options: expands domain to prevent clipping
    ///
    /// For scales that don't support normalization (e.g., categorical scales),
    /// this returns the original domain.
    ///
    /// # Example
    /// ```no_run
    /// # use avenger_scales::scales::linear::LinearScale;
    /// let scale = LinearScale::configured((3.7, 97.2), (0.0, 100.0))
    ///     .with_option("nice", true)
    ///     .with_option("zero", true);
    ///
    /// // Original domain is (3.7, 97.2)
    /// let original = scale.domain();
    ///
    /// // Normalized domain is (0.0, 100.0) after applying zero and nice
    /// let normalized = scale.normalized_domain()?;
    /// # Ok::<(), avenger_scales::error::AvengerScaleError>(())
    /// ```
    pub fn normalized_domain(&self) -> Result<ArrayRef, AvengerScaleError> {
        if self.needs_normalization() {
            let normalized_config = self.get_normalized_config()?;
            Ok(normalized_config.domain)
        } else {
            Ok(self.config.domain.clone())
        }
    }

    pub fn numeric_interval_domain(&self) -> Result<(f32, f32), AvengerScaleError> {
        self.config.numeric_interval_domain()
    }

    pub fn range(&self) -> &ArrayRef {
        &self.config.range
    }

    pub fn numeric_interval_range(&self) -> Result<(f32, f32), AvengerScaleError> {
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

    pub fn option_f32(&self, key: &str, default: f32) -> f32 {
        self.config.option_f32(key, default)
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

/// Formatter pass through methods
impl ConfiguredScale {
    pub fn format(&self, values: &ArrayRef) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        self.config.context.formatters.format(values, None)
    }

    pub fn format_numbers(&self, values: &[Option<f32>]) -> ScalarOrArray<String> {
        ScalarOrArray::new_array(self.config.context.formatters.number.format(values, None))
    }

    pub fn format_dates(&self, values: &[Option<NaiveDate>]) -> ScalarOrArray<String> {
        ScalarOrArray::new_array(self.config.context.formatters.date.format(values, None))
    }

    pub fn format_timestamps(&self, values: &[Option<NaiveDateTime>]) -> ScalarOrArray<String> {
        ScalarOrArray::new_array(
            self.config
                .context
                .formatters
                .timestamp
                .format(values, None),
        )
    }

    pub fn format_timestamptz(&self, values: &[Option<DateTime<Utc>>]) -> ScalarOrArray<String> {
        ScalarOrArray::new_array(
            self.config
                .context
                .formatters
                .timestamptz
                .format(values, None),
        )
    }
}
