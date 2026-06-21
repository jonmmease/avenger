//! Treemap coordinate system for `avenger-chart`.
//!
//! This crate owns treemap-specific hierarchy aggregation, layout measurement,
//! marks, and guides. It depends on lower-level chart crates and does not
//! require treemap-specific branches in the high-level `avenger-chart` facade.
//!
//! By default, treemap layout is strict: visible rectangles fill the plot area
//! without reserving space for labels or group chrome. Enabling
//! [`TreemapHeaderBars`] switches to a decorated layout contract where parent
//! `outer_rect` areas remain value-proportional among siblings, while children
//! are packed into each parent `content_rect` after subtracting fixed-pixel
//! header bars and padding. In decorated mode, leaf areas are therefore locally
//! proportional within each parent content area rather than globally exact
//! across branches with different header or padding chrome.

mod coord;
pub mod event;
mod guide;
mod layout;
mod mark;

pub use coord::{
    HierarchyViewWindow, ROOT_PATH_ID, Treemap, TreemapCoordMeasurement, TreemapHeaderBars,
    TreemapLayoutOptions, TreemapNode, TreemapPadding, TreemapPathComponent, TreemapPathLevel,
    TreemapRect, TreemapTransform, VisibleTreemapNode,
};
pub use guide::TreemapGuide;
pub use mark::{TreeHeader, TreeLabel, TreeLabelFit, TreeRect, TreeRectNodeMode};
