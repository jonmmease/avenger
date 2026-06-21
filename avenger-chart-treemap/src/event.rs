//! Treemap event datum helpers.
//!
//! These fields are emitted by treemap marks and guides. Hierarchy-level
//! concepts use `__hierarchy_*` names so future hierarchical coordinates can
//! share the same event shape; treemap-only cell geometry uses `__treemap_*`.

use avenger_chart_core::{EventDatumFieldSpec, event::datum};
use datafusion::{arrow::datatypes::DataType, logical_expr::Expr};

pub const HIERARCHY_SURFACE_KIND_FIELD: &str = "__hierarchy_surface_kind";
pub const HIERARCHY_SURFACE_KIND_LEAF_RECT: &str = "leaf-rect";
pub const HIERARCHY_SURFACE_KIND_NODE_RECT: &str = "node-rect";
pub const HIERARCHY_SURFACE_KIND_COLLAPSED_RECT: &str = "collapsed-rect";
pub const HIERARCHY_PATH_ID_FIELD: &str = "__hierarchy_path_id";
pub const HIERARCHY_PARENT_PATH_ID_FIELD: &str = "__hierarchy_parent_path_id";
pub const HIERARCHY_DEPTH_FIELD: &str = "__hierarchy_depth";
pub const HIERARCHY_VIEW_DEPTH_FIELD: &str = "__hierarchy_view_depth";
pub const HIERARCHY_DISPLAY_LEVELS_FIELD: &str = "__hierarchy_display_levels";
pub const HIERARCHY_IS_DATA_LEAF_FIELD: &str = "__hierarchy_is_data_leaf";
pub const HIERARCHY_IS_VISIBLE_LEAF_FIELD: &str = "__hierarchy_is_visible_leaf";
pub const HIERARCHY_HAS_HIDDEN_DESCENDANTS_FIELD: &str = "__hierarchy_has_hidden_descendants";
pub const HIERARCHY_CAN_ZOOM_FIELD: &str = "__hierarchy_can_zoom";
pub const HIERARCHY_VALUE_FIELD: &str = "__hierarchy_value";

pub const TREEMAP_RECT_X_FIELD: &str = "__treemap_rect_x";
pub const TREEMAP_RECT_Y_FIELD: &str = "__treemap_rect_y";
pub const TREEMAP_RECT_WIDTH_FIELD: &str = "__treemap_rect_width";
pub const TREEMAP_RECT_HEIGHT_FIELD: &str = "__treemap_rect_height";

pub(crate) const RESERVED_GENERATED_EVENT_FIELDS: &[&str] = &[
    HIERARCHY_SURFACE_KIND_FIELD,
    HIERARCHY_PATH_ID_FIELD,
    HIERARCHY_PARENT_PATH_ID_FIELD,
    HIERARCHY_DEPTH_FIELD,
    HIERARCHY_VIEW_DEPTH_FIELD,
    HIERARCHY_DISPLAY_LEVELS_FIELD,
    HIERARCHY_IS_DATA_LEAF_FIELD,
    HIERARCHY_IS_VISIBLE_LEAF_FIELD,
    HIERARCHY_HAS_HIDDEN_DESCENDANTS_FIELD,
    HIERARCHY_CAN_ZOOM_FIELD,
    HIERARCHY_VALUE_FIELD,
    TREEMAP_RECT_X_FIELD,
    TREEMAP_RECT_Y_FIELD,
    TREEMAP_RECT_WIDTH_FIELD,
    TREEMAP_RECT_HEIGHT_FIELD,
];

pub(crate) fn tree_rect_event_datum_field_specs() -> Vec<EventDatumFieldSpec> {
    vec![
        spec(HIERARCHY_SURFACE_KIND_FIELD, DataType::Utf8),
        spec(HIERARCHY_PATH_ID_FIELD, DataType::Utf8),
        spec(HIERARCHY_PARENT_PATH_ID_FIELD, DataType::Utf8),
        spec(HIERARCHY_DEPTH_FIELD, DataType::Int64),
        spec(HIERARCHY_VIEW_DEPTH_FIELD, DataType::Int64),
        spec(HIERARCHY_DISPLAY_LEVELS_FIELD, DataType::Int64),
        spec(HIERARCHY_IS_DATA_LEAF_FIELD, DataType::Boolean),
        spec(HIERARCHY_IS_VISIBLE_LEAF_FIELD, DataType::Boolean),
        spec(HIERARCHY_HAS_HIDDEN_DESCENDANTS_FIELD, DataType::Boolean),
        spec(HIERARCHY_CAN_ZOOM_FIELD, DataType::Boolean),
        spec(HIERARCHY_VALUE_FIELD, DataType::Float64),
        spec(TREEMAP_RECT_X_FIELD, DataType::Float64),
        spec(TREEMAP_RECT_Y_FIELD, DataType::Float64),
        spec(TREEMAP_RECT_WIDTH_FIELD, DataType::Float64),
        spec(TREEMAP_RECT_HEIGHT_FIELD, DataType::Float64),
    ]
}

fn spec(name: &str, data_type: DataType) -> EventDatumFieldSpec {
    EventDatumFieldSpec {
        name: name.to_string(),
        data_type,
    }
}

pub fn hierarchy_surface_kind() -> Expr {
    datum(HIERARCHY_SURFACE_KIND_FIELD)
}

pub fn hierarchy_path_id() -> Expr {
    datum(HIERARCHY_PATH_ID_FIELD)
}

pub fn hierarchy_parent_path_id() -> Expr {
    datum(HIERARCHY_PARENT_PATH_ID_FIELD)
}

pub fn hierarchy_depth() -> Expr {
    datum(HIERARCHY_DEPTH_FIELD)
}

pub fn hierarchy_view_depth() -> Expr {
    datum(HIERARCHY_VIEW_DEPTH_FIELD)
}

pub fn hierarchy_display_levels() -> Expr {
    datum(HIERARCHY_DISPLAY_LEVELS_FIELD)
}

pub fn hierarchy_is_data_leaf() -> Expr {
    datum(HIERARCHY_IS_DATA_LEAF_FIELD)
}

pub fn hierarchy_is_visible_leaf() -> Expr {
    datum(HIERARCHY_IS_VISIBLE_LEAF_FIELD)
}

pub fn hierarchy_has_hidden_descendants() -> Expr {
    datum(HIERARCHY_HAS_HIDDEN_DESCENDANTS_FIELD)
}

pub fn hierarchy_can_zoom() -> Expr {
    datum(HIERARCHY_CAN_ZOOM_FIELD)
}

pub fn hierarchy_value() -> Expr {
    datum(HIERARCHY_VALUE_FIELD)
}

pub fn treemap_rect_x() -> Expr {
    datum(TREEMAP_RECT_X_FIELD)
}

pub fn treemap_rect_y() -> Expr {
    datum(TREEMAP_RECT_Y_FIELD)
}

pub fn treemap_rect_width() -> Expr {
    datum(TREEMAP_RECT_WIDTH_FIELD)
}

pub fn treemap_rect_height() -> Expr {
    datum(TREEMAP_RECT_HEIGHT_FIELD)
}
