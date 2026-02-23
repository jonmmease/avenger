//! Facet module for row and column faceting.
//!
//! This module provides the infrastructure for faceted visualizations.

pub(crate) mod band_ir;
pub mod band_positions;
pub mod coord;
pub mod coord_row;
pub mod coordination;
pub(crate) mod coordination_ir;
pub(crate) mod coordination_remeasure;
pub(crate) mod coordination_sidecar;
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
pub mod padding_policy;
pub mod path_math;
pub mod scalar_cmp;
pub(crate) mod scale_precompute;
pub mod sharing_kernel;
pub(crate) mod sharing_level;
pub mod sharing_policy;
