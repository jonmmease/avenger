// Public submodules
pub mod builder;
pub mod codec;
pub mod extensions;
pub mod udf;

// Internal modules
mod defaults;
mod domain;
mod domain_inference;
mod range;
mod scale;
pub mod spec;

// Re-export the main types
pub use builder::{ChannelScaleBuilder, ScaleBuilder};
pub use codec::AvengerChartExtensionCodec;
pub use defaults::default_range_for_channel;
pub use domain::{DomainExpr, ResolvedDomain, ScaleDefaultDomain, ScaleDomain};
pub use extensions::{ConfiguredScaleDataFusionExt, ConfiguredScaleLegendExt, DomainValues};
pub use range::ScaleRange;
pub use scale::Scale;
pub use spec::{
    Auto, Band, Linear, Log, Ordinal, Point, Pow, Quantile, Quantize, ScaleSpec, Sqrt, Symlog,
    Threshold, Time,
};
pub use udf::create_scale_udf;

use avenger_scales::scales::ConfiguredScale;

/// Wrapper that holds both the original Scale<Auto> specification and its ConfiguredScale
///
/// This allows us to maintain full extensibility - when creating DataFusion expressions,
/// we need the original Scale<Auto> to recreate the ScaleUDF, but for most operations
/// we just need the ConfiguredScale.
#[derive(Debug, Clone)]
pub struct ConfiguredScaleWithSpec {
    /// The original scale specification
    scale: Scale<Auto>,
    /// The configured scale with resolved domain/range
    configured: ConfiguredScale,
}

impl ConfiguredScaleWithSpec {
    /// Create a new ConfiguredScaleWithSpec
    pub fn new(scale: Scale<Auto>, configured: ConfiguredScale) -> Self {
        Self { scale, configured }
    }

    /// Access the scale specification
    pub fn spec(&self) -> &Scale<Auto> {
        &self.scale
    }

    /// Access the configured scale
    pub fn configured(&self) -> &ConfiguredScale {
        &self.configured
    }
}
