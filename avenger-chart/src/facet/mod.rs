//! Facet module for row and column faceting.
//!
//! Facet layout is split into a few phases:
//! 1. `evaluated_facet_tree` builds the data-driven facet hierarchy and path metadata.
//! 2. `coord` measures each facet band locally: cell semantics, coordinated
//!    plot-scale domains, estimated subplot overflows, local band layout, and
//!    child measurements.
//! 3. `coordination` reconciles requirements across matching facet bands:
//!    overflows, layout, final plot-area sizes, and scale ranges.
//! 4. `placement` and `marks` turn coordinated measurements into renderable facet
//!    guide and subplot positions.
//!
//! Faceted charts use one runtime sizing policy. Each physical dimension is
//! either canvas-constrained, where final subplot plot areas are derived from
//! the available canvas after coordinated overflows, or leaf-plot-area-sized,
//! where requested leaf plot areas determine explicit facet placement and the
//! outer canvas grows to contain them. Canvas-fit, plot-area-sized, and mixed
//! sizing are public configurations of this per-dimension policy.
//!
//! Vocabulary used throughout the facet layout code:
//! - A frame is the rectangle allocated to a chart or facet subtree.
//! - Content is the plot-area rectangle inside a frame.
//! - Residuals/overflows are the side slabs needed for axes, legends, and
//!   facet guides around content.
//! - A sibling boundary is the space between adjacent facet siblings.
//! - A global edge is an outer edge of the whole faceted chart; its overflow
//!   affects the parent frame but should not inflate sibling spacing.
//! - A lane is a branch-local alignment scope in a jagged facet tree.
//! - A guide anchor is the coordinated slab used to align facet guide labels
//!   and rules across compatible lanes.

pub(crate) mod band_attributes;
pub mod coord;
pub mod coord_row;
pub mod coordination;
pub(crate) mod coordination_apply;
pub(crate) mod coordination_plans;
pub(crate) mod coordination_policy;
pub(crate) mod data_scope;
pub mod debug;
pub mod dimension_config;
pub mod direction;
pub(crate) mod domain_coordination;
pub mod empty_cell_policy;
pub mod evaluated_facet_tree;
pub mod guide;
pub mod guide_utils;
pub mod layout_plan;
pub mod marks;
pub(crate) mod overflow_projection;
pub(crate) mod ownership_policy;
pub mod padding_policy;
pub mod path_math;
pub(crate) mod placement;
pub(crate) mod probe_summary;
pub(crate) mod round_tree;
pub(crate) mod scale_precompute;
pub mod sharing_policy;
pub(crate) mod subtree_plot_area;

pub use direction::FacetDirection;
