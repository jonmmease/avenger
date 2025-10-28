pub mod facet;
pub mod facet_config;
pub(crate) mod facet_evaluation;

// Re-export compiled facet marks for convenience
pub use facet::{CompiledFacetCol, CompiledFacetRow};
pub use facet_config::{FacetColChannelConfig, FacetRowChannelConfig};
