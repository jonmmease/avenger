use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use opts::NumericScaleOptions;

pub mod linear;
pub mod log;
pub mod opts;
pub mod pow;
pub mod symlog;

/// A trait for scales that map to a continuous numeric range
pub trait ContinuousNumericScale<D>
where
    D: 'static + Send + Sync + Clone,
{
    /// Returns the current domain as (start, end)
    fn get_domain(&self) -> (D, D);
    /// Returns the current range as (start, end)
    fn get_range(&self) -> (f32, f32);
    /// Returns whether output clamping is enabled
    fn get_clamp(&self) -> bool;
    /// Maps input values from domain to range
    fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, D>>,
        opts: &NumericScaleOptions,
    ) -> ScalarOrArray<f32>;
    /// Maps output values from range back to domain
    fn invert<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
        opts: &NumericScaleOptions,
    ) -> ScalarOrArray<D>;
    /// Generates evenly spaced tick values within the domain
    fn ticks(&self, count: Option<f32>) -> Vec<D>;
}
