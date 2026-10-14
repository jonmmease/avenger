use super::DomainFitError;

/// The support of a quantile scale's domain: the sample of values that the scale divides.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleSupport {
    /// The values in ascending order.
    values: Vec<f64>,
}

impl SampleSupport {
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
    pub fn union(&self, other: &Self) -> Self {
        let mut values = Vec::with_capacity(self.values.len() + other.values.len());
        let (mut i, mut j) = (0, 0);
        while i < self.values.len() && j < other.values.len() {
            if self.values[i] <= other.values[j] {
                values.push(self.values[i]);
                i += 1;
            } else {
                values.push(other.values[j]);
                j += 1;
            }
        }
        values.extend_from_slice(&self.values[i..]);
        values.extend_from_slice(&other.values[j..]);
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
    fn union_is_the_sorted_sample_of_both() {
        let a = SampleSupport::new(&[3.0, 1.0, 2.0]).unwrap();
        let b = SampleSupport::new(&[2.0, 0.5]).unwrap();
        let all = SampleSupport::new(&[3.0, 1.0, 2.0, 2.0, 0.5]).unwrap();
        assert_eq!(a.union(&b), all);
        assert_eq!(b.union(&a), all);
        assert_eq!(all.values(), &[0.5, 1.0, 2.0, 2.0, 3.0]);
        assert!(SampleSupport::new(&[f64::NAN]).is_err());
    }
}
