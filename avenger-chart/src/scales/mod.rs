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
