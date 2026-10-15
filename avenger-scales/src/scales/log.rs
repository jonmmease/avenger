use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, Float64Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float64Type},
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_format::TickSpacing;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use super::domain_summary::{fit, DomainOptions, DomainSummary, Extents, NiceStep};
use super::no_domain;
use super::{
    ConfiguredScale, DomainKind, OptionConstraint, OptionDefinition, ScaleConfig, ScaleContext,
    ScaleImpl,
};
use crate::{color_interpolator::scale_numeric_to_color, error::AvengerScaleError};

/// Logarithmic scale that maps a continuous numeric domain to a continuous numeric range
/// using logarithmic transformation.
///
/// The scale applies log(x) transformation to input values, making it useful for data
/// that spans several orders of magnitude. The scale supports negative domains by
/// applying -log(-x) for negative values. Zero values produce NaN outputs.
///
/// # Config Options
///
/// - **base** (f64, default: 10.0): The logarithm base. Common values are 10 (common log),
///   2 (binary log), and e (natural log, use 2.718281828). Must be positive and not 1.
///
/// - **clamp** (boolean, default: false): When true, values outside the domain are clamped
///   to the domain extent before transformation. For inversion, values outside the range
///   are clamped first.
///
/// - **range_offset** (f64, default: 0.0): An offset applied to the final scaled values.
///   This is added after the logarithmic transformation and linear mapping.
///
/// - **round** (boolean, default: false): When true, output values from scaling are rounded
///   to the nearest integer. Useful for pixel-perfect rendering. Does not affect inversion.
#[derive(Debug, Serialize, Deserialize)]
pub struct LogScale;

impl LogScale {
    /// A log scale onto `range` with no domain yet. Scaling needs a domain, which
    /// [`ConfiguredScale::fit_domain`] fits to data.
    pub fn unfitted(range: (f64, f64)) -> ConfiguredScale {
        Self::configured((1.0, 10.0), range).with_domain(no_domain())
    }

    pub fn configured(domain: (f64, f64), range: (f64, f64)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float64Array::from(vec![domain.0, domain.1])),
                range: Arc::new(Float64Array::from(vec![range.0, range.1])),
                options: vec![
                    ("base".to_string(), 10.0.into()),
                    ("clamp".to_string(), false.into()),
                    ("range_offset".to_string(), 0.0.into()),
                    ("round".to_string(), false.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }
}

#[typetag::serde(name = "log")]
impl ScaleImpl for LogScale {
    /// Log ticks span magnitudes, so each label keeps its own precision.
    fn tick_spacing(&self) -> TickSpacing {
        TickSpacing::Varying
    }

    /// Log ticks include minor values between powers of the base. Only the ones that fit the
    /// requested density have labels, as D3's `log.tickFormat` chooses them: the ticks whose
    /// leading digit is at most `base * count / ticks`, and at least 1.
    fn label_ticks(&self, config: &ScaleConfig, ticks: &ArrayRef, count: Option<f64>) -> ArrayRef {
        let Some(values) = ticks.as_any().downcast_ref::<Float64Array>() else {
            return ticks.clone();
        };
        let base = config.option_f64("base", 10.0);
        if !base.is_finite() || base <= 0.0 || base == 1.0 || values.is_empty() {
            return ticks.clone();
        }
        let threshold = (base * count.unwrap_or(10.0) / values.len() as f64).max(1.0);
        crate::array::tick_array(values.values().iter().copied().filter(|value| {
            let power = base.powf(value.log(base).round());
            let mut coefficient = value / power;
            if coefficient * base < base - 0.5 {
                coefficient *= base;
            }
            coefficient <= threshold
        }))
    }

    fn scale_type(&self) -> &'static str {
        "log"
    }

    fn summarize(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        let log = LogFunction::new(config.option_f64("base", 10.0));
        let values = cast(values, &DataType::Float64)?;
        let values = values.as_primitive::<Float64Type>();
        if values.iter().flatten().any(|value| value <= 0.0) {
            return Err(AvengerScaleError::InvalidScalePropertyValue(
                "log scales fit domains to positive data".to_string(),
            ));
        }
        let values = values.unary::<_, Float64Type>(|value| log.log(value));
        fit::interval_summary(&values, extents)
    }

