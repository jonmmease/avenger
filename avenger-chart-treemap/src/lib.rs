//! Treemap coordinate system for `avenger-chart`.
//!
//! This crate owns treemap-specific hierarchy aggregation, layout measurement,
//! marks, and guides. It depends on lower-level chart crates and does not
//! require treemap-specific branches in the high-level `avenger-chart` facade.

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
pub use mark::{TreeLabel, TreeLabelFit, TreeRect, TreeRectNodeMode};
