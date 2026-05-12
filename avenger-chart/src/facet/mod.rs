//! Facet module for row and column faceting.
//!
//! This module provides the infrastructure for faceted visualizations.

pub(crate) mod band_attributes;
pub mod band_positions;
pub mod coord;
pub(crate) mod coord_canvas_fit;
pub(crate) mod coord_fixed_subplot;
pub mod coord_row;
pub mod coordination;
pub(crate) mod coordination_apply;
pub(crate) mod coordination_apply_fixed;
pub(crate) mod coordination_canvas_fit;
pub(crate) mod coordination_fixed_subplot;
pub(crate) mod coordination_plans;
pub mod debug;
pub mod dimension_config;
pub mod empty_cell_policy;
pub mod evaluated_facet_tree;
pub mod guide;
pub mod guide_utils;
pub mod keys;
pub mod layout_plan;
pub mod layout_slabs;
pub mod marks;
pub(crate) mod ownership_policy;
pub mod padding_policy;
pub mod path_math;
pub(crate) mod probe_summary;
pub mod scalar_cmp;
pub(crate) mod scale_precompute;
pub mod sharing_kernel;
pub(crate) mod sharing_level;
pub mod sharing_policy;
