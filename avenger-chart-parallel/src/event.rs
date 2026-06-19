//! Parallel-coordinate event datum helpers.
//!
//! These fields are emitted by parallel-coordinate marks and guides. They are
//! ordinary chart event datum fields, so the helpers are thin wrappers around
//! `avenger_chart_core::event::datum(...)`.

use avenger_chart_core::event::datum;
use datafusion::prelude::Expr;

pub const PARALLEL_SURFACE_KIND_FIELD: &str = "__parallel_surface_kind";
pub const PARALLEL_SURFACE_KIND_DIMENSION_TITLE: &str = "dimension-title";
pub const PARALLEL_SURFACE_KIND_POINT: &str = "point";
pub const PARALLEL_DIMENSION_ID_FIELD: &str = "__parallel_dimension_id";
pub const PARALLEL_SCALE_NAME_FIELD: &str = "__parallel_scale_name";
pub const PARALLEL_TITLE_FIELD: &str = "__parallel_title";
pub const PARALLEL_ORDER_INDEX_FIELD: &str = "__parallel_order_index";
pub const PARALLEL_EQUILIBRIUM_X_FIELD: &str = "__parallel_equilibrium_x";
pub const PARALLEL_DISPLAY_X_FIELD: &str = "__parallel_display_x";
pub const PARALLEL_DISPLACEMENT_PX_FIELD: &str = "__parallel_displacement_px";
pub const PARALLEL_DISPLACEMENT_SLOTS_FIELD: &str = "__parallel_displacement_slots";

/// Kind of parallel-coordinate guide or mark surface under the pointer.
pub fn parallel_surface_kind() -> Expr {
    datum(PARALLEL_SURFACE_KIND_FIELD)
}

/// Stable dimension id associated with a parallel-coordinate surface.
pub fn parallel_dimension_id() -> Expr {
    datum(PARALLEL_DIMENSION_ID_FIELD)
}

/// Scale name associated with a parallel-coordinate surface.
pub fn parallel_scale_name() -> Expr {
    datum(PARALLEL_SCALE_NAME_FIELD)
}

/// Display title associated with a parallel-coordinate guide surface.
pub fn parallel_title() -> Expr {
    datum(PARALLEL_TITLE_FIELD)
}

/// Zero-based equilibrium order index for a parallel-coordinate dimension.
pub fn parallel_order_index() -> Expr {
    datum(PARALLEL_ORDER_INDEX_FIELD)
}

/// Equilibrium x position for a parallel-coordinate dimension header.
pub fn parallel_equilibrium_x() -> Expr {
    datum(PARALLEL_EQUILIBRIUM_X_FIELD)
}

/// Current display x position for a parallel-coordinate dimension header.
pub fn parallel_display_x() -> Expr {
    datum(PARALLEL_DISPLAY_X_FIELD)
}

/// Current display displacement from equilibrium, in pixels.
pub fn parallel_displacement_px() -> Expr {
    datum(PARALLEL_DISPLACEMENT_PX_FIELD)
}

/// Current display displacement from equilibrium, in axis-slot units.
pub fn parallel_displacement_slots() -> Expr {
    datum(PARALLEL_DISPLACEMENT_SLOTS_FIELD)
}
