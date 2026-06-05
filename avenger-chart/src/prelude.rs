//! Prelude for avenger-chart
//!
//! This module provides a convenient way to import the most commonly used types and traits
//! when working with avenger-chart.
//!
//! # Example
//! ```rust,ignore
//! use avenger_chart::prelude::*;
//!
//! let plot = Plot::<Cartesian>::new()
//!     .mark(Symbol::new()
//!         .x(col("x"))
//!         .y(col("y"))
//!         .fill_with(col("category"), |c| c
//!             .scale(|s| s.scheme("category10"))
//!             .legend(|l| l.title("Category"))
//!         )
//!     );
//! ```

// Re-export coordinate systems
pub use crate::concat::{ConcatGuide, HConcat, VConcat};
pub use crate::event::{
    ChartEventBinding, ChartEventEvaluationMode, ChartEventStream, ChartEventType,
};
pub use crate::facet::coord::{FacetColumn, FacetRow, FacetWrap};
pub use avenger_chart_cartesian::Cartesian;
pub use avenger_chart_core::{
    CursorStyle, EmptySelectionBehavior, FacetEmptyCellPolicy, Selection, SelectionClauseUpdate,
    SelectionCombine, SelectionSceneQuery, SelectionUpdate, Sharing, Store, StoreData,
    StoreFieldPatch, StoreFieldRef, StoreKey, StoreRow, StoreUpdate, ZeroDCoord,
};
pub use avenger_chart_core::{
    SceneGeometryHitPolicy, SceneGeometryQuery, SceneQueryClauseId, SceneQueryDatumField,
};
pub use avenger_chart_polar::Polar;

// Re-export the Plot type
pub use crate::layout::{
    CanvasConstraint, ChartResizeAxisPolicy, ChartResizePolicy, PlotConstraint,
};
pub use crate::plot::{
    EvaluationRequest, Plot, PlotSession, PlotSubtitle, PlotTitle, TitleAlign, TitleSpan,
};
pub use avenger_chart_core::{
    ChartTool, ToolExpansion, ToolExpansionContext, ToolMetadata, ToolParamExpansion,
    ToolParamSharing, ToolScaleEdit,
};
pub use avenger_chart_tools::{BoxZoom, LassoSelection, PanScrollZoom, PointSelection};
pub use avenger_chart_transforms::lump;
pub use avenger_chart_transforms::{
    Aggregate, AggregateOutput, Bin, BinOutput, Calculate, Filter, Lump, LumpOutput, Select, Stack,
    StackOffset, StackOutput,
};

// Re-export theme types
pub use avenger_chart_core::Theme;

// Re-export marks
pub use crate::facet::marks::{
    FacetColumnSubplotChannels, FacetRowSubplotChannels, FacetWrapSubplotChannels,
};
pub use avenger_chart_cartesian::{
    CartesianAreaPositionChannels, CartesianImagePositionChannels, CartesianLinePositionChannels,
    CartesianPathPositionChannels, CartesianRectPositionChannels, CartesianRulePositionChannels,
    CartesianSubplotPositionChannels, CartesianSymbolPositionChannels,
    CartesianTextPositionChannels, CartesianTrailPositionChannels,
};
pub use avenger_chart_marks::{
    Area, Image, Line, PathMark, Rect, Rule, Subplot, Symbol, Text, Trail,
};
pub use avenger_chart_polar::PolarSymbolPositionChannels;

// Re-export mark traits and types
pub use avenger_chart_core::{
    ChannelValue, ConditionalValue, DataTransform, DataTransformCompileContext, FacetDataScope,
    Mark, MarkState, RadiusExpression, derived_scalar,
};

// Re-export channel config traits - ESSENTIAL for using channel methods
pub use avenger_chart_core::ChannelConfig;
pub use avenger_chart_core::{Scale, ScaleChannelConfig, ScaleChannelValue};
pub use avenger_chart_legend::{LegendableChannel, LegendableChannelValue};

// Re-export channel configs for direct use
pub use avenger_chart_core::{
    AngleChannelConfig, ColorChannelConfig, OpacityChannelConfig, PositionConfig,
    ShapeChannelConfig, SizeChannelConfig, StrokeDashChannelConfig, StrokeWidthChannelConfig,
};

// Re-export position channel configs
pub use avenger_chart_cartesian::CartesianPositionConfig;

// Re-export scale types
pub use avenger_chart_core::{Auto, ScaleConfigSpec, ScaleOrderingSpec};
pub use avenger_chart_scales::{
    Band, BandScaleExt, Linear, LinearScaleExt, Log, LogScaleExt, Ordinal, OrdinalScaleExt, Point,
    PointScaleExt, Pow, PowScaleExt, Quantile, Quantize, ScaleRuntimeExt, Sqrt, SqrtScaleExt,
    Symlog, SymlogScaleExt, Threshold, Time, TimeScaleExt,
};

// Re-export legend types
pub use crate::legend::ColorbarOverlay;
pub use avenger_chart_core::Legend;
pub use avenger_chart_core::{LegendOrientation, LegendPosition};
pub use avenger_chart_legend::{
    AngleLegendBuilder, ColorLegendBuilder, LegendBuilder, OpacityLegendBuilder,
    ShapeLegendBuilder, SizeLegendBuilder, StrokeDashLegendBuilder, StrokeWidthLegendBuilder,
};

// Re-export axis types
pub use avenger_chart_cartesian::CartesianAxis;
pub use avenger_chart_core::AxisPosition;
pub use avenger_chart_polar::PolarAxis;

// Re-export rendering types
pub use crate::render::CanvasExt;
pub use crate::render::{
    CoordinationCheckpoint, EvaluationMode, EvaluationOptions, FacetLayoutRefinement,
    FacetSubtreeCheckpoint, FacetSubtreeSelector, FacetSubtreeSnapshot, LayoutDebugOverlayMode,
    LayoutSnapshot, RefinementCheckpoint, WholeChartSnapshot,
};

// Re-export error type
pub use avenger_chart_core::AvengerChartError;

// Re-export parameter type
pub use avenger_chart_core::param::Param;

// Re-export DataFusion types for data manipulation
pub use datafusion::{
    dataframe::DataFrame,
    prelude::{Expr, col, lit},
};
