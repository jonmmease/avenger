// Public submodules
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
pub use codec::AvengerChartExtensionCodec;
pub use defaults::{
    create_default_scale_for_channel, get_channel_characteristics, is_categorical_data_type,
    is_numeric_data_type, is_temporal_data_type,
};
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
use serde::{Deserialize, Serialize};

/// Wrapper that holds both the original Scale<Auto> specification and its ConfiguredScale
///
/// This allows us to maintain full extensibility - when creating DataFusion expressions,
/// we need the original Scale<Auto> to recreate the ScaleUDF, but for most operations
/// we just need the ConfiguredScale.
///
/// Note: For serialization, we only serialize the Scale<Auto> specification since
/// ConfiguredScale contains runtime state that can be reconstructed.
#[derive(Debug, Clone)]
pub struct ConfiguredScaleWithSpec {
    /// The original scale specification
    pub scale: Scale<Auto>,
    /// The configured scale with resolved domain/range
    pub configured: ConfiguredScale,
}

impl ConfiguredScaleWithSpec {
    /// Create a new ConfiguredScaleWithSpec
    pub fn new(scale: Scale<Auto>, configured: ConfiguredScale) -> Self {
        Self { scale, configured }
    }
}

impl Serialize for ConfiguredScaleWithSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Only serialize the scale specification
        // ConfiguredScale will need to be reconstructed during deserialization
        self.scale.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ConfiguredScaleWithSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        // Deserialize the Scale<Auto> specification
        let scale = Scale::<Auto>::deserialize(deserializer)?;

        // We can't reconstruct ConfiguredScale here without additional context
        // This will need to be handled at a higher level where the domain/range are available
        Err(de::Error::custom(
            "ConfiguredScaleWithSpec cannot be deserialized directly; it must be reconstructed with domain/range context"
        ))
    }
}