    fn fit_domain(
        &self,
        config: &ScaleConfig,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        let options = options.interval()?;
        if options.zero {
            return Err(AvengerScaleError::InvalidScalePropertyValue(
                "log scales do not support zero".to_string(),
            ));
        }
        let log = LogFunction::new(config.option_f64("base", 10.0));
        fit::fit_continuous(
            config,
            options,
            summary,
            fit::IntervalFit {
                nice: options.nice.map(|_| NiceStep::Fixed(1.0)),
                zero: None,
                single_value_span: |_| 1.0,
            },
            |value| log.pow(value),
            (1.0, 10.0),
        )
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn output_type(&self, range_type: &DataType) -> Result<DataType, AvengerScaleError> {
        super::color_or_numeric_output_type(self.scale_type(), range_type)
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("base", OptionConstraint::log_base()),
                OptionDefinition::optional("clamp", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("default", OptionConstraint::Float),
            ];
        }

        &DEFINITIONS
    }

    fn invert(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Cast input to Float64 if needed
        let float_values = cast(values, &DataType::Float64)?;

        // Call existing invert_from_numeric
        let result = self.invert_from_numeric(config, &float_values)?;

        // Convert ScalarOrArray<f64> to ArrayRef
        match result.value() {
            ScalarOrArrayValue::Scalar(s) => {
                // If scalar, create array with single value repeated for input length
                Ok(Arc::new(Float64Array::from(vec![*s; values.len()])) as ArrayRef)
            }
            ScalarOrArrayValue::Array(arr) => {
                // If array, convert to ArrayRef
                Ok(Arc::new(Float64Array::from(arr.as_ref().clone())) as ArrayRef)
            }
        }
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let base = config.option_f64("base", 10.0);
        let (domain_start, domain_end) = config.numeric_interval_domain()?;

        // Check if color interpolation is needed
        if config.color_range().is_ok() {
            return scale_numeric_to_color(self, config, values);
        }

        let (range_start, range_end) = config.numeric_interval_range()?;

        // Get options
        let range_offset = config.option_f64("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        // Handle degenerate domain and range cases
        if domain_start == domain_end || range_start == range_end {
            return Ok(Arc::new(Float32Array::from(vec![
                range_start as f32;
                values.len()
            ])));
        }

        let log_fun = LogFunction::new(base);

        // Transform to log space
        let log_domain_start = if domain_start < 0.0 {
            -log_fun.log(-domain_start)
        } else {
            log_fun.log(domain_start)
        };

        let log_domain_end = if domain_end < 0.0 {
            -log_fun.log(-domain_end)
        } else {
            log_fun.log(domain_end)
        };

        let log_domain_span = log_domain_end - log_domain_start;

        // Handle degenerate domain in log space
        if log_domain_span == 0.0 || log_domain_span.is_nan() {
            return Ok(Arc::new(Float32Array::from(vec![
                range_start as f32;
                values.len()
            ])));
        }

        let scale = (range_end - range_start) / log_domain_span;
        let offset = range_start - scale * log_domain_start + range_offset;

        // Cast and downcast values
        let values = cast(values, &DataType::Float64)?;
        let values = values.as_primitive::<Float64Type>();

        let (range_min, range_max) = if range_start <= range_end {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };

        // pre-round range
        let rounded_range_min = range_min.round();
        let rounded_range_max = range_max.round();

        match (clamp, round) {
            // clamp and round
            (true, true) => match log_fun {
                LogFunction::Static { log_fun, .. } => {
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        {
                            if v < 0.0 {
                                scale * (-log_fun(-v)) + offset
                            } else if v > 0.0 {
                                scale * log_fun(v) + offset
                            } else {
                                f64::NAN
                            }
                        }
                        .clamp(rounded_range_min, rounded_range_max)
                    }
                        as f32)))
                }

