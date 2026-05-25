pub mod facet;
pub mod facet_config;

// Re-export compiled facet subplot marks for convenience
pub use facet::{
    CompiledFacetColumnSubplot, CompiledFacetRowSubplot, FacetColumnSubplotChannels,
    FacetRowSubplotChannels,
};
pub use facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
