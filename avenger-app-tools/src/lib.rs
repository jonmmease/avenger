//! Reusable interactions installed with `avenger_app::app::AppBuilder::tool`.
//! [`tooltips`] shows data under the pointer, and [`text_selection`] selects and copies the
//! text of labels. `navigation` pans and zooms views when the `scales` or `geo` feature is
//! enabled.

#[cfg(any(feature = "scales", feature = "geo"))]
pub mod navigation;
pub mod text_selection;
pub mod tooltips;
