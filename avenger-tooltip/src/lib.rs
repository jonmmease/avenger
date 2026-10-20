//! Tooltips for an avenger app: an overlay that shows the data under the pointer, with the event
//! streams that find it. Give the marks to describe their rows as scene data, and add the
//! tooltips to the app as an overlay:
//!
//! ```ignore
//! let app = app.with_overlay(Arc::new(Tooltips::new()))?;
//! ```
//!
//! The tooltips find the data in the app's hit-testing tree and draw in its overlay, so a
//! hover never builds the scene or the tree. The text engine formats the values, so it needs a
//! number formatter for numbers and a datetime formatter for dates and times.

pub mod draw;
pub mod error;
mod hover;
mod target;
mod tooltips;

pub use avenger_geometry::focus::{FocusStrategy, Side};
pub use draw::{TooltipField, TooltipStyle};
pub use target::{GroupOrder, GroupTarget, InstanceTarget, TooltipAnchor, TooltipTarget};
pub use tooltips::Tooltips;
