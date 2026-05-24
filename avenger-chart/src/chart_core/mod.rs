//! Internal staging namespace for the future `avenger-chart-core` crate.
//!
//! This module owns low-risk core utilities as they move toward extraction and
//! re-exports existing in-crate modules for areas that have not moved yet. New
//! internal imports can target this namespace when code is being prepared for
//! the crate split.

#![allow(unused_imports)]

pub mod error;
pub mod evaluation_context;
pub mod expr_eval {
    pub use avenger_chart_core::expr_eval::*;
}
pub mod maybe;

pub mod axis_position {
    pub use avenger_chart_core::axis_position::*;
}

pub mod axis {
    pub use avenger_chart_core::axis::*;
}

pub mod channel_config {
    pub use avenger_chart_core::channel_config::*;
}

pub mod channel_configs {
    pub use avenger_chart_core::channel_configs::*;
}

pub mod color {
    pub use avenger_chart_core::color::*;
}

pub mod compiled_data_context {
    pub use avenger_chart_core::compiled_data_context::*;
}

pub mod compiled_mark {
    pub use avenger_chart_core::compiled_mark::*;
}

pub mod coord_measurement {
    pub use avenger_chart_core::coord_measurement::*;
}

pub mod coordinate_transform {
    pub use avenger_chart_core::coordinate_transform::*;
}

pub mod coordinate_system {
    pub use avenger_chart_core::coordinate_system::*;
}

pub mod coordination_values {
    pub use avenger_chart_core::coordination_values::*;
}

pub mod data_context {
    pub use avenger_chart_core::data_context::*;
}

pub mod datafusion_utils {
    pub use avenger_chart_core::datafusion_utils::*;
}

pub mod facet_axis {
    pub use avenger_chart_core::facet_axis::*;
}

pub mod facet_strategy {
    pub use avenger_chart_core::facet_strategy::*;
}

pub mod geometry {
    pub use avenger_chart_core::geometry::*;
}

pub mod guide_context {
    pub use avenger_chart_core::guide_context::*;
}

pub mod guide_overflow_phase {
    pub use avenger_chart_core::guide_overflow_phase::*;
}

pub mod into_expr {
    pub use avenger_chart_core::into_expr::*;
}

pub mod layout_types {
    pub use avenger_chart_core::layout_types::*;
}

pub mod legend {
    pub use avenger_chart_core::legend::*;
}

pub mod mark_state {
    pub use avenger_chart_core::mark_state::*;
}

pub mod mark_render_context {
    pub use avenger_chart_core::mark_render_context::*;
}

pub mod overflow {
    pub use avenger_chart_core::overflow::*;
}

pub mod param {
    pub use avenger_chart_core::param::*;
}

pub mod position_config {
    pub use avenger_chart_core::position_config::*;
}

pub mod radius_expression {
    pub use avenger_chart_core::radius_expression::*;
}

pub mod resolved_domain {
    pub use avenger_chart_core::resolved_domain::*;
}

pub mod scale_sharing {
    pub use avenger_chart_core::scale_sharing::*;
}

pub mod scale_range {
    pub use avenger_chart_core::scale_range::*;
}

pub mod scale_range_binding {
    pub use avenger_chart_core::scale_range_binding::*;
}

pub mod scale_config_spec {
    pub use avenger_chart_core::scale_config_spec::*;
}

pub mod scale_type {
    pub use avenger_chart_core::scale_type::*;
}

pub use avenger_chart_core::{
    AngleChannelConfig, ArrayRefHelpers, Axis, AxisPosition, BandPosition, BaseChannelName,
    ChannelConfig, ChannelDefault, ChannelDescriptor, ChannelValue, ColorChannelConfig,
    CompiledDataContext, CompiledMarkCore, CompiledMarkState, ConditionalValue, CoordMeasurement,
    CoordinateSystemCore, CoordinateSystemTransformCore, CoordinatedLayout, CoordinatedOverflow,
    DataContext, DataFrameChartHelpers, EdgeSlabs, EmptyCoordMeasurement, ExprHelpers, FacetAxis,
    FacetStrategy, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout,
    FrameSizingPolicy, GenericPositionConfig, GuideContext, GuideOverflowPhase, GuideUpdate,
    IntoExpr, LayoutBounds, Legend, LegendOrientation, LegendPosition, LegendRendererKind,
    MarkRenderContext, MarkState, Maybe, MaybeOptionalExpr, MeasurementResult,
    OpacityChannelConfig, OverflowSide, OverflowSpaceRequirement, OwnedEdgeSlabs, PaddingSpec,
    Param, PlotAreaDimension, PlotAreaRangeEndpoint, PlotAreaRangeExpr, PlotGeometry,
    PointGeometry, PositionConfig, RadiusExpression, ScalarValueHelpers, ScaleConfigSpec,
    ScaleRange, ScaleRangeBinding, ScaleSharing, ScaleSpec, ScaleTypePreference,
    ShapeChannelConfig, Size2D, SizeChannelConfig, StrokeDashChannelConfig,
    StrokeWidthChannelConfig, SubplotGeometry, SubplotRect, contains_aggregate,
    default_channel_value_for_eval, default_scale_type_for_data_type, eval_to_scalars,
    evaluate_axis_position_expr, evaluate_bool_expr, evaluate_f32_expr, evaluate_f64_expr,
    evaluate_i32_expr, evaluate_legend_orientation_expr, evaluate_legend_position_expr,
    evaluate_string_expr, evaluate_usize_expr, extract_channel_title_from_marks,
    is_continuous_scale, params_to_datafusion, partition_expressions, scalar_to_scalar_value,
    simplify_to_scalar_sync, strip_trailing_numbers,
};
pub use error::AvengerChartError;
pub use evaluation_context::EvaluationContext;
pub use resolved_domain::ResolvedDomain;

pub(crate) use crate::channel;
pub(crate) use crate::coords;
pub(crate) use crate::guide;
pub(crate) use crate::layout;
pub(crate) use crate::render;
pub(crate) use crate::serialization;
pub(crate) use crate::theme;
pub(crate) use crate::zerod;
