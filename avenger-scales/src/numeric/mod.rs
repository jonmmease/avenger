use std::sync::Arc;

use avenger_common::value::ScalarOrArray;

pub mod linear;
pub mod log;
pub mod pow;
pub mod symlog;

pub type ContinuousNumericScaleBuilder<D: 'static + Send + Sync + Clone> =
    Arc<dyn Fn() -> Box<dyn ContinuousNumericScale<Domain = D>> + Send + Sync>;

/// A trait for scales that map to a continuous numeric range
pub trait ContinuousNumericScale: Send + Sync {
    type Domain: 'static + Send + Sync + Clone;

    /// Returns the current domain as (start, end)
    fn domain(&self) -> (Self::Domain, Self::Domain);

    /// Sets the domain
    fn set_domain(&mut self, domain: (Self::Domain, Self::Domain));

    /// Returns the current range as (start, end)
    fn range(&self) -> (f32, f32);

    /// Sets the range
    fn set_range(&mut self, range: (f32, f32));

    /// Returns the current range length
    fn range_length(&self) -> f32 {
        self.range().1 - self.range().0
    }

    /// Returns whether output clamping is enabled
    fn clamp(&self) -> bool;

    /// Sets whether output clamping is enabled
    fn set_clamp(&mut self, clamp: bool);

    /// Returns whether output rounding is enabled
    fn round(&self) -> bool;

    /// Sets whether output rounding is enabled
    fn set_round(&mut self, round: bool);

    /// Maps input values from domain to range
    fn scale(&self, values: &[Self::Domain]) -> ScalarOrArray<f32>;

    /// Maps a single input value from domain to range
    fn scale_scalar(&self, value: Self::Domain) -> f32 {
        self.scale(&vec![value])
            .as_iter(1, None)
            .next()
            .cloned()
            .unwrap()
    }

    /// Scale while overriding the range
    fn scale_with_range(
        &mut self,
        values: &[Self::Domain],
        range: (f32, f32),
    ) -> ScalarOrArray<f32> {
        let original_range = self.range();
        self.set_range(range);
        let result = self.scale(values);
        self.set_range(original_range);
        result
    }

    /// Scale while overriding the range and domain
    fn scale_with_domain_and_range<'a>(
        &mut self,
        values: &[Self::Domain],
        range: (f32, f32),
        domain: (Self::Domain, Self::Domain),
    ) -> ScalarOrArray<f32> {
        let original_range = self.range();
        let original_domain = self.domain();
        self.set_range(range);
        self.set_domain(domain);
        let result = self.scale(values);
        self.set_range(original_range);
        self.set_domain(original_domain);
        result
    }

    /// Maps output values from range back to domain
    fn invert(&self, values: &[f32]) -> ScalarOrArray<Self::Domain>;

    /// Invert a single value from range back to domain
    fn invert_scalar(&self, value: f32) -> Self::Domain {
        self.invert(&vec![value])
            .as_iter(1, None)
            .next()
            .cloned()
            .unwrap()
    }

    /// Generates evenly spaced tick values within the domain
    fn ticks(&self, count: Option<f32>) -> Vec<Self::Domain>;
}
