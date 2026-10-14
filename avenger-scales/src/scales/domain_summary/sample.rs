use super::DomainFitError;

/// A summary of data for a quantile scale's domain: the sample of values that the scale divides.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleSummary {
    /// The values in ascending order.
    values: Vec<f64>,
}

impl SampleSummary {
    /// Prepares a sample of finite values.
    pub fn new(values: &[f64]) -> Result<Self, DomainFitError> {
        if values.iter().any(|value| !value.is_finite()) {
            return Err(DomainFitError::InvalidInput("values must be finite".into()));
        }
        let mut values = values.to_vec();
        values.sort_by(f64::total_cmp);
        Ok(Self { values })
    }

    /// Both samples as one.
    pub fn merge(&self, other: &Self) -> Self {
        let mut values = [self.values.as_slice(), &other.values].concat();
        values.sort_by(f64::total_cmp);
        Self { values }
    }

    /// The values in ascending order.
    pub fn values(&self) -> &[f64] {
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_is_the_sorted_sample_of_both() {
        let a = SampleSummary::new(&[3.0, 1.0, 2.0]).unwrap();
        let b = SampleSummary::new(&[2.0, 0.5]).unwrap();
        let all = SampleSummary::new(&[3.0, 1.0, 2.0, 2.0, 0.5]).unwrap();
        assert_eq!(a.merge(&b), all);
        assert_eq!(b.merge(&a), all);
        assert_eq!(all.values(), &[0.5, 1.0, 2.0, 2.0, 3.0]);
        assert!(SampleSummary::new(&[f64::NAN]).is_err());
    }
}
