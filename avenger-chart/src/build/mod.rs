//! Helpers for producers of charts that write the messages of the format: parts that collect a
//! chart's tables and values, fragments of messages with their own values, the combination of
//! finished charts, and the inference of the scale types and domains that marks and columns
//! imply.
//!
//! A producer writes the protobuf messages and lets these helpers do the work that every producer
//! would otherwise repeat. Each helper returns a chart that the reader checked, so a chart that a
//! helper returns is valid and canonical.

mod combine;
mod fragment;
mod infer;
mod parts;

pub use combine::{concat, facet_grid, overlay, with_partition, wrap};
pub use fragment::Fragment;
pub use infer::infer_scales;
pub use parts::{cartesian, chart, column, geo, polar, scaled_column, ChartParts};