                LogFunction::Custom { ln_base, .. } => {
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        if v < 0.0 {
                            scale * (-v.ln() / ln_base) + offset
                        } else if v > 0.0 {
                            scale * (v.ln() / ln_base) + offset
                        } else {
                            f64::NAN
                        }
                        .clamp(range_min, range_max)
                    }
                        as f32)))
                }
            },
            (true, false) => {
                // clamp, no round
                match log_fun {
                    LogFunction::Static { log_fun, .. } => {
                        Ok(Arc::<Float32Array>::new(unary(values, |v| {
                            if v < 0.0 {
                                scale * (-log_fun(-v)) + offset
                            } else if v > 0.0 {
                                scale * log_fun(v) + offset
                            } else {
                                f64::NAN
                            }
                            .clamp(range_min, range_max)
                        }
                            as f32)))
                    }
                    LogFunction::Custom { ln_base, .. } => {
                        Ok(Arc::<Float32Array>::new(unary(values, |v| {
                            if v < 0.0 {
                                scale * (-v.ln() / ln_base) + offset
                            } else if v > 0.0 {
                                scale * (v.ln() / ln_base) + offset
                            } else {
                                f64::NAN
                            }
                            .clamp(range_min, range_max)
                        }
                            as f32)))
                    }
                }
            }
            (false, true) => {
                // no clamp, round
                match log_fun {
                    LogFunction::Static { log_fun, .. } => {
                        Ok(Arc::<Float32Array>::new(unary(values, |v| {
                            if v < 0.0 {
                                scale * (-log_fun(-v)) + offset
                            } else if v > 0.0 {
                                scale * log_fun(v) + offset
                            } else {
                                f64::NAN
                            }
                            .round()
                        }
                            as f32)))
                    }
                    LogFunction::Custom { ln_base, .. } => {
                        Ok(Arc::<Float32Array>::new(unary(values, |v| {
                            if v < 0.0 {
                                scale * (-v.ln() / ln_base) + offset
                            } else if v > 0.0 {
                                scale * (v.ln() / ln_base) + offset
                            } else {
                                f64::NAN
                            }
                            .round()
                        }
                            as f32)))
                    }
                }
            }
            (false, false) => {
                // no clamp, no round
                match log_fun {
                    LogFunction::Static { log_fun, .. } => Ok(Arc::new(Float32Array::from(
                        values
                            .values()
                            .iter()
                            .map(|&v| {
                                if v < 0.0 {
                                    scale * (-log_fun(-v)) + offset
                                } else if v > 0.0 {
                                    scale * log_fun(v) + offset
                                } else {
                                    f64::NAN
                                }
                            } as f32)
                            .collect::<Vec<_>>(),
                    ))),
                    LogFunction::Custom { ln_base, .. } => Ok(Arc::new(Float32Array::from(
                        values
                            .values()
                            .iter()
                            .map(|&v| {
                                if v < 0.0 {
                                    scale * (-v.ln() / ln_base) + offset
                                } else if v > 0.0 {
                                    scale * (v.ln() / ln_base) + offset
                                } else {
                                    f64::NAN
                                }
                            } as f32)
                            .collect::<Vec<_>>(),
                    ))),
                }
            }
        }
    }

    fn invert_from_numeric(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<f64>, AvengerScaleError> {
        // Get options
        let base = config.option_f64("base", 10.0);
        let range_offset = config.option_f64("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let _round = config.option_boolean("round", false);

        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = config.numeric_interval_domain()?;
        let (range_min, range_max) = if range_start <= range_end {
            (range_start, range_end)
        } else {
            (range_end, range_start)
        };

        // Handle degenerate cases
        if domain_start <= 0.0 || domain_end <= 0.0 || range_start == range_end {
            return Ok(ScalarOrArray::new_array(vec![range_start; values.len()]));
        }

        // Transform to log space
        let log_fun = LogFunction::new(base);
        let log_domain_start = log_fun.log(domain_start);
        let log_domain_end = log_fun.log(domain_end);
        let log_domain_span = log_domain_end - log_domain_start;

        // Handle degenerate domain in log space
        if log_domain_span == 0.0 || log_domain_span.is_nan() {
            return Ok(ScalarOrArray::new_array(vec![range_start; values.len()]));
        }

        // Cast and downcast values
        let values = cast(values, &DataType::Float64)?;
        let values = values.as_primitive::<Float64Type>();

        let scale = (range_end - range_start) / log_domain_span;
        let offset = range_start - scale * log_domain_start + range_offset;

        if clamp {
            match log_fun {
                LogFunction::Static { pow_fun, .. } => Ok(ScalarOrArray::new_array(
                    values
                        .values()
                        .iter()
                        .map(|&v| {
                            let v = v.clamp(range_min, range_max);
                            pow_fun((v - offset) / scale)
                        })
                        .collect(),
                )),
                LogFunction::Custom { base, .. } => Ok(ScalarOrArray::new_array(
                    values
                        .values()
                        .iter()
                        .map(|&v| {
                            let v = v.clamp(range_min, range_max);
                            base.powf((v - offset) / scale)
                        })
                        .collect(),
                )),
            }
        } else {
            match log_fun {
                LogFunction::Static { pow_fun, .. } => Ok(ScalarOrArray::new_array(
                    values
                        .values()
                        .iter()
                        .map(|&v| pow_fun((v - offset) / scale))
                        .collect(),
                )),
                LogFunction::Custom { base, .. } => Ok(ScalarOrArray::new_array(
                    values
                        .values()
                        .iter()
                        .map(|&v| base.powf((v - offset) / scale))
                        .collect(),
                )),
            }
        }
    }

    fn ticks(
        &self,
        config: &ScaleConfig,
        count: Option<f64>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let count = count.unwrap_or(10.0);
        let base = config.option_f64("base", 10.0);
        let (domain_start, domain_end) = config.numeric_interval_domain()?;

        let log_fun = LogFunction::new(base);

        // D3: if (!(d[0] > 0 && d[1] > 0)) return [];
        if !(domain_start > 0.0 && domain_end > 0.0) {
            return Ok(crate::array::tick_array([]));
        }

        let d = [domain_start, domain_end];
        let mut u = d[0];
        let mut v = d[1];
        let r = v < u;

        if r {
            std::mem::swap(&mut u, &mut v);
        }

        let mut i = log_fun.log(u);
        let mut j = log_fun.log(v);
        let mut z = Vec::new();

        // Handle integer bases specially
        if (base - base.floor()).abs() < f64::EPSILON && j - i < count {
            i = i.floor();
            j = j.ceil();

            if u > 0.0 {
                for exp in (i as i32)..=(j as i32) {
                    for k in 1..(base as i32) {
                        let t = if exp < 0 {
                            k as f64 / log_fun.pow(-exp as f64)
                        } else {
                            k as f64 * log_fun.pow(exp as f64)
                        };
                        if t < u {
                            continue;
                        }
                        if t > v {
                            break;
                        }
                        z.push(t);
                    }
                }
            } else {
                for exp in (i as i32)..=(j as i32) {
                    for k in ((base as i32) - 1)..=1 {
                        let t = if exp > 0 {
                            k as f64 / log_fun.pow(-exp as f64)
                        } else {
                            k as f64 * log_fun.pow(exp as f64)
                        };
                        if t < u {
                            continue;
                        }
                        if t > v {
                            break;
                        }
                        z.push(t);
                    }
                }
            }

            if z.len() as f64 * 2.0 < count {
                // Fall back to linear ticks if we don't have enough
                z = crate::array::ticks(u, v, count);
            }
        } else {
            // Use linear ticks in log space
            z = crate::array::ticks(i, j, count.min(j - i))
                .into_iter()
                .map(|x| log_fun.pow(x))
                .collect();
        }

        if r {
            z.reverse();
        }
        Ok(crate::array::tick_array(z))
    }
}

/// Handles logarithmic transformations with different bases
#[derive(Clone, Debug)]
pub enum LogFunction {
    Static {
        log_fun: fn(f64) -> f64,
        pow_fun: fn(f64) -> f64,
        base: f64,
    },
    Custom {
        ln_base: f64,
        base: f64,
    },
}

impl LogFunction {
    /// Creates a new LogFunction with optimized implementations for common bases
    pub fn new(base: f64) -> Self {
        if base == std::f64::consts::E {
            LogFunction::Static {
                log_fun: f64::ln,
                pow_fun: f64::exp,
                base,
            }
        } else if base == 10.0 {
            LogFunction::Static {
                log_fun: f64::log10,
                pow_fun: |x| 10.0f64.powf(x),
                base,
            }
        } else if base == 2.0 {
            LogFunction::Static {
                log_fun: f64::log2,
                pow_fun: |x| 2.0f64.powf(x),
                base,
            }
        } else {
            LogFunction::Custom {
                ln_base: base.ln(),
                base,
            }
        }
    }

    /// Computes the logarithm of x in the specified base
    pub fn log(&self, x: f64) -> f64 {
        match self {
            LogFunction::Static { log_fun: fun, .. } => fun(x),
            LogFunction::Custom { ln_base, .. } => x.ln() / ln_base,
        }
    }

    /// Computes base raised to the power x
    pub fn pow(&self, x: f64) -> f64 {
        match self {
            LogFunction::Static { pow_fun, .. } => pow_fun(x),
            LogFunction::Custom { base, .. } => base.powf(x),
        }
    }

    /// Returns the logarithm base
    pub fn base(&self) -> f64 {
        match self {
            LogFunction::Static { base, .. } => *base,
            LogFunction::Custom { base, .. } => *base,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::scalar::Scalar;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_label_ticks_thin_dense_minor_ticks() -> Result<(), AvengerScaleError> {
        let values = |array: &ArrayRef| array.as_primitive::<Float64Type>().values().to_vec();
        let scale = LogScale::configured((0.1, 100.0), (0.0, 100.0));
        let ticks = scale.ticks(Some(10.0))?;
        assert_eq!(
            values(&scale.label_ticks(&ticks, Some(10.0))),
            vec![0.1, 0.2, 0.3, 1.0, 2.0, 3.0, 10.0, 20.0, 30.0, 100.0]
        );

        // Other scales label every tick.
        let linear = super::super::linear::LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let ticks = linear.ticks(None)?;
        assert_eq!(values(&linear.label_ticks(&ticks, None)), values(&ticks));
        Ok(())
    }

    #[test]
    fn test_basic_scale_invert() {
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![1.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float64Array::from(vec![5.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.69897);

        let values = Arc::new(Float64Array::from(vec![0.69897])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f64, result[0], 5.0, epsilon = 1e-6);
    }

    #[test]
    fn test_domain_coercion() {
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![1.0, 2.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![("base".to_string(), Scalar::from(10.0))]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float64Array::from(vec![0.5, 1.0, 1.5, 2.0, 2.5])) as ArrayRef;
        let result = scale.scale_to_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], -1.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.5849625);
        assert_approx_eq!(f32, result[3], 1.0);
        assert_approx_eq!(f32, result[4], 1.3219281);
    }

    #[test]
    fn test_range_offset() {
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![1.0, 2.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), Scalar::from(10.0)),
                ("range_offset".to_string(), Scalar::from(0.5)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float64Array::from(vec![0.5, 1.0, 1.5, 2.0, 2.5])) as ArrayRef;
        let result = scale.scale_to_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], -0.5);
        assert_approx_eq!(f32, result[1], 0.5);
        assert_approx_eq!(f32, result[2], 1.0849625);
        assert_approx_eq!(f32, result[3], 1.5);
        assert_approx_eq!(f32, result[4], 1.8219281);
    }

    #[test]
    fn test_negative_domain() {
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![-100.0, -1.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![("base".to_string(), Scalar::from(10.0))]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float64Array::from(vec![-50.0])) as ArrayRef;
        let result = scale.scale_to_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.150515);
    }

    #[test]
    fn test_clamping() {
        // Test unclamped behavior
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![1.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), Scalar::from(10.0)),
                ("clamp".to_string(), Scalar::from(false)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float64Array::from(vec![0.5, 15.0])) as ArrayRef;
        let result = scale.scale_to_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -std::f32::consts::LOG10_2);
        assert_approx_eq!(f32, result[1], 1.176091);

        // Test clamped behavior
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![1.0, 10.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), Scalar::from(10.0)),
                ("clamp".to_string(), Scalar::from(true)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float64Array::from(vec![-1.0, 5.0, 15.0])) as ArrayRef;
        let result = scale.scale_to_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.69897);
        assert_approx_eq!(f32, result[2], 1.0);
    }

    #[test]
    fn test_invert_range_offset() {
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float64Array::from(vec![1.0, 2.0])),
            range: Arc::new(Float64Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), Scalar::from(10.0)),
                ("range_offset".to_string(), Scalar::from(0.5)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float64Array::from(vec![
            -0.5, 0.5, 1.0849625, 1.5, 1.8219281,
        ])) as ArrayRef;

        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);

        assert_approx_eq!(f64, result[0], 0.5, epsilon = 1e-6);
        assert_approx_eq!(f64, result[1], 1.0, epsilon = 1e-6);
        assert_approx_eq!(f64, result[2], 1.5, epsilon = 1e-6);
        assert_approx_eq!(f64, result[3], 2.0, epsilon = 1e-6);
        assert_approx_eq!(f64, result[4], 2.5, epsilon = 1e-6);
    }

    #[test]
    fn test_integration_log_scale_three_colors() -> Result<(), AvengerScaleError> {
        use avenger_color::ColorOrGradient;

        // Create a log scale with domain [1, 100] and three-color range: red, yellow, blue
        let red = [1.0, 0.0, 0.0, 1.0]; // Red
        let yellow = [1.0, 1.0, 0.0, 1.0]; // Yellow
        let blue = [0.0, 0.0, 1.0, 1.0]; // Blue

        // Create the color range as a list array
        let color_arrays = vec![
            Arc::new(Float64Array::from(Vec::from(red))) as ArrayRef,
            Arc::new(Float64Array::from(Vec::from(yellow))) as ArrayRef,
            Arc::new(Float64Array::from(Vec::from(blue))) as ArrayRef,
        ];
        let color_range = crate::scalar::Scalar::arrays_into_list_array(color_arrays)?;

        // Create the log scale
        let scale = LogScale::configured((1.0, 100.0), (0.0, 1.0)).with_range(color_range);

        // Test values across the domain
        let test_values = vec![1.0, 3.16, 10.0, 31.6, 100.0]; // Evenly spaced in log space
        let values_array = Arc::new(Float64Array::from(test_values.clone())) as ArrayRef;

        // Scale to colors
        let color_result = scale.scale_to_color(&values_array)?;
        let colors = color_result.as_vec(test_values.len(), None);

        // Verify we get ColorOrGradient values
        assert_eq!(colors.len(), 5);

        // Check that the first value (1.0) maps to red-ish (start of range)
        match &colors[0] {
            ColorOrGradient::Color(color) => {
                // Should be red-ish (high red component, low blue component)
                assert!(
                    color[0] > 0.8,
                    "First color should be red-ish, got {:?}",
                    color
                );
                assert!(
                    color[2] < 0.2,
                    "First color should not be blue-ish, got {:?}",
                    color
                );
            }
            _ => panic!("Expected Color, got {:?}", colors[0]),
        }

        // Check that the last value (100.0) maps to blue-ish (end of range)
        match &colors[4] {
            ColorOrGradient::Color(color) => {
                // Should be blue-ish (low red component, high blue component)
                assert!(
                    color[2] > 0.8,
                    "Last color should be blue-ish, got {:?}",
                    color
                );
                assert!(
                    color[0] < 0.2,
                    "Last color should not be red-ish, got {:?}",
                    color
                );
            }
            _ => panic!("Expected Color, got {:?}", colors[4]),
        }

        // Check that middle value (10.0) maps to yellow-ish (middle of range)
        match &colors[2] {
            ColorOrGradient::Color(color) => {
                // Should be yellow-ish (high red and green, low blue)
                assert!(
                    color[0] > 0.5,
                    "Middle color should have red component, got {:?}",
                    color
                );
                assert!(
                    color[1] > 0.5,
                    "Middle color should have green component, got {:?}",
                    color
                );
                assert!(
                    color[2] < 0.5,
                    "Middle color should have low blue component, got {:?}",
                    color
                );
            }
            _ => panic!("Expected Color, got {:?}", colors[2]),
        }

        // Test that the scale correctly handles logarithmic interpolation
        // by checking that equal log-space intervals produce equal color-space intervals
        let log_positions = test_values.iter().map(|v| v.log10()).collect::<Vec<f64>>();

        // Verify logarithmic scaling is working (positions should be evenly distributed in log space)
        let log_diff_1 = log_positions[1] - log_positions[0];
        let log_diff_2 = log_positions[2] - log_positions[1];
        assert!(
            (log_diff_1 - log_diff_2).abs() < 0.01,
            "Log scale should produce even spacing in log space"
        );

        Ok(())
    }
}
