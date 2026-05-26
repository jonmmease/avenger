pub mod axis;
pub mod axis_position;
pub mod axis_spec;
pub mod channel;
pub mod channel_config;
pub mod channel_configs;
pub mod channel_resolution;
pub mod channel_resolution_error;
pub mod channel_value;
pub mod color;
pub mod compiled_data_context;
pub mod compiled_mark;
pub mod configured_scale_legend;
pub mod container_sharing;
pub mod coord_measurement;
pub mod coordinate_guide;
pub mod coordinate_system;
pub mod coordinate_transform;
pub mod coordination_values;
pub mod data_context;
pub mod datafusion_utils;
pub mod error;
pub mod evaluation_context;
pub mod expr_eval;
pub mod facet_axis;
pub mod facet_data_scope;
pub mod facet_dimension;
pub mod facet_empty_cell_policy;
pub mod geometry;
pub mod guide_context;
pub mod guide_overflow_phase;
pub mod guide_sharing;
pub mod guide_update;
pub mod into_expr;
pub mod layout_types;
pub mod legend;
pub mod legend_renderer;
pub mod legend_spec;
pub mod mark;
pub mod mark_channel_coercion;
pub mod mark_macros;
pub mod mark_render_context;
pub mod mark_runtime_context;
pub mod mark_state;
pub mod maybe;
pub mod no_guide;
pub mod overflow;
pub mod param;
pub mod position_config;
pub mod radius_expression;
pub mod resolved_domain;
pub mod scalar_cmp;
pub mod scale;
pub mod scale_channel_config;
pub mod scale_config_spec;
pub mod scale_domain;
pub mod scale_range;
pub mod scale_range_binding;
pub mod scale_sharing;
pub mod scale_spec;
pub mod scale_type;
pub mod serialization;
pub mod sharing;
pub mod subplot_child_plot;
pub mod theme;
pub mod theme_context;
pub mod title_spec;
pub mod zero_d;

#[doc(hidden)]
pub mod __private {
    pub use paste;
}

pub use axis::Axis;
pub use axis_position::AxisPosition;
pub use axis_spec::AxisSpec;
pub use channel::{BaseChannelName, ChannelDefault, ChannelDescriptor, strip_trailing_numbers};
pub use channel_config::ChannelConfig;
pub use channel_configs::{
    AngleChannelConfig, ColorChannelConfig, OpacityChannelConfig, ShapeChannelConfig,
    SizeChannelConfig, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};
