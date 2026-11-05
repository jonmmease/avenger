//! # avenger-chart
//!
//! The avenger chart format and its reader. A chart is a protobuf message of package
//! `avenger.chart.v1`, whose files are in `proto/avenger/chart/v1/`, with its data tables and its
//! other Arrow values as Arrow IPC streams inside the message. The comments of the `.proto` files
//! are the specification of the format, and they are the documentation of the generated messages
//! in [`proto`].
//!
//! A [`Chart`] holds a message with its tables and its values decoded: its layers refer to tables
//! by `TableRef`, and its fields refer to values, such as constants and explicit domains, by
//! `ValueRef`. Every constructor checks the chart against the rules of the format and rewrites it
//! into the one canonical form that [`Chart::to_bytes`] writes. A chart decodes without a copy:
//! the writer aligns each table and the values, and [`Chart::from_bytes`] decodes their columns in
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

mod chart;
mod codec;
mod error;
pub mod layer;
mod normalize;
mod read;
mod validate;
pub mod walk;

pub use chart::Chart;
pub use codec::{
    chart_field, charts_from_arrow, charts_to_arrow, list, Decode, EXTENSION_NAME, FORMAT_VERSION,
};
pub use error::ChartError;
pub use layer::{CoordKind, LayerRef, MarkMessage, MarkType};
pub use validate::FitKind;
