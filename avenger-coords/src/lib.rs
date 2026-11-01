//! # avenger-coords
//!
//! The coordinate systems of plots: how a plot's pixels relate to the positions of its two
//! positional dimensions, which tools such as brushes and navigations work in.
//!
//! A plot's coordinate system is its kind, the scales of its positional dimensions, and its
//! projection. A scale maps the data of one dimension to a range position, and a
//! [`PlotProjection`] maps the range positions of both dimensions to the plot's pixels and back.
//! [`CartesianProjection`] projects a Cartesian plot, whose positions are its pixels,
//! [`PolarProjection`] projects a polar plot, whose positions are angles and distances from its
//! center, and, with the `geo` feature, [`MapProjection`] projects a map through its projector,
//! whose positions are longitudes and latitudes.

pub mod cartesian;
#[cfg(feature = "geo")]
pub mod map;
pub mod polar;
pub mod projection;

pub use cartesian::CartesianProjection;
#[cfg(feature = "geo")]
pub use map::MapProjection;
pub use polar::PolarProjection;
pub use projection::{densified, pieces, short_way, within_period, PlotProjection, RING_STEP};
