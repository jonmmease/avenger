//! # avenger-chart
//!
//! A [`Chart`] is a serializable description of a chart: its data as Arrow record batches, its
//! marks and encodings, its scales and guides, and how it combines other charts in layers, rows,
//! and columns. A chart serializes to the protobuf message `Chart` of package `avenger.chart.v1`,
//! whose files are in `proto/avenger/chart/v1/`, with its data tables and its other Arrow values
//! as Arrow IPC streams inside the message.
//!
//! The generated messages in [`proto`] are the model of a chart, and the comments of the `.proto`
//! files are their documentation. A [`Chart`] holds the message with its tables and its values
//! decoded: its units refer to tables by index, and its fields refer to values, such as constants
//! and explicit domains, by index. A chart decodes without a copy: the writer aligns each table
//! and the values, and [`Chart::from_bytes`] decodes their columns in place in its input.

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
mod merge;
pub mod payload;
pub mod resolve;
pub mod table;
mod tree;
mod validate;

pub use builder::{
    Child, ConcatBuilder, Contribution, Empty, Encode, OverlayBuilder, Scale, Selection, TupleTest,
    UnitBuilder, Value, WrapBuilder,
};
pub use chart::Chart;
pub use codec::{
    chart_field, charts_from_arrow, charts_to_arrow, Decode, EXTENSION_NAME, FORMAT_VERSION,
};
pub use error::ChartError;
pub use validate::FitKind;
