use std::sync::Arc;

use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

use super::ContinuousNumericScale;

/// Handles logarithmic transformations with different bases
#[derive(Clone, Debug)]
enum LogFunction {
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

#[derive(Debug, Clone)]
pub struct LogNumericScaleConfig {
    pub domain: (f32, f32),
    pub range: (f32, f32),
    pub base: f32,
    pub clamp: bool,
    pub range_offset: f32,
    pub nice: bool,
}

impl Default for LogNumericScaleConfig {
    fn default() -> Self {
        Self {
            domain: (1.0, 10.0),
            range: (0.0, 1.0),
            base: 10.0,
            clamp: false,
            range_offset: 0.0,
            nice: false,
        }
    }
}

/// A logarithmic scale that maps numeric input values using a log transform.
/// Supports different bases, clamping, domain niceing, and tick generation.
#[derive(Clone, Debug)]
pub struct LogNumericScale {
    domain_start: f32,
    domain_end: f32,
    range_start: f32,
    range_end: f32,
    clamp: bool,
    range_offset: f32,
    log_fun: Arc<LogFunction>,
}

impl LogNumericScale {
    /// Creates a new log scale with default domain [1, 10] and range [0, 1]
    pub fn new(config: &LogNumericScaleConfig) -> Self {
        let mut this = Self {
            domain_start: config.domain.0,
            domain_end: config.domain.1,
            range_start: config.range.0,
            range_end: config.range.1,
            clamp: config.clamp,
            range_offset: config.range_offset,
            log_fun: Arc::new(LogFunction::new(config.base)),
        };
        if config.nice {
            this = this.nice();
        }
        this
    }

    /// Returns the current logarithm base
    pub fn get_base(&self) -> f32 {
        self.log_fun.base()
    }

    /// Computes the logarithm of x in the current base
    pub fn log(&self, x: f32) -> f32 {
        self.log_fun.log(x)
    }

    /// Computes the current base raised to power x
    pub fn pow(&self, x: f32) -> f32 {
        self.log_fun.pow(x)
    }

    /// Extends the domain to nice round numbers in log space
    pub fn nice(mut self) -> Self {
        if self.domain_start.is_nan() || self.domain_end.is_nan() {
            return self;
        }

        // Special case for exact zero domain
        if self.domain_start == 0.0 && self.domain_end == 0.0 {
            return self;
        }

        // Handle degenerate non-zero domain by expanding to nearest log boundaries
        if self.domain_start == self.domain_end && self.domain_start != 0.0 {
            let value = self.domain_start;
            let log_val = self.log(value.abs() as f32);
            self.domain_start = self.pow(log_val.floor()) as f32;
            self.domain_end = self.pow(log_val.ceil()) as f32;
            return self;
        }

        let (start, stop, reverse) = if self.domain_start < self.domain_end {
            (self.domain_start, self.domain_end, false)
        } else {
            (self.domain_end, self.domain_start, true)
        };

        // Handle negative domains
        if start < 0.0 && stop < 0.0 {
            let nstart = -stop;
            let nstop = -start;

            let nstart = self.pow(self.log(nstart as f32).floor());
            let nstop = self.pow(self.log(nstop as f32).ceil());

            if reverse {
                self.domain_start = -nstart;
                self.domain_end = -nstop;
            } else {
                self.domain_start = -nstop;
                self.domain_end = -nstart;
            }
        } else {
            let nstart = self.pow(self.log(start as f32).floor());
            let nstop = self.pow(self.log(stop as f32).ceil());

            if reverse {
                self.domain_start = nstop;
                self.domain_end = nstart;
            } else {
                self.domain_start = nstart;
                self.domain_end = nstop;
            }
        }
        self
    }

    pub fn with_domain(mut self, domain: (f32, f32)) -> Self {
        self.domain_start = domain.0;
        self.domain_end = domain.1;
        self
    }

    pub fn with_range(mut self, range: (f32, f32)) -> Self {
        self.range_start = range.0;
        self.range_end = range.1;
        self
    }

    pub fn with_clamp(mut self, clamp: bool) -> Self {
        self.clamp = clamp;
        self
    }

