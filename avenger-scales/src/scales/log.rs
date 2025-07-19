use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float32Type},
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use lazy_static::lazy_static;

use crate::{color_interpolator::scale_numeric_to_color, error::AvengerScaleError, scalar::Scalar};

use super::{
    ConfiguredScale, InferDomainFromDataMethod, OptionConstraint, OptionDefinition, ScaleConfig,
    ScaleContext, ScaleImpl,
};

/// Logarithmic scale that maps a continuous numeric domain to a continuous numeric range
/// using logarithmic transformation.
///
/// The scale applies log(x) transformation to input values, making it useful for data
/// that spans several orders of magnitude. The scale supports negative domains by
/// applying -log(-x) for negative values. Zero values produce NaN outputs.
///
/// # Config Options
///
/// - **base** (f32, default: 10.0): The logarithm base. Common values are 10 (common log),
///   2 (binary log), and e (natural log, use 2.718281828). Must be positive and not 1.
///
/// - **clamp** (boolean, default: false): When true, values outside the domain are clamped
///   to the domain extent before transformation. For inversion, values outside the range
///   are clamped first.
///
/// - **range_offset** (f32, default: 0.0): An offset applied to the final scaled values.
///   This is added after the logarithmic transformation and linear mapping.
///
/// - **round** (boolean, default: false): When true, output values from scaling are rounded
///   to the nearest integer. Useful for pixel-perfect rendering. Does not affect inversion.
///
/// - **nice** (boolean or f32, default: false): When true or a number, extends the domain
///   to nice round values in logarithmic space (powers of the base). If true, uses a
///   default count of 10. If a number, uses that as the target tick count. For example,
///   with base 10, a domain of [8, 95] might become [1, 100].
///
/// - **padding** (f32, default: 0.0): Expands the scale domain by the specified number of pixels
///   on each side of the scale range. The domain expansion is computed in logarithmic space.
///   Applied before zero and nice transformations. Must be non-negative.
///
/// - **zero** (boolean, default: false): When true, ensures that the domain includes zero. However, zero
///   is invalid for logarithmic scales, so this option is ignored for log scales.
#[derive(Debug)]
pub struct LogScale;

