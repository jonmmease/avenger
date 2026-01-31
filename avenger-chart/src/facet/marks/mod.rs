pub mod facet;
pub mod facet_config;

// Re-export compiled facet marks for convenience
pub use facet::{CompiledFacetCol, CompiledFacetRow};
pub use facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
