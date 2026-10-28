//! # avenger-chart
//!
//! A [`Chart`] is a serializable description of a chart: its data as Arrow record batches, its
//! marks and encodings, its scales and guides, and how it combines other charts in layers, rows,
//! and columns. A chart serializes to the protobuf message `Chart` of package `avenger.chart.v1`,
//! whose files are in `proto/avenger/chart/v1/`, with its data tables as Arrow IPC streams inside
//! the message.
//!
//! The generated messages in [`proto`] are the model of a chart, and the comments of the `.proto`
//! files are their documentation. A [`Chart`] holds the message with its tables decoded, and
//! [`payload`] decodes the other Arrow fields of the message. A chart decodes its tables without a
//! copy: the writer aligns each table, and [`Chart::from_bytes`] decodes each table's columns in
//! place in its input.

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

mod builder;
mod chart;
mod codec;
mod error;
pub mod payload;
mod tree;
mod validate;

pub use builder::{
    Child, ConcatBuilder, Empty, Encode, OverlayBuilder, Scale, UnitBuilder, Value, WrapBuilder,
};
pub use chart::Chart;
pub use codec::FORMAT_VERSION;
pub use error::ChartError;