impl LogScale {
    pub fn configured(domain: (f32, f32), range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain.0, domain.1])),
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
                    ("base".to_string(), 10.0.into()),
                    ("clamp".to_string(), false.into()),
                    ("range_offset".to_string(), 0.0.into()),
                    ("round".to_string(), false.into()),
                    ("nice".to_string(), false.into()),
                    ("zero".to_string(), false.into()),
                    ("padding".to_string(), 0.0.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }

    /// Compute nice domain
    pub fn apply_nice(
        domain: (f32, f32),
        base: f32,
        count: Option<&Scalar>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        // Extract count, or return raw domain if no nice option
        let _count = if let Some(count) = count {
            if count.array().data_type().is_numeric() {
                count.as_f32()?
            } else if let Ok(true) = count.as_boolean() {
                10.0
            } else {
                return Ok(domain);
            }
        } else {
            return Ok(domain);
        };

        let log_fun = LogFunction::new(base);

        let (mut domain_start, mut domain_end) = domain;

        if domain_start.is_nan() || domain_end.is_nan() {
            return Ok(domain);
        }

        // Special case for exact zero domain
        if domain_start == 0.0 && domain_end == 0.0 {
            return Ok(domain);
        }

        // Handle degenerate non-zero domain by expanding to nearest log boundaries
        if domain_start == domain_end && domain_start != 0.0 {
            let value = domain_start;
            let log_val = log_fun.log(value.abs());
            domain_start = log_fun.pow(log_val.floor());
            domain_end = log_fun.pow(log_val.ceil());
            return Ok((domain_start, domain_end));
        }

        let (start, stop, reverse) = if domain_start < domain_end {
            (domain_start, domain_end, false)
        } else {
            (domain_end, domain_start, true)
        };

        // Handle negative domains
        if start < 0.0 && stop < 0.0 {
            let nstart = -stop;
            let nstop = -start;

            let nstart = log_fun.pow(log_fun.log(nstart).floor());
            let nstop = log_fun.pow(log_fun.log(nstop).ceil());

            if reverse {
                domain_start = -nstart;
                domain_end = -nstop;
            } else {
                domain_start = -nstop;
                domain_end = -nstart;
            }
        } else {
            let nstart = log_fun.pow(log_fun.log(start).floor());
            let nstop = log_fun.pow(log_fun.log(stop).ceil());

            if reverse {
                domain_start = nstop;
                domain_end = nstart;
            } else {
                domain_start = nstart;
                domain_end = nstop;
            }
        }
        Ok((domain_start, domain_end))
    }

    /// Apply padding to a log scale domain
    /// Transforms to log space, applies linear padding, then transforms back
    pub fn apply_padding(
        domain: (f32, f32),
        range: (f32, f32),
        padding: f32,
        base: f32,
    ) -> Result<(f32, f32), AvengerScaleError> {
        let (domain_start, domain_end) = domain;
        let (range_start, range_end) = range;

        // Early return for degenerate cases
        if domain_start == domain_end || range_start == range_end || padding <= 0.0 {
            return Ok(domain);
        }

        // Handle domains that include zero or negative values
        if domain_start <= 0.0 || domain_end <= 0.0 {
            // Can't apply padding to domains that include zero for log scales
            return Ok(domain);
        }

        let log_fun = LogFunction::new(base);

        // Transform to log space
        let log_start = log_fun.log(domain_start);
        let log_end = log_fun.log(domain_end);

        // Calculate the span of the range in pixels
        let span = (range_end - range_start).abs();

        // Calculate scale factor: frac = span / (span - 2 * pad)
        let frac = span / (span - 2.0 * padding);

        // For log scale, zoom from center in log space
        let log_center = (log_start + log_end) / 2.0;

        // Expand domain in log space by scale factor
        let new_log_start = log_center + (log_start - log_center) * frac;
        let new_log_end = log_center + (log_end - log_center) * frac;

        // Transform back to linear space
        let new_start = log_fun.pow(new_log_start);
        let new_end = log_fun.pow(new_log_end);

        Ok((new_start, new_end))
    }

    /// Apply normalization (padding, zero and nice) to domain
    /// For log scales, zero is ignored since it's invalid in logarithmic space
    pub fn apply_normalization(
        domain: (f32, f32),
        range: (f32, f32),
        padding: Option<&Scalar>,
        base: f32,
        _zero: Option<&Scalar>, // Zero is ignored for log scales
        nice: Option<&Scalar>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        let mut current_domain = domain;

        // Apply padding first
        if let Some(padding) = padding {
            if let Ok(padding_value) = padding.as_f32() {
                if padding_value > 0.0 {
                    current_domain =
                        Self::apply_padding(current_domain, range, padding_value, base)?;
                }
            }
        }

        // For log scales, zero is invalid, so we only apply nice transformation
        Self::apply_nice(current_domain, base, nice)
    }
}

impl ScaleImpl for LogScale {
    fn scale_type(&self) -> &'static str {
        "log"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("base", OptionConstraint::log_base()),
                OptionDefinition::optional("clamp", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("nice", OptionConstraint::nice()),
                OptionDefinition::optional("padding", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional("zero", OptionConstraint::Boolean),
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
        // Cast input to Float32 if needed
        let float_values = cast(values, &DataType::Float32)?;

        // Call existing invert_from_numeric
        let result = self.invert_from_numeric(config, &float_values)?;

        // Convert ScalarOrArray<f32> to ArrayRef
        match result.value() {
            ScalarOrArrayValue::Scalar(s) => {
                // If scalar, create array with single value repeated for input length
                Ok(Arc::new(Float32Array::from(vec![*s; values.len()])) as ArrayRef)
            }
            ScalarOrArrayValue::Array(arr) => {
                // If array, convert to ArrayRef
                Ok(Arc::new(Float32Array::from(arr.as_ref().clone())) as ArrayRef)
            }
        }
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let base = config.option_f32("base", 10.0);

        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));

        let (domain_start, domain_end) = LogScale::apply_normalization(
            config.numeric_interval_domain()?,
            range_for_padding,
            config.options.get("padding"),
            base,
            config.options.get("zero"),
            config.options.get("nice"),
        )?;

