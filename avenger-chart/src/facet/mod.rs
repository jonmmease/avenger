//! Facet module for row and column faceting.
//!
//! Facet layout is split into a few phases:
//! 1. `evaluated_facet_tree` builds the data-driven facet hierarchy and path metadata.
//! 2. `coord` measures each facet band locally: cell semantics, estimated subplot
//!    overflows, local band layout, and child measurements.
//! 3. `coordination` reconciles requirements across matching facet bands:
//!    overflows, layout, shared domains, final plot-area sizes, and scale ranges.
//! 4. `placement` and `marks` turn coordinated measurements into renderable facet
//!    guide and subplot positions.
//!
//! Canvas-fit facets start from an outer canvas and resize subplot plot areas to
//! fit coordinated overflows. Plot-area-sized facets start from requested leaf
//! plot areas; `subtree_plot_area` provides initial subtree-size estimates, and
//! final extents come from coordinated explicit placement.

pub(crate) mod band_attributes;
pub mod band_positions;
pub mod coord;
pub(crate) mod coord_canvas_fit;
pub(crate) mod coord_plot_area_sized;
pub mod coord_row;
pub mod coordination;
pub(crate) mod coordination_apply;
pub(crate) mod coordination_canvas_fit;
pub(crate) mod coordination_plans;
pub(crate) mod coordination_plot_area_sized;
pub(crate) mod coordination_strategy;
pub mod debug;
pub mod dimension_config;
pub mod empty_cell_policy;
pub mod evaluated_facet_tree;
pub mod guide;
pub mod guide_utils;
pub mod keys;
pub mod layout_plan;
pub mod marks;
pub(crate) mod overflow_projection;
pub(crate) mod ownership_policy;
pub mod padding_policy;
pub mod path_math;
pub(crate) mod placement;
pub(crate) mod probe_summary;
pub mod scalar_cmp;
pub(crate) mod scale_precompute;
pub mod sharing_kernel;
pub(crate) mod sharing_level;
pub mod sharing_policy;
pub(crate) mod subtree_plot_area;
