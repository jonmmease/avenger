use crate::error::AvengerScaleError;
use crate::numeric::linear::LinearNumericScale;
use std::fmt::Debug;
/// A quantize scale divides a continuous domain into uniform segments and maps values to a discrete range.
///
/// The quantize scale is like a linear scale, except it divides the domain into uniform segments
/// based on the number of values in the range. Each segment is then mapped to a corresponding
/// discrete value in the range.
#[derive(Debug, Clone)]
pub struct QuantizeScale<R>
where
    R: Clone + Debug,
{
    domain: (f32, f32),
    range: Vec<R>,
    default: R,
    thresholds: Vec<f32>,
}

impl<R> QuantizeScale<R>
where
    R: Clone + Debug,
{
    /// Creates a new quantize scale with default domain [0,1] and range [0,1]
    pub fn new(range: Vec<R>, default: R) -> Self {
        let mut this = Self {
            domain: (0.0, 1.0),
            range,
            default,
            thresholds: vec![],
        };
        this.update_thresholds();
        this
    }

    /// Sets the input domain as a tuple of (min, max)
    pub fn domain(mut self, domain: (f32, f32)) -> Self {
        self.domain = domain;
        self.update_thresholds();
        self
    }

    /// Sets the output range as an Arrow array
    pub fn range(mut self, range: Vec<R>) -> Self {
        self.range = range;
        self.update_thresholds();
        self
    }

    /// Extends the domain to nice round numbers for better quantization boundaries
    pub fn nice(mut self, count: Option<usize>) -> Self {
        // Use linear scale for the nice calculation
        self.domain = LinearNumericScale::new()
            .domain(self.domain)
            .nice(count)
            .get_domain();
        self
    }

    /// Returns the input domain
    pub fn get_domain(&self) -> (f32, f32) {
        self.domain
    }

    /// Returns a reference to the output range
    pub fn get_range(&self) -> &Vec<R> {
        &self.range
    }

    /// Returns the threshold values that divide the domain
    pub fn thresholds(&self) -> &[f32] {
        &self.thresholds
    }

    fn update_thresholds(&mut self) {
        let n = self.range.len();
        if n <= 1 {
            self.thresholds = vec![];
            return;
        }

        // Calculate n-1 threshold values that divide the domain into n segments
        self.thresholds = (1..n)
            .map(|i| {
                let t = (i as f32) / (n as f32);
                self.domain.0 * (1.0 - t) + self.domain.1 * t
            })
            .collect();
    }

    pub fn scale(&self, values: &[f32]) -> Result<Vec<R>, AvengerScaleError> {
        let n = self.range.len();

        // If there is only one range value, return it for all values
        if n == 1 {
            return Ok(values.iter().map(|_| self.range[0].clone()).collect());
        }

        // Pre-compute scaling factors
        let domain_span = self.domain.1 - self.domain.0;
        let segments = n as f32;

        let mut result: Vec<R> = Vec::with_capacity(values.len());

        // Build array of indices, with nulls for non-finite values
        for x in values.iter() {
            if x.is_finite() {
                // Direct calculation of index based on position in domain
                let normalized = (x - self.domain.0) / domain_span;
                let idx = ((normalized * segments).floor() as usize).clamp(0, n - 1);
                result.push(self.range[idx].clone());
            } else {
                result.push(self.default.clone());
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    #[test]
    fn test_quantize_scale_basic() -> Result<(), AvengerScaleError> {
        let scale = QuantizeScale::new(vec![0.0, 0.5, 1.0], f32::NAN).domain((0.0, 1.0));

        // Test array scaling with all test cases
        let values = vec![0.3, 0.5, 0.8];
        let result = scale.scale(&values)?;
        assert_eq!(result[0], 0.0);
        assert_eq!(result[1], 0.5);
        assert_eq!(result[2], 1.0);

        Ok(())
    }

    #[test]
    fn test_quantize_thresholds() {
        let scale = QuantizeScale::new(vec![0.0, 0.5, 1.0], f32::NAN).domain((0.0, 100.0));

        let thresholds = scale.thresholds();
        assert_approx_eq!(f32, thresholds[0], 33.333332);
        assert_approx_eq!(f32, thresholds[1], 66.66667);
    }

    #[test]
    fn test_quantize_string_range() -> Result<(), AvengerScaleError> {
        let scale =
            QuantizeScale::new(vec!["small", "medium", "large"], "default").domain((0.0, 1.0));

        let values = vec![0.3, 0.5, 0.8];
        let result = scale.scale(&values)?;

        assert_eq!(result[0], "small");
        assert_eq!(result[1], "medium");
        assert_eq!(result[2], "large");

        Ok(())
    }

    #[test]
    fn test_quantize_scale_nice() -> Result<(), AvengerScaleError> {
        let scale = QuantizeScale::new(vec![0.0, 25.0, 50.0, 75.0, 100.0], f32::NAN)
            .domain((1.1, 10.9))
            .nice(Some(5));

        // Domain should be extended to nice numbers
        let (start, end) = scale.get_domain();
        assert_approx_eq!(f32, start, 0.0);
        assert_approx_eq!(f32, end, 12.0);

        let values = vec![1.0, 6.0, 11.0];
        let result = scale.scale(&values)?;

        assert_eq!(result[0], 0.0); // Near start of domain
        assert_eq!(result[1], 50.0); // Middle of domain
        assert_eq!(result[2], 100.0); // Near end of domain

        Ok(())
    }
}
