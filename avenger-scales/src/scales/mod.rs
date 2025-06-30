pub mod band;
pub mod coerce;
pub mod linear;
pub mod log;
pub mod ordinal;
pub mod point;
pub mod pow;
pub mod quantile;
pub mod quantize;
pub mod symlog;
pub mod threshold;

use std::{collections::HashMap, fmt::Debug, sync::Arc};

use crate::{color_interpolator::ColorInterpolator, error::AvengerScaleError, scalar::Scalar};
use crate::{color_interpolator::ColorInterpolatorConfig, formatter::Formatters};
use crate::{
    color_interpolator::SrgbaColorInterpolator,
    scales::coerce::{ColorCoercer, CssColorCoercer},
};
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
}

// Pass through methods
impl ConfiguredScale {
    /// Method that should be used to infer a scale's domain from the data that it will scale
    pub fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        self.scale_impl.infer_domain_from_data_method()
    }

    pub fn scale(&self, values: &ArrayRef) -> Result<ArrayRef, AvengerScaleError> {
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
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
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
    /// # use avenger_scales::LinearScale;
    /// let scale = LinearScale::configured((0.0, 100.0), (0.0, 1.0));
    /// let range_values = Arc::new(Float32Array::from(vec![0.0, 0.5, 1.0])) as ArrayRef;
    /// let domain_values = scale.invert(&range_values)?;
    /// // domain_values will contain [0.0, 50.0, 100.0]
    /// # Ok::<(), avenger_scales::error::AvengerScaleError>(())
    /// ```
    pub fn invert(&self, values: &ArrayRef) -> Result<ArrayRef, AvengerScaleError> {
        self.scale_impl.invert(&self.config, values)
    }

    pub fn invert_scalar(&self, value: f32) -> Result<f32, AvengerScaleError> {
        self.scale_impl.invert_scalar(&self.config, value)
    }

    /// Invert a range interval to a subset of the domain
    pub fn invert_range_interval(&self, range: (f32, f32)) -> Result<ArrayRef, AvengerScaleError> {
        self.scale_impl.invert_range_interval(&self.config, range)
    }

    /// Get the domain values for ticks for the scale
    /// These can be scaled to number for position, and scaled to string for labels
    pub fn ticks(&self, count: Option<f32>) -> Result<ArrayRef, AvengerScaleError> {
        self.scale_impl.ticks(&self.config, count)
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
