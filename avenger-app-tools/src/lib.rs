//! Tools for Avenger apps, the reusable interactions that an app adds with
//! `AppBuilder::tool`: [`tooltips`] describes the data under the pointer, and [`navigation`],
//! behind the `scales` and `geo` features, pans and zooms an app's views.

#[cfg(any(feature = "scales", feature = "geo"))]
pub mod navigation;
pub mod tooltips;
