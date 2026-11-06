#![doc = include_str!("../README.md")]

#[allow(
    clippy::enum_variant_names,
    clippy::large_enum_variant,
    clippy::doc_overindented_list_items
)]
pub mod proto {
    //! The prost messages of package `avenger.chart.v1`, generated from the `.proto` files in
    //! `proto/avenger/chart/v1/`, whose comments are their documentation.
    include!(concat!(env!("OUT_DIR"), "/avenger.chart.v1.rs"));
}

mod chart;
mod codec;
mod error;
pub mod layer;
mod merge;
mod normalize;
mod read;
pub mod resolve;
pub mod table;
mod validate;
pub mod walk;

pub use chart::Chart;
pub use codec::{
    chart_field, charts_from_arrow, charts_to_arrow, list_value, Decode, EXTENSION_NAME,
    FORMAT_VERSION,
};
pub use error::ChartError;
pub use layer::{CoordKind, LayerRef, MarkMessage, MarkType};
pub use validate::FitKind;
