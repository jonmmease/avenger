use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::{kernels::cast, unary},
    datatypes::{DataType, Float32Type},
};
use avenger_common::value::ScalarOrArray;
use datafusion_common::ScalarValue;

use crate::{
    color_interpolator::scale_numeric_to_color2, error::AvengerScaleError,
    scales::linear::LinearScale, utils::ScalarValueUtils,
};

use super::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext, ScaleImpl};

#[derive(Debug)]
pub struct LogScale;

impl LogScale {
    pub fn new(domain: (f32, f32), range: (f32, f32)) -> ConfiguredScale {
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
        count: Option<&ScalarValue>,
    ) -> Result<(f32, f32), AvengerScaleError> {
        // Extract count, or return raw domain if no nice option
        let _count = if let Some(count) = count {
            if count.data_type().is_numeric() {
                count.as_f32()?
            } else if count == &ScalarValue::from(true) {
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
}

impl ScaleImpl for LogScale {
    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let (domain_start, domain_end) = LinearScale::apply_nice(
            config.numeric_interval_domain()?,
            config.options.get("nice"),
        )?;

        // Check if color interpolation is needed
        if config.color_range().is_ok() {
            // Create new config with niced domain
            let config = ScaleConfig {
                domain: Arc::new(Float32Array::from(vec![domain_start, domain_end])),
                ..config.clone()
            };
            return scale_numeric_to_color2(self, &config, values);
        }

        // Get options
        let base = config.option_f32("base", 10.0);
        let range_offset = config.option_f32("range_offset", 0.0);
        let clamp = config.option_boolean("clamp", false);
        let round = config.option_boolean("round", false);

        // Get options
        let (range_start, range_end) = config.numeric_interval_range()?;
        let (domain_start, domain_end) = LogScale::apply_nice(
            config.numeric_interval_domain()?,
            base,
            config.options.get("nice"),
        )?;

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
        let (domain_start, domain_end) = LogScale::apply_nice(
            config.numeric_interval_domain()?,
            base,
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
    use float_cmp::assert_approx_eq;

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
            options: vec![("base".to_string(), ScalarValue::from(10.0))]
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
                ("base".to_string(), ScalarValue::from(10.0)),
                ("range_offset".to_string(), ScalarValue::from(0.5)),
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
            options: vec![("base".to_string(), ScalarValue::from(10.0))]
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
                ("base".to_string(), ScalarValue::from(10.0)),
                ("clamp".to_string(), ScalarValue::from(false)),
            ]
            .into_iter()
            .collect(),
            context: ScaleContext::default(),
        };
        let values = Arc::new(Float32Array::from(vec![0.5, 15.0])) as ArrayRef;
        let result = scale.scale_to_numeric(&config, &values).unwrap();
        let result = result.as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -0.30103);
        assert_approx_eq!(f32, result[1], 1.176091);

        // Test clamped behavior
        let scale = LogScale;
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![1.0, 10.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0])),
            options: vec![
                ("base".to_string(), ScalarValue::from(10.0)),
                ("clamp".to_string(), ScalarValue::from(true)),
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
                ("base".to_string(), ScalarValue::from(10.0)),
                ("range_offset".to_string(), ScalarValue::from(0.5)),
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
}