pub use channel_resolution::{resolve_all_channel_refs, resolve_channel_refs};
pub use channel_resolution_error::{ChannelResolutionError, suggest_similar_channel_name};
pub use channel_value::{ChannelValue, ConditionalValue};
pub use compiled_data_context::CompiledDataContext;
pub use compiled_mark::{
    CompiledMark, CompiledMarkCore, default_channel_value_for_eval,
    extract_channel_title_from_marks,
};
pub use configured_scale_legend::{ConfiguredScaleLegendExt, DomainValues};
pub use container_sharing::{
    ContainerEdgeLevelProjection, SharingGroupEdge, enumeration_ancestor_path, is_group_end,
    is_group_start, owner_for_edge, project_container_edge_levels, shared_path_key,
    sharing_group_boundary,
};
pub use coord_measurement::{CoordMeasurement, EmptyCoordMeasurement};
pub use coordinate_guide::{CompiledGuide, CoordinateGuide, GuideSharingContext};
pub use coordinate_system::{CoordinateSystem, CoordinateSystemCore};
pub use coordinate_transform::{CoordinateSystemTransform, CoordinateSystemTransformCore};
pub use coordination_values::{CoordinatedLayout, CoordinatedOverflow};
pub use data_context::DataContext;
pub use datafusion_utils::{
    ArrayRefHelpers, DataFrameChartHelpers, ExprHelpers, ScalarValueHelpers, array_value_to_f64,
    contains_aggregate, eval_to_scalars, params_to_datafusion, partition_expressions,
    scalar_to_scalar_value, simplify_to_scalar_sync,
};
pub use error::AvengerChartError;
pub use evaluation_context::EvaluationContext;
pub use expr_eval::{
    evaluate_axis_position_expr, evaluate_bool_expr, evaluate_f32_expr, evaluate_f64_expr,
    evaluate_i32_expr, evaluate_legend_orientation_expr, evaluate_legend_position_expr,
    evaluate_string_expr, evaluate_usize_expr,
};
pub use facet_axis::FacetAxis;
pub use facet_data_scope::FacetDataScope;
pub use facet_dimension::{
    ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig, WrapDimensionConfig,
};
pub use facet_empty_cell_policy::FacetEmptyCellPolicy;
pub use geometry::{
    BandPosition, PaddingSpec, PlotGeometry, PointGeometry, SubplotGeometry, SubplotRect,
};
pub use guide_context::GuideContext;
pub use guide_overflow_phase::GuideOverflowPhase;
pub use guide_sharing::{
    AXIS_OWNER_IGNORE_EMPTY_CELLS_PARAM, AxisOwnershipMode, AxisVisibility,
    ChildFrameGuideSharingView, FacetGuideSharingView,
    INVALID_FACET_PATH_AXIS_FALLBACK_HIDDEN_PARAM, axis_owner_ignore_empty_cells_from_params,
    axis_ownership_mode_from_params,
};
pub use guide_update::GuideUpdate;
pub use into_expr::IntoExpr;
pub use layout_types::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, OverflowSide, OwnedEdgeSlabs, Size2D,
};
pub use legend::{LegendOrientation, LegendPosition, LegendRendererKind};
pub use legend_renderer::{
    ChannelInfo, ChannelLegendCapability, LegendChannel, LegendGroup, LegendRenderer,
    LegendRendererSelection, MergeKey, compute_range_hash, helpers, normalize_expression,
};
pub use legend_spec::Legend;
pub use mark::Mark;
pub use mark_channel_coercion::{
    coerce_bool_channel, coerce_bool_channel_with_renderer, coerce_channel, coerce_color_channel,
    coerce_color_channel_with_renderer, coerce_numeric_channel,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel, coerce_stroke_cap_channel,
    coerce_stroke_cap_channel_with_renderer, coerce_stroke_dash_channel,
    coerce_stroke_join_channel, coerce_stroke_join_channel_with_renderer, coerce_text_channel,
};
pub use mark_render_context::MarkRenderContext;
pub use mark_runtime_context::MarkRuntimeContext;
pub use mark_state::{CompiledMarkState, MarkState};
pub use maybe::{Maybe, MaybeOptionalExpr};
pub use no_guide::NoGuide;
pub use overflow::{MeasurementResult, OverflowSpaceRequirement};
pub use param::Param;
pub use position_config::{GenericPositionConfig, PositionConfig};
pub use radius_expression::RadiusExpression;
pub use resolved_domain::ResolvedDomain;
pub use scalar_cmp::scalar_total_cmp;
pub use scale::Scale;
pub use scale_channel_config::{ScaleChannelConfig, ScaleChannelValue};
pub use scale_config_spec::{ScaleConfigSpec, ScaleOrderingSpec};
pub use scale_domain::{DomainExpr, ScaleDefaultDomain, ScaleDomain};
pub use scale_range::ScaleRange;
pub use scale_range_binding::{
    PlotAreaDimension, PlotAreaRangeEndpoint, PlotAreaRangeExpr, ScaleRangeBinding,
};
pub use scale_sharing::ScaleSharing;
pub use scale_spec::{Auto, ScaleSpec};
pub use scale_type::{ScaleTypePreference, default_scale_type_for_data_type, is_continuous_scale};
pub use serialization::{
    DefaultLogicalExprNodeExt, LogicalPlanNodeExt, SerializableDataFrame, SerializableDataType,
    SerializableExpr, SerializableNestedScalarMap, SerializableScalar, SerializableScalarMap,
};
pub use sharing::{CoordinationAxis, SharingLevel};
pub use subplot_child_plot::{
    CompiledPositionedSubplot, CompiledSubplotChildPlot, CompiledSubplotPayload,
    PositionedSubplotChannel, PositionedSubplotMarkCore, PositionedSubplotSpec,
    SubplotChildPlotSpec, SubplotContainerCoordinateSystem, SubplotDataSource, SubplotMarkCore,
    compile_positioned_subplot_mark, compile_subplot_payload,
};
pub use theme::{AngleUnit, CssRgba, LengthUnit, Theme, ThemeValue};
pub use theme_context::ThemeContext;
pub use title_spec::{TitleAlign, TitleSpan};
pub use zero_d::ZeroDCoord;