    pub fn with_range_offset(mut self, range_offset: f32) -> Self {
        self.range_offset = range_offset;
        self
    }

    pub fn with_base(mut self, base: f32) -> Self {
        self.log_fun = Arc::new(LogFunction::new(base));
        self
    }
}

impl ContinuousNumericScale<f32> for LogNumericScale {
    fn range(&self) -> (f32, f32) {
        (self.range_start, self.range_end)
    }

    fn domain(&self) -> (f32, f32) {
        (self.domain_start, self.domain_end)
    }

    fn clamp(&self) -> bool {
        self.clamp
    }

    fn set_domain(&mut self, domain: (f32, f32)) {
        self.domain_start = domain.0;
        self.domain_end = domain.1;
    }

    fn set_range(&mut self, range: (f32, f32)) {
        self.range_start = range.0;
        self.range_end = range.1;
    }

    fn set_clamp(&mut self, clamp: bool) {
        self.clamp = clamp;
    }

    fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<f32> {
        // Handle degenerate domain and range cases
        if self.domain_start == self.domain_end || self.range_start == self.range_end {
            return values.into().map(|_| self.range_start);
        }

        // Transform to log space
        let log_domain_start = if self.domain_start < 0.0 {
            -self.log(-self.domain_start)
        } else {
            self.log(self.domain_start)
        };

        let log_domain_end = if self.domain_end < 0.0 {
            -self.log(-self.domain_end)
        } else {
            self.log(self.domain_end)
        };

        let log_domain_span = log_domain_end - log_domain_start;

        // Handle degenerate domain in log space
        if log_domain_span == 0.0 || log_domain_span.is_nan() {
            return values.into().map(|_| self.range_start);
        }

        let scale = (self.range_end - self.range_start) / log_domain_span;
        let offset = self.range_start - scale * log_domain_start + self.range_offset;

        if self.clamp {
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            values.into().map(|&v| {
                if v.is_nan() {
                    return f32::NAN;
                }
                if v <= 0.0 {
                    return if self.range_start <= self.range_end {
                        range_min
                    } else {
                        range_max
                    };
                }
                let log_v = self.log(v);
                (scale * log_v + offset).clamp(range_min, range_max)
            })
        } else {
            match self.log_fun.as_ref() {
                LogFunction::Static { log_fun, .. } => values.into().map(|&v| {
                    if v < 0.0 {
                        scale * (-log_fun(-v)) + offset
                    } else if v > 0.0 {
                        scale * log_fun(v) + offset
                    } else {
                        f32::NAN
                    }
                }),
                LogFunction::Custom { ln_base, .. } => {
                    let ln_base = *ln_base;
                    values.into().map(|&v| {
                        if v < 0.0 {
                            scale * (-v.ln() / ln_base) + offset
                        } else if v > 0.0 {
                            scale * (v.ln() / ln_base) + offset
                        } else {
                            f32::NAN
                        }
                    })
                }
            }
        }
    }

    fn invert<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<f32> {
        // Handle degenerate cases
        if self.domain_start <= 0.0 || self.domain_end <= 0.0 || self.range_start == self.range_end
        {
            return values.into().map(|_| self.range_start);
        }

        // Transform to log space
        let log_domain_start = self.log(self.domain_start);
        let log_domain_end = self.log(self.domain_end);
        let log_domain_span = log_domain_end - log_domain_start;

        // Handle degenerate domain in log space
        if log_domain_span == 0.0 || log_domain_span.is_nan() {
            return values.into().map(|_| self.range_start);
        }

        let scale = (self.range_end - self.range_start) / log_domain_span;
        let offset = self.range_start - scale * log_domain_start + self.range_offset;

        if self.clamp {
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            match self.log_fun.as_ref() {
                LogFunction::Static { pow_fun, .. } => values.into().map(|&v| {
                    let v = v.clamp(range_min, range_max);
                    pow_fun((v - offset) / scale)
                }),
                LogFunction::Custom { base, .. } => values.into().map(|&v| {
                    let v = v.clamp(range_min, range_max);
                    base.powf((v - offset) / scale)
                }),
            }
        } else {
            match self.log_fun.as_ref() {
                LogFunction::Static { pow_fun, .. } => {
                    values.into().map(|&v| pow_fun((v - offset) / scale))
                }
                LogFunction::Custom { base, .. } => {
                    values.into().map(|&v| base.powf((v - offset) / scale))
                }
            }
        }
    }

