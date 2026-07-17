//! Statistical compound marks built from ordinary Avenger chart primitives.
//!
//! These marks lower to `MarkGroup` plus primitive marks and transforms. The
//! high-level `avenger-chart` crate re-exports them for convenient authoring.

pub mod box_plot;
pub mod language;
pub mod violin;

pub use box_plot::{BoxPlot, BoxPlotOrientation};
pub use violin::{Violin, ViolinOrientation, ViolinWidthNormalization};
