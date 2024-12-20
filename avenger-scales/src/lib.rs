pub mod array;
pub mod band;
pub mod bin_ordinal;
pub mod color;
pub mod config;
pub mod error;
pub mod identity;
pub mod numeric;
pub mod ordinal;
pub mod point;
pub mod quantile;
pub mod quantize;
pub mod threshold;

// #[cfg(feature = "arrow")]
pub mod arrow;

#[cfg(feature = "temporal")]
pub mod temporal;
