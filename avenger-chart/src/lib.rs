#![doc = include_str!("../README.md")]

#[allow(
    clippy::enum_variant_names,
    clippy::large_enum_variant,
    clippy::doc_overindented_list_items
)]
pub mod proto {
    //! The prost messages of package `avenger.chart.v1`, generated from the `.proto` files in
    //! `proto/avenger/chart/v1/`, whose comments are their documentation, and their builders.
    //!
    //! Each message `Foo` has a builder `FooBuilder`, from `Foo::builder()`, with one method for
    //! each field and each oneof member, named after it, which sets it and nothing else, and each
    //! oneof member has a constructor on its message that returns the builder with the member set.
    //! The builders hold the tables and the values that their fields take, and
    //! [`ChartBuilder::build`] writes them into the chart.
    include!(concat!(env!("OUT_DIR"), "/avenger.chart.v1.rs"));
    include!(concat!(env!("OUT_DIR"), "/builders.rs"));
}

pub mod builder;
mod chart;
mod codec;
mod compose;
mod error;
mod infer;
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
    chart_field, charts_from_arrow, charts_to_arrow, list_value, values_from_ipc, values_to_ipc,
    Decode, EXTENSION_NAME, FORMAT_VERSION,
};
pub use error::ChartError;
pub use layer::{CoordKind, LayerRef, MarkMessage, MarkType};
pub use validate::FitKind;
