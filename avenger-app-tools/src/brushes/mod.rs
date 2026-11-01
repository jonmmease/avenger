//! Selection brushes: boxes that select a range of each dimension of an area, and clicks and
//! lassos that select instances. One [`Brushes`] tool draws every brush of the app in one scene,
//! so a grid of panels costs one render pass. Its marks are noninteractive and clipped to each
//! area. Each box and lasso brush covers one area, and its keyed form, such as
//! [`KeyedIntervalBrush`], covers a set of areas that the app names with keys, such as the panels
//! of a facet grid.
//!
//! Each area's [`PlotProjection`](avenger_coords::PlotProjection) maps its pixels to the
//! range positions of its two dimensions: the Cartesian projection of the area's rectangle by
//! default, or the projection of a polar plot or a map that the app sets. The scale of a dimension, with the `scales` feature,
//! maps its data to the dimension's range, and a dimension without a scale, such as the longitude
//! of a map, selects range positions. So one [`IntervalBrush`] selects a box of a scatter plot, an
//! annular sector of a polar plot, and a box of longitudes and latitudes on a map, and one
//! [`LassoBrush`] selects the instances inside a path in each.
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
//! every click, whichever scene took its press, and a [`KeyedPointBrush`] on each click in its
//! areas. A click clears the box in its area, unless a
//! point brush selected an instance with it. Escape, or a cancellation such as focus loss,
//! restores the value at the press and ends the gesture, and the press's release is no click. The
//! second press of a double click passes to the scenes below, and so does the wheel while no brush
//! holds a press, so a `PanZoom` still restores its home view and zooms.

mod binding;
mod brush;
mod coords;
mod draw;
mod instances;
mod interval;
mod lasso;
mod point;
mod style;
mod tool;

pub use brush::Brush;
pub use instances::Instances;
pub use interval::{Aspect, Interval, IntervalBrush, KeyedIntervalBrush};
pub use lasso::{KeyedLassoBrush, Lasso, LassoBrush};
pub use point::{KeyedPointBrush, PointBrush};
pub use style::BrushStyle;
pub use tool::{Brushes, BrushesState};
