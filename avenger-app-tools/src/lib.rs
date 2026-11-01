//! Reusable interactions installed with `avenger_app::app::AppBuilder::tool`.
//! [`tooltips`] shows data under the pointer, and [`text_selection`] selects and copies the
//! text of labels. With the `scales` or `geo` feature, `navigation` pans and zooms views, and
//! `brushes` selects ranges of the dimensions of plots, through each plot's projection, and
//! instances.

#[cfg(any(feature = "scales", feature = "geo"))]
pub mod brushes;
#[cfg(any(feature = "scales", feature = "geo"))]
pub mod navigation;
pub mod text_selection;
pub mod tooltips;

/// How far the pointer can move from a press, in pixels, while the press stays a click: a
/// navigation tool doesn't pan, and a brush treats the release as a click.
#[cfg(any(feature = "scales", feature = "geo"))]
const CLICK_DISTANCE: f32 = 4.0;
