//! Reusable interactions installed with `avenger_app::app::AppBuilder::tool`.
//! [`tooltips`] shows data under the pointer. `navigation` pans and zooms views
//! when the `scales` or `geo` feature is enabled.

#[cfg(any(feature = "scales", feature = "geo"))]
pub mod navigation;
pub mod tooltips;
