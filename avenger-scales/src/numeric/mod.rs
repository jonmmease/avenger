use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

pub mod linear;
pub mod log;
pub mod pow;
pub mod symlog;

/// A trait for scales that map to a continuous numeric range
pub trait ContinuousNumericScale<D>: Clone
where
    D: 'static + Send + Sync + Clone,
{
    /// Returns the current domain as (start, end)
    fn domain(&self) -> (D, D);
    /// Returns the current range as (start, end)
    fn range(&self) -> (f32, f32);
    /// Returns whether output clamping is enabled
    fn clamp(&self) -> bool;
    /// Maps input values from domain to range
    fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, D>>) -> ScalarOrArray<f32>;
    /// Maps output values from range back to domain
    fn invert<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, f32>>) -> ScalarOrArray<D>;
    /// Generates evenly spaced tick values within the domain
    fn ticks(&self, count: Option<f32>) -> Vec<D>;

    /// Sets the domain
    fn set_domain(&mut self, domain: (D, D));
    /// Sets the range
    fn set_range(&mut self, range: (f32, f32));
    /// Sets whether output clamping is enabled
    fn set_clamp(&mut self, clamp: bool);
}