        // Check if color interpolation is needed
        if config.color_range().is_ok() {
            // Create new config with normalized domain
            let config = ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain_start, domain_end])),
                ..config.clone()
            };
            return scale_numeric_to_color(self, &config, values);
        }

        let (range_start, range_end) = config.numeric_interval_range()?;

        // Get options
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        // Handle degenerate domain and range cases
        if domain_start == domain_end || range_start == range_end {
            return Ok(Arc::new(Float32Array::from(vec![
                range_start;
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
                range_start;
                values.len()
            ])));
        }

        let scale = (range_end - range_start) / log_domain_span;
        let offset = range_start - scale * log_domain_start + range_offset;

        // Cast and downcast values
        let values = cast(values, &DataType::Float32)?;
        let values = values.as_primitive::<Float32Type>();

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
                                f32::NAN
                            }
                        }
                        .clamp(rounded_range_min, rounded_range_max)
                    })))
                }

                LogFunction::Custom { ln_base, .. } => {
                    Ok(Arc::<Float32Array>::new(unary(values, |v| {
                        if v < 0.0 {
                            scale * (-v.ln() / ln_base) + offset
                        } else if v > 0.0 {
                            scale * (v.ln() / ln_base) + offset
                        } else {
                            f32::NAN
                        }
                        .clamp(range_min, range_max)
                    })))
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
                                f32::NAN
                            }
                            .clamp(range_min, range_max)
                        })))
                    }
                    LogFunction::Custom { ln_base, .. } => {
                        Ok(Arc::<Float32Array>::new(unary(values, |v| {
                            if v < 0.0 {
                                scale * (-v.ln() / ln_base) + offset
                            } else if v > 0.0 {
                                scale * (v.ln() / ln_base) + offset
                            } else {
                                f32::NAN
                            }
                            .clamp(range_min, range_max)
                        })))
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
                                f32::NAN
                            }
                            .round()
                        })))
                    }
                    LogFunction::Custom { ln_base, .. } => {
                        Ok(Arc::<Float32Array>::new(unary(values, |v| {
                            if v < 0.0 {
                                scale * (-v.ln() / ln_base) + offset
                            } else if v > 0.0 {
                                scale * (v.ln() / ln_base) + offset
                            } else {
                                f32::NAN
                            }
                            .round()
                        })))
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
                                    f32::NAN
                                }
                            })
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
                                    f32::NAN
                                }
                            })
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
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        // Get options
        let base = config.option_f32("base", 10.0);
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let _round = config.option_boolean("round", false);

        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = LogScale::apply_normalization(
            config.numeric_interval_domain()?,
            (range_start, range_end),
            config.options.get("padding"),
            base,
            config.options.get("zero"),
            config.options.get("nice"),
        )?;
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
        let values = cast(values, &DataType::Float32)?;
        let values = values.as_primitive::<Float32Type>();

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
        count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let count = count.unwrap_or(10.0);
        let base = config.option_f32("base", 10.0);
        let (domain_start, domain_end) = LogScale::apply_nice(
            config.numeric_interval_domain()?,
            base,
            config.options.get("nice"),
        )?;

        let log_fun = LogFunction::new(base);

        // D3: if (!(d[0] > 0 && d[1] > 0)) return [];
        if !(domain_start > 0.0 && domain_end > 0.0) {
            return Ok(Arc::new(Float32Array::from(Vec::<f32>::new())));
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
        if (base - base.floor()).abs() < f32::EPSILON && j - i < count {
            i = i.floor();
            j = j.ceil();

            if u > 0.0 {
                for exp in (i as i32)..=(j as i32) {
                    for k in 1..(base as i32) {
                        let t = if exp < 0 {
                            k as f32 / log_fun.pow(-exp as f32)
                        } else {
                            k as f32 * log_fun.pow(exp as f32)
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
                            k as f32 / log_fun.pow(-exp as f32)
                        } else {
                            k as f32 * log_fun.pow(exp as f32)
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

            if z.len() as f32 * 2.0 < count {
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
        Ok(Arc::new(Float32Array::from(z)))
    }

    fn compute_nice_domain(&self, config: &ScaleConfig) -> Result<ArrayRef, AvengerScaleError> {
        let base = config.option_f32("base", 10.0);
        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));
        let (domain_start, domain_end) = LogScale::apply_normalization(
            config.numeric_interval_domain()?,
            range_for_padding,
            config.options.get("padding"),
            base,
            config.options.get("zero"),
            config.options.get("nice"),
        )?;

        Ok(Arc::new(Float32Array::from(vec![domain_start, domain_end])) as ArrayRef)
    }
}

/// Handles logarithmic transformations with different bases
#[derive(Clone, Debug)]
pub enum LogFunction {
    Static {
        log_fun: fn(f32) -> f32,
        pow_fun: fn(f32) -> f32,
        base: f32,
    },
    Custom {
        ln_base: f32,
        base: f32,
    },
}

impl LogFunction {
    /// Creates a new LogFunction with optimized implementations for common bases
    pub fn new(base: f32) -> Self {
        if base == std::f32::consts::E {
            LogFunction::Static {
                log_fun: f32::ln,
                pow_fun: f32::exp,
                base,
            }
        } else if base == 10.0 {
            LogFunction::Static {
                log_fun: f32::log10,
                pow_fun: |x| 10.0f32.powf(x),
                base,
            }
        } else if base == 2.0 {
            LogFunction::Static {
                log_fun: f32::log2,
                pow_fun: |x| 2.0f32.powf(x),
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
    pub fn log(&self, x: f32) -> f32 {
        match self {
            LogFunction::Static { log_fun: fun, .. } => fun(x),
            LogFunction::Custom { ln_base, .. } => x.ln() / ln_base,
        }
    }

    /// Computes base raised to the power x
    pub fn pow(&self, x: f32) -> f32 {
        match self {
            LogFunction::Static { pow_fun, .. } => pow_fun(x),
            LogFunction::Custom { base, .. } => base.powf(x),
        }
    }

    /// Returns the logarithm base
    pub fn base(&self) -> f32 {
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
    use float_cmp::{assert_approx_eq, F32Margin};

    #[test]
    fn test_basic_scale_invert() {
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![1.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };

        let values = Arc::new(Float32Array::from(vec![5.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.69897);

        let values = Arc::new(Float32Array::from(vec![0.69897])) as ArrayRef;
        let result = scale
            .invert_from_numeric(&config, &values)
            .unwrap()
            .as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 5.0);
    }

    #[test]
    fn test_domain_coercion() {
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![1.0, 2.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("base".to_string(), Scalar::from(10.0))]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float32Array::from(vec![0.5, 1.0, 1.5, 2.0, 2.5])) as ArrayRef;
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
            domain: Arc::new(Float32Array::from(vec![1.0, 2.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), Scalar::from(10.0)),
                ("range_offset".to_string(), Scalar::from(0.5)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float32Array::from(vec![0.5, 1.0, 1.5, 2.0, 2.5])) as ArrayRef;
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
            domain: Arc::new(Float32Array::from(vec![-100.0, -1.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![("base".to_string(), Scalar::from(10.0))]
                .into_iter()
                .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float32Array::from(vec![-50.0])) as ArrayRef;
        let result = scale.scale_to_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.150515);
    }

    #[test]
    fn test_clamping() {
        // Test unclamped behavior
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![1.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), Scalar::from(10.0)),
                ("clamp".to_string(), Scalar::from(false)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float32Array::from(vec![0.5, 15.0])) as ArrayRef;
        let result = scale.scale_to_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -std::f32::consts::LOG10_2);
        assert_approx_eq!(f32, result[1], 1.176091);

        // Test clamped behavior
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![1.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), Scalar::from(10.0)),
                ("clamp".to_string(), Scalar::from(true)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float32Array::from(vec![-1.0, 5.0, 15.0])) as ArrayRef;
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
            domain: Arc::new(Float32Array::from(vec![1.0, 2.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), Scalar::from(10.0)),
                ("range_offset".to_string(), Scalar::from(0.5)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float32Array::from(vec![
            -0.5, 0.5, 1.0849625, 1.5, 1.8219281,
        ])) as ArrayRef;

        let result = scale.invert_from_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.5);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.5);
        assert_approx_eq!(f32, result[3], 2.0);
        assert_approx_eq!(f32, result[4], 2.5);
    }

    #[test]
    fn test_nice() -> Result<(), AvengerScaleError> {
        // Test nice with ascending domain
        let (domain_start, domain_end) =
            LogScale::apply_nice((1.1, 10.9), 10.0, Some(&10.0.into()))?;
        assert_eq!(domain_start, 1.0);
        assert_eq!(domain_end, 100.0);

        // // Test nice with descending domain
        let (domain_start, domain_end) =
            LogScale::apply_nice((10.9, 1.1), 10.0, Some(&10.0.into()))?;
        assert_eq!(domain_start, 100.0);
        assert_eq!(domain_end, 1.0);

        // Test nice with domain crossing decades
        let (domain_start, domain_end) =
            LogScale::apply_nice((0.7, 11.001), 10.0, Some(&10.0.into()))?;
        assert_eq!(domain_start, 0.1);
        assert_eq!(domain_end, 100.0);

        // Test nice with reversed domain crossing decades
        let (domain_start, domain_end) =
            LogScale::apply_nice((123.1, 6.7), 10.0, Some(&10.0.into()))?;
        assert_eq!(domain_start, 1000.0);
        assert_eq!(domain_end, 1.0);

        // Test nice with small domain
        let (domain_start, domain_end) =
            LogScale::apply_nice((0.01, 0.49), 10.0, Some(&10.0.into()))?;
        assert_eq!(domain_start, 0.01);
        assert_eq!(domain_end, 1.0);

        Ok(())
    }

    #[test]
    fn test_integration_log_scale_three_colors() -> Result<(), AvengerScaleError> {
        use avenger_common::types::ColorOrGradient;

        // Create a log scale with domain [1, 100] and three-color range: red, yellow, blue
        let red = [1.0, 0.0, 0.0, 1.0]; // Red
        let yellow = [1.0, 1.0, 0.0, 1.0]; // Yellow
        let blue = [0.0, 0.0, 1.0, 1.0]; // Blue

        // Create the color range as a list array
        let color_arrays = vec![
            Arc::new(Float32Array::from(Vec::from(red))) as ArrayRef,
            Arc::new(Float32Array::from(Vec::from(yellow))) as ArrayRef,
            Arc::new(Float32Array::from(Vec::from(blue))) as ArrayRef,
        ];
        let color_range = crate::scalar::Scalar::arrays_into_list_array(color_arrays)?;

        // Create the log scale
        let scale = LogScale::configured((1.0, 100.0), (0.0, 1.0)).with_range(color_range);

        // Test values across the domain
        let test_values = vec![1.0, 3.16, 10.0, 31.6, 100.0]; // Evenly spaced in log space
        let values_array = Arc::new(Float32Array::from(test_values.clone())) as ArrayRef;

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
        let log_positions = test_values.iter().map(|v| v.log10()).collect::<Vec<f32>>();

        // Verify logarithmic scaling is working (positions should be evenly distributed in log space)
        let log_diff_1 = log_positions[1] - log_positions[0];
        let log_diff_2 = log_positions[2] - log_positions[1];
        assert!(
            (log_diff_1 - log_diff_2).abs() < 0.01,
            "Log scale should produce even spacing in log space"
        );

        Ok(())
    }

    #[test]
    fn test_apply_padding() -> Result<(), AvengerScaleError> {
        // Test basic padding with base 10
        let result = LogScale::apply_padding((10.0, 100.0), (0.0, 100.0), 10.0, 10.0)?;
        // Domain [10, 100] in log space is [1, 2]
        // With padding 10 on range 100, scale factor = 100 / 80 = 1.25
        // Center in log space is 1.5
        // New log domain: [1.5 + (1 - 1.5) * 1.25, 1.5 + (2 - 1.5) * 1.25] = [0.875, 2.125]
        // Back to linear: [10^0.875, 10^2.125] ≈ [7.498, 133.35]
        assert_approx_eq!(
            f32,
            result.0,
            7.498942,
            F32Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );
        assert_approx_eq!(
            f32,
            result.1,
            133.35214,
            F32Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );

        // Test with base 2
        let result = LogScale::apply_padding((4.0, 16.0), (0.0, 100.0), 10.0, 2.0)?;
        // Domain [4, 16] in log2 space is [2, 4]
        // Center in log2 space is 3
        // New log domain: [3 + (2 - 3) * 1.25, 3 + (4 - 3) * 1.25] = [1.75, 4.25]
        // Back to linear: [2^1.75, 2^4.25] = [3.364, 19.027]
        assert_approx_eq!(
            f32,
            result.0,
            3.3635857,
            F32Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );
        assert_approx_eq!(
            f32,
            result.1,
            19.027313,
            F32Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );

        // Test with zero padding (no change)
        let result = LogScale::apply_padding((10.0, 100.0), (0.0, 100.0), 0.0, 10.0)?;
        assert_approx_eq!(f32, result.0, 10.0);
        assert_approx_eq!(f32, result.1, 100.0);

        // Test with domain including zero (no change)
        let result = LogScale::apply_padding((0.0, 100.0), (0.0, 100.0), 10.0, 10.0)?;
        assert_approx_eq!(f32, result.0, 0.0);
        assert_approx_eq!(f32, result.1, 100.0);

        // Test with negative domain (no change)
        let result = LogScale::apply_padding((-100.0, -10.0), (0.0, 100.0), 10.0, 10.0)?;
        assert_approx_eq!(f32, result.0, -100.0);
        assert_approx_eq!(f32, result.1, -10.0);

        Ok(())
    }

    #[test]
    fn test_log_scale_with_padding() -> Result<(), AvengerScaleError> {
        // Create a log scale with padding
        let scale = LogScale::configured((10.0, 100.0), (0.0, 100.0))
            .with_option("padding", 10.0)
            .with_option("base", 10.0);

        // Normalize the scale to apply padding
        let normalized = scale.normalize()?;

        // Check that domain has been expanded
        let domain = normalized.numeric_interval_domain()?;
        assert_approx_eq!(
            f32,
            domain.0,
            7.498942,
            F32Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );
        assert_approx_eq!(
            f32,
            domain.1,
            133.35214,
            F32Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );

        // Test scaling values
        let values = Arc::new(Float32Array::from(vec![10.0, 31.622776, 100.0])) as ArrayRef;
        let result = normalized.scale(&values)?;
        let result_array = result.as_primitive::<Float32Type>();

        // With expanded domain, the original values should map differently
        // 10 should map to slightly above 0 (around 10)
        // 31.622776 (10^1.5) should map to around 50
        // 100 should map to slightly below 100 (around 90)
        assert_approx_eq!(
            f32,
            result_array.value(0),
            10.0,
            F32Margin {
                epsilon: 0.5,
                ..Default::default()
            }
        );
        assert_approx_eq!(
            f32,
            result_array.value(1),
            50.0,
            F32Margin {
                epsilon: 0.5,
                ..Default::default()
            }
        );
        assert_approx_eq!(
            f32,
            result_array.value(2),
            90.0,
            F32Margin {
                epsilon: 0.5,
                ..Default::default()
            }
        );

        Ok(())
    }

    #[test]
    fn test_padding_with_nice() -> Result<(), AvengerScaleError> {
        // Test that transformations are applied in order: padding -> nice
        let scale = LogScale::configured((10.0, 100.0), (0.0, 100.0))
            .with_option("padding", 10.0)
            .with_option("nice", true)
            .with_option("base", 10.0);

        let normalized = scale.normalize()?;
        let domain = normalized.numeric_interval_domain()?;

        // Expected transformations:
        // 1. Padding: [10, 100] → expanded in log space
        //    - log10(10) = 1, log10(100) = 2
        //    - center = 1.5, span = 1
        //    - frac = 100 / (100 - 20) = 1.25
        //    - new log domain: [1.5 - 0.5*1.25, 1.5 + 0.5*1.25] = [0.875, 2.125]
        //    - linear domain: [10^0.875, 10^2.125] ≈ [7.5, 133.35]
        // 2. Nice: [7.5, 133.35] → [1, 1000] (nice powers of 10)
        assert_approx_eq!(f32, domain.0, 1.0);
        assert_approx_eq!(f32, domain.1, 1000.0);

        // Verify all normalization options are disabled
        assert_eq!(normalized.option_f32("padding", -1.0), 0.0);
        assert!(!normalized.option_boolean("nice", true));

        Ok(())
    }
}
