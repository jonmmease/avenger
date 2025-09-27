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
pub use udf::{ScaleUDFMetadata, create_scale_udf};

/// Infer a scale specification from its name
pub fn infer_scale_type_from_name(scale_type: &str) -> Box<dyn ScaleSpec> {
    match scale_type {
        "linear" => Box::new(Linear),
        "log" => Box::new(Log),
        "pow" => Box::new(Pow),
        "sqrt" => Box::new(Sqrt),
        "symlog" => Box::new(Symlog),
        "time" => Box::new(Time),
        "band" => Box::new(Band),
        "point" => Box::new(Point),
        "ordinal" => Box::new(Ordinal),
        "threshold" => Box::new(Threshold),
        "quantile" => Box::new(Quantile),
        "quantize" => Box::new(Quantize),
        _ => Box::new(Auto), // Default to Auto for unknown types
    }
}