    fn ticks(&self, count: Option<f32>) -> Vec<f32> {
        let count = count.unwrap_or(10.0);
        // D3: if (!(d[0] > 0 && d[1] > 0)) return [];
        if !(self.domain_start > 0.0 && self.domain_end > 0.0) {
            return vec![];
        }

        let d = [self.domain_start as f32, self.domain_end as f32];
        let mut u = d[0];
        let mut v = d[1];
        let r = v < u;

        if r {
            let temp = u;
            u = v;
            v = temp;
        }

        let mut i = self.log(u);
        let mut j = self.log(v);
        let mut z = Vec::new();

        // Handle integer bases specially
        let base = self.log_fun.base();
        if (base - base.floor()).abs() < f32::EPSILON && j - i < count {
            i = i.floor();
            j = j.ceil();

            if u > 0.0 {
                for exp in (i as i32)..=(j as i32) {
                    for k in 1..(base as i32) {
                        let t = if exp < 0 {
                            k as f32 / self.pow(-exp as f32)
                        } else {
                            k as f32 * self.pow(exp as f32)
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
                            k as f32 / self.pow(-exp as f32)
                        } else {
                            k as f32 * self.pow(exp as f32)
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
                .map(|x| self.pow(x))
                .collect();
        }

        if r {
            z.reverse();
        }
        z
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_defaults() {
        let scale = LogNumericScale::new(&Default::default());
        assert_eq!(scale.domain_start, 1.0);
        assert_eq!(scale.domain_end, 10.0);
        assert_eq!(scale.range_start, 0.0);
        assert_eq!(scale.range_end, 1.0);
        assert_eq!(scale.clamp, false);
        assert_eq!(scale.log_fun.base(), 10.0);

        let values = vec![5.0];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.69897);

        let values = vec![0.69897];
        let result = scale.invert(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 5.0);
    }

    #[test]
    fn test_domain_coercion() {
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (1.0, 2.0),
            range: (0.0, 1.0),
            clamp: false,
            range_offset: 0.0,
            ..Default::default()
        });
        let values = vec![0.5, 1.0, 1.5, 2.0, 2.5];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], -1.0);
        assert_approx_eq!(f32, result[1], 0.0);
        assert_approx_eq!(f32, result[2], 0.5849625);
        assert_approx_eq!(f32, result[3], 1.0);
        assert_approx_eq!(f32, result[4], 1.3219281);
    }

