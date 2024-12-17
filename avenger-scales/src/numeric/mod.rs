use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

pub mod linear;
pub mod log;
pub mod pow;
pub mod symlog;

/// A trait for scales that map to a continuous numeric range
pub trait ContinuousNumericScale: Clone {
    type Domain: 'static + Send + Sync + Clone;

    /// Returns the current domain as (start, end)
    fn domain(&self) -> (Self::Domain, Self::Domain);

    /// Sets the domain
    fn with_domain(self, domain: (Self::Domain, Self::Domain)) -> Self;

    /// Returns the current range as (start, end)
    fn range(&self) -> (f32, f32);

    /// Sets the range
    fn with_range(self, range: (f32, f32)) -> Self;

    /// Returns the current range length
    fn range_length(&self) -> f32 {
        self.range().1 - self.range().0
    }

    /// Returns whether output clamping is enabled
    fn clamp(&self) -> bool;

    /// Sets whether output clamping is enabled
    fn with_clamp(self, clamp: bool) -> Self;

    /// Returns whether output rounding is enabled
    fn round(&self) -> bool;

    /// Sets whether output rounding is enabled
    fn with_round(self, round: bool) -> Self;

    /// Maps input values from domain to range
    fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, Self::Domain>>,
    ) -> ScalarOrArray<f32>;

    /// Maps a single input value from domain to range
    fn scale_scalar(&self, value: Self::Domain) -> f32 {
        self.scale(&vec![value])
            .as_iter(1, None)
            .next()
            .cloned()
            .unwrap()
    }
    /// Maps output values from range back to domain
    fn invert<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
    ) -> ScalarOrArray<Self::Domain>;

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
