pub mod facet;
pub mod facet_config;

// Re-export compiled facet subplot marks for convenience
pub use facet::{
    CompiledFacetColumnSubplot, CompiledFacetRowSubplot, CompiledFacetWrapSubplot,
    FacetColumnSubplotChannels, FacetRowSubplotChannels, FacetWrapSubplotChannels,
};
pub use facet_config::{
    FacetColChannelConfig, FacetGuideOptions, FacetRowChannelConfig, FacetWrapChannelConfig,
};
