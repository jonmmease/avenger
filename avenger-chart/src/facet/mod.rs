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

pub(crate) mod band_attributes;
pub mod band_positions;
pub mod coord;
pub mod coord_row;
pub mod coordination;
pub(crate) mod coordination_apply;
pub(crate) mod coordination_plans;
pub(crate) mod coordination_strategy;
pub mod debug;
pub mod dimension_config;
pub(crate) mod domain_coordination;
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
