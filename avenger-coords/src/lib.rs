//! # avenger-coords
//!
//! The coordinate systems of plots: how a plot's pixels relate to its positions, which have two
//! dimensions and which tools such as brushes and navigations work in.
//!
//! A plot's coordinate system is its kind, the scales of its positional data, and its projection. A
//! [`PlotProjection`] maps the plot's positions to its pixels and back. In a Cartesian, polar, or
//! geographic plot, each dimension of a position is one positional dimension, whose scale maps its
//! data to a range position. A plot whose positions combine several columns, such as a ternary
//! plot, has positions that no single scale gives, and shows a region of them rather than a box.
//! [`CartesianProjection`] projects a Cartesian plot, whose positions are its pixels,
//! [`PolarProjection`] projects a polar plot, whose positions are angles and distances from its
//! center, and, with the `geo` feature, [`GeoProjection`] projects a geo plot through its
//! projector, whose positions are longitudes and latitudes, or the planar coordinates of a planar
//! geo plot.
//!
//! [`PlotCoords`] holds a plot's projection and the scale of each dimension that has one, and
//! converts the data of each dimension to positions and back, as tools such as brushes need. Its
//! [`pixels`](PlotCoords::pixels) places a batch of rows, through the scales and the projection's
//! [`pixels`](PlotProjection::pixels), which takes the [`Positions`] of the rows as Arrow arrays.

pub mod cartesian;
pub mod coords;
#[cfg(feature = "geo")]
pub mod geo;
pub mod polar;
pub mod positions;
pub mod projection;

pub use cartesian::CartesianProjection;
pub use coords::{Dimension, PlotCoords};
#[cfg(feature = "geo")]
pub use geo::GeoProjection;
pub use polar::PolarProjection;
pub use positions::Positions;
pub use projection::{densified, pieces, short_way, within_period, PlotProjection, RING_STEP};
