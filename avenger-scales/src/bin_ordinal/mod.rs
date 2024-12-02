use crate::error::AvengerScaleError;
use std::fmt::Debug;

/// A bin-ordinal scale maps continuous values to discrete values based on bin boundaries,
/// cycling through range values using modulo arithmetic.
///
/// Similar to threshold scales, but cycles through range values when there are more bins
/// than range values.
#[derive(Debug, Clone)]
pub struct BinOrdinalScale<R>
where
    R: Clone + Debug,
{
    bins: Vec<f32>,
    range: Vec<R>,
}

impl<R> BinOrdinalScale<R>
where
    R: Clone + Debug,
{
    pub fn try_new(bins: Vec<f32>, range: Vec<R>) -> Result<Self, AvengerScaleError> {
        if bins.is_empty() {
            return Err(AvengerScaleError::EmptyDomain);
        }
        if !bins.windows(2).all(|w| w[0] <= w[1]) {
            return Err(AvengerScaleError::BinsNotAscending(bins));
        }
        if range.len() == 0 {
            return Err(AvengerScaleError::EmptyRange);
        }

        Ok(Self { bins, range })
    }

    /// Returns a reference to the bin boundaries
    pub fn get_bins(&self) -> &[f32] {
        &self.bins
    }

    /// Returns a reference to the output range
    pub fn get_range(&self) -> &[R] {
        &self.range
    }

    pub fn scale(&self, values: &[f32]) -> Result<Vec<Option<R>>, AvengerScaleError> {
        let bins = &self.bins;
        let range_len = self.range.len() as usize;

        // Build array of indices, with nulls for non-finite values
        // let mut indices: Vec<Option<u32>> = Vec::with_capacity(values.len());
        let mut result: Vec<Option<R>> = Vec::with_capacity(values.len());

        for x in values.iter() {
            if x.is_finite() {
                // Find bin index and apply modulo to cycle through range values
                let idx = match bins.binary_search_by(|t| t.partial_cmp(&x).unwrap()) {
                    Ok(i) => i as usize,
                    Err(i) => {
                        if i == 0 || i >= bins.len() {
                            // Out of domain - return None
                            result.push(None);
                            continue;
                        }
                        (i - 1) as usize
                    }
                } % range_len;
                result.push(Some(self.range[idx].clone()));
            } else {
                result.push(None);
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use float_cmp::assert_approx_eq;

    use super::*;

    #[test]
    fn test_bin_ordinal_scale_basic() -> Result<(), AvengerScaleError> {
        let scale = BinOrdinalScale::try_new(vec![0.0, 10.0, 20.0, 30.0], vec!["red", "blue"])?;

        let values = vec![5.0, 15.0, 25.0];
        let result = scale.scale(&values)?;

        assert_eq!(result[0], Some("red")); // First bin maps to first color
        assert_eq!(result[1], Some("blue")); // Second bin maps to second color
        assert_eq!(result[2], Some("red")); // Third bin wraps back to first color

        Ok(())
    }

    #[test]
    fn test_bin_ordinal_scale_out_of_domain() -> Result<(), AvengerScaleError> {
        let scale = BinOrdinalScale::try_new(vec![0.0, 10.0, 20.0], vec!["red", "blue"])?;

        let values = vec![-5.0, 5.0, 25.0];
        let result = scale.scale(&values)?;

        assert_eq!(result[0], None); // Below first bin
        assert_eq!(result[1], Some("red")); // In first bin
        assert_eq!(result[2], None); // Above last bin

        Ok(())
    }

    #[test]
    fn test_bin_ordinal_scale_numeric() -> Result<(), AvengerScaleError> {
        let scale = BinOrdinalScale::try_new(vec![0.0, 1.0, 2.0, 3.0, 4.0], vec![10.0, 20.0])?;

        let values = vec![0.5, 1.5, 2.5, 3.5];
        let result = scale.scale(&values)?;

        assert_approx_eq!(f32, result[0].unwrap(), 10.0); // First bin -> first value
        assert_approx_eq!(f32, result[1].unwrap(), 20.0); // Second bin -> second value
        assert_approx_eq!(f32, result[2].unwrap(), 10.0); // Third bin -> back to first value
        assert_approx_eq!(f32, result[3].unwrap(), 20.0); // Fourth bin -> back to second value

        Ok(())
    }
}
