//! # avenger-chart
//!
//! The serialized form of a chart: the protobuf message `Chart` of package `avenger.chart.v1`,
//! whose files are in `proto/avenger/chart/v1/`. A chart holds its data as Arrow record batches,
//! its marks and encodings, its scales and guides, and how it combines other charts in layers,
//! rows, columns, and wraps. Its data tables and its other Arrow values are Arrow IPC streams
//! inside the message, which `Chart.tables` and `Chart.values` hold, and the fields of the message
//! refer to them by `TableRef` and `ValueRef`.
//!
//! The comments of the `.proto` files are the documentation of the format, and [`proto`] holds
//! the prost messages that they generate.

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