    #[test]
    fn test_range_offset() {
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (1.0, 2.0),
            range: (0.0, 1.0),
            range_offset: 0.5,
            ..Default::default()
        });
        let values = vec![0.5, 1.0, 1.5, 2.0, 2.5];
        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], -0.5);
        assert_approx_eq!(f32, result[1], 0.5);
        assert_approx_eq!(f32, result[2], 1.0849625);
        assert_approx_eq!(f32, result[3], 1.5);
        assert_approx_eq!(f32, result[4], 1.8219281);
    }

    #[test]
    fn test_negative_domain() {
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (-100.0, -1.0),
            range: (0.0, 1.0),
            ..Default::default()
        });
        let values = vec![-50.0];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.150515);
    }

    #[test]
    fn test_clamping() {
        // Test unclamped behavior
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            ..Default::default()
        });
        let values = vec![0.5, 15.0];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], -0.30103);
        assert_approx_eq!(f32, result[1], 1.176091);

        // Test clamped behavior
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            clamp: true,
            ..Default::default()
        });
        let values = vec![-1.0, 5.0, 15.0];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_approx_eq!(f32, result[0], 0.0);
        assert_approx_eq!(f32, result[1], 0.69897);
        assert_approx_eq!(f32, result[2], 1.0);
    }

    #[test]
    fn test_invert_range_offset() {
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (1.0, 2.0),
            range_offset: 0.5,
            ..Default::default()
        });
        let values = vec![-0.5, 0.5, 1.0849625, 1.5, 1.8219281];
        let result = scale.invert(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.5);
        assert_approx_eq!(f32, result[1], 1.0);
        assert_approx_eq!(f32, result[2], 1.5);
        assert_approx_eq!(f32, result[3], 2.0);
        assert_approx_eq!(f32, result[4], 2.5);
    }

    #[test]
    fn test_nice() {
        // Test nice with ascending domain
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (1.1, 10.9),
            ..Default::default()
        })
        .nice();
        assert_eq!(scale.domain_start, 1.0);
        assert_eq!(scale.domain_end, 100.0);

        // Test nice with descending domain
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (10.9, 1.1),
            ..Default::default()
        })
        .nice();
        assert_eq!(scale.domain_start, 100.0);
        assert_eq!(scale.domain_end, 1.0);

        // Test nice with domain crossing decades
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (0.7, 11.001),
            ..Default::default()
        })
        .nice();
        assert_eq!(scale.domain_start, 0.1);
        assert_eq!(scale.domain_end, 100.0);

        // Test nice with reversed domain crossing decades
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (123.1, 6.7),
            ..Default::default()
        })
        .nice();
        assert_eq!(scale.domain_start, 1000.0);
        assert_eq!(scale.domain_end, 1.0);

        // Test nice with small domain
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (0.01, 0.49),
            ..Default::default()
        })
        .nice();
        assert_eq!(scale.domain_start, 0.01);
        assert_eq!(scale.domain_end, 1.0);
    }

    #[test]
    fn test_nice_degenerate() {
        // Test nice with zero domain
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (0.0, 0.0),
            ..Default::default()
        })
        .nice();
        assert_eq!(scale.domain_start, 0.0);
        assert_eq!(scale.domain_end, 0.0);

        // Test nice with point domain
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (0.5, 0.5),
            ..Default::default()
        })
        .nice();
        assert_eq!(scale.domain_start, 0.1);
        assert_eq!(scale.domain_end, 1.0);
    }

    #[test]
    fn test_ticks() {
        // Test ascending ticks
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (0.1, 10.0),
            ..Default::default()
        });
        assert_eq!(
            scale.ticks(Some(10.0)),
            vec![
                0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0,
                8.0, 9.0, 10.0
            ]
        );

        // Test descending ticks
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (10.0, 0.1),
            ..Default::default()
        });
        assert_eq!(
            scale.ticks(Some(10.0)),
            vec![
                10.0, 9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4,
                0.3, 0.2, 0.1
            ]
        );
    }

    #[test]
    fn test_base() {
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 2.0,
            domain: (1.0, 32.0),
            ..Default::default()
        });
        assert_eq!(
            scale.ticks(Some(10.0)),
            vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0]
        );
    }

    #[test]
    fn test_degenerate_domain() {
        fn make_ticks(domain: (f32, f32)) -> Vec<f32> {
            LogNumericScale::new(&LogNumericScaleConfig {
                base: 10.0,
                domain,
                ..Default::default()
            })
            .ticks(None)
        }

        // Test various degenerate domains
        assert!(make_ticks((0.0, 1.0)).is_empty());
        assert!(make_ticks((1.0, 0.0)).is_empty());
        assert!(make_ticks((0.0, -1.0)).is_empty());
        assert!(make_ticks((-1.0, 0.0)).is_empty());
        assert!(make_ticks((-1.0, 1.0)).is_empty());
        assert!(make_ticks((0.0, 0.0)).is_empty());
    }

    #[test]
    fn test_edge_cases() {
        // Test zero domain
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            domain: (0.0, 0.0),
            ..Default::default()
        });
        let values = vec![1.0, 2.0];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_eq!(result[0], 0.0);
        assert_eq!(result[1], 0.0);

        // Test negative/zero input with clamping
        let scale = LogNumericScale::new(&LogNumericScaleConfig {
            base: 10.0,
            clamp: true,
            ..Default::default()
        });
        let values = vec![-1.0, 0.0];
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_eq!(result[0], 0.0);
        assert_eq!(result[1], 0.0);
    }
}
