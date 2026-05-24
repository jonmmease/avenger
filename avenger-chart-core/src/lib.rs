pub mod axis;
pub mod axis_position;
pub mod channel;
pub mod channel_config;
pub mod channel_configs;
pub mod channel_resolution_error;
pub mod channel_value;
pub mod color;
pub mod compiled_data_context;
pub mod coord_measurement;
pub mod coordination_values;
pub mod data_context;
pub mod datafusion_utils;
pub mod error;
pub mod evaluation_context;
pub mod expr_eval;
pub mod facet_axis;
pub mod facet_strategy;
pub mod geometry;
pub mod guide_update;
pub mod into_expr;
pub mod layout_types;
pub mod legend;
pub mod legend_spec;
pub mod mark_channel_coercion;
pub mod mark_macros;
pub mod mark_render_context;
pub mod mark_state;
pub mod maybe;
pub mod overflow;
pub mod param;
pub mod position_config;
pub mod radius_expression;
pub mod resolved_domain;
pub mod scale_config_spec;
pub mod scale_domain;
pub mod scale_range;
pub mod scale_range_binding;
pub mod scale_sharing;
pub mod scale_spec;
pub mod scale_type;
pub mod serialization;
pub mod theme;
pub mod theme_context;

#[doc(hidden)]
pub mod __private {
    pub use paste;
}

pub use axis::Axis;
pub use axis_position::AxisPosition;
pub use channel::{BaseChannelName, ChannelDefault, ChannelDescriptor, strip_trailing_numbers};
pub use channel_config::ChannelConfig;
pub use channel_configs::{
    AngleChannelConfig, ColorChannelConfig, OpacityChannelConfig, ShapeChannelConfig,
    SizeChannelConfig, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};
pub use channel_resolution_error::{ChannelResolutionError, suggest_similar_channel_name};
pub use channel_value::{ChannelValue, ConditionalValue};
pub use compiled_data_context::CompiledDataContext;
pub use coord_measurement::{CoordMeasurement, EmptyCoordMeasurement};
pub use coordination_values::{CoordinatedLayout, CoordinatedOverflow};
pub use data_context::DataContext;
pub use datafusion_utils::{
    ArrayRefHelpers, DataFrameChartHelpers, ExprHelpers, ScalarValueHelpers, contains_aggregate,
    eval_to_scalars, params_to_datafusion, partition_expressions, scalar_to_scalar_value,
    simplify_to_scalar_sync,
};
pub use error::AvengerChartError;
pub use evaluation_context::EvaluationContext;
pub use expr_eval::{
    evaluate_axis_position_expr, evaluate_bool_expr, evaluate_f32_expr, evaluate_f64_expr,
    evaluate_i32_expr, evaluate_legend_orientation_expr, evaluate_legend_position_expr,
    evaluate_string_expr, evaluate_usize_expr,
};
pub use facet_axis::FacetAxis;
pub use facet_strategy::FacetStrategy;
pub use geometry::{
    BandPosition, PaddingSpec, PlotGeometry, PointGeometry, SubplotGeometry, SubplotRect,
};
pub use guide_update::GuideUpdate;
pub use into_expr::IntoExpr;
pub use layout_types::{
    EdgeSlabs, FrameAllocation, FrameDemand, FrameDimensionSizing, FrameLayout, FrameSizingPolicy,
    LayoutBounds, OverflowSide, OwnedEdgeSlabs, Size2D,
};
pub use legend::{LegendOrientation, LegendPosition, LegendRendererKind};
pub use legend_spec::Legend;
pub use mark_channel_coercion::{
    coerce_bool_channel, coerce_channel, coerce_color_channel, coerce_numeric_channel,
    coerce_opacity_channel, coerce_stroke_cap_channel, coerce_stroke_dash_channel,
    coerce_stroke_join_channel, coerce_text_channel,
};
pub use mark_render_context::MarkRenderContext;
pub use mark_state::{CompiledMarkState, MarkState};
pub use maybe::{Maybe, MaybeOptionalExpr};
pub use overflow::{MeasurementResult, OverflowSpaceRequirement};
pub use param::Param;
pub use position_config::PositionConfig;
pub use radius_expression::RadiusExpression;
pub use resolved_domain::ResolvedDomain;
pub use scale_config_spec::ScaleConfigSpec;
pub use scale_domain::{DomainExpr, ScaleDefaultDomain, ScaleDomain};
pub use scale_range::ScaleRange;
pub use scale_range_binding::{
    PlotAreaDimension, PlotAreaRangeEndpoint, PlotAreaRangeExpr, ScaleRangeBinding,
};
pub use scale_sharing::ScaleSharing;
pub use scale_spec::{
    Auto, Band, Linear, Log, Ordinal, Point, Pow, Quantile, Quantize, ScaleSpec, Sqrt, Symlog,
    Threshold, Time, scale_spec_for_preference,
};
pub use scale_type::{ScaleTypePreference, default_scale_type_for_data_type, is_continuous_scale};
pub use serialization::{
    LogicalPlanNodeExt, SerializableDataFrame, SerializableDataType, SerializableExpr,
    SerializableNestedScalarMap, SerializableScalar, SerializableScalarMap,
};
pub use theme::{AngleUnit, CssRgba, LengthUnit, Theme, ThemeValue};
pub use theme_context::ThemeContext;
