//! # avenger-chart
//!
//! A [`Chart`] is a serializable description of a chart: its data as Arrow record batches, its
//! marks and encodings, its scales and guides, and how it combines other charts in layers, rows,
//! and columns. A chart serializes to the protobuf message `Chart` of package `avenger.chart.v1`,
//! whose files are in `proto/avenger/chart/v1/`, with its data tables as Arrow IPC streams inside
//! the message.

/// The prost messages of package `avenger.chart.v1`.
mod proto {
    include!(concat!(env!("OUT_DIR"), "/avenger.chart.v1.rs"));
}
