// Public submodules
pub mod color_defaults;
pub mod dash_defaults;
pub mod extensions;
pub mod inference;
pub mod shape_defaults;
pub mod udf;
pub mod validation;

// Internal modules
mod domain;
mod domain_inference;
mod factory;
mod range;
mod registry;
mod scale;

// Re-export the main types
pub use domain::{DomainExpr, ScaleDefaultDomain, ScaleDomain};
pub use extensions::{ConfiguredScaleDataFusionExt, ConfiguredScaleLegendExt, DomainValues};
pub use range::ScaleRange;
pub use registry::ScaleRegistry;
pub use scale::Scale;
