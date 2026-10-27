//! Selection brushes: boxes that select ranges of an app's scales, or of longitude and latitude
//! on maps, and clicks and lassos that select instances. One [`Brushes`] tool draws every brush
//! of the app in one scene, so a grid of panels costs one render pass. Its marks are
//! noninteractive and clipped to each area. Interval brushes need the `scales` feature, and
//! geographic boxes the `geo` feature. Each box and lasso brush covers one area, and its keyed
//! form, such as [`KeyedIntervalBrush`], covers a set of areas that the app names with keys, such
//! as the panels of a facet grid.
//!
//! The app stores each selection in its own state, in data terms, and reads it back through a
//! brush's `value`. A brush computes each next value from the gesture and reports it through
//! `on_change`, and every brush but a [`PointBrush`] also through `on_settle`. The callbacks
//! store the value and request the scenes that show it. The brush draws the value that the app
//! stores, so the app can constrain or replace it.
//!
//! The tool applies these rules to a left-button press, considering only active brushes:
//!
//! 1. A press within 5 pixels of a box's edge or corner resizes the box, and a press inside it
//!    moves it. Later brushes and later areas win where boxes overlap.
//! 2. Otherwise, the first brush with an area that contains the press draws a new box or a
//!    [`LassoBrush`] path there.
//! 3. Otherwise, the press passes to the scenes below, where a `PanZoom` can pan.
//!
//! The tool consumes the presses that it claims, so a pan below never starts from one. A press
//! whose pointer stays within 4 pixels until its release is a click. A [`PointBrush`] acts on
//! every click, whichever scene took its press. A click clears the box in its area, unless a
//! point brush selected an instance with it. Escape, or a cancellation such as focus loss,
//! restores the value at the press and ends the gesture, and the press's release is no click. The
//! second press of a double click passes to the scenes below, and so does the wheel while no brush
//! holds a press, so a `PanZoom` still restores its home view and zooms.

#[cfg(feature = "scales")]
mod axis;
mod binding;
mod brush;
mod draw;
#[cfg(feature = "geo")]
mod geo_box;
mod instances;
#[cfg(feature = "scales")]
mod interval;
mod lasso;
mod point;
mod style;
mod tool;

pub use brush::Brush;
#[cfg(feature = "geo")]
pub use geo_box::{GeoBox, GeoBoxBrush, KeyedGeoBoxBrush};
pub use instances::Instances;
#[cfg(feature = "scales")]
pub use interval::{Aspect, Interval, IntervalBrush, KeyedIntervalBrush};
pub use lasso::{KeyedLassoBrush, Lasso, LassoBrush};
pub use point::PointBrush;
pub use style::BrushStyle;
pub use tool::{Brushes, BrushesState};
