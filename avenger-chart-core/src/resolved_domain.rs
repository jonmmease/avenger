use datafusion::logical_expr::lit;

use crate::ScaleRange;

/// Resolved domain type for marks to make range decisions.
///
/// This enum indicates whether a domain has been resolved to discrete values or
/// a continuous interval, without needing to evaluate expressions.
#[derive(Debug, Clone, Copy)]
pub enum ResolvedDomain {
    /// Discrete domain with the number of unique values.
    Discrete(usize),
    /// Continuous interval domain.
    Interval,
}

impl ResolvedDomain {
    /// Create a numeric interval or linear spaced ScaleRange.
    pub fn make_interval_or_linspaced_range(&self, start: f32, end: f32) -> ScaleRange {
        match self {
            ResolvedDomain::Interval => ScaleRange::new_interval(lit(start), lit(end)),
            ResolvedDomain::Discrete(count) => {
                ScaleRange::new_linspace_discrete(start, end, *count)
            }
        }
    }
}
