//! Reusable interactions installed with `avenger_app::app::AppBuilder::tool`.
//! [`tooltips`] shows data under the pointer, and [`text_selection`] selects and copies the
//! text of labels. `navigation` pans and zooms views when the `scales` or `geo` feature is
//! enabled, and `brushes` selects ranges of scales and maps with the same features.

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
