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
pub use crate::concat::{ConcatGuide, GridConcat, HConcat, TrackSizing, VConcat, WrapConcat};
pub use crate::event::{
    ChartEventBinding, ChartEventEvaluationMode, ChartEventStream, ChartEventType,
};
pub use crate::facet::coord::{FacetColumn, FacetRow, FacetWrap};
pub use crate::repeat;
pub use crate::repeat::{RepeatColumns, RepeatGrid, RepeatRows, RepeatWrap};
pub use avenger_chart_cartesian::{Cartesian, CartesianUnitAspect};
pub use avenger_chart_core::time;
pub use avenger_chart_core::{
    AxisGuideVisibilityConfig, AxisGuideVisibilityPolicy, CoordinationScope, CursorStyle,
    EmptySelectionBehavior, FacetEmptyCellPolicy, FormattingContext, IntoPlotMark, MarkGroup,
    MaterializationIdentity, MaterializationKey, MaterializationKind, MaterializationOutputKind,
    MaterializationPolicy, MaterializationRequest, PlotMark, Selection, SelectionClauseUpdate,
    SelectionCombine, SelectionSceneQuery, SelectionUpdate, Store, StoreData, StoreFieldPatch,
    StoreFieldRef, StoreKey, StoreRow, StoreUpdate, TimeContext, View, ViewAsyncPolicy, ViewRef,
    ViewStalePolicy, WeekStart, ZeroDCoord,
};
pub use avenger_chart_core::{
    NestScope, NestedBandLevelSpec, NestedBandSpec, PositionBoundary, nested,
};
pub use avenger_chart_core::{
    RepeatContext, RepeatDomainCoordination, RepeatPlaceholderKind, RepeatTypeHint, RepeatVariable,
    ResolvedRepeatVariable,
};
pub use avenger_chart_core::{
    SceneGeometryHitPolicy, SceneGeometryQuery, SceneQueryClauseId, SceneQueryDatumField,
};
#[cfg(feature = "parallel")]
pub use avenger_chart_parallel::{
    PARALLEL_LOCAL_X_CHANNEL, PARALLEL_LOCAL_Y_CHANNEL, Parallel, ParallelAxis,
    ParallelAxisOverlay, ParallelDimensionConfig, ParallelDimensionSpec, ParallelDisplayState,
    ParallelGuide, ParallelLine, ParallelOrderState, ParallelSymbol, ParallelTransform,
};
pub use avenger_chart_polar::Polar;

// Re-export the Plot type
pub use crate::layout::{
    CanvasConstraint, ChartResizeAxisPolicy, ChartResizePolicy, PlotConstraint,
};
pub use crate::plot::{
    EvaluationRequest, Plot, PlotSession, PlotSessionOptions, PlotSubtitle, PlotTitle, TitleAlign,
    TitleSpan,
};
pub use avenger_chart_core::{
    ChartTool, ToolExpansion, ToolExpansionContext, ToolMetadata, ToolParamExpansion,
    ToolParamSharing, ToolScaleEdit,
};
pub use avenger_chart_tools::{
    BoxSelection, BoxSelectionResolve, BoxZoom, LassoSelection, PanScrollZoom, PointSelection,
    UnitAspectBox,
};
pub use avenger_chart_transforms::lump;
pub use avenger_chart_transforms::{
    Aggregate, AggregateOutput, Bin, BinOutput, Calculate, CompiledKdeTransform, Filter, Fold,
    FoldOutput, Impute, ImputeOutput, JoinAggregate, Kde, KdeOutput, KdeResolve, Lump, LumpOutput,
    Rasterize2D, Rasterize2DOutput, ScalarAggregate, ScalarAggregateEvaluation,
    ScalarAggregateOutput, Select, Stack, StackOffset, StackOutput, TimeFill,
    TimeFillOutput, TimeLevel, TimeLevelConfig, TimeLevelKey, TimeLevelKeys, TimeLevelLabel,
    TimeLevels, TimeLevelsOutput, TimeUnit, TimeUnitOutput, TimeUnitPart, Window,
};
pub use avenger_text::types::TextSyntaxMode;

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
    CartesianUniformRaster2DChannels,
};
pub use avenger_chart_marks::{
    Area, Image, IntoDerivedPrimitiveMark, Line, PathMark, RasterChannelsConfig, RasterDim,
    RasterPositionConfig, RasterPositionSpec, Rect, Rule, Subplot, Symbol, Text, Trail,
    UniformRaster2D, dim,
};
pub use avenger_chart_marks_statistical::{
    BoxPlot, BoxPlotOrientation, Violin, ViolinOrientation, ViolinWidthNormalization,
};
pub use avenger_chart_polar::{
    PolarLinePositionChannels, PolarSymbolPositionChannels, PolarTextPositionChannels,
};

// Re-export mark traits and types
pub use avenger_chart_core::{
    AdjustItem, AdjustmentTransformContext, AdjustmentTransformRequirements, AreaGeometryItem,
    BasePlotAreaScene, BboxExpr, CompiledMarkAdjustmentTransform, Dodge, DodgeOutput,
    GeometryBounds, ImageGeometryItem, Jitter, JitterOutput, MarkAdjustmentCompileContext,
    MarkAdjustmentTransform, MarkEvaluationFrame, Nudge, NudgeOutput, PlotAreaInfo,
    PointGeometryItem, RectGeometryItem, RuleGeometryItem, TextMeasurementService,
};
pub use avenger_chart_core::{
    ChannelExpr, ChannelValue, ConditionalValue, DataTransform, DataTransformCompileContext,
    FacetDataScope, GeometrySpace, Mark, MarkState, PatternChannelValue, RadiusExpression,
    derived_scalar,
};
pub use avenger_scenegraph::marks::pattern::{
    PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternLayerOperation, PatternSymbol,
    StripeDash, StripePatternLayer, SymbolLattice2d, SymbolPaint, SymbolPatternLayer,
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
    Band, BandScaleExt, Linear, LinearScaleExt, Log, LogScaleExt, NestedBand, Ordinal,
    OrdinalScaleExt, Point, PointScaleExt, Pow, PowScaleExt, Quantile, Quantize, ScaleRuntimeExt,
    Sqrt, SqrtScaleExt, Symlog, SymlogScaleExt, Threshold, Time, TimeScaleExt,
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
#[cfg(feature = "wgpu")]
pub use crate::render::CanvasExt;
#[cfg(feature = "pdf")]
pub use crate::render::PdfRenderer;
#[cfg(feature = "svg")]
pub use crate::render::SvgRenderer;
pub use crate::render::{
    CoordinationCheckpoint, EvaluationMode, EvaluationOptions, FacetLayoutRefinement,
    FacetSubtreeCheckpoint, FacetSubtreeSelector, FacetSubtreeSnapshot, LayoutDebugOverlayMode,
    LayoutSnapshot, RefinementCheckpoint, WholeChartSnapshot,
};
#[cfg(feature = "image-resources")]
pub use crate::render::{ImageResourceCache, ImageResourceLoadOptions, ImageResourceResolver};

// Re-export error type
pub use avenger_chart_core::AvengerChartError;

// Re-export parameter type
pub use avenger_chart_core::param::Param;

// Re-export DataFusion types for data manipulation
pub use datafusion::{
    dataframe::DataFrame,
    prelude::{Expr, col, lit},
};
