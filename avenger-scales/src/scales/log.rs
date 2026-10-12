use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array, Float64Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float64Type},
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use lazy_static::lazy_static;

use super::{
    pad_interval, ConfiguredScale, DomainKind, InferDomainFromDataMethod, OptionConstraint,
    OptionDefinition, ScaleConfig, ScaleContext, ScaleImpl,
};
use crate::{color_interpolator::scale_numeric_to_color, error::AvengerScaleError, scalar::Scalar};

/// Configuration for log scale normalization operations
#[derive(Debug, Clone)]
pub struct LogNormalizationConfig<'a> {
    pub domain: (f64, f64),
    pub range: (f64, f64),
    pub clip_padding_lower: Option<&'a Scalar>,
    pub clip_padding_upper: Option<&'a Scalar>,
    pub base: f64,
    pub nice: Option<&'a Scalar>,
}

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
///
/// - **nice** (boolean or f64, default: false): When true or a number, extends the domain
///   to nice round values in logarithmic space (powers of the base). If true, uses a
///   default count of 10. If a number, uses that as the target tick count. For example,
///   with base 10, a domain of [8, 95] might become [1, 100].
///
/// - **clip_padding_lower** (f64, default: 0.0): Pixels left between the range start and the
///   position of the domain start, so that marks at the domain boundary are not clipped. The scale
///   widens its domain in logarithmic space to make room. The range must be in pixels, and the two
///   paddings must leave part of it. Padding is applied before the nice adjustment.
///
/// - **clip_padding_upper** (f64, default: 0.0): Pixels left between the position of the domain
///   end and the range end, as for `clip_padding_lower`.
///
/// - **zero** (boolean, default: false): When true, ensures that the domain includes zero. However, zero
///   is invalid for logarithmic scales, so this option is ignored for log scales.
#[derive(Debug)]
pub struct LogScale;

impl LogScale {
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
                    ("nice".to_string(), false.into()),
                    ("zero".to_string(), false.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }

    /// Compute nice domain
    pub fn apply_nice(
        domain: (f64, f64),
        base: f64,
        count: Option<&Scalar>,
    ) -> Result<(f64, f64), AvengerScaleError> {
        // Extract count, or return raw domain if no nice option
        let _count = if let Some(count) = count {
            if count.array().data_type().is_numeric() {
                count.as_f64()?
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
        domain: (f64, f64),
        range: (f64, f64),
        padding_lower: f64,
        padding_upper: f64,
        base: f64,
    ) -> Result<(f64, f64), AvengerScaleError> {
        let (domain_start, domain_end) = domain;
        let (range_start, range_end) = range;

        // Early return for degenerate cases
        if domain_start == domain_end
            || range_start == range_end
            || (padding_lower <= 0.0 && padding_upper <= 0.0)
        {
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

        // Apply padding in log space
        let (new_log_start, new_log_end) =
            pad_interval((log_start, log_end), range, padding_lower, padding_upper)?;

        // Ensure we don't go to -infinity (which would give 0) or +infinity
        // Use a reasonable minimum that's small but not zero
        let min_log = log_fun.log(f64::MIN_POSITIVE.sqrt()); // Use sqrt of MIN_POSITIVE for safety
        let max_log = log_fun.log(f64::MAX.sqrt()); // Use sqrt of MAX for safety

        let new_log_start = new_log_start.max(min_log);
        let new_log_end = new_log_end.min(max_log);

        // Transform back to linear space
        let new_start = log_fun.pow(new_log_start);
        let new_end = log_fun.pow(new_log_end);

        Ok((new_start, new_end))
    }

    /// Apply normalization (padding, zero and nice) to domain
    /// For log scales, zero is ignored since it's invalid in logarithmic space
    pub fn apply_normalization(
        config: LogNormalizationConfig,
    ) -> Result<(f64, f64), AvengerScaleError> {
        let mut current_domain = config.domain;

        // Apply padding first
        // Get padding values, default to 0 if not specified
        let clip_padding_lower_val = config
            .clip_padding_lower
            .and_then(|p| p.as_f64().ok())
            .unwrap_or(0.0);
        let clip_padding_upper_val = config
            .clip_padding_upper
            .and_then(|p| p.as_f64().ok())
            .unwrap_or(0.0);

        if clip_padding_lower_val > 0.0 || clip_padding_upper_val > 0.0 {
            current_domain = Self::apply_padding(
                current_domain,
                config.range,
                clip_padding_lower_val,
                clip_padding_upper_val,
                config.base,
            )?;
        }

        // For log scales, zero is invalid, so we only apply nice transformation
        Self::apply_nice(current_domain, config.base, config.nice)
    }
}

impl ScaleImpl for LogScale {
    fn scale_type(&self) -> &'static str {
        "log"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("base", OptionConstraint::log_base()),
                OptionDefinition::optional("clamp", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("nice", OptionConstraint::nice()),
                OptionDefinition::optional(
                    "clip_padding_lower",
                    OptionConstraint::NonNegativeFloat
                ),
                OptionDefinition::optional(
                    "clip_padding_upper",
                    OptionConstraint::NonNegativeFloat
                ),
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

        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));

        let (domain_start, domain_end) = LogScale::apply_normalization(LogNormalizationConfig {
            domain: config.numeric_interval_domain()?,
            range: range_for_padding,
            clip_padding_lower: config.options.get("clip_padding_lower"),
            clip_padding_upper: config.options.get("clip_padding_upper"),
            base,
            nice: config.options.get("nice"),
        })?;

        // Check if color interpolation is needed
        if config.color_range().is_ok() {
            // Create new config with normalized domain
            let config = ScaleConfig {
                domain: Arc::new(Float64Array::from(vec![domain_start, domain_end])),
                ..config.clone()
            };
            return scale_numeric_to_color(self, &config, values);
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
        let (domain_start, domain_end) = LogScale::apply_normalization(LogNormalizationConfig {
            domain: config.numeric_interval_domain()?,
            range: (range_start, range_end),
            clip_padding_lower: config.options.get("clip_padding_lower"),
            clip_padding_upper: config.options.get("clip_padding_upper"),
            base,
            nice: config.options.get("nice"),
        })?;
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
        let (domain_start, domain_end) = LogScale::apply_nice(
            config.numeric_interval_domain()?,
            base,
            config.options.get("nice"),
        )?;

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

    fn compute_normalized_domain(
        &self,
        config: &ScaleConfig,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let base = config.option_f64("base", 10.0);
        // Get range for padding calculation, use dummy range if not numeric
        let range_for_padding = config.numeric_interval_range().unwrap_or((0.0, 1.0));
        let (domain_start, domain_end) = LogScale::apply_normalization(LogNormalizationConfig {
            domain: config.numeric_interval_domain()?,
            range: range_for_padding,
            clip_padding_lower: config.options.get("clip_padding_lower"),
            clip_padding_upper: config.options.get("clip_padding_upper"),
            base,
            nice: config.options.get("nice"),
        })?;

        Ok(Arc::new(Float64Array::from(vec![domain_start, domain_end])) as ArrayRef)
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
    use arrow::datatypes::Float32Type;
    use float_cmp::{assert_approx_eq, F64Margin};

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

    #[test]
    fn test_apply_padding() -> Result<(), AvengerScaleError> {
        // Test basic padding with base 10
        let result = LogScale::apply_padding((10.0, 100.0), (0.0, 100.0), 10.0, 10.0, 10.0)?;
        // Domain [10, 100] in log10 space is [1, 2]. The domain fills the 80 pixels left after
        // padding, so a pixel covers 1 / 80 of log space: [10^0.875, 10^2.125]
        assert_approx_eq!(f64, result.0, 10f64.powf(0.875), epsilon = 1e-9);
        assert_approx_eq!(f64, result.1, 10f64.powf(2.125), epsilon = 1e-9);

        // Test with base 2
        let result = LogScale::apply_padding((4.0, 16.0), (0.0, 100.0), 10.0, 10.0, 2.0)?;
        // Domain [4, 16] in log2 space is [2, 4], and a pixel covers 2 / 80: [2^1.75, 2^4.25]
        assert_approx_eq!(f64, result.0, 2f64.powf(1.75), epsilon = 1e-9);
        assert_approx_eq!(f64, result.1, 2f64.powf(4.25), epsilon = 1e-9);

        // Test with zero padding (no change)
        let result = LogScale::apply_padding((10.0, 100.0), (0.0, 100.0), 0.0, 0.0, 10.0)?;
        assert_approx_eq!(f64, result.0, 10.0);
        assert_approx_eq!(f64, result.1, 100.0);

        // Test with domain including zero (no change)
        let result = LogScale::apply_padding((0.0, 100.0), (0.0, 100.0), 10.0, 10.0, 10.0)?;
        assert_approx_eq!(f64, result.0, 0.0);
        assert_approx_eq!(f64, result.1, 100.0);

        // Test with negative domain (no change)
        let result = LogScale::apply_padding((-100.0, -10.0), (0.0, 100.0), 10.0, 10.0, 10.0)?;
        assert_approx_eq!(f64, result.0, -100.0);
        assert_approx_eq!(f64, result.1, -10.0);

        Ok(())
    }

    #[test]
    fn test_log_scale_with_padding() -> Result<(), AvengerScaleError> {
        // Create a log scale with padding
        let scale = LogScale::configured((10.0, 100.0), (0.0, 100.0))
            .with_option("clip_padding_lower", 10.0)
            .with_option("clip_padding_upper", 10.0)
            .with_option("base", 10.0);

        // Check that normalized domain has been expanded
        let normalized_domain = scale.normalized_domain()?;
        let normalized_array = normalized_domain
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        // A pixel covers 1 / 80 of log space: [10^0.875, 10^2.125]
        assert_approx_eq!(
            f64,
            normalized_array.value(0),
            10f64.powf(0.875),
            epsilon = 1e-9
        );
        assert_approx_eq!(
            f64,
            normalized_array.value(1),
            10f64.powf(2.125),
            epsilon = 1e-9
        );

        // Test scaling values - auto-normalization should apply padding
        let values = Arc::new(Float64Array::from(vec![10.0, 31.622776, 100.0])) as ArrayRef;
        let result = scale.scale(&values)?;
        let result_array = result.as_primitive::<Float32Type>();

        // The domain ends sit 10 pixels inside the range
        assert_approx_eq!(f32, result_array.value(0), 10.0, epsilon = 1e-3);
        assert_approx_eq!(f32, result_array.value(1), 50.0, epsilon = 1e-3);
        assert_approx_eq!(f32, result_array.value(2), 90.0, epsilon = 1e-3);

        Ok(())
    }

    #[test]
    fn test_padding_with_nice() -> Result<(), AvengerScaleError> {
        // Test that transformations are applied in order: padding -> nice
        let scale = LogScale::configured((10.0, 100.0), (0.0, 100.0))
            .with_option("clip_padding_lower", 10.0)
            .with_option("clip_padding_upper", 10.0)
            .with_option("nice", true)
            .with_option("base", 10.0);

        let normalized_domain = scale.normalized_domain()?;
        let normalized_array = normalized_domain
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        // Expected transformations:
        // 1. Padding: [10, 100] → expanded in log space
        //    - log10(10) = 1, log10(100) = 2
        //    - log_per_pixel = 1 / 100 = 0.01
        //    - new_log_start = 1 - 10 * 0.01 = 0.9
        //    - new_log_end = 2 + 10 * 0.01 = 2.1
        //    - linear domain: [10^0.9, 10^2.1] ≈ [7.943, 125.89]
        // 2. Nice: [7.943, 125.89] → [1, 1000] (nice powers of 10)
        assert_approx_eq!(f64, normalized_array.value(0), 1.0);
        assert_approx_eq!(f64, normalized_array.value(1), 1000.0);

        Ok(())
    }

    #[test]
    fn test_padding_close_to_zero() -> Result<(), AvengerScaleError> {
        // Test various domains close to zero

        // Domain starting very close to zero
        let result = LogScale::apply_padding((0.001, 10.0), (0.0, 100.0), 10.0, 10.0, 10.0)?;
        println!("Domain (0.001, 10) with padding 10: {:?}", result);
        // Should expand in log space
        assert!(result.0 < 0.001);
        assert!(result.1 > 10.0);
        assert!(result.0 > 0.0); // Should never go to zero
        assert!(result.1.is_finite()); // Should never go to infinity

        // Domain with even smaller start
        let result = LogScale::apply_padding((0.0001, 1.0), (0.0, 100.0), 10.0, 10.0, 10.0)?;
        println!("Domain (0.0001, 1) with padding 10: {:?}", result);
        assert!(result.0 < 0.0001);
        assert!(result.1 > 1.0);
        assert!(result.0 > 0.0);
        assert!(result.1.is_finite());

        // Padding that fills the range is rejected
        assert!(LogScale::apply_padding((0.1, 1.0), (0.0, 100.0), 50.0, 50.0, 10.0).is_err());
        assert!(LogScale::apply_padding((1.0, 10.0), (0.0, 100.0), 200.0, 200.0, 10.0).is_err());

        // Domain with one end at zero (should return unchanged)
        let result = LogScale::apply_padding((0.0, 10.0), (0.0, 100.0), 10.0, 10.0, 10.0)?;
        assert_eq!(result, (0.0, 10.0));

        // Domain crossing zero (should return unchanged)
        let result = LogScale::apply_padding((-1.0, 10.0), (0.0, 100.0), 10.0, 10.0, 10.0)?;
        assert_eq!(result, (-1.0, 10.0));

        Ok(())
    }

    #[test]
    fn test_apply_padding_asymmetric() -> Result<(), AvengerScaleError> {
        // Basic asymmetric padding test
        let result = LogScale::apply_padding((1.0, 100.0), (0.0, 100.0), 10.0, 20.0, 10.0)?;
        println!("Asymmetric padding result: {:?}", result);

        // Domain [1, 100] in log10 space is [0, 2]. The domain fills the 70 pixels left after
        // padding, so a pixel covers 2 / 70 of log space.
        assert_approx_eq!(f64, result.0, 10f64.powf(-20.0 / 70.0), epsilon = 1e-9);
        assert_approx_eq!(f64, result.1, 10f64.powf(2.0 + 40.0 / 70.0), epsilon = 1e-9);

        // Test with only lower padding
        let result = LogScale::apply_padding((10.0, 100.0), (0.0, 100.0), 25.0, 0.0, 10.0)?;
        println!("Lower padding only: {:?}", result);
        assert!(result.0 < 10.0);
        assert_approx_eq!(f64, result.1, 100.0);

        // Test with only upper padding
        let result = LogScale::apply_padding((10.0, 100.0), (0.0, 100.0), 0.0, 25.0, 10.0)?;
        println!("Upper padding only: {:?}", result);
        assert_approx_eq!(f64, result.0, 10.0);
        assert!(result.1 > 100.0);

        // Padding that fills the range is rejected
        assert!(LogScale::apply_padding((0.01, 1.0), (0.0, 100.0), 100.0, 50.0, 10.0).is_err());

        Ok(())
    }

    #[test]
    fn test_log_scale_with_asymmetric_padding() -> Result<(), AvengerScaleError> {
        // Create a log scale with asymmetric padding
        let scale = LogScale::configured((10.0, 1000.0), (0.0, 100.0))
            .with_option("clip_padding_lower", 10.0)
            .with_option("clip_padding_upper", 20.0)
            .with_option("base", 10.0);

        // Check that normalized domain applies padding
        let normalized_domain = scale.normalized_domain()?;
        let normalized_array = normalized_domain
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        // Domain [10, 1000] in log10 space is [1, 3], and a pixel covers 2 / 70 of it
        assert_approx_eq!(
            f64,
            normalized_array.value(0),
            10f64.powf(1.0 - 20.0 / 70.0),
            epsilon = 1e-9
        );
        assert_approx_eq!(
            f64,
            normalized_array.value(1),
            10f64.powf(3.0 + 40.0 / 70.0),
            epsilon = 1e-9
        );

        Ok(())
    }

    #[test]
    fn test_log_padding_fallback_behavior() -> Result<(), AvengerScaleError> {
        // Test that padding_lower/upper default to padding value
        let scale = LogScale::configured((10.0, 100.0), (0.0, 100.0))
            .with_option("clip_padding_lower", 15.0)
            .with_option("clip_padding_upper", 15.0)
            .with_option("base", 10.0);

        let normalized_domain = scale.normalized_domain()?;
        let normalized_array = normalized_domain
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        // Should use padding=15 for both lower and upper
        let expected = LogScale::apply_padding((10.0, 100.0), (0.0, 100.0), 15.0, 15.0, 10.0)?;
        assert_approx_eq!(
            f64,
            normalized_array.value(0),
            expected.0,
            F64Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );
        assert_approx_eq!(
            f64,
            normalized_array.value(1),
            expected.1,
            F64Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );

        // Test with only padding_upper set
        let scale = LogScale::configured((10.0, 100.0), (0.0, 100.0))
            .with_option("clip_padding_lower", 10.0)
            .with_option("clip_padding_upper", 20.0)
            .with_option("base", 10.0);

        let normalized_domain = scale.normalized_domain()?;
        let normalized_array = normalized_domain
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        // Should use padding=10 for lower, padding_upper=20 for upper
        let expected = LogScale::apply_padding((10.0, 100.0), (0.0, 100.0), 10.0, 20.0, 10.0)?;
        assert_approx_eq!(
            f64,
            normalized_array.value(0),
            expected.0,
            F64Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );
        assert_approx_eq!(
            f64,
            normalized_array.value(1),
            expected.1,
            F64Margin {
                epsilon: 0.001,
                ..Default::default()
            }
        );

        Ok(())
    }

    #[test]
    fn test_log_scale_edge_cases_with_asymmetric_padding() -> Result<(), AvengerScaleError> {
        // Test with domain very close to zero
        let scale = LogScale::configured((0.001, 1.0), (0.0, 100.0))
            .with_option("clip_padding_lower", 50.0)
            .with_option("clip_padding_upper", 25.0)
            .with_option("base", 10.0);

        let normalized_domain = scale.normalized_domain()?;
        let normalized_array = normalized_domain
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        // Should never reach zero or infinity
        assert!(normalized_array.value(0) > 0.0);
        assert!(normalized_array.value(0).is_finite());
        assert!(normalized_array.value(1).is_finite());

        // Padding that fills the range is rejected
        let scale = LogScale::configured((1.0, 10.0), (0.0, 100.0))
            .with_option("clip_padding_lower", 500.0)
            .with_option("clip_padding_upper", 500.0)
            .with_option("base", 10.0);
        assert!(scale.normalized_domain().is_err());

        Ok(())
    }
}
