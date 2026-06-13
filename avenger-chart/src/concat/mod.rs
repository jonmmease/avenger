//! Concatenation coordinate systems.
//!
//! `HConcat`, `VConcat`, and `GridConcat` are container coordinate systems:
//! their marks are child `Subplot` marks, and their coordinate measurement
//! produces child-frame placement metadata for later rendering/debug consumers.

mod subplot;

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    container::{
        BoundaryDemand, ChildFrameKey, ChildFrameScopeKey, ChildFrameSharingLevel,
        ContainerPathSegment, PlacedRegion, PlacementSolution,
    },
    coords::{
        CoordMeasurement, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
        CoordinateSystemTransformCore, PlotGeometry, PointGeometry,
    },
    error::AvengerChartError,
    facet::evaluated_facet_tree::EvaluatedFacetTree,
    guide::{
        CompiledGuide, CoordinateGuide, GuideSharingContext, GuideUpdate, OverflowSpaceRequirement,
    },
    layout::{
        ChartRegionMeta, EdgeDemand, Edges, GridShape, GridSlot, LayoutBounds, Orientation, Size,
        TrackSpacing, layout_edges,
    },
    marks::{CompiledMark, CompiledMarkCore},
    plot::compiled::{
        ChildFrameDataSelection, ChildFrameDomainSharingInput, ChildFrameLayoutSlot,
        ChildFrameRuntime, CompiledPlot, ComponentsMeasurement, ContainerLabelPlacement,
        PreparedChildFramePlot, child_frame_container_view_from_concat,
        container_path_without_facet_segments, coordinated_child_frame_domain_extents,
        measure_child_frame_container_guide_overflow, render_child_frame_container_guide_labels,
    },
    render::EvaluationContext,
    scales::{DomainExtent, ScaleRangeBinding},
    theme::Theme,
};
use avenger_chart_core::{
    AxisGuideVisibilityConfig, AxisGuideVisibilityPolicy, CoordinationAxis,
    DefaultLogicalExprNodeExt, ExprHelpers, FacetWrapColumnMode, IntoExpr, SharingLevel,
    contains_aggregate, params_to_datafusion,
};
use datafusion_proto::protobuf::LogicalExprNode;

use subplot::GridPlacementConfig;

pub use subplot::{CompiledConcatSubplot, compiled_subplot};

/// Per-track sizing for concat rows/columns, CSS-grid style.
///
/// A track spans the subplots' **plot areas**; axis and legend chrome rides
/// the gaps between tracks, so `Px(200.0)` pins a 200px plot area, not a
/// 200px column including chrome.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TrackSizing {
    /// Content-sized (the default for every track).
    Auto,
    /// Rigid plot-area pixels: never stretched, never grown by content
    /// (oversized subplots overflow the track honestly).
    Px(f32),
    /// A weighted share of the leftover space after `Px` and content are
    /// paid (CSS `minmax(auto, fr)`).
    Flex(f32),
}

impl TrackSizing {
    fn to_track_size(self) -> avenger_layout::TrackSize {
        match self {
            TrackSizing::Auto => avenger_layout::TrackSize::Auto,
            TrackSizing::Px(pixels) => avenger_layout::TrackSize::Fixed(pixels),
            TrackSizing::Flex(weight) => avenger_layout::TrackSize::Flex(weight),
        }
    }
}

fn layout_track_sizes(sizes: Option<&[TrackSizing]>) -> Option<Vec<avenger_layout::TrackSize>> {
    sizes.map(|sizes| sizes.iter().map(|size| size.to_track_size()).collect())
}

/// Validate a declared sizing vector against the resolved track count.
fn validate_track_sizing(
    sizes: Option<&[TrackSizing]>,
    track_count: usize,
    what: &str,
) -> Result<(), AvengerChartError> {
    let Some(sizes) = sizes else {
        return Ok(());
    };
    if sizes.len() != track_count {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{what} declares {} track sizes but resolves to {track_count} tracks",
            sizes.len()
        )));
    }
    for size in sizes {
        match size {
            TrackSizing::Px(pixels) if *pixels < 0.0 => {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "{what} has a negative Px track size: {pixels}"
                )));
            }
            TrackSizing::Flex(weight) if *weight <= 0.0 => {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "{what} has a non-positive Flex weight: {weight}"
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Per-track measurement budgets: the initial operating point children are
/// measured at. `Px` tracks take their declared size; `Auto` and `Flex`
/// split the remaining budget by weight (`Auto` counts as weight 1).
fn seeded_track_budgets(
    sizes: Option<&[TrackSizing]>,
    track_count: usize,
    budget: f32,
) -> Vec<f32> {
    let track_count = track_count.max(1);
    let Some(sizes) = sizes else {
        return vec![budget / track_count as f32; track_count];
    };
    let fixed_total: f32 = sizes
        .iter()
        .map(|size| match size {
            TrackSizing::Px(pixels) => pixels.max(0.0),
            _ => 0.0,
        })
        .sum();
    let weight = |size: &TrackSizing| match size {
        TrackSizing::Auto => 1.0,
        TrackSizing::Flex(weight) => weight.max(0.0),
        TrackSizing::Px(_) => 0.0,
    };
    let total_weight: f32 = sizes.iter().map(weight).sum();
    let leftover = (budget - fixed_total).max(0.0);
    (0..track_count)
        .map(|index| match sizes.get(index) {
            Some(TrackSizing::Px(pixels)) => pixels.max(0.0),
            Some(size) if total_weight > 0.0 => leftover * weight(size) / total_weight,
            _ => 0.0,
        })
        .collect()
}

/// Horizontal concatenation of `Subplot` marks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HConcat {
    #[serde(default)]
    spacing: Option<f32>,
    #[serde(default)]
    widths: Option<Vec<TrackSizing>>,
}

impl HConcat {
    pub fn new() -> Self {
        Self::default()
    }

    /// Minimum gap in pixels between adjacent children. Children may still
    /// sit further apart when their rendered edge chrome demands it.
    pub fn spacing(mut self, px: f32) -> Self {
        self.spacing = Some(px);
        self
    }

    pub(crate) fn spacing_px(&self) -> f32 {
        self.spacing.unwrap_or(0.0)
    }

    /// Per-column plot-area sizing, one entry per child. See [`TrackSizing`].
    pub fn widths(mut self, widths: impl IntoIterator<Item = TrackSizing>) -> Self {
        self.widths = Some(widths.into_iter().collect());
        self
    }

    pub(crate) fn widths_config(&self) -> Option<&[TrackSizing]> {
        self.widths.as_deref()
    }
}

/// Vertical concatenation of `Subplot` marks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VConcat {
    #[serde(default)]
    spacing: Option<f32>,
    #[serde(default)]
    heights: Option<Vec<TrackSizing>>,
}

impl VConcat {
    pub fn new() -> Self {
        Self::default()
    }

    /// Minimum gap in pixels between adjacent children. Children may still
    /// sit further apart when their rendered edge chrome demands it.
    pub fn spacing(mut self, px: f32) -> Self {
        self.spacing = Some(px);
        self
    }

    pub(crate) fn spacing_px(&self) -> f32 {
        self.spacing.unwrap_or(0.0)
    }

    /// Per-row plot-area sizing, one entry per child. See [`TrackSizing`].
    pub fn heights(mut self, heights: impl IntoIterator<Item = TrackSizing>) -> Self {
        self.heights = Some(heights.into_iter().collect());
        self
    }

    pub(crate) fn heights_config(&self) -> Option<&[TrackSizing]> {
        self.heights.as_deref()
    }
}

/// Explicit two-dimensional concatenation of `Subplot` marks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GridConcat {
    rows: Option<usize>,
    columns: Option<usize>,
    #[serde(default)]
    spacing: Option<f32>,
    #[serde(default)]
    column_widths: Option<Vec<TrackSizing>>,
    #[serde(default)]
    row_heights: Option<Vec<TrackSizing>>,
    #[serde(default)]
    axis_guide_visibility: AxisGuideVisibilityConfig,
}

/// Row-major wrapped concatenation of `Subplot` marks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapConcat {
    column_mode: FacetWrapColumnMode,
    #[serde(default)]
    spacing: Option<f32>,
    #[serde(default)]
    axis_guide_visibility: AxisGuideVisibilityConfig,
}

impl Default for WrapConcat {
    fn default() -> Self {
        Self {
            column_mode: FacetWrapColumnMode::Auto,
            spacing: None,
            axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
        }
    }
}

impl WrapConcat {
    pub fn new() -> Self {
        Self::default()
    }

    /// Minimum gap in pixels between adjacent tracks. Children may still
    /// sit further apart when their rendered edge chrome demands it.
    pub fn spacing(mut self, px: f32) -> Self {
        self.spacing = Some(px);
        self
    }

    pub(crate) fn spacing_px(&self) -> f32 {
        self.spacing.unwrap_or(0.0)
    }

    pub fn columns(mut self, expr: impl IntoExpr) -> Self {
        self.column_mode = FacetWrapColumnMode::Fixed(
            LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("Failed to serialize wrap concat columns expression"),
        );
        self
    }

    pub fn responsive_columns(mut self, width: impl IntoExpr) -> Self {
        self.column_mode = FacetWrapColumnMode::ResponsiveWidth(
            LogicalExprNode::from_default_expr(width.into_expr())
                .expect("Failed to serialize wrap concat responsive column width"),
        );
        self
    }

    pub(crate) fn with_column_mode(mut self, column_mode: FacetWrapColumnMode) -> Self {
        self.column_mode = column_mode;
        self
    }

    pub fn axis_guide_visibility(mut self, policy: AxisGuideVisibilityPolicy) -> Self {
        self.axis_guide_visibility = AxisGuideVisibilityConfig::same(policy);
        self
    }

    pub(crate) fn column_mode(&self) -> &FacetWrapColumnMode {
        &self.column_mode
    }

    pub(crate) fn axis_guide_visibility_config(&self) -> AxisGuideVisibilityConfig {
        self.axis_guide_visibility
    }
}

impl GridConcat {
    pub fn new() -> Self {
        Self::default()
    }

    /// Minimum gap in pixels between adjacent tracks. Children may still
    /// sit further apart when their rendered edge chrome demands it.
    pub fn spacing(mut self, px: f32) -> Self {
        self.spacing = Some(px);
        self
    }

    pub(crate) fn spacing_px(&self) -> f32 {
        self.spacing.unwrap_or(0.0)
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = Some(rows);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = Some(columns);
        self
    }

    /// Per-column plot-area sizing, one entry per column. See
    /// [`TrackSizing`].
    pub fn column_widths(mut self, widths: impl IntoIterator<Item = TrackSizing>) -> Self {
        self.column_widths = Some(widths.into_iter().collect());
        self
    }

    /// Per-row plot-area sizing, one entry per row. See [`TrackSizing`].
    pub fn row_heights(mut self, heights: impl IntoIterator<Item = TrackSizing>) -> Self {
        self.row_heights = Some(heights.into_iter().collect());
        self
    }

    pub(crate) fn column_widths_config(&self) -> Option<&[TrackSizing]> {
        self.column_widths.as_deref()
    }

    pub(crate) fn row_heights_config(&self) -> Option<&[TrackSizing]> {
        self.row_heights.as_deref()
    }

    pub fn axis_guide_visibility(mut self, policy: AxisGuideVisibilityPolicy) -> Self {
        self.axis_guide_visibility = AxisGuideVisibilityConfig::same(policy);
        self
    }

    pub(crate) fn with_axis_guide_visibility_config(
        mut self,
        config: AxisGuideVisibilityConfig,
    ) -> Self {
        self.axis_guide_visibility = config;
        self
    }

    pub(crate) fn rows_config(&self) -> Option<usize> {
        self.rows
    }

    pub(crate) fn columns_config(&self) -> Option<usize> {
        self.columns
    }

    pub(crate) fn axis_guide_visibility_config(&self) -> AxisGuideVisibilityConfig {
        self.axis_guide_visibility
    }
}

impl CoordinateSystemCore for HConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for HConcat {
    type Guide = ConcatGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemCore for VConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for VConcat {
    type Guide = ConcatGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemCore for GridConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for GridConcat {
    type Guide = ConcatGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemCore for WrapConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for WrapConcat {
    type Guide = ConcatGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

/// Guide for concat containers.
///
/// The guide reserves parent-frame space for measured child frames and renders
/// optional child labels supplied by `Subplot::label`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConcatGuide;

impl GuideUpdate for ConcatGuide {
    fn update(self, _other: Self) -> Self {
        self
    }
}

impl CoordinateGuide for ConcatGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks<M>(
        &mut self,
        _compiled_marks: &[Arc<M>],
        _session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized,
    {
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for ConcatGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let Some(concat) = coord_measurement.and_then(concat_coord_ref) else {
            return Ok(OverflowSpaceRequirement::default());
        };

        let container = child_frame_container_view_from_concat(concat)?;
        measure_child_frame_container_guide_overflow(
            plot_width,
            plot_height,
            &container,
            concat_label_placement(concat),
            theme,
            params,
        )
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let concat = concat_coord_ref(coord_measurement).ok_or_else(|| {
            AvengerChartError::InternalError(
                "ConcatGuide received non-concat coordinate measurement".to_string(),
            )
        })?;
        let container = child_frame_container_view_from_concat(concat)?;
        render_child_frame_container_guide_labels(
            &container,
            concat_label_placement(concat),
            plot_bounds,
            theme,
            params,
        )
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CoordinateSystemTransformCore for HConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        container_point_geometry(position_channels, position_values, plot_width, plot_height)
    }

    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for HConcat {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for VConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        container_point_geometry(position_channels, position_values, plot_width, plot_height)
    }

    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for VConcat {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for GridConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        container_point_geometry(position_channels, position_values, plot_width, plot_height)
    }

    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for GridConcat {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for WrapConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        container_point_geometry(position_channels, position_values, plot_width, plot_height)
    }

    fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for WrapConcat {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

use crate::layout::concat_grid::{
    ChartGridData, GridCell, GridMemberSpec, SolvedConcatGrid, solve_concat_grid,
};

#[derive(Clone, Debug)]
pub(crate) enum ConcatChildPlacement {
    Band {
        placement: PlacementSolution,
        orientation: Orientation,
    },
    Grid {
        placement: PlacementSolution,
        shape: GridShape,
        retarget_plot_area_size: bool,
    },
}

impl ConcatChildPlacement {
    fn child_frame_placement(&self) -> PlacementSolution {
        match self {
            Self::Band { placement, .. } => placement.clone(),
            Self::Grid { placement, .. } => placement.clone(),
        }
    }

    fn band_direction(&self) -> Option<Orientation> {
        match self {
            Self::Band { orientation, .. } => Some(*orientation),
            Self::Grid { .. } => None,
        }
    }

    fn grid_shape(&self) -> Option<GridShape> {
        match self {
            Self::Band { .. } => None,
            Self::Grid { shape, .. } => Some(*shape),
        }
    }
}

#[derive(Clone)]
pub struct ConcatCoordMeasurement {
    pub(crate) children: Vec<ConcatChildMeasurement>,
    pub(crate) placement: ConcatChildPlacement,
    pub(crate) fallback_content_size: Size,
    /// Configured minimum gap between adjacent children (0 when unset).
    pub(crate) min_gap: f32,
    /// Declared per-track sizing (None = all Auto).
    pub(crate) column_sizes: Option<Vec<avenger_layout::TrackSize>>,
    pub(crate) row_sizes: Option<Vec<avenger_layout::TrackSize>>,
}

impl ConcatCoordMeasurement {
    pub(crate) fn children(&self) -> &[ConcatChildMeasurement] {
        &self.children
    }

    pub(crate) fn child(&self, child_index: usize) -> Option<&ConcatChildMeasurement> {
        self.children
            .iter()
            .find(|child| child.child_index == child_index)
    }

    pub(crate) fn child_scope_key(&self, child_index: usize) -> Option<ChildFrameScopeKey> {
        self.child(child_index)
            .map(ConcatChildMeasurement::scope_key)
    }

    pub(crate) fn child_frame_placement(&self) -> PlacementSolution {
        self.placement.child_frame_placement()
    }

    pub(crate) fn retarget_plot_area_size(
        &mut self,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<(), AvengerChartError> {
        self.fallback_content_size = Size::new(plot_width, plot_height);
        let ConcatChildPlacement::Grid {
            shape,
            retarget_plot_area_size,
            ..
        } = self.placement
        else {
            return Ok(());
        };

        let base_child_content_size = Size::new(
            plot_width / shape.columns.max(1) as f32,
            plot_height / shape.rows.max(1) as f32,
        );
        let placement = grid_child_frame_placement(
            &self.children,
            shape,
            base_child_content_size,
            retarget_plot_area_size,
            self.min_gap,
            self.column_sizes.as_deref(),
            self.row_sizes.as_deref(),
        )?;
        self.placement = ConcatChildPlacement::Grid {
            placement,
            shape,
            retarget_plot_area_size,
        };
        Ok(())
    }

    pub(crate) fn band_direction(&self) -> Option<Orientation> {
        self.placement.band_direction()
    }

    pub(crate) fn grid_shape(&self) -> Option<GridShape> {
        self.placement.grid_shape()
    }

    pub(crate) fn layout_coordination_shape(&self) -> Option<GridShape> {
        match &self.placement {
            ConcatChildPlacement::Grid { shape, .. } => Some(*shape),
            ConcatChildPlacement::Band { orientation, .. } => match orientation {
                Orientation::Horizontal => Some(GridShape {
                    rows: 1,
                    columns: self.children.len().max(1),
                }),
                Orientation::Vertical => Some(GridShape {
                    rows: self.children.len().max(1),
                    columns: 1,
                }),
            },
        }
    }

    pub(crate) fn container_path(&self) -> Result<Vec<ContainerPathSegment>, AvengerChartError> {
        let Some(first) = self.children.first() else {
            return Ok(Vec::new());
        };
        for child in &self.children[1..] {
            if child.container_path != first.container_path {
                return Err(AvengerChartError::InternalError(
                    "Concat children reported different container paths".to_string(),
                ));
            }
        }
        Ok(first.container_path.clone())
    }

    pub(crate) fn grid_layout_slots(&self) -> Result<Vec<ChildFrameLayoutSlot>, AvengerChartError> {
        if self.layout_coordination_shape().is_none() {
            return Ok(Vec::new());
        }
        self.children
            .iter()
            .enumerate()
            .map(|child| {
                let (slot_index, child) = child;
                Ok(ChildFrameLayoutSlot {
                    child_index: child.child_index,
                    child_key: child.scope_key().child_key,
                    slot: self.layout_coordination_slot_for_child(slot_index, child)?,
                })
            })
            .collect()
    }

    /// Everything needed to rebuild this container's grid as one member of
    /// a coordination group solve.
    pub(crate) fn grid_member_spec(&self) -> Result<GridMemberSpec, AvengerChartError> {
        let shape = self.layout_coordination_shape().ok_or_else(|| {
            AvengerChartError::InternalError(
                "Grid requirements requested for non-child-frame concat measurement".to_string(),
            )
        })?;
        let base_cell_size = Size::new(
            self.fallback_content_size.width / shape.columns.max(1) as f32,
            self.fallback_content_size.height / shape.rows.max(1) as f32,
        );
        let spacing = TrackSpacing {
            min_gap: self.min_gap,
            ..Default::default()
        };
        Ok(GridMemberSpec {
            shape,
            base_cell_size,
            cells: self.layout_coordination_grid_items()?,
            column_spacing: spacing,
            row_spacing: spacing,
            column_sizes: self.column_sizes.clone(),
            row_sizes: self.row_sizes.clone(),
        })
    }

    pub(crate) fn grid_requirements(&self) -> Result<ChartGridData, AvengerChartError> {
        let spec = self.grid_member_spec()?;
        let exported = solve_concat_grid(
            spec.shape,
            spec.base_cell_size,
            &spec.cells,
            spec.column_spacing,
            spec.row_spacing,
            spec.column_sizes.as_deref(),
            spec.row_sizes.as_deref(),
            true,
        )
        .map_err(AvengerChartError::InvalidArgument)?;
        Ok(exported.data)
    }

    /// Install a solved grid produced by a coordination group solve for this
    /// container (this member's extracted view): pure placement write-back
    /// with change detection, no solving.
    pub(crate) fn install_grid_solution(
        &mut self,
        solution: &SolvedConcatGrid,
    ) -> Result<bool, AvengerChartError> {
        let shape = self.layout_coordination_shape().ok_or_else(|| {
            AvengerChartError::InternalError(
                "Grid solution applied to non-child-frame concat measurement".to_string(),
            )
        })?;
        if solution.data.shape != shape {
            return Err(AvengerChartError::InternalError(format!(
                "Grid solution shape {:?} did not match concat coordination shape {:?}",
                solution.data.shape, shape
            )));
        }

        let old_placement = self.child_frame_placement();
        let placement = match &self.placement {
            ConcatChildPlacement::Grid {
                retarget_plot_area_size,
                ..
            } => {
                let placements = self
                    .children
                    .iter()
                    .enumerate()
                    .map(|(slot_index, child)| {
                        let slot = self.layout_coordination_slot_for_child(slot_index, child)?;
                        let origin = solution.content_origin_for_slot(slot);
                        let edge_targets = solution.edge_targets_for_slot(slot);
                        let content_size_override =
                            retarget_plot_area_size.then(|| solution.content_size_for_slot(slot));
                        Ok(PlacedRegion::with_meta(
                            child.child_index,
                            origin,
                            ChartRegionMeta {
                                content_size_override,
                                edge_targets: Some(edge_targets),
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, AvengerChartError>>()?;
                ConcatChildPlacement::Grid {
                    placement: PlacementSolution::new(solution.content_size, placements),
                    shape,
                    retarget_plot_area_size: *retarget_plot_area_size,
                }
            }
            ConcatChildPlacement::Band { orientation, .. } => {
                let direction = *orientation;
                let placements = self
                    .children
                    .iter()
                    .enumerate()
                    .map(|(slot_index, child)| {
                        let slot = self.layout_coordination_slot_for_child(slot_index, child)?;
                        let origin = match direction {
                            Orientation::Horizontal => {
                                [solution.column_starts[slot.column], solution.row_starts[0]]
                            }
                            Orientation::Vertical => {
                                [solution.column_starts[0], solution.row_starts[slot.row]]
                            }
                        };
                        Ok(PlacedRegion::new(child.child_index, origin))
                    })
                    .collect::<Result<Vec<_>, AvengerChartError>>()?;
                ConcatChildPlacement::Band {
                    placement: PlacementSolution::new(solution.content_size, placements),
                    orientation: direction,
                }
            }
        };
        let placement_result = placement.child_frame_placement();
        let changed = placement_result != old_placement;
        if changed {
            self.placement = placement;
        }
        Ok(changed)
    }

    fn layout_coordination_slot_for_child(
        &self,
        slot_index: usize,
        child: &ConcatChildMeasurement,
    ) -> Result<GridSlot, AvengerChartError> {
        match self.band_direction() {
            Some(Orientation::Horizontal) => Ok(GridSlot {
                row: 0,
                column: slot_index,
                row_span: 1,
                column_span: 1,
            }),
            Some(Orientation::Vertical) => Ok(GridSlot {
                row: slot_index,
                column: 0,
                row_span: 1,
                column_span: 1,
            }),
            None => {
                let placement = child.grid_placement.ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing grid placement for child {} while exporting layout coordination slot",
                        child.child_index
                    ))
                })?;
                Ok(grid_slot_from_placement(placement))
            }
        }
    }

    fn layout_coordination_grid_items(&self) -> Result<Vec<GridCell>, AvengerChartError> {
        self.children
            .iter()
            .enumerate()
            .map(|(slot_index, child)| {
                let frame_demand = child.measurement.frame_demand();
                Ok(GridCell {
                    slot: self.layout_coordination_slot_for_child(slot_index, child)?,
                    content_size: Size::new(
                        child.measurement.plot_area_width,
                        child.measurement.plot_area_height,
                    ),
                    edges: layered_cell_edges(
                        layout_edges(frame_demand.guide_slabs),
                        layout_edges(frame_demand.legend_slabs),
                    ),
                })
            })
            .collect()
    }
}

impl CoordMeasurement for ConcatCoordMeasurement {
    fn clone_box(&self) -> Box<dyn CoordMeasurement> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Clone)]
pub(crate) struct ConcatChildMeasurement {
    pub(crate) child_index: usize,
    pub(crate) key: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) grid_placement: Option<GridPlacementConfig>,
    pub(crate) container_path: Vec<ContainerPathSegment>,
    pub(crate) local_facet_tree: Option<Arc<EvaluatedFacetTree>>,
    pub(crate) facet_data_root: Option<DataFrame>,
    pub(crate) data_override: Option<DataFrame>,
    pub(crate) sharing_levels: Vec<ChildFrameSharingLevel>,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) measurement: ComponentsMeasurement,
}

impl ConcatChildMeasurement {
    pub(crate) fn scope_key(&self) -> ChildFrameScopeKey {
        concat_child_scope_key(&self.container_path, self.child_index, self.key.as_deref())
    }

    pub(crate) fn debug_label(&self) -> String {
        match (&self.key, &self.label) {
            (Some(key), Some(label)) => {
                format!("{} (key={key:?}, label={label:?})", self.child_index)
            }
            (Some(key), None) => format!("{} (key={key:?})", self.child_index),
            (None, Some(label)) => format!("{} (label={label:?})", self.child_index),
            (None, None) => self.child_index.to_string(),
        }
    }
}

fn concat_child_scope_key(
    container_path: &[ContainerPathSegment],
    child_index: usize,
    key: Option<&str>,
) -> ChildFrameScopeKey {
    ChildFrameScopeKey::new(
        container_path.to_vec(),
        ChildFrameKey::ConcatChild {
            index: child_index,
            key: key.map(ToOwned::to_owned),
        },
    )
}

pub(crate) fn concat_coord_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&ConcatCoordMeasurement> {
    coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
}

fn concat_label_placement(concat: &ConcatCoordMeasurement) -> Option<ContainerLabelPlacement> {
    match concat.band_direction()? {
        Orientation::Horizontal => Some(ContainerLabelPlacement::Top),
        Orientation::Vertical => Some(ContainerLabelPlacement::Left),
    }
}

fn boundary_demand_for_child(
    direction: Orientation,
    measurement: &ComponentsMeasurement,
) -> BoundaryDemand {
    let slabs = measurement.frame_demand().rendered_envelope;
    match direction {
        Orientation::Horizontal => BoundaryDemand {
            before: slabs.left,
            after: slabs.right,
        },
        Orientation::Vertical => BoundaryDemand {
            before: slabs.top,
            after: slabs.bottom,
        },
    }
}

/// (child id, main size, cross size, sibling boundary) for one concat
/// child on the band axis.
fn band_input_for_child(
    direction: Orientation,
    child: &ConcatChildMeasurement,
) -> (usize, f32, f32, BoundaryDemand) {
    let main_size = match direction {
        Orientation::Horizontal => child.measurement.plot_area_width,
        Orientation::Vertical => child.measurement.plot_area_height,
    };
    let cross_size = match direction {
        Orientation::Horizontal => child.measurement.plot_area_height,
        Orientation::Vertical => child.measurement.plot_area_width,
    };
    (
        child.child_index,
        main_size,
        cross_size,
        boundary_demand_for_child(direction, &child.measurement),
    )
}

struct PreparedConcatChild<'a> {
    subplot: &'a CompiledConcatSubplot,
    child_plot: PreparedChildFramePlot<'a>,
    container_path: Vec<ContainerPathSegment>,
    relative_facet_child_frame_path: Vec<ContainerPathSegment>,
}

impl PreparedConcatChild<'_> {
    fn child_index(&self) -> usize {
        self.subplot.child_index()
    }

    fn key(&self) -> Option<&str> {
        self.subplot.key()
    }

    fn label(&self) -> Option<&str> {
        self.subplot.label()
    }

    fn grid_placement(&self) -> Option<GridPlacementConfig> {
        self.subplot.grid_placement()
    }

    fn scope_key(&self) -> ChildFrameScopeKey {
        concat_child_scope_key(&self.container_path, self.child_index(), self.key())
    }

    fn sharing_level(&self, direction: Orientation, child_count: usize) -> ChildFrameSharingLevel {
        match direction {
            Orientation::Horizontal => {
                ChildFrameSharingLevel::hconcat_child(self.child_index(), child_count, self.key())
            }
            Orientation::Vertical => {
                ChildFrameSharingLevel::vconcat_child(self.child_index(), child_count, self.key())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GridAxisGuideVisibilityConfig {
    vertical: AxisGuideVisibilityConfig,
    horizontal: AxisGuideVisibilityConfig,
}

impl GridAxisGuideVisibilityConfig {
    #[cfg(test)]
    fn same(config: AxisGuideVisibilityConfig) -> Self {
        Self {
            vertical: config,
            horizontal: config,
        }
    }

    fn for_axis(self, axis: CoordinationAxis) -> AxisGuideVisibilityConfig {
        match axis {
            CoordinationAxis::Vertical => self.vertical,
            CoordinationAxis::Horizontal => self.horizontal,
            CoordinationAxis::Positioned => AxisGuideVisibilityConfig::auto(),
        }
    }
}

struct GridSemanticChild<'a> {
    child_index: usize,
    placement: GridPlacementConfig,
    channel_domain_coordinations: &'a HashMap<String, avenger_chart_core::DomainCoordination>,
    scale_type_signatures: HashMap<String, String>,
    axis_config_signatures: HashMap<String, Vec<String>>,
}

impl GridSemanticChild<'_> {
    fn domain_coordination_for_channel(
        &self,
        channel: &str,
    ) -> Option<&avenger_chart_core::DomainCoordination> {
        self.channel_domain_coordinations.get(channel)
    }

    fn scale_type_signature_for_channel(&self, channel: &str) -> Option<&str> {
        self.scale_type_signatures.get(channel).map(String::as_str)
    }

    fn axis_config_signature_for_channel(&self, channel: &str) -> Option<&[String]> {
        self.axis_config_signatures.get(channel).map(Vec::as_slice)
    }
}

fn policy_for_equivalence(
    policy: AxisGuideVisibilityPolicy,
    equivalent: bool,
) -> AxisGuideVisibilityPolicy {
    match policy {
        AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups if equivalent => {
            AxisGuideVisibilityPolicy::OuterEdges
        }
        AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups => AxisGuideVisibilityPolicy::All,
        other => other,
    }
}

fn config_for_equivalence(
    config: AxisGuideVisibilityConfig,
    equivalent: bool,
) -> AxisGuideVisibilityConfig {
    AxisGuideVisibilityConfig::new(
        policy_for_equivalence(config.labels, equivalent),
        policy_for_equivalence(config.title, equivalent),
    )
}

fn semantic_axis_channel(axis: CoordinationAxis) -> Option<&'static str> {
    match axis {
        // Cartesian x axes are compacted along vertical row levels.
        CoordinationAxis::Vertical => Some("x"),
        // Cartesian y axes are compacted along horizontal column levels.
        CoordinationAxis::Horizontal => Some("y"),
        CoordinationAxis::Positioned => None,
    }
}

fn child_in_same_axis_strip(
    placement: GridPlacementConfig,
    reference: GridPlacementConfig,
    axis: CoordinationAxis,
) -> bool {
    match axis {
        CoordinationAxis::Vertical => ranges_overlap(
            placement.column,
            placement.column + placement.column_span,
            reference.column,
            reference.column + reference.column_span,
        ),
        CoordinationAxis::Horizontal => ranges_overlap(
            placement.row,
            placement.row + placement.row_span,
            reference.row,
            reference.row + reference.row_span,
        ),
        CoordinationAxis::Positioned => false,
    }
}

fn ranges_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start < b_end && b_start < a_end
}

fn strip_has_equivalent_domain_coordination(
    semantic_children: &[GridSemanticChild<'_>],
    child_index: usize,
    axis: CoordinationAxis,
) -> bool {
    let Some(channel) = semantic_axis_channel(axis) else {
        return false;
    };
    let Some(reference_child) = semantic_children
        .iter()
        .find(|child| child.child_index == child_index)
    else {
        return false;
    };
    let Some(reference) = reference_child.domain_coordination_for_channel(channel) else {
        return false;
    };
    if SharingLevel::from(reference.scope).is_free() {
        return false;
    }
    let reference_scale_type = reference_child.scale_type_signature_for_channel(channel);
    let reference_axis_config = reference_child.axis_config_signature_for_channel(channel);

    semantic_children
        .iter()
        .filter(|child| child_in_same_axis_strip(child.placement, reference_child.placement, axis))
        .all(|child| {
            child.domain_coordination_for_channel(channel) == Some(reference)
                && child.scale_type_signature_for_channel(channel) == reference_scale_type
                && child.axis_config_signature_for_channel(channel) == reference_axis_config
        })
}

fn semantic_axis_guide_visibility_for_child(
    base: AxisGuideVisibilityConfig,
    semantic_children: &[GridSemanticChild<'_>],
    child_index: usize,
) -> GridAxisGuideVisibilityConfig {
    let vertical_equivalent = strip_has_equivalent_domain_coordination(
        semantic_children,
        child_index,
        CoordinationAxis::Vertical,
    );
    let horizontal_equivalent = strip_has_equivalent_domain_coordination(
        semantic_children,
        child_index,
        CoordinationAxis::Horizontal,
    );

    GridAxisGuideVisibilityConfig {
        vertical: config_for_equivalence(base, vertical_equivalent),
        horizontal: config_for_equivalence(base, horizontal_equivalent),
    }
}

fn coordinated_domain_extents_for_concat_children(
    children: &[PreparedConcatChild<'_>],
) -> Vec<HashMap<String, DomainExtent>> {
    let scope_keys = children
        .iter()
        .map(PreparedConcatChild::scope_key)
        .collect::<Vec<_>>();
    let inputs = children
        .iter()
        .zip(scope_keys.iter())
        .map(|(child, scope_key)| {
            ChildFrameDomainSharingInput::new(
                scope_key,
                child.child_plot.local_domain_extents(),
                child.child_plot.channel_domain_sharing_levels(),
            )
        })
        .collect::<Vec<_>>();

    coordinated_child_frame_domain_extents(&inputs)
}

async fn prepare_concat_child<'a>(
    subplot: &'a CompiledConcatSubplot,
    eval_ctx: &EvaluationContext,
    inherited_data: Option<&DataFrame>,
) -> Result<PreparedConcatChild<'a>, AvengerChartError> {
    let child_plot = subplot.compiled_subplot();
    let data_selection = if subplot.inherits_parent_data() {
        ChildFrameDataSelection::InheritParent
    } else {
        ChildFrameDataSelection::ExplicitChild
    };
    let runtime = ChildFrameRuntime::new();
    let child_plot =
        Box::pin(runtime.prepare_plot(child_plot, data_selection, inherited_data, eval_ctx))
            .await?;
    let mut relative_facet_child_frame_path =
        container_path_without_facet_segments(eval_ctx.child_frame_container_path());
    relative_facet_child_frame_path.push(ContainerPathSegment::concat_child(
        subplot.child_index(),
        subplot.key(),
    ));

    Ok(PreparedConcatChild {
        subplot,
        child_plot,
        container_path: eval_ctx.child_frame_container_path().to_vec(),
        relative_facet_child_frame_path,
    })
}

async fn measure_prepared_concat_child(
    prepared: &PreparedConcatChild<'_>,
    sharing_levels: Vec<ChildFrameSharingLevel>,
    child_plot_area: Size,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    coordinated_domain_extents: &HashMap<String, DomainExtent>,
    facet_scoped_domain_extents: &HashMap<String, DomainExtent>,
) -> Result<ConcatChildMeasurement, AvengerChartError> {
    let runtime = ChildFrameRuntime::new();
    let child_layout_spec =
        runtime.fixed_plot_area_layout_spec(child_plot_area.width, child_plot_area.height);
    let mut sharing_level_iter = sharing_levels.iter().cloned();
    let first_level = sharing_level_iter.next().ok_or_else(|| {
        AvengerChartError::InternalError("Concat child measurement requires a sharing level".into())
    })?;
    let mut child_eval_ctx = runtime.eval_context(eval_ctx, first_level);
    for level in sharing_level_iter {
        child_eval_ctx = child_eval_ctx.with_child_frame_sharing_level_appended(level);
    }
    let measurement = Box::pin(prepared.child_plot.measure(
        &child_eval_ctx,
        &child_layout_spec,
        facet_path,
        &[coordinated_domain_extents, facet_scoped_domain_extents],
    ))
    .await?;

    Ok(ConcatChildMeasurement {
        child_index: prepared.child_index(),
        key: prepared.key().map(ToOwned::to_owned),
        label: prepared.label().map(ToOwned::to_owned),
        grid_placement: prepared.grid_placement(),
        container_path: prepared.container_path.clone(),
        local_facet_tree: prepared.child_plot.local_facet_tree(),
        facet_data_root: prepared.child_plot.facet_data_root(),
        data_override: prepared.child_plot.data_override().cloned(),
        sharing_levels,
        compiled_subplot: prepared.subplot.compiled_subplot_arc(),
        measurement,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn measure_concat_coord_system(
    direction: Orientation,
    spacing: f32,
    main_sizes: Option<&[TrackSizing]>,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    let subplots = compiled_marks
        .iter()
        .filter_map(|mark| compiled_subplot(mark.as_ref()))
        .collect::<Vec<_>>();
    validate_track_sizing(
        main_sizes,
        subplots.len(),
        match direction {
            Orientation::Horizontal => "hconcat widths",
            Orientation::Vertical => "vconcat heights",
        },
    )?;
    let main_budget = match direction {
        Orientation::Horizontal => plot_width,
        Orientation::Vertical => plot_height,
    };
    let track_budgets = seeded_track_budgets(main_sizes, subplots.len(), main_budget);

    let mut prepared_children = Vec::with_capacity(subplots.len());
    for subplot in subplots {
        prepared_children.push(Box::pin(prepare_concat_child(subplot, eval_ctx, data)).await?);
    }

    let coordinated_domain_extents =
        coordinated_domain_extents_for_concat_children(&prepared_children);

    let mut children = Vec::with_capacity(prepared_children.len());
    let child_count = prepared_children.len();
    for (index, (prepared, coordinated_extents)) in prepared_children
        .iter()
        .zip(coordinated_domain_extents.iter())
        .enumerate()
    {
        let track_budget = track_budgets.get(index).copied().unwrap_or(0.0);
        let child_plot_area = match direction {
            Orientation::Horizontal => Size::new(track_budget, plot_height),
            Orientation::Vertical => Size::new(plot_width, track_budget),
        };
        let facet_scoped_extents = eval_ctx
            .facet_scale_precompute_store()
            .coordinated_child_frame_domain_extents(
                &prepared.relative_facet_child_frame_path,
                facet_path,
            );
        children.push(
            Box::pin(measure_prepared_concat_child(
                prepared,
                vec![prepared.sharing_level(direction, child_count)],
                child_plot_area,
                eval_ctx,
                facet_path,
                coordinated_extents,
                &facet_scoped_extents,
            ))
            .await?,
        );
    }

    // One track per child: leaves at plot-area sizes, sibling boundaries
    // as inner-stratum edge demands, declared track sizes when given.
    let inputs = children
        .iter()
        .map(|child| band_input_for_child(direction, child))
        .collect::<Vec<_>>();
    let layout_sizes = layout_track_sizes(main_sizes);
    let vertical = matches!(direction, Orientation::Vertical);
    let (before_side, after_side) = if vertical {
        (avenger_layout::Side::Top, avenger_layout::Side::Bottom)
    } else {
        (avenger_layout::Side::Left, avenger_layout::Side::Right)
    };
    let leaves = inputs.iter().map(|(_, main, cross, boundary)| {
        let size = if vertical {
            Size::new(*cross, *main)
        } else {
            Size::new(*main, *cross)
        };
        avenger_layout::Layout::<usize>::leaf(size)
            .demand(
                before_side,
                avenger_layout::EdgeDemand {
                    guide: boundary.before,
                    legend: 0.0,
                },
            )
            .demand(
                after_side,
                avenger_layout::EdgeDemand {
                    guide: boundary.after,
                    legend: 0.0,
                },
            )
    });
    let band_spacing = TrackSpacing {
        min_gap: spacing,
        ..Default::default()
    };
    let mut stack = if vertical {
        avenger_layout::Layout::column(leaves).row_spacing(band_spacing)
    } else {
        avenger_layout::Layout::row(leaves).column_spacing(band_spacing)
    };
    if let Some(sizes) = layout_sizes.as_deref() {
        stack = if vertical {
            stack.rows(sizes.iter().copied())
        } else {
            stack.columns(sizes.iter().copied())
        };
    }
    let solved = stack
        .solve(&avenger_layout::SolveOptions::default())
        .expect("a band of leaves always solves");
    let root = solved.at_path(&[]).expect("root region exists");
    let avenger_layout::RegionDetail::Grid { tracks } = &root.detail else {
        unreachable!("a band root is a grid");
    };
    let (main_starts, cross_extent, main_extent) = if vertical {
        (
            &tracks.row_starts,
            tracks.column_sizes[0],
            root.content.height,
        )
    } else {
        (
            &tracks.column_starts,
            tracks.row_sizes[0],
            root.content.width,
        )
    };
    let placements = inputs
        .iter()
        .enumerate()
        .map(|(slot_index, (id, _, _, _))| {
            let origin = if vertical {
                [0.0, main_starts[slot_index]]
            } else {
                [main_starts[slot_index], 0.0]
            };
            PlacedRegion::new(*id, origin)
        })
        .collect();
    let content_size = if vertical {
        Size::new(cross_extent, main_extent)
    } else {
        Size::new(main_extent, cross_extent)
    };
    let band_placement = PlacementSolution::new(content_size, placements);

    let (column_sizes, row_sizes) = match direction {
        Orientation::Horizontal => (layout_sizes, None),
        Orientation::Vertical => (None, layout_sizes),
    };
    Ok(Box::new(ConcatCoordMeasurement {
        children,
        placement: ConcatChildPlacement::Band {
            placement: band_placement,
            orientation: direction,
        },
        fallback_content_size: Size::new(plot_width, plot_height),
        min_gap: spacing,
        column_sizes,
        row_sizes,
    }))
}

pub(crate) async fn measure_grid_concat_coord_system(
    grid: &GridConcat,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    let subplots = compiled_marks
        .iter()
        .filter_map(|mark| compiled_subplot(mark.as_ref()))
        .collect::<Vec<_>>();
    let grid_shape = resolve_grid_shape(grid, &subplots)?;
    validate_track_sizing(
        grid.column_widths_config(),
        grid_shape.columns,
        "grid concat column_widths",
    )?;
    validate_track_sizing(
        grid.row_heights_config(),
        grid_shape.rows,
        "grid concat row_heights",
    )?;
    let column_budgets =
        seeded_track_budgets(grid.column_widths_config(), grid_shape.columns, plot_width);
    let row_budgets = seeded_track_budgets(grid.row_heights_config(), grid_shape.rows, plot_height);
    let base_child_content_size = Size::new(
        plot_width / grid_shape.columns.max(1) as f32,
        plot_height / grid_shape.rows.max(1) as f32,
    );
    let guide_sharing_slots = GridGuideSharingSlots::from_placements(
        grid_shape,
        subplots
            .iter()
            .filter_map(|subplot| subplot.grid_placement()),
    );

    let mut prepared_children = Vec::with_capacity(subplots.len());
    for subplot in subplots {
        prepared_children.push(Box::pin(prepare_concat_child(subplot, eval_ctx, data)).await?);
    }
    let semantic_children = prepared_children
        .iter()
        .filter_map(|prepared| {
            prepared
                .grid_placement()
                .map(|placement| GridSemanticChild {
                    child_index: prepared.child_index(),
                    placement,
                    channel_domain_coordinations: prepared
                        .child_plot
                        .channel_domain_sharing_levels(),
                    scale_type_signatures: prepared.child_plot.scale_type_signatures_by_channel(),
                    axis_config_signatures: prepared
                        .child_plot
                        .axis_config_signatures_by_channel(eval_ctx.session_context()),
                })
        })
        .collect::<Vec<_>>();

    let coordinated_domain_extents =
        coordinated_domain_extents_for_concat_children(&prepared_children);

    let mut children = Vec::with_capacity(prepared_children.len());
    for (prepared, coordinated_extents) in prepared_children
        .iter()
        .zip(coordinated_domain_extents.iter())
    {
        let placement = prepared.grid_placement().ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "GridConcat subplots require `.grid_cell(row, column)`".to_string(),
            )
        })?;
        let span_budget = |budgets: &[f32], start: usize, span: usize| -> f32 {
            budgets.iter().skip(start).take(span.max(1)).sum()
        };
        let child_plot_area = Size::new(
            span_budget(&column_budgets, placement.column, placement.column_span),
            span_budget(&row_budgets, placement.row, placement.row_span),
        );
        let facet_scoped_extents = eval_ctx
            .facet_scale_precompute_store()
            .coordinated_child_frame_domain_extents(
                &prepared.relative_facet_child_frame_path,
                facet_path,
            );
        children.push(
            Box::pin(measure_prepared_concat_child(
                prepared,
                grid_concat_sharing_levels_with_axis_configs(
                    prepared.child_index(),
                    prepared.key(),
                    placement,
                    &guide_sharing_slots,
                    semantic_axis_guide_visibility_for_child(
                        grid.axis_guide_visibility_config(),
                        &semantic_children,
                        prepared.child_index(),
                    ),
                ),
                child_plot_area,
                eval_ctx,
                facet_path,
                coordinated_extents,
                &facet_scoped_extents,
            ))
            .await?,
        );
    }

    let column_sizes = layout_track_sizes(grid.column_widths_config());
    let row_sizes = layout_track_sizes(grid.row_heights_config());
    let placement = grid_child_frame_placement(
        &children,
        grid_shape,
        base_child_content_size,
        true,
        grid.spacing_px(),
        column_sizes.as_deref(),
        row_sizes.as_deref(),
    )?;
    Ok(Box::new(ConcatCoordMeasurement {
        children,
        placement: ConcatChildPlacement::Grid {
            placement,
            shape: grid_shape,
            retarget_plot_area_size: true,
        },
        fallback_content_size: Size::new(plot_width, plot_height),
        min_gap: grid.spacing_px(),
        column_sizes,
        row_sizes,
    }))
}

pub(crate) async fn measure_wrap_concat_coord_system(
    wrap: &WrapConcat,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    let subplots = compiled_marks
        .iter()
        .filter_map(|mark| compiled_subplot(mark.as_ref()))
        .collect::<Vec<_>>();
    let child_count = subplots.len();
    let columns = resolve_wrap_concat_columns(
        wrap.column_mode(),
        child_count,
        plot_width,
        eval_ctx.session_context().as_ref(),
        eval_ctx.params(),
    )
    .await?;
    let rows = child_count.div_ceil(columns).max(1);
    let child_plot_area = Size::new(
        plot_width / columns.max(1) as f32,
        plot_height / rows.max(1) as f32,
    );
    let grid_shape = GridShape { rows, columns };
    let guide_sharing_slots = GridGuideSharingSlots::from_placements(
        grid_shape,
        (0..child_count).map(|slot_index| GridPlacementConfig {
            row: slot_index / columns,
            column: slot_index % columns,
            row_span: 1,
            column_span: 1,
        }),
    );

    let mut prepared_children = Vec::with_capacity(subplots.len());
    for subplot in subplots {
        prepared_children.push(Box::pin(prepare_concat_child(subplot, eval_ctx, data)).await?);
    }
    let semantic_children = prepared_children
        .iter()
        .enumerate()
        .map(|(slot_index, prepared)| GridSemanticChild {
            child_index: prepared.child_index(),
            placement: GridPlacementConfig {
                row: slot_index / columns,
                column: slot_index % columns,
                row_span: 1,
                column_span: 1,
            },
            channel_domain_coordinations: prepared.child_plot.channel_domain_sharing_levels(),
            scale_type_signatures: prepared.child_plot.scale_type_signatures_by_channel(),
            axis_config_signatures: prepared
                .child_plot
                .axis_config_signatures_by_channel(eval_ctx.session_context()),
        })
        .collect::<Vec<_>>();

    let coordinated_domain_extents =
        coordinated_domain_extents_for_concat_children(&prepared_children);

    let mut children = Vec::with_capacity(prepared_children.len());
    for (slot_index, (prepared, coordinated_extents)) in prepared_children
        .iter()
        .zip(coordinated_domain_extents.iter())
        .enumerate()
    {
        let facet_scoped_extents = eval_ctx
            .facet_scale_precompute_store()
            .coordinated_child_frame_domain_extents(
                &prepared.relative_facet_child_frame_path,
                facet_path,
            );
        let mut child = Box::pin(measure_prepared_concat_child(
            prepared,
            grid_concat_sharing_levels_with_axis_configs(
                prepared.child_index(),
                prepared.key(),
                GridPlacementConfig {
                    row: slot_index / columns,
                    column: slot_index % columns,
                    row_span: 1,
                    column_span: 1,
                },
                &guide_sharing_slots,
                semantic_axis_guide_visibility_for_child(
                    wrap.axis_guide_visibility_config(),
                    &semantic_children,
                    prepared.child_index(),
                ),
            ),
            child_plot_area,
            eval_ctx,
            facet_path,
            coordinated_extents,
            &facet_scoped_extents,
        ))
        .await?;
        child.grid_placement = Some(GridPlacementConfig {
            row: slot_index / columns,
            column: slot_index % columns,
            row_span: 1,
            column_span: 1,
        });
        children.push(child);
    }

    let placement = grid_child_frame_placement(
        &children,
        grid_shape,
        child_plot_area,
        false,
        wrap.spacing_px(),
        None,
        None,
    )?;
    Ok(Box::new(ConcatCoordMeasurement {
        children,
        placement: ConcatChildPlacement::Grid {
            placement,
            shape: grid_shape,
            retarget_plot_area_size: false,
        },
        fallback_content_size: Size::new(plot_width, plot_height),
        min_gap: wrap.spacing_px(),
        // Wrap columns are resolved dynamically; declared track sizing is
        // not supported for wrap concats.
        column_sizes: None,
        row_sizes: None,
    }))
}

async fn resolve_wrap_concat_columns(
    column_mode: &FacetWrapColumnMode,
    child_count: usize,
    available_width: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<usize, AvengerChartError> {
    match column_mode {
        FacetWrapColumnMode::Auto => Ok((child_count.max(1) as f64).sqrt().ceil() as usize),
        FacetWrapColumnMode::Fixed(expr) => {
            let expr = expr.to_expr(ctx)?;
            if contains_aggregate(&expr) || expr.any_column_refs() {
                return Err(AvengerChartError::InvalidArgument(
                    "WrapConcat columns expression must be a constant or parameter expression"
                        .to_string(),
                ));
            }
            let datafusion_params = params_to_datafusion(params);
            let scalar = expr
                .eval_to_scalar(Some(ctx), datafusion_params.as_ref())
                .await
                .map_err(AvengerChartError::DataFusionError)?;
            scalar_to_columns(scalar, "WrapConcat columns expression")
        }
        FacetWrapColumnMode::ResponsiveWidth(expr) => {
            let expr = expr.to_expr(ctx)?;
            if contains_aggregate(&expr) || expr.any_column_refs() {
                return Err(AvengerChartError::InvalidArgument(
                    "WrapConcat responsive_columns target width must be a constant or parameter expression"
                        .to_string(),
                ));
            }
            if !available_width.is_finite() || available_width <= 0.0 {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "WrapConcat responsive_columns available width must be positive, got {available_width}"
                )));
            }
            let datafusion_params = params_to_datafusion(params);
            let target_width = scalar_to_positive_f32(
                expr.eval_to_scalar(Some(ctx), datafusion_params.as_ref())
                    .await
                    .map_err(AvengerChartError::DataFusionError)?,
                "WrapConcat responsive_columns target width",
            )?;
            let mut best_columns = 1usize;
            let mut best_delta = f32::INFINITY;
            for columns in 1..=child_count.max(1) {
                let estimated_width = available_width / columns as f32;
                let delta = (estimated_width - target_width).abs();
                if delta < best_delta {
                    best_columns = columns;
                    best_delta = delta;
                }
            }
            Ok(best_columns)
        }
    }
}

fn scalar_to_positive_f32(value: ScalarValue, label: &str) -> Result<f32, AvengerChartError> {
    let value = match value {
        ScalarValue::Int8(Some(v)) => v as f32,
        ScalarValue::Int16(Some(v)) => v as f32,
        ScalarValue::Int32(Some(v)) => v as f32,
        ScalarValue::Int64(Some(v)) => v as f32,
        ScalarValue::UInt8(Some(v)) => v as f32,
        ScalarValue::UInt16(Some(v)) => v as f32,
        ScalarValue::UInt32(Some(v)) => v as f32,
        ScalarValue::UInt64(Some(v)) => v as f32,
        ScalarValue::Float32(Some(v)) => v,
        ScalarValue::Float64(Some(v)) => v as f32,
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{label} must evaluate to a positive number, got {other:?}"
            )));
        }
    };
    if !value.is_finite() || value <= 0.0 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must evaluate to a positive number, got {value}"
        )));
    }
    Ok(value)
}

fn scalar_to_columns(value: ScalarValue, label: &str) -> Result<usize, AvengerChartError> {
    let columns = match value {
        ScalarValue::Int8(Some(v)) => v as i64,
        ScalarValue::Int16(Some(v)) => v as i64,
        ScalarValue::Int32(Some(v)) => v as i64,
        ScalarValue::Int64(Some(v)) => v,
        ScalarValue::UInt8(Some(v)) => v as i64,
        ScalarValue::UInt16(Some(v)) => v as i64,
        ScalarValue::UInt32(Some(v)) => v as i64,
        ScalarValue::UInt64(Some(v)) => i64::try_from(v).unwrap_or(i64::MAX),
        ScalarValue::Float32(Some(v)) => v.round() as i64,
        ScalarValue::Float64(Some(v)) => v.round() as i64,
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{label} must evaluate to a positive number, got {other:?}"
            )));
        }
    };
    if columns < 1 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must evaluate to a positive number, got {columns}"
        )));
    }
    Ok(columns as usize)
}

#[cfg(test)]
pub(crate) fn grid_concat_sharing_levels(
    child_index: usize,
    key: Option<&str>,
    placement: GridPlacementConfig,
    slots: &GridGuideSharingSlots,
    axis_guide_visibility: AxisGuideVisibilityConfig,
) -> Vec<ChildFrameSharingLevel> {
    grid_concat_sharing_levels_with_axis_configs(
        child_index,
        key,
        placement,
        slots,
        GridAxisGuideVisibilityConfig::same(axis_guide_visibility),
    )
}

fn grid_concat_sharing_levels_with_axis_configs(
    child_index: usize,
    key: Option<&str>,
    placement: GridPlacementConfig,
    slots: &GridGuideSharingSlots,
    axis_guide_visibility: GridAxisGuideVisibilityConfig,
) -> Vec<ChildFrameSharingLevel> {
    let row = slots.row_edge_ownership(placement);
    let column = slots.column_edge_ownership(placement);
    let vertical_visibility = axis_guide_visibility
        .for_axis(CoordinationAxis::Vertical)
        .with_span_ambiguity(row.ambiguous);
    let horizontal_visibility = axis_guide_visibility
        .for_axis(CoordinationAxis::Horizontal)
        .with_span_ambiguity(column.ambiguous);
    vec![
        ChildFrameSharingLevel::grid_concat_row(child_index, key, row.index, row.count)
            .with_axis_guide_visibility(vertical_visibility),
        ChildFrameSharingLevel::grid_concat_column(column.index, column.count)
            .with_axis_guide_visibility(horizontal_visibility),
    ]
}

trait AxisGuideVisibilitySpanExt {
    fn with_span_ambiguity(self, ambiguous: bool) -> Self;
}

impl AxisGuideVisibilitySpanExt for AxisGuideVisibilityConfig {
    fn with_span_ambiguity(self, ambiguous: bool) -> Self {
        if ambiguous {
            AxisGuideVisibilityConfig::same(AxisGuideVisibilityPolicy::All)
        } else {
            self
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GridGuideSharingSlots {
    rows_by_column: Vec<Vec<usize>>,
    columns_by_row: Vec<Vec<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GridGuideEdgeOwnership {
    index: usize,
    count: usize,
    ambiguous: bool,
}

impl GridGuideSharingSlots {
    pub(crate) fn from_placements(
        shape: GridShape,
        placements: impl IntoIterator<Item = GridPlacementConfig>,
    ) -> Self {
        let mut rows_by_column = vec![Vec::<usize>::new(); shape.columns];
        let mut columns_by_row = vec![Vec::<usize>::new(); shape.rows];

        for placement in placements {
            let row_end = placement
                .row
                .saturating_add(placement.row_span)
                .min(shape.rows);
            let column_end = placement
                .column
                .saturating_add(placement.column_span)
                .min(shape.columns);
            for column in placement.column..column_end {
                for row in placement.row..row_end {
                    if !rows_by_column[column].contains(&row) {
                        rows_by_column[column].push(row);
                    }
                    if !columns_by_row[row].contains(&column) {
                        columns_by_row[row].push(column);
                    }
                }
            }
        }

        for rows in &mut rows_by_column {
            rows.sort_unstable();
        }
        for columns in &mut columns_by_row {
            columns.sort_unstable();
        }

        Self {
            rows_by_column,
            columns_by_row,
        }
    }

    fn row_slot_index(&self, column: usize, row: usize) -> usize {
        self.rows_by_column
            .get(column)
            .and_then(|rows| rows.iter().position(|&candidate| candidate == row))
            .unwrap_or(row)
    }

    fn row_slot_count(&self, column: usize) -> usize {
        self.rows_by_column
            .get(column)
            .map(|rows| rows.len().max(1))
            .unwrap_or(1)
    }

    fn row_edge_ownership(&self, placement: GridPlacementConfig) -> GridGuideEdgeOwnership {
        let row = placement.row + placement.row_span.saturating_sub(1);
        let mut ownership = None;
        let mut ambiguous = false;
        for column in placement.column..placement.column + placement.column_span {
            let candidate = (
                self.row_slot_index(column, row),
                self.row_slot_count(column),
            );
            match ownership {
                Some(existing) if existing != candidate => ambiguous = true,
                None => ownership = Some(candidate),
                _ => {}
            }
        }
        let (index, count) = ownership.unwrap_or((row, 1));
        GridGuideEdgeOwnership {
            index,
            count,
            ambiguous,
        }
    }

    fn column_slot_index(&self, row: usize, column: usize) -> usize {
        self.columns_by_row
            .get(row)
            .and_then(|columns| columns.iter().position(|&candidate| candidate == column))
            .unwrap_or(column)
    }

    fn column_slot_count(&self, row: usize) -> usize {
        self.columns_by_row
            .get(row)
            .map(|columns| columns.len().max(1))
            .unwrap_or(1)
    }

    fn column_edge_ownership(&self, placement: GridPlacementConfig) -> GridGuideEdgeOwnership {
        let column = placement.column;
        let mut ownership = None;
        let mut ambiguous = false;
        for row in placement.row..placement.row + placement.row_span {
            let candidate = (
                self.column_slot_index(row, column),
                self.column_slot_count(row),
            );
            match ownership {
                Some(existing) if existing != candidate => ambiguous = true,
                None => ownership = Some(candidate),
                _ => {}
            }
        }
        let (index, count) = ownership.unwrap_or((column, 1));
        GridGuideEdgeOwnership {
            index,
            count,
            ambiguous,
        }
    }
}

fn resolve_grid_shape(
    grid: &GridConcat,
    subplots: &[&CompiledConcatSubplot],
) -> Result<GridShape, AvengerChartError> {
    if matches!(grid.rows_config(), Some(0)) || matches!(grid.columns_config(), Some(0)) {
        return Err(AvengerChartError::InvalidArgument(
            "GridConcat rows and columns must be greater than zero".to_string(),
        ));
    }

    let mut inferred_rows = 0usize;
    let mut inferred_columns = 0usize;
    let mut occupied = std::collections::HashSet::new();
    for subplot in subplots {
        let placement = subplot.grid_placement().ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "GridConcat subplots require `.grid_cell(row, column)`".to_string(),
            )
        })?;
        if placement.row_span == 0 || placement.column_span == 0 {
            return Err(AvengerChartError::InvalidArgument(
                "GridConcat row and column spans must be greater than zero".to_string(),
            ));
        }
        let row_end = placement
            .row
            .checked_add(placement.row_span)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "GridConcat subplot at row {}, column {} has row span {} that overflows usize",
                    placement.row, placement.column, placement.row_span
                ))
            })?;
        let column_end = placement
            .column
            .checked_add(placement.column_span)
            .ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "GridConcat subplot at row {}, column {} has column span {} that overflows usize",
                    placement.row, placement.column, placement.column_span
                ))
            })?;
        inferred_rows = inferred_rows.max(row_end);
        inferred_columns = inferred_columns.max(column_end);
        for row in placement.row..row_end {
            for column in placement.column..column_end {
                if !occupied.insert((row, column)) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "GridConcat has multiple subplots assigned to cell ({row}, {column})"
                    )));
                }
            }
        }
    }

    let rows = grid.rows_config().unwrap_or(inferred_rows);
    let columns = grid.columns_config().unwrap_or(inferred_columns);
    for subplot in subplots {
        let placement = subplot.grid_placement().expect("validated above");
        let row_end = placement.row + placement.row_span;
        let column_end = placement.column + placement.column_span;
        if row_end > rows || column_end > columns {
            return Err(AvengerChartError::InvalidArgument(format!(
                "GridConcat subplot at row {}, column {} with span {}x{} exceeds configured grid size {rows}x{columns}",
                placement.row, placement.column, placement.row_span, placement.column_span
            )));
        }
    }

    Ok(GridShape {
        rows: rows.max(1),
        columns: columns.max(1),
    })
}

fn grid_slot_from_placement(placement: GridPlacementConfig) -> GridSlot {
    GridSlot {
        row: placement.row,
        column: placement.column,
        row_span: placement.row_span,
        column_span: placement.column_span,
    }
}

#[allow(clippy::too_many_arguments)]
fn grid_child_frame_placement(
    children: &[ConcatChildMeasurement],
    shape: GridShape,
    base_cell_size: Size,
    retarget_plot_area_size: bool,
    min_gap: f32,
    column_sizes: Option<&[avenger_layout::TrackSize]>,
    row_sizes: Option<&[avenger_layout::TrackSize]>,
) -> Result<PlacementSolution, AvengerChartError> {
    let cells = grid_child_items(children)?;
    let spacing = TrackSpacing {
        min_gap,
        ..Default::default()
    };
    let solution = solve_concat_grid(
        shape,
        base_cell_size,
        &cells,
        spacing,
        spacing,
        column_sizes,
        row_sizes,
        false,
    )
    .map_err(AvengerChartError::InvalidArgument)?;

    let placements = children
        .iter()
        .map(|child| {
            let placement = child.grid_placement.expect("validated above");
            let slot = grid_slot_from_placement(placement);
            let origin = solution.content_origin_for_slot(slot);
            let edge_targets = solution.edge_targets_for_slot(slot);
            let content_size_override =
                retarget_plot_area_size.then(|| solution.content_size_for_slot(slot));
            PlacedRegion::with_meta(
                child.child_index,
                origin,
                ChartRegionMeta {
                    content_size_override,
                    edge_targets: Some(edge_targets),
                },
            )
        })
        .collect();

    Ok(PlacementSolution::new(solution.content_size, placements))
}

fn grid_child_items(
    children: &[ConcatChildMeasurement],
) -> Result<Vec<GridCell>, AvengerChartError> {
    children
        .iter()
        .map(|child| {
            let placement = child.grid_placement.ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing grid placement for child {}",
                    child.child_index
                ))
            })?;
            Ok(GridCell {
                slot: grid_slot_from_placement(placement),
                content_size: Size::new(
                    child.measurement.plot_area_width,
                    child.measurement.plot_area_height,
                ),
                edges: layered_cell_edges(
                    layout_edges(child.measurement.frame_demand().guide_slabs),
                    layout_edges(child.measurement.frame_demand().legend_slabs),
                ),
            })
        })
        .collect()
}

/// Per-side layered cell-edge declarations from guide (inner) and legend
/// (outer) slab extents. Legend slabs are the clamped remainder of the
/// rendered envelope after guides
/// (`FrameDemand::from_guide_and_rendered_envelope`), so `inner + outer`
/// covers the cell's rendered edge.
fn layered_cell_edges(guide: Edges<f32>, legend: Edges<f32>) -> Edges<EdgeDemand> {
    Edges::new(
        EdgeDemand {
            guide: guide.top,
            legend: legend.top,
        },
        EdgeDemand {
            guide: guide.right,
            legend: legend.right,
        },
        EdgeDemand {
            guide: guide.bottom,
            legend: legend.bottom,
        },
        EdgeDemand {
            guide: guide.left,
            legend: legend.left,
        },
    )
}

fn container_point_geometry(
    position_channels: &HashMap<&str, ScalarOrArray<f32>>,
    position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
    plot_width: f32,
    plot_height: f32,
) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
    let _ = position_values;
    let len = position_channels
        .values()
        .find_map(|v| match v.value() {
            ScalarOrArrayValue::Array(arr) => Some(arr.len()),
            ScalarOrArrayValue::Scalar(_) => None,
        })
        .unwrap_or(1);
    let x = plot_width / 2.0;
    let y = plot_height / 2.0;
    let geometry = if len == 1 {
        PointGeometry {
            x: ScalarOrArray::new_scalar(x),
            y: ScalarOrArray::new_scalar(y),
        }
    } else {
        PointGeometry {
            x: ScalarOrArray::new_array(vec![x; len]),
            y: ScalarOrArray::new_array(vec![y; len]),
        }
    };
    Ok(Box::new(geometry))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use avenger_scenegraph::marks::{mark::SceneMark, symbol::SceneSymbolMark};
    use datafusion::{
        arrow::{
            array::{Array, Float64Array, StringArray},
            record_batch::RecordBatch as ArrowRecordBatch,
        },
        prelude::{SessionContext, col},
    };

    use super::*;
    use crate::{
        cartesian::{Cartesian, CartesianLinePositionChannels, CartesianSymbolPositionChannels},
        coords::FacetAxis,
        facet::{
            coord::{FacetBandCoordMeasurement, FacetColumn, FacetRow},
            evaluated_facet_tree::EvaluatedFacetTree,
            marks::{FacetColumnSubplotChannels, FacetRowSubplotChannels},
        },
        layout::{
            EdgeGrant, EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode, TrackSpacing,
        },
        marks::{Subplot, line::Line, symbol::Symbol},
        plot::{
            CompiledPlot, Plot,
            compiled::{
                CoordinationKind, CoordinationScopeKey, EvaluationRequest,
                child_frame_container_view_from_concat,
                child_frame_coordination::{
                    ChartGridRequirements, ChildFrameContainerInstanceKey, ChildFrameContainerKind,
                    ChildFrameContainerTemplateKey, ChildFrameLayoutCoordinationNode,
                    ChildFrameLayoutRequirements, ChildFrameLayoutSlot,
                    ChildFrameLayoutSlotTopology, ChildFrameLayoutTopology,
                    LayoutCoordinationScope, build_child_frame_layout_alignment_diagnostics,
                    collect_child_frame_layout_coordination_nodes,
                    diagnose_child_frame_layout_alignment,
                },
                container_label_items_from_child_frame_container,
                scale_provider::DynamicScaleProvider,
                scales::build_scale_builder_from_marks,
            },
        },
        render::EvaluationContext,
        repeat::{self, RepeatGrid, RepeatVariable},
        scales::{Linear, LinearScaleExt, ScaleChannelConfig},
        zerod::ZeroDCoord,
    };
    use avenger_chart_core::{
        AxisGuideVisibilityConfig, AxisGuideVisibilityPolicy, ChartEventBinding, ChartEventType,
        CoordinationAxis, CoordinationScope, DomainCoordination,
    };

    fn zero_edge_demands(len: usize) -> Vec<EdgeGrant> {
        vec![EdgeGrant::default(); len]
    }
    async fn measurement_for_plot(
        compiled: &CompiledPlot,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        measurement_for_plot_with_container_path(compiled, plot_width, plot_height, ctx, &[]).await
    }

    async fn measurement_for_plot_with_container_path(
        compiled: &CompiledPlot,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
        container_path: &[ContainerPathSegment],
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        let params = compiled.get_default_params().clone();
        let facet_tree = EvaluatedFacetTree::from_compiled_plot(compiled, ctx).await?;
        let mut eval_ctx = EvaluationContext::new(
            compiled.get_theme(),
            Arc::new(ctx.clone()),
            params.clone(),
            Arc::new(facet_tree),
        );
        for segment in container_path {
            eval_ctx = eval_ctx.with_child_frame_container_path_appended(segment.clone());
        }
        let layout_spec = EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Auto,
            plot_area: EvaluatedSizeMode::Fixed {
                width: plot_width,
                height: plot_height,
            },
            margins: EvaluatedMargins {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
        };
        let scale_builder = build_scale_builder_from_marks(
            &compiled.marks,
            &compiled.scale_specs,
            &compiled.coord_transform,
            &compiled.data,
            None,
            &eval_ctx,
            compiled.get_theme().as_ref(),
        )
        .await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: compiled,
        };

        compiled
            .measure_plot_components(&eval_ctx, &layout_spec, &provider, None, &[])
            .await
    }

    fn xy_dataframe(
        ctx: &SessionContext,
        x_values: Vec<f64>,
        y_values: Vec<f64>,
    ) -> datafusion::dataframe::DataFrame {
        let batch = ArrowRecordBatch::try_from_iter(vec![
            (
                "x",
                Arc::new(Float64Array::from(x_values)) as Arc<dyn Array>,
            ),
            (
                "y",
                Arc::new(Float64Array::from(y_values)) as Arc<dyn Array>,
            ),
        ])
        .expect("create xy record batch");
        ctx.read_batch(batch).expect("read xy test batch")
    }

    fn grouped_xy_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        let batch = ArrowRecordBatch::try_from_iter(vec![
            (
                "x",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 100.0, 101.0])) as Arc<dyn Array>,
            ),
            (
                "y",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 1.0, 2.0])) as Arc<dyn Array>,
            ),
            (
                "group",
                Arc::new(StringArray::from(vec!["left", "left", "right", "right"]))
                    as Arc<dyn Array>,
            ),
        ])
        .expect("create grouped xy record batch");
        ctx.read_batch(batch).expect("read grouped xy test batch")
    }

    fn nested_grouped_xy_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        let batch = ArrowRecordBatch::try_from_iter(vec![
            (
                "x",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 100.0, 101.0])) as Arc<dyn Array>,
            ),
            (
                "y",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 1.0, 2.0])) as Arc<dyn Array>,
            ),
            (
                "group",
                Arc::new(StringArray::from(vec!["left", "left", "right", "right"]))
                    as Arc<dyn Array>,
            ),
            (
                "subgroup",
                Arc::new(StringArray::from(vec!["top", "bottom", "top", "bottom"]))
                    as Arc<dyn Array>,
            ),
        ])
        .expect("create nested grouped xy record batch");
        ctx.read_batch(batch)
            .expect("read nested grouped xy test batch")
    }

    fn find_symbol_mark(mark: &SceneMark) -> Option<&SceneSymbolMark> {
        match mark {
            SceneMark::Symbol(symbol) => Some(symbol),
            SceneMark::Group(group) => group.marks.iter().find_map(find_symbol_mark),
            _ => None,
        }
    }

    fn child_scatter_plot(
        data: datafusion::dataframe::DataFrame,
        share_x: bool,
    ) -> Plot<Cartesian> {
        let symbol = if share_x {
            Symbol::new()
                .x_with(col("x"), |c| c.with_domain_scope(CoordinationScope::Shared))
                .y(col("y"))
        } else {
            Symbol::new().x(col("x")).y(col("y"))
        };
        Plot::<Cartesian>::new().data(data).mark(symbol)
    }

    fn line_mark(share_x: bool) -> Line<Cartesian> {
        let mark = Line::<Cartesian>::new().y(col("y"));
        if share_x {
            mark.x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_scope(CoordinationScope::Shared)
            })
        } else {
            mark.x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
            })
        }
    }

    fn line_child_plot(data: datafusion::dataframe::DataFrame, share_x: bool) -> Plot<Cartesian> {
        Plot::<Cartesian>::new().data(data).mark(line_mark(share_x))
    }

    fn child_x_domain(child: &ConcatChildMeasurement) -> (f32, f32) {
        measurement_x_domain(&child.measurement)
    }

    fn measurement_x_domain(measurement: &ComponentsMeasurement) -> (f32, f32) {
        measurement
            .scales
            .get("x")
            .expect("x scale should exist")
            .configured()
            .numeric_interval_domain()
            .expect("x scale should have a numeric interval domain")
    }

    fn measurement_y_domain(measurement: &ComponentsMeasurement) -> (f32, f32) {
        measurement
            .scales
            .get("y")
            .expect("y scale should exist")
            .configured()
            .numeric_interval_domain()
            .expect("y scale should have a numeric interval domain")
    }

    fn zero_plot() -> Plot<ZeroDCoord> {
        Plot::<ZeroDCoord>::new()
    }

    fn repeat_vars(names: &[&str]) -> Vec<RepeatVariable> {
        names
            .iter()
            .map(|name| RepeatVariable::new(*name, col(*name)))
            .collect()
    }

    fn repeated_grid_cell() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(Symbol::new().x(repeat::column()).y(repeat::row()))
    }

    fn repeated_facet_column_cell() -> Plot<FacetColumn> {
        let child = Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x(repeat::column())
                .y(repeat::row())
                .size(32.0),
        );
        Plot::<FacetColumn>::new().mark(Subplot::new(child).column(col("group")))
    }

    fn manual_grid_plot() -> Plot<GridConcat> {
        Plot::<GridConcat>::new()
            .rows(1)
            .columns(2)
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0).key("left"))
            .mark(Subplot::new(zero_plot()).grid_cell(0, 1).key("right"))
    }

    fn keyed_hconcat(left_key: &str, right_key: &str) -> Plot<HConcat> {
        Plot::<HConcat>::new()
            .mark(Subplot::new(zero_plot()).key(left_key))
            .mark(Subplot::new(zero_plot()).key(right_key))
    }

    fn wrapped_zero_plot(count: usize) -> Plot<WrapConcat> {
        (0..count).fold(Plot::<WrapConcat>::new(), |plot, idx| {
            plot.mark(Subplot::new(zero_plot()).key(format!("child-{idx}")))
        })
    }

    fn synthetic_wrap_alignment_node(
        shape: GridShape,
        facet_value: &str,
    ) -> ChildFrameLayoutCoordinationNode {
        let slots = (0..5)
            .map(|child_index| {
                let slot = GridSlot {
                    row: child_index / shape.columns,
                    column: child_index % shape.columns,
                    row_span: 1,
                    column_span: 1,
                };
                let child_key = ChildFrameKey::ConcatChild {
                    index: child_index,
                    key: Some(format!("child-{child_index}")),
                };
                ChildFrameLayoutSlot {
                    child_index,
                    child_key,
                    slot,
                }
            })
            .collect::<Vec<_>>();
        let topology_slots = slots
            .iter()
            .map(|slot| ChildFrameLayoutSlotTopology {
                child_key: slot.child_key.clone(),
                slot: slot.slot,
            })
            .collect();
        let template_path = vec![ContainerPathSegment::concat_child(0, Some("repeat_wrap"))];
        let mut instance_path = vec![ContainerPathSegment::facet_value(
            FacetAxis::Column,
            0,
            ScalarValue::Utf8(Some(facet_value.to_string())),
        )];
        instance_path.extend(template_path.clone());

        ChildFrameLayoutCoordinationNode {
            instance_key: ChildFrameContainerInstanceKey::new(instance_path),
            template_key: ChildFrameContainerTemplateKey {
                container_path_template: template_path,
                semantic_tag: None,
            },
            kind: ChildFrameContainerKind::GridConcat,
            alignment_scope: LayoutCoordinationScope::TemplatePathWithoutFacetSegments,
            topology: ChildFrameLayoutTopology::Grid {
                shape,
                slots: topology_slots,
            },
            slots,
            requirements: ChildFrameLayoutRequirements::Grid(ChartGridRequirements {
                grid: ChartGridData {
                    shape,
                    column_spacing: TrackSpacing::default(),
                    row_spacing: TrackSpacing::default(),
                    column_widths: vec![10.0; shape.columns],
                    row_heights: vec![10.0; shape.rows],
                    column_left: zero_edge_demands(shape.columns),
                    column_right: zero_edge_demands(shape.columns),
                    row_top: zero_edge_demands(shape.rows),
                    row_bottom: zero_edge_demands(shape.rows),
                },
                guide_slot_gap_px: 0.0,
            }),
            member: None,
        }
    }

    fn synthetic_grid_alignment_node(
        shape: GridShape,
        slots: &[GridSlot],
        facet_value: &str,
    ) -> ChildFrameLayoutCoordinationNode {
        let slots = slots
            .iter()
            .enumerate()
            .map(|(child_index, &slot)| {
                let child_key = ChildFrameKey::ConcatChild {
                    index: child_index,
                    key: Some(format!("child-{child_index}")),
                };
                ChildFrameLayoutSlot {
                    child_index,
                    child_key,
                    slot,
                }
            })
            .collect::<Vec<_>>();
        let topology_slots = slots
            .iter()
            .map(|slot| ChildFrameLayoutSlotTopology {
                child_key: slot.child_key.clone(),
                slot: slot.slot,
            })
            .collect();
        let template_path = vec![ContainerPathSegment::concat_child(0, Some("manual_grid"))];
        let mut instance_path = vec![ContainerPathSegment::facet_value(
            FacetAxis::Column,
            0,
            ScalarValue::Utf8(Some(facet_value.to_string())),
        )];
        instance_path.extend(template_path.clone());

        ChildFrameLayoutCoordinationNode {
            instance_key: ChildFrameContainerInstanceKey::new(instance_path),
            template_key: ChildFrameContainerTemplateKey {
                container_path_template: template_path,
                semantic_tag: None,
            },
            kind: ChildFrameContainerKind::GridConcat,
            alignment_scope: LayoutCoordinationScope::TemplatePathWithoutFacetSegments,
            topology: ChildFrameLayoutTopology::Grid {
                shape,
                slots: topology_slots,
            },
            slots,
            requirements: ChildFrameLayoutRequirements::Grid(ChartGridRequirements {
                grid: ChartGridData {
                    shape,
                    column_spacing: TrackSpacing::default(),
                    row_spacing: TrackSpacing::default(),
                    column_widths: vec![10.0; shape.columns],
                    row_heights: vec![10.0; shape.rows],
                    column_left: zero_edge_demands(shape.columns),
                    column_right: zero_edge_demands(shape.columns),
                    row_top: zero_edge_demands(shape.rows),
                    row_bottom: zero_edge_demands(shape.rows),
                },
                guide_slot_gap_px: 0.0,
            }),
            member: None,
        }
    }

    fn spanned_grid_topology_slots() -> [GridSlot; 4] {
        [
            GridSlot {
                row: 0,
                column: 0,
                row_span: 2,
                column_span: 2,
            },
            GridSlot {
                row: 0,
                column: 2,
                row_span: 1,
                column_span: 1,
            },
            GridSlot {
                row: 2,
                column: 0,
                row_span: 1,
                column_span: 1,
            },
            GridSlot {
                row: 2,
                column: 2,
                row_span: 1,
                column_span: 1,
            },
        ]
    }

    #[tokio::test]
    async fn hconcat_spacing_floors_child_gaps() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::with_coord(HConcat::new().spacing(25.0))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        assert_eq!(concat.min_gap, 25.0);
        let placement = concat.child_frame_placement();
        let first = placement.child(0).expect("first child placed");
        let second = placement.child(1).expect("second child placed");
        let first_right = first.origin[0] + concat.children[0].measurement.plot_area_width;
        assert!(
            second.origin[0] - first_right >= 25.0 - 0.01,
            "configured spacing should floor the inter-child gap: gap = {}",
            second.origin[0] - first_right
        );
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_spacing_floors_track_gaps() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::with_coord(GridConcat::new().rows(1).columns(2).spacing(30.0))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).grid_cell(0, 0))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).grid_cell(0, 1))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 240.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("GridConcat should measure as ConcatCoordMeasurement");
        assert_eq!(concat.min_gap, 30.0);
        let placement = concat.child_frame_placement();
        let first = placement.child(0).expect("first child placed");
        let second = placement.child(1).expect("second child placed");
        let first_right = first.origin[0] + concat.children[0].measurement.plot_area_width;
        assert!(
            second.origin[0] - first_right >= 30.0 - 0.01,
            "configured spacing should floor the inter-track gap: gap = {}",
            second.origin[0] - first_right
        );
        Ok(())
    }

    #[tokio::test]
    async fn hconcat_measurement_exposes_child_frame_container_view()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(
                Subplot::new(Plot::<ZeroDCoord>::new())
                    .key("left")
                    .label("Left"),
            )
            .mark(
                Subplot::new(Plot::<ZeroDCoord>::new())
                    .key("right")
                    .label("Right"),
            )
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        assert_eq!(concat.band_direction(), Some(Orientation::Horizontal));
        assert_eq!(concat.children.len(), 2);
        assert_eq!(concat.children[0].key.as_deref(), Some("left"));
        assert_eq!(concat.children[1].key.as_deref(), Some("right"));
        let left_scope = concat
            .child_scope_key(0)
            .expect("left child should have a scope key");
        assert!(left_scope.container_path.is_empty());
        assert_eq!(
            &left_scope.child_key,
            &ChildFrameKey::ConcatChild {
                index: 0,
                key: Some("left".to_string())
            }
        );

        let container = measurement
            .child_frame_container_view()?
            .expect("concat measurement should expose a child-frame container view");
        assert_eq!(container.placement().placements().len(), 2);
        assert_eq!(container.placement().placements()[0].origin, [0.0, 0.0]);
        assert!(container.child_measurement(0).is_some());
        assert!(container.child_measurement(1).is_some());
        assert_eq!(container.child_scope_key(0), Some(&left_scope));
        assert_eq!(container.child_label(0), Some("Left"));
        assert_eq!(container.child_label(1), Some("Right"));
        assert_eq!(container.child_scope_keys().count(), 2);

        let direct_container = child_frame_container_view_from_concat(concat)?;
        assert_eq!(direct_container.placement(), container.placement());
        let label_items = container_label_items_from_child_frame_container(&direct_container)?;
        assert_eq!(label_items.len(), 2);
        assert_eq!(label_items[0].text, "Left");
        assert_eq!(label_items[1].text, "Right");
        Ok(())
    }

    #[tokio::test]
    async fn hconcat_child_event_bindings_are_targeted_to_child_scopes()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let left = child_scatter_plot(xy_dataframe(&ctx, vec![1.0], vec![1.0]), false)
            .event_binding(ChartEventBinding::on(ChartEventType::CursorMoved));
        let right = child_scatter_plot(xy_dataframe(&ctx, vec![2.0], vec![2.0]), false)
            .event_binding(ChartEventBinding::on(ChartEventType::CursorMoved));

        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(left).key("left"))
            .mark(Subplot::new(right).key("right"))
            .compile(&ctx)
            .await?;

        let targets = compiled
            .event_bindings()
            .iter()
            .map(|binding| {
                binding
                    .scope_target
                    .as_ref()
                    .and_then(|target| target.resolved_coord_node_path_prefix())
                    .map(|target| target.to_vec())
            })
            .collect::<Vec<_>>();

        assert_eq!(targets, vec![Some(vec![0]), Some(vec![1])]);
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_measurement_places_complete_grid() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<GridConcat>::new()
            .rows(2)
            .columns(2)
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0).key("a"))
            .mark(Subplot::new(zero_plot()).grid_cell(0, 1).key("b"))
            .mark(Subplot::new(zero_plot()).grid_cell(1, 0).key("c"))
            .mark(Subplot::new(zero_plot()).grid_cell(1, 1).key("d"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("GridConcat should measure as ConcatCoordMeasurement");
        assert_eq!(concat.band_direction(), None);
        assert_eq!(concat.children.len(), 4);
        assert_eq!(
            concat
                .children()
                .iter()
                .map(|child| child
                    .grid_placement
                    .map(|placement| (placement.row, placement.column)))
                .collect::<Vec<_>>(),
            vec![Some((0, 0)), Some((0, 1)), Some((1, 0)), Some((1, 1))]
        );

        let placement = concat.child_frame_placement();
        let origins = placement
            .placements()
            .iter()
            .map(|placement| (placement.id, placement.origin))
            .collect::<Vec<_>>();
        assert_eq!(
            origins,
            vec![
                (0, [0.0, 0.0]),
                (1, [100.0, 0.0]),
                (2, [0.0, 50.0]),
                (3, [100.0, 50.0]),
            ]
        );
        assert_eq!(placement.content_size, Size::new(200.0, 100.0));
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_applies_merged_track_requirements_idempotently()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<GridConcat>::new()
            .rows(1)
            .columns(2)
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0).key("left"))
            .mark(Subplot::new(zero_plot()).grid_cell(0, 1).key("right"))
            .compile(&ctx)
            .await?;

        let mut measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<ConcatCoordMeasurement>()
            .expect("GridConcat should measure as ConcatCoordMeasurement");
        let old_placement = concat.child_frame_placement();
        assert_eq!(old_placement.placements()[1].origin, [100.0, 0.0]);

        // A cousin whose second cell demands a wider left edge: the group
        // solve patches the local grid to the cousin's folds.
        let local_spec = concat.grid_member_spec()?;
        let mut cousin_spec = local_spec.clone();
        cousin_spec.cells[1].edges.left = EdgeDemand {
            guide: 32.0,
            legend: 0.0,
        };
        let solutions =
            crate::layout::concat_grid::solve_concat_grid_group(&[local_spec, cousin_spec])
                .map_err(AvengerChartError::InternalError)?;
        assert!(concat.install_grid_solution(&solutions[0])?);

        let applied_placement = concat.child_frame_placement();
        assert_eq!(applied_placement.placements()[0].origin, [0.0, 0.0]);
        assert_eq!(applied_placement.placements()[1].origin, [132.0, 0.0]);
        assert_eq!(applied_placement.content_size, Size::new(232.0, 100.0));
        assert_ne!(applied_placement, old_placement);
        assert!(!concat.install_grid_solution(&solutions[0])?);
        Ok(())
    }

    #[tokio::test]
    async fn hconcat_applies_merged_track_requirements_idempotently()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = keyed_hconcat("left", "right").compile(&ctx).await?;

        let mut measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let old_placement = concat.child_frame_placement();
        assert_eq!(old_placement.placements()[1].origin, [100.0, 0.0]);

        // A cousin whose second cell demands a wider left edge: the group
        // solve patches the local band to the cousin's folds.
        let local_spec = concat.grid_member_spec()?;
        let mut cousin_spec = local_spec.clone();
        cousin_spec.cells[1].edges.left = EdgeDemand {
            guide: 32.0,
            legend: 0.0,
        };
        let solutions =
            crate::layout::concat_grid::solve_concat_grid_group(&[local_spec, cousin_spec])
                .map_err(AvengerChartError::InternalError)?;
        assert!(concat.install_grid_solution(&solutions[0])?);

        let applied_placement = concat.child_frame_placement();
        assert_eq!(applied_placement.placements()[0].origin, [0.0, 0.0]);
        assert_eq!(applied_placement.placements()[1].origin, [132.0, 0.0]);
        assert_eq!(applied_placement.content_size, Size::new(232.0, 100.0));
        assert_ne!(applied_placement, old_placement);
        assert!(!concat.install_grid_solution(&solutions[0])?);
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_holes_preserve_track_indices() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<GridConcat>::new()
            .rows(2)
            .columns(3)
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0).key("top-left"))
            .mark(
                Subplot::new(zero_plot())
                    .grid_cell(1, 2)
                    .key("bottom-right"),
            )
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 300.0, 200.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("GridConcat should measure as ConcatCoordMeasurement");

        let placement = concat.child_frame_placement();
        let origins = placement
            .placements()
            .iter()
            .map(|placement| (placement.id, placement.origin))
            .collect::<Vec<_>>();
        assert_eq!(origins, vec![(0, [0.0, 0.0]), (1, [200.0, 100.0])]);
        assert_eq!(placement.content_size, Size::new(300.0, 200.0));
        Ok(())
    }

    #[test]
    fn grid_guide_sharing_slots_compact_empty_edge_cells() {
        let slots = GridGuideSharingSlots::from_placements(
            GridShape {
                rows: 2,
                columns: 3,
            },
            [
                GridPlacementConfig {
                    row: 0,
                    column: 0,
                    row_span: 1,
                    column_span: 1,
                },
                GridPlacementConfig {
                    row: 0,
                    column: 2,
                    row_span: 1,
                    column_span: 1,
                },
                GridPlacementConfig {
                    row: 1,
                    column: 1,
                    row_span: 1,
                    column_span: 1,
                },
            ],
        );

        let top_right = grid_concat_sharing_levels(
            1,
            Some("top-right"),
            GridPlacementConfig {
                row: 0,
                column: 2,
                row_span: 1,
                column_span: 1,
            },
            &slots,
            AxisGuideVisibilityConfig::auto(),
        );
        assert_eq!(top_right.len(), 2);
        assert_eq!(top_right[0].axis, CoordinationAxis::Vertical);
        assert_eq!(top_right[0].index, 0);
        assert_eq!(top_right[0].count, 1);
        assert_eq!(top_right[1].axis, CoordinationAxis::Horizontal);
        assert_eq!(top_right[1].index, 1);
        assert_eq!(top_right[1].count, 2);

        let bottom_middle = grid_concat_sharing_levels(
            2,
            Some("bottom-middle"),
            GridPlacementConfig {
                row: 1,
                column: 1,
                row_span: 1,
                column_span: 1,
            },
            &slots,
            AxisGuideVisibilityConfig::auto(),
        );
        assert_eq!(bottom_middle[0].index, 0);
        assert_eq!(bottom_middle[0].count, 1);
        assert_eq!(bottom_middle[1].index, 0);
        assert_eq!(bottom_middle[1].count, 1);
    }

    #[test]
    fn grid_guide_sharing_slots_treat_spans_as_occupied_rectangles() {
        let slots = GridGuideSharingSlots::from_placements(
            GridShape {
                rows: 3,
                columns: 3,
            },
            [GridPlacementConfig {
                row: 0,
                column: 0,
                row_span: 2,
                column_span: 2,
            }],
        );

        assert_eq!(slots.rows_by_column, vec![vec![0, 1], vec![0, 1], vec![]]);
        assert_eq!(slots.columns_by_row, vec![vec![0, 1], vec![0, 1], vec![]]);
        assert_eq!(slots.row_slot_index(1, 1), 1);
        assert_eq!(slots.row_slot_count(1), 2);
        assert_eq!(slots.column_slot_index(1, 1), 1);
        assert_eq!(slots.column_slot_count(1), 2);
    }

    #[test]
    fn grid_guide_sharing_uses_span_bottom_edge_for_x_axis() {
        let slots = GridGuideSharingSlots::from_placements(
            GridShape {
                rows: 3,
                columns: 1,
            },
            [
                GridPlacementConfig {
                    row: 0,
                    column: 0,
                    row_span: 2,
                    column_span: 1,
                },
                GridPlacementConfig {
                    row: 2,
                    column: 0,
                    row_span: 1,
                    column_span: 1,
                },
            ],
        );

        let levels = grid_concat_sharing_levels(
            0,
            Some("spanned"),
            GridPlacementConfig {
                row: 0,
                column: 0,
                row_span: 2,
                column_span: 1,
            },
            &slots,
            AxisGuideVisibilityConfig::auto(),
        );
        assert_eq!(levels[0].axis, CoordinationAxis::Vertical);
        assert_eq!(levels[0].index, 1);
        assert_eq!(levels[0].count, 3);
    }

    #[test]
    fn grid_guide_sharing_keeps_ambiguous_span_edges_visible() {
        let row_ambiguous_slots = GridGuideSharingSlots::from_placements(
            GridShape {
                rows: 3,
                columns: 2,
            },
            [
                GridPlacementConfig {
                    row: 0,
                    column: 0,
                    row_span: 2,
                    column_span: 2,
                },
                GridPlacementConfig {
                    row: 2,
                    column: 1,
                    row_span: 1,
                    column_span: 1,
                },
            ],
        );
        let row_levels = grid_concat_sharing_levels(
            0,
            Some("row-ambiguous"),
            GridPlacementConfig {
                row: 0,
                column: 0,
                row_span: 2,
                column_span: 2,
            },
            &row_ambiguous_slots,
            AxisGuideVisibilityConfig::same(AxisGuideVisibilityPolicy::OuterEdges),
        );
        assert_eq!(
            row_levels[0].axis_guide_visibility.labels,
            AxisGuideVisibilityPolicy::All
        );
        assert_eq!(
            row_levels[0].axis_guide_visibility.title,
            AxisGuideVisibilityPolicy::All
        );

        let column_ambiguous_slots = GridGuideSharingSlots::from_placements(
            GridShape {
                rows: 2,
                columns: 3,
            },
            [
                GridPlacementConfig {
                    row: 0,
                    column: 1,
                    row_span: 2,
                    column_span: 2,
                },
                GridPlacementConfig {
                    row: 1,
                    column: 0,
                    row_span: 1,
                    column_span: 1,
                },
            ],
        );
        let column_levels = grid_concat_sharing_levels(
            0,
            Some("column-ambiguous"),
            GridPlacementConfig {
                row: 0,
                column: 1,
                row_span: 2,
                column_span: 2,
            },
            &column_ambiguous_slots,
            AxisGuideVisibilityConfig::same(AxisGuideVisibilityPolicy::OuterEdges),
        );
        assert_eq!(
            column_levels[1].axis_guide_visibility.labels,
            AxisGuideVisibilityPolicy::All
        );
        assert_eq!(
            column_levels[1].axis_guide_visibility.title,
            AxisGuideVisibilityPolicy::All
        );
    }

    #[tokio::test]
    async fn grid_concat_measures_spanned_child() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<GridConcat>::new()
            .rows(2)
            .columns(2)
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0).grid_span(2, 1))
            .mark(Subplot::new(zero_plot()).grid_cell(0, 1))
            .mark(Subplot::new(zero_plot()).grid_cell(1, 1))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = concat_coord_ref(measurement.coord_measurement.as_ref())
            .expect("grid concat measurement");
        let spanned = concat.child(0).expect("spanned child measurement");
        assert_eq!(spanned.measurement.plot_area_width, 100.0);
        assert_eq!(spanned.measurement.plot_area_height, 100.0);

        let placement = concat.child_frame_placement();
        assert_eq!(placement.content_size, Size::new(200.0, 100.0));
        assert_eq!(
            placement.child(0).expect("spanned child").origin,
            [0.0, 0.0]
        );
        assert_eq!(
            placement
                .child(0)
                .expect("spanned child")
                .meta
                .content_size_override,
            Some(Size::new(100.0, 100.0))
        );
        assert_eq!(
            placement.child(1).expect("top right child").origin,
            [100.0, 0.0]
        );
        assert_eq!(
            placement.child(2).expect("bottom right child").origin,
            [100.0, 50.0]
        );
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_rejects_zero_grid_span() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let result = Plot::<GridConcat>::new()
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0).grid_span(0, 1))
            .compile(&ctx)
            .await;
        let err = match result {
            Ok(_) => panic!("zero row span should fail during compile"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("GridConcat subplot spans must be greater than zero")
        );
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_rejects_span_outside_configured_shape() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<GridConcat>::new()
            .rows(2)
            .columns(2)
            .mark(Subplot::new(zero_plot()).grid_cell(1, 1).grid_span(2, 1))
            .compile(&ctx)
            .await?;

        let err = measurement_for_plot(&compiled, 200.0, 100.0, &ctx)
            .await
            .expect_err("span outside configured rows should fail");
        assert!(err.to_string().contains("exceeds configured grid size"));
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_rejects_overlapping_span_rectangles() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<GridConcat>::new()
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0).grid_span(2, 2))
            .mark(Subplot::new(zero_plot()).grid_cell(1, 1))
            .compile(&ctx)
            .await?;

        let err = measurement_for_plot(&compiled, 200.0, 100.0, &ctx)
            .await
            .expect_err("span overlap should fail");
        assert!(err.to_string().contains("multiple subplots assigned"));
        Ok(())
    }

    #[tokio::test]
    async fn grid_layout_coordination_nodes_group_repeat_instances_across_facets()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('G1', 1.0, 10.0),
                    ('G1', 2.0, 20.0),
                    ('G2', 3.0, 30.0),
                    ('G2', 4.0, 40.0)
                ) AS t(group_name, a, b)",
            )
            .await?;
        let repeat = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell());
        let compiled = Plot::<FacetColumn>::new()
            .data(df)
            .mark(Subplot::new(repeat).column(col("group_name")))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 400.0, 200.0, &ctx).await?;
        let all_nodes = collect_child_frame_layout_coordination_nodes(&measurement)?;
        let nodes = all_nodes
            .iter()
            .filter(|node| node.kind == ChildFrameContainerKind::GridConcat)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(nodes.len(), 2);
        assert_ne!(nodes[0].instance_key, nodes[1].instance_key);
        assert_eq!(nodes[0].template_key, nodes[1].template_key);
        assert_eq!(nodes[0].alignment_key(), nodes[1].alignment_key());

        for node in nodes {
            assert_eq!(node.slots.len(), 4);
            match node.topology {
                ChildFrameLayoutTopology::Grid { shape, slots } => {
                    assert_eq!(
                        shape,
                        GridShape {
                            rows: 2,
                            columns: 2
                        }
                    );
                    assert_eq!(slots.len(), 4);
                }
            }
            match node.requirements {
                ChildFrameLayoutRequirements::Grid(requirements) => {
                    assert_eq!(
                        requirements.grid.shape,
                        GridShape {
                            rows: 2,
                            columns: 2
                        }
                    );
                    assert_eq!(requirements.grid.column_widths.len(), 2);
                    assert_eq!(requirements.grid.row_heights.len(), 2);
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn layout_coordination_nodes_export_facet_band_topology() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = Plot::<FacetColumn>::new()
            .data(grouped_xy_dataframe(&ctx))
            .mark(
                Subplot::new(Plot::<Cartesian>::new().mark(line_mark(false))).column(col("group")),
            )
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 320.0, 120.0, &ctx).await?;
        let nodes = collect_child_frame_layout_coordination_nodes(&measurement)?;
        let facet_nodes = nodes
            .iter()
            .filter(|node| node.kind == ChildFrameContainerKind::FacetColumn)
            .collect::<Vec<_>>();

        assert_eq!(facet_nodes.len(), 1);
        let node = facet_nodes[0];
        assert_eq!(
            node.template_key.semantic_tag.as_deref(),
            Some("facet:col:1:group")
        );
        match &node.topology {
            ChildFrameLayoutTopology::Grid { shape, slots } => {
                assert_eq!(
                    *shape,
                    GridShape {
                        rows: 1,
                        columns: 2
                    }
                );
                assert_eq!(slots.len(), 2);
                assert_eq!(slots[0].slot.row, 0);
                assert_eq!(slots[0].slot.column, 0);
                assert_eq!(slots[1].slot.row, 0);
                assert_eq!(slots[1].slot.column, 1);
            }
        }
        match &node.requirements {
            ChildFrameLayoutRequirements::Grid(requirements) => {
                assert_eq!(
                    requirements.grid.shape,
                    GridShape {
                        rows: 1,
                        columns: 2
                    }
                );
                assert_eq!(requirements.grid.column_widths.len(), 2);
                assert_eq!(requirements.grid.row_heights.len(), 1);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn layout_coordination_groups_facet_instances_across_repeat_cells()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let variables = repeat_vars(&["x", "y"]);
        let compiled = Plot::<RepeatGrid>::new()
            .data(grouped_xy_dataframe(&ctx))
            .rows(variables.clone())
            .columns(variables)
            .cell(repeated_facet_column_cell())
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 500.0, 260.0, &ctx).await?;
        let nodes = collect_child_frame_layout_coordination_nodes(&measurement)?;
        let facet_nodes = nodes
            .iter()
            .filter(|node| node.kind == ChildFrameContainerKind::FacetColumn)
            .collect::<Vec<_>>();
        assert_eq!(facet_nodes.len(), 4);
        assert!(facet_nodes.iter().all(|node| {
            node.template_key.container_path_template
                == vec![ContainerPathSegment::concat_child(0, Some("repeat_cell:*"))]
        }));

        let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);
        let facet_groups = diagnostics
            .merged_groups
            .iter()
            .filter(|group| group.key.kind == ChildFrameContainerKind::FacetColumn)
            .collect::<Vec<_>>();
        assert_eq!(facet_groups.len(), 1);
        assert_eq!(facet_groups[0].node_count, 4);
        Ok(())
    }

    #[tokio::test]
    async fn layout_coordination_groups_equivalent_manual_grid_facet_siblings()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let facet_child = || {
            Plot::<FacetColumn>::new().mark(
                Subplot::new(Plot::<Cartesian>::new().mark(line_mark(false))).column(col("group")),
            )
        };
        let compiled = Plot::<GridConcat>::new()
            .data(grouped_xy_dataframe(&ctx))
            .rows(1)
            .columns(2)
            .mark(
                Subplot::new(facet_child())
                    .grid_cell(0, 0)
                    .key("left_facets"),
            )
            .mark(
                Subplot::new(facet_child())
                    .grid_cell(0, 1)
                    .key("right_facets"),
            )
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 500.0, 180.0, &ctx).await?;
        let nodes = collect_child_frame_layout_coordination_nodes(&measurement)?;
        let facet_nodes = nodes
            .iter()
            .filter(|node| node.kind == ChildFrameContainerKind::FacetColumn)
            .collect::<Vec<_>>();
        assert_eq!(facet_nodes.len(), 2);
        assert_ne!(facet_nodes[0].instance_key, facet_nodes[1].instance_key);
        assert_eq!(
            facet_nodes[0].template_key, facet_nodes[1].template_key,
            "equivalent nested facets should ignore their immediate manual grid sibling key"
        );
        assert_eq!(
            facet_nodes[0].template_key.container_path_template,
            Vec::<ContainerPathSegment>::new()
        );
        assert_eq!(
            facet_nodes[0].template_key.semantic_tag.as_deref(),
            Some("facet:col:1:group")
        );

        let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);
        let facet_groups = diagnostics
            .merged_groups
            .iter()
            .filter(|group| group.key.kind == ChildFrameContainerKind::FacetColumn)
            .collect::<Vec<_>>();
        assert_eq!(facet_groups.len(), 1);
        assert_eq!(facet_groups[0].node_count, 2);
        Ok(())
    }

    #[tokio::test]
    async fn layout_coordination_keeps_different_manual_grid_facet_fields_separate()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let group_facet = Plot::<FacetColumn>::new().mark(
            Subplot::new(Plot::<Cartesian>::new().mark(line_mark(false))).column(col("group")),
        );
        let subgroup_facet = Plot::<FacetColumn>::new().mark(
            Subplot::new(Plot::<Cartesian>::new().mark(line_mark(false))).column(col("subgroup")),
        );
        let compiled = Plot::<GridConcat>::new()
            .data(nested_grouped_xy_dataframe(&ctx))
            .rows(1)
            .columns(2)
            .mark(
                Subplot::new(group_facet)
                    .grid_cell(0, 0)
                    .key("group_facets"),
            )
            .mark(
                Subplot::new(subgroup_facet)
                    .grid_cell(0, 1)
                    .key("subgroup_facets"),
            )
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 500.0, 180.0, &ctx).await?;
        let nodes = collect_child_frame_layout_coordination_nodes(&measurement)?;
        let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);
        let facet_groups = diagnostics
            .merged_groups
            .iter()
            .filter(|group| group.key.kind == ChildFrameContainerKind::FacetColumn)
            .collect::<Vec<_>>();
        assert!(
            facet_groups.is_empty(),
            "different facet field identities must not share one alignment group"
        );
        let facet_singletons = diagnostics
            .skipped_groups
            .iter()
            .filter(|group| group.key.kind == ChildFrameContainerKind::FacetColumn)
            .collect::<Vec<_>>();
        assert_eq!(facet_singletons.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn grid_layout_coordination_nodes_keep_unrelated_manual_grids_separate()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(manual_grid_plot()).key("left_grid"))
            .mark(Subplot::new(manual_grid_plot()).key("right_grid"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 400.0, 160.0, &ctx).await?;
        let nodes = collect_child_frame_layout_coordination_nodes(&measurement)?;
        let nodes = nodes
            .into_iter()
            .filter(|node| node.kind == ChildFrameContainerKind::GridConcat)
            .collect::<Vec<_>>();

        assert_eq!(nodes.len(), 2);
        assert_ne!(nodes[0].instance_key, nodes[1].instance_key);
        assert_ne!(nodes[0].template_key, nodes[1].template_key);
        assert_ne!(nodes[0].alignment_key(), nodes[1].alignment_key());
        Ok(())
    }

    #[tokio::test]
    async fn layout_coordination_nodes_export_degenerate_hconcat_and_vconcat()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let horizontal = keyed_hconcat("left", "right").compile(&ctx).await?;
        let horizontal_measurement = measurement_for_plot(&horizontal, 240.0, 100.0, &ctx).await?;
        let horizontal_nodes =
            collect_child_frame_layout_coordination_nodes(&horizontal_measurement)?;
        assert_eq!(horizontal_nodes.len(), 1);
        assert_eq!(horizontal_nodes[0].kind, ChildFrameContainerKind::HConcat);
        match &horizontal_nodes[0].topology {
            ChildFrameLayoutTopology::Grid { shape, slots } => {
                assert_eq!(
                    *shape,
                    GridShape {
                        rows: 1,
                        columns: 2
                    }
                );
                assert_eq!(slots.len(), 2);
                assert_eq!(slots[0].slot.row, 0);
                assert_eq!(slots[0].slot.column, 0);
                assert_eq!(slots[1].slot.row, 0);
                assert_eq!(slots[1].slot.column, 1);
            }
        }

        let vertical = Plot::<VConcat>::new()
            .mark(Subplot::new(zero_plot()).key("top"))
            .mark(Subplot::new(zero_plot()).key("bottom"))
            .compile(&ctx)
            .await?;
        let vertical_measurement = measurement_for_plot(&vertical, 120.0, 200.0, &ctx).await?;
        let vertical_nodes = collect_child_frame_layout_coordination_nodes(&vertical_measurement)?;
        assert_eq!(vertical_nodes.len(), 1);
        assert_eq!(vertical_nodes[0].kind, ChildFrameContainerKind::VConcat);
        match &vertical_nodes[0].topology {
            ChildFrameLayoutTopology::Grid { shape, slots } => {
                assert_eq!(
                    *shape,
                    GridShape {
                        rows: 2,
                        columns: 1
                    }
                );
                assert_eq!(slots.len(), 2);
                assert_eq!(slots[0].slot.row, 0);
                assert_eq!(slots[0].slot.column, 0);
                assert_eq!(slots[1].slot.row, 1);
                assert_eq!(slots[1].slot.column, 0);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn layout_coordination_nodes_export_wrap_concat_grid_topology()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = wrapped_zero_plot(5).columns(3).compile(&ctx).await?;

        let measurement = measurement_for_plot(&compiled, 300.0, 200.0, &ctx).await?;
        let nodes = collect_child_frame_layout_coordination_nodes(&measurement)?;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].kind, ChildFrameContainerKind::GridConcat);
        match &nodes[0].topology {
            ChildFrameLayoutTopology::Grid { shape, slots } => {
                assert_eq!(
                    *shape,
                    GridShape {
                        rows: 2,
                        columns: 3
                    }
                );
                assert_eq!(slots.len(), 5);
                assert_eq!(slots[0].slot.row, 0);
                assert_eq!(slots[0].slot.column, 0);
                assert_eq!(slots[3].slot.row, 1);
                assert_eq!(slots[3].slot.column, 0);
            }
        }
        Ok(())
    }

    #[test]
    fn layout_coordination_keeps_different_wrap_topologies_separate() {
        let nodes = vec![
            synthetic_wrap_alignment_node(
                GridShape {
                    rows: 2,
                    columns: 3,
                },
                "alpha",
            ),
            synthetic_wrap_alignment_node(
                GridShape {
                    rows: 3,
                    columns: 2,
                },
                "beta",
            ),
        ];

        let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);
        assert_eq!(diagnostics.exported_node_count, 2);
        assert_eq!(diagnostics.alignment_group_count, 2);
        assert!(diagnostics.merged_groups.is_empty());
        assert_eq!(diagnostics.skipped_groups.len(), 2);
    }

    #[test]
    fn grid_layout_coordination_groups_equivalent_spanned_topologies() {
        let shape = GridShape {
            rows: 3,
            columns: 3,
        };
        let slots = spanned_grid_topology_slots();
        let nodes = vec![
            synthetic_grid_alignment_node(shape, &slots, "alpha"),
            synthetic_grid_alignment_node(shape, &slots, "beta"),
        ];

        let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);
        assert_eq!(diagnostics.exported_node_count, 2);
        assert_eq!(diagnostics.alignment_group_count, 1);
        assert_eq!(diagnostics.merged_groups.len(), 1);
        assert_eq!(diagnostics.merged_groups[0].node_count, 2);
        assert!(diagnostics.skipped_groups.is_empty());
    }

    #[test]
    fn grid_layout_coordination_keeps_spanned_and_unspanned_topologies_separate() {
        let shape = GridShape {
            rows: 3,
            columns: 3,
        };
        let unspanned_slots = [
            GridSlot {
                row: 0,
                column: 0,
                row_span: 1,
                column_span: 1,
            },
            GridSlot {
                row: 0,
                column: 2,
                row_span: 1,
                column_span: 1,
            },
            GridSlot {
                row: 2,
                column: 0,
                row_span: 1,
                column_span: 1,
            },
            GridSlot {
                row: 2,
                column: 2,
                row_span: 1,
                column_span: 1,
            },
        ];
        let nodes = vec![
            synthetic_grid_alignment_node(shape, &spanned_grid_topology_slots(), "alpha"),
            synthetic_grid_alignment_node(shape, &unspanned_slots, "beta"),
        ];

        let diagnostics = build_child_frame_layout_alignment_diagnostics(&nodes);
        assert_eq!(diagnostics.exported_node_count, 2);
        assert_eq!(diagnostics.alignment_group_count, 2);
        assert!(diagnostics.merged_groups.is_empty());
        assert_eq!(diagnostics.skipped_groups.len(), 2);
    }

    #[tokio::test]
    async fn grid_layout_alignment_diagnostics_detect_repeat_in_facet_deltas()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "SELECT * FROM (VALUES
                    ('small', 1.0, 10.0),
                    ('small', 2.0, 20.0),
                    ('large', 100000.0, 1000000.0),
                    ('large', 200000.0, 2000000.0)
                ) AS t(group_name, a, b)",
            )
            .await?;
        let repeat = Plot::<RepeatGrid>::new()
            .rows(repeat_vars(&["a", "b"]))
            .columns(repeat_vars(&["a", "b"]))
            .cell(repeated_grid_cell());
        let compiled = Plot::<FacetColumn>::new()
            .data(df)
            .mark(Subplot::new(repeat).column(col("group_name")))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 420.0, 220.0, &ctx).await?;
        let diagnostics = diagnose_child_frame_layout_alignment(&measurement)?;

        assert_eq!(diagnostics.exported_node_count, 3);
        assert_eq!(diagnostics.alignment_group_count, 2);
        assert_eq!(diagnostics.skipped_groups.len(), 1);
        assert_eq!(diagnostics.merged_groups.len(), 1);
        let group = &diagnostics.merged_groups[0];
        assert_eq!(group.node_count, 2);
        assert!(
            group.node_deltas.iter().any(|delta| delta.has_delta()),
            "expected merged requirements to differ from at least one local repeat grid"
        );
        assert!(
            diagnostics.total_track_delta() > 0.0 || diagnostics.total_slab_delta() > 0.0,
            "expected repeat-in-facet diagnostics to report a nonzero merge delta"
        );
        Ok(())
    }

    fn semantic_child_domains<'a>(
        domains: &'a [HashMap<String, DomainCoordination>],
    ) -> Vec<GridSemanticChild<'a>> {
        semantic_child_domains_with_compatibility(domains, &[], &[])
    }

    fn semantic_child_domains_with_placements<'a>(
        domains: &'a [HashMap<String, DomainCoordination>],
        placements: &[GridPlacementConfig],
    ) -> Vec<GridSemanticChild<'a>> {
        semantic_child_domains_with_compatibility_and_placements(domains, &[], &[], placements)
    }

    fn semantic_child_domains_with_compatibility<'a>(
        domains: &'a [HashMap<String, DomainCoordination>],
        scale_types: &[HashMap<String, String>],
        axis_configs: &[HashMap<String, Vec<String>>],
    ) -> Vec<GridSemanticChild<'a>> {
        let placements = domains
            .iter()
            .enumerate()
            .map(|(slot_index, _)| GridPlacementConfig {
                row: slot_index / 2,
                column: slot_index % 2,
                row_span: 1,
                column_span: 1,
            })
            .collect::<Vec<_>>();
        semantic_child_domains_with_compatibility_and_placements(
            domains,
            scale_types,
            axis_configs,
            &placements,
        )
    }

    fn semantic_child_domains_with_compatibility_and_placements<'a>(
        domains: &'a [HashMap<String, DomainCoordination>],
        scale_types: &[HashMap<String, String>],
        axis_configs: &[HashMap<String, Vec<String>>],
        placements: &[GridPlacementConfig],
    ) -> Vec<GridSemanticChild<'a>> {
        domains
            .iter()
            .enumerate()
            .map(|(slot_index, domain)| GridSemanticChild {
                child_index: slot_index,
                placement: placements[slot_index],
                channel_domain_coordinations: domain,
                scale_type_signatures: scale_types.get(slot_index).cloned().unwrap_or_default(),
                axis_config_signatures: axis_configs.get(slot_index).cloned().unwrap_or_default(),
            })
            .collect()
    }

    fn named_domain(group: &str) -> DomainCoordination {
        DomainCoordination::named(CoordinationScope::Shared, group).unwrap()
    }

    #[test]
    fn semantic_axis_visibility_compacts_equivalent_domain_group_strips() {
        let domains = vec![
            HashMap::from([
                ("x".to_string(), named_domain("a")),
                ("y".to_string(), named_domain("a")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("b")),
                ("y".to_string(), named_domain("a")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("a")),
                ("y".to_string(), named_domain("b")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("b")),
                ("y".to_string(), named_domain("b")),
            ]),
        ];
        let semantic_children = semantic_child_domains(&domains);
        let config = semantic_axis_guide_visibility_for_child(
            AxisGuideVisibilityConfig::same(
                AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups,
            ),
            &semantic_children,
            0,
        );

        assert_eq!(
            config.vertical.labels,
            AxisGuideVisibilityPolicy::OuterEdges
        );
        assert_eq!(config.vertical.title, AxisGuideVisibilityPolicy::OuterEdges);
        assert_eq!(
            config.horizontal.labels,
            AxisGuideVisibilityPolicy::OuterEdges
        );
        assert_eq!(
            config.horizontal.title,
            AxisGuideVisibilityPolicy::OuterEdges
        );
    }

    #[test]
    fn semantic_axis_visibility_keeps_incompatible_or_free_strips_visible() {
        let domains = vec![
            HashMap::from([
                (
                    "x".to_string(),
                    DomainCoordination::named(CoordinationScope::Free, "a").unwrap(),
                ),
                ("y".to_string(), named_domain("a")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("b")),
                ("y".to_string(), named_domain("a")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("different")),
                ("y".to_string(), named_domain("b")),
            ]),
            HashMap::from([("y".to_string(), named_domain("b"))]),
        ];
        let semantic_children = semantic_child_domains(&domains);
        let config = semantic_axis_guide_visibility_for_child(
            AxisGuideVisibilityConfig::same(
                AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups,
            ),
            &semantic_children,
            0,
        );

        assert_eq!(config.vertical.labels, AxisGuideVisibilityPolicy::All);
        assert_eq!(config.vertical.title, AxisGuideVisibilityPolicy::All);
        assert_eq!(
            config.horizontal.labels,
            AxisGuideVisibilityPolicy::OuterEdges
        );
        assert_eq!(
            config.horizontal.title,
            AxisGuideVisibilityPolicy::OuterEdges
        );
    }

    #[test]
    fn semantic_axis_visibility_requires_compatible_scale_and_axis_config() {
        let domains = vec![
            HashMap::from([
                ("x".to_string(), named_domain("a")),
                ("y".to_string(), named_domain("a")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("b")),
                ("y".to_string(), named_domain("a")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("a")),
                ("y".to_string(), named_domain("b")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("b")),
                ("y".to_string(), named_domain("b")),
            ]),
        ];
        let scale_types = vec![
            HashMap::from([
                ("x".to_string(), "linear".to_string()),
                ("y".to_string(), "linear".to_string()),
            ]),
            HashMap::from([
                ("x".to_string(), "linear".to_string()),
                ("y".to_string(), "log".to_string()),
            ]),
            HashMap::from([
                ("x".to_string(), "linear".to_string()),
                ("y".to_string(), "linear".to_string()),
            ]),
            HashMap::from([
                ("x".to_string(), "linear".to_string()),
                ("y".to_string(), "linear".to_string()),
            ]),
        ];
        let axis_configs = vec![
            HashMap::from([("x".to_string(), vec!["tick_count=5".to_string()])]),
            HashMap::from([("x".to_string(), vec!["tick_count=5".to_string()])]),
            HashMap::from([("x".to_string(), vec!["tick_count=8".to_string()])]),
            HashMap::from([("x".to_string(), vec!["tick_count=5".to_string()])]),
        ];
        let semantic_children =
            semantic_child_domains_with_compatibility(&domains, &scale_types, &axis_configs);
        let config = semantic_axis_guide_visibility_for_child(
            AxisGuideVisibilityConfig::same(
                AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups,
            ),
            &semantic_children,
            0,
        );

        assert_eq!(config.vertical.labels, AxisGuideVisibilityPolicy::All);
        assert_eq!(config.vertical.title, AxisGuideVisibilityPolicy::All);
        assert_eq!(config.horizontal.labels, AxisGuideVisibilityPolicy::All);
        assert_eq!(config.horizontal.title, AxisGuideVisibilityPolicy::All);
    }

    #[test]
    fn semantic_axis_visibility_checks_spanned_axis_strips() {
        let domains = vec![
            HashMap::from([
                ("x".to_string(), named_domain("a")),
                ("y".to_string(), named_domain("a")),
            ]),
            HashMap::from([
                ("x".to_string(), named_domain("different-x")),
                ("y".to_string(), named_domain("different-y")),
            ]),
        ];
        let semantic_children = semantic_child_domains_with_placements(
            &domains,
            &[
                GridPlacementConfig {
                    row: 0,
                    column: 0,
                    row_span: 2,
                    column_span: 2,
                },
                GridPlacementConfig {
                    row: 1,
                    column: 1,
                    row_span: 1,
                    column_span: 1,
                },
            ],
        );
        let config = semantic_axis_guide_visibility_for_child(
            AxisGuideVisibilityConfig::same(
                AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups,
            ),
            &semantic_children,
            0,
        );

        assert_eq!(config.vertical.labels, AxisGuideVisibilityPolicy::All);
        assert_eq!(config.vertical.title, AxisGuideVisibilityPolicy::All);
        assert_eq!(config.horizontal.labels, AxisGuideVisibilityPolicy::All);
        assert_eq!(config.horizontal.title, AxisGuideVisibilityPolicy::All);
    }

    #[tokio::test]
    async fn grid_concat_requires_grid_cell_placement() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let result = Plot::<GridConcat>::new()
            .mark(Subplot::new(zero_plot()))
            .compile(&ctx)
            .await;
        let err = match result {
            Ok(_) => panic!("missing grid cell should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("GridConcat subplots require"));
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_rejects_duplicate_cells() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<GridConcat>::new()
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0))
            .mark(Subplot::new(zero_plot()).grid_cell(0, 0))
            .compile(&ctx)
            .await?;

        let err = measurement_for_plot(&compiled, 200.0, 100.0, &ctx)
            .await
            .expect_err("duplicate grid cell should fail");
        assert!(err.to_string().contains("multiple subplots assigned"));
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_child_event_bindings_are_targeted_to_child_scopes()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let top_left = child_scatter_plot(xy_dataframe(&ctx, vec![1.0], vec![1.0]), false)
            .event_binding(ChartEventBinding::on(ChartEventType::CursorMoved));
        let bottom_right = child_scatter_plot(xy_dataframe(&ctx, vec![2.0], vec![2.0]), false)
            .event_binding(ChartEventBinding::on(ChartEventType::CursorMoved));

        let compiled = Plot::<GridConcat>::new()
            .rows(2)
            .columns(2)
            .mark(Subplot::new(top_left).grid_cell(0, 0).key("top-left"))
            .mark(
                Subplot::new(bottom_right)
                    .grid_cell(1, 1)
                    .key("bottom-right"),
            )
            .compile(&ctx)
            .await?;

        let targets = compiled
            .event_bindings()
            .iter()
            .map(|binding| {
                binding
                    .scope_target
                    .as_ref()
                    .and_then(|target| target.resolved_coord_node_path_prefix())
                    .map(|target| target.to_vec())
            })
            .collect::<Vec<_>>();

        assert_eq!(targets, vec![Some(vec![0]), Some(vec![1])]);
        Ok(())
    }

    #[tokio::test]
    async fn wrap_concat_auto_columns_use_ceil_sqrt() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = wrapped_zero_plot(5).compile(&ctx).await?;

        let measurement = measurement_for_plot(&compiled, 300.0, 200.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("WrapConcat should measure as ConcatCoordMeasurement");
        assert_eq!(concat.band_direction(), None);
        assert_eq!(
            concat
                .children()
                .iter()
                .map(|child| child
                    .grid_placement
                    .map(|placement| (placement.row, placement.column)))
                .collect::<Vec<_>>(),
            vec![
                Some((0, 0)),
                Some((0, 1)),
                Some((0, 2)),
                Some((1, 0)),
                Some((1, 1))
            ]
        );

        let placement = concat.child_frame_placement();
        let origins = placement
            .placements()
            .iter()
            .map(|placement| (placement.id, placement.origin))
            .collect::<Vec<_>>();
        assert_eq!(
            origins,
            vec![
                (0, [0.0, 0.0]),
                (1, [100.0, 0.0]),
                (2, [200.0, 0.0]),
                (3, [0.0, 100.0]),
                (4, [100.0, 100.0]),
            ]
        );
        assert_eq!(placement.content_size, Size::new(300.0, 200.0));
        Ok(())
    }

    #[tokio::test]
    async fn wrap_concat_fixed_columns_place_children_row_major() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = wrapped_zero_plot(5).columns(2).compile(&ctx).await?;

        let measurement = measurement_for_plot(&compiled, 200.0, 300.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("WrapConcat should measure as ConcatCoordMeasurement");
        assert_eq!(
            concat
                .children()
                .iter()
                .map(|child| child
                    .grid_placement
                    .map(|placement| (placement.row, placement.column)))
                .collect::<Vec<_>>(),
            vec![
                Some((0, 0)),
                Some((0, 1)),
                Some((1, 0)),
                Some((1, 1)),
                Some((2, 0))
            ]
        );

        let placement = concat.child_frame_placement();
        let origins = placement
            .placements()
            .iter()
            .map(|placement| (placement.id, placement.origin))
            .collect::<Vec<_>>();
        assert_eq!(
            origins,
            vec![
                (0, [0.0, 0.0]),
                (1, [100.0, 0.0]),
                (2, [0.0, 100.0]),
                (3, [100.0, 100.0]),
                (4, [0.0, 200.0]),
            ]
        );
        assert_eq!(placement.content_size, Size::new(200.0, 300.0));
        Ok(())
    }

    #[tokio::test]
    async fn wrap_concat_trailing_holes_preserve_column_tracks() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = wrapped_zero_plot(1).columns(3).compile(&ctx).await?;

        let measurement = measurement_for_plot(&compiled, 300.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("WrapConcat should measure as ConcatCoordMeasurement");
        assert_eq!(
            concat
                .children()
                .iter()
                .map(|child| child
                    .grid_placement
                    .map(|placement| (placement.row, placement.column)))
                .collect::<Vec<_>>(),
            vec![Some((0, 0))]
        );

        let placement = concat.child_frame_placement();
        let origins = placement
            .placements()
            .iter()
            .map(|placement| (placement.id, placement.origin))
            .collect::<Vec<_>>();
        assert_eq!(origins, vec![(0, [0.0, 0.0])]);
        assert_eq!(
            placement
                .placements()
                .iter()
                .map(|placement| placement.meta.content_size_override)
                .collect::<Vec<_>>(),
            vec![None],
            "wrap cells should keep their measured one-slot plot area and leave trailing holes"
        );
        assert_eq!(placement.content_size, Size::new(300.0, 100.0));
        Ok(())
    }

    #[tokio::test]
    async fn wrap_concat_responsive_columns_change_with_width() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = wrapped_zero_plot(5)
            .responsive_columns(180.0)
            .compile(&ctx)
            .await?;

        let narrow = measurement_for_plot(&compiled, 520.0, 200.0, &ctx).await?;
        let narrow_concat = narrow
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("WrapConcat should measure as ConcatCoordMeasurement");
        assert_eq!(
            narrow_concat
                .children()
                .iter()
                .map(|child| child
                    .grid_placement
                    .map(|placement| (placement.row, placement.column)))
                .collect::<Vec<_>>(),
            vec![
                Some((0, 0)),
                Some((0, 1)),
                Some((0, 2)),
                Some((1, 0)),
                Some((1, 1))
            ]
        );

        let wide = measurement_for_plot(&compiled, 700.0, 200.0, &ctx).await?;
        let wide_concat = wide
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("WrapConcat should measure as ConcatCoordMeasurement");
        assert_eq!(
            wide_concat
                .children()
                .iter()
                .map(|child| child
                    .grid_placement
                    .map(|placement| (placement.row, placement.column)))
                .collect::<Vec<_>>(),
            vec![
                Some((0, 0)),
                Some((0, 1)),
                Some((0, 2)),
                Some((0, 3)),
                Some((1, 0))
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn hconcat_interaction_scopes_include_child_coord_path_prefixes()
    -> Result<(), AvengerChartError> {
        let ctx = Arc::new(SessionContext::new());
        let left = child_scatter_plot(xy_dataframe(&ctx, vec![1.0], vec![1.0]), false);
        let right = child_scatter_plot(xy_dataframe(&ctx, vec![2.0], vec![2.0]), false);

        let compiled = Arc::new(
            Plot::<HConcat>::new()
                .canvas_size(520.0, 240.0)
                .mark(Subplot::new(left).key("left"))
                .mark(Subplot::new(right).key("right"))
                .compile(&ctx)
                .await?,
        );
        let mut session = compiled.instantiate(ctx);
        let evaluated = session.evaluate(EvaluationRequest::new().exact()).await?;
        let mut paths = evaluated
            .interaction
            .scopes
            .iter()
            .map(|scope| scope.coord_node_path.clone())
            .collect::<Vec<_>>();
        paths.sort();

        assert_eq!(paths, vec![vec![0], vec![1]]);
        Ok(())
    }

    #[tokio::test]
    async fn concat_coordination_scope_hooks_group_children_by_container()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("left"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("right"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let left_scope = concat
            .child_scope_key(0)
            .expect("left child should have a scope key");
        let right_scope = concat
            .child_scope_key(1)
            .expect("right child should have a scope key");

        let shared_left =
            CoordinationScopeKey::child_frame_container(CoordinationKind::ScaleDomain, &left_scope)
                .with_channel("x");
        let shared_right = CoordinationScopeKey::child_frame_container(
            CoordinationKind::ScaleDomain,
            &right_scope,
        )
        .with_channel("x");
        let free_left =
            CoordinationScopeKey::child_frame(CoordinationKind::ScaleDomain, &left_scope)
                .with_channel("x");
        let free_right =
            CoordinationScopeKey::child_frame(CoordinationKind::ScaleDomain, &right_scope)
                .with_channel("x");

        assert_eq!(shared_left, shared_right);
        assert_ne!(free_left, free_right);

        let legend_left = CoordinationScopeKey::child_frame_container(
            CoordinationKind::LegendOwnership,
            &left_scope,
        )
        .with_channel("fill:Right");
        let legend_right = CoordinationScopeKey::child_frame_container(
            CoordinationKind::LegendOwnership,
            &right_scope,
        )
        .with_channel("fill:Right");
        assert_eq!(legend_left, legend_right);

        let horizontal_lane = CoordinationScopeKey::container_lane(
            CoordinationKind::GuideLane,
            CoordinationAxis::Horizontal,
            vec![],
        );
        let vertical_lane = CoordinationScopeKey::container_lane(
            CoordinationKind::GuideLane,
            CoordinationAxis::Vertical,
            vec![],
        );
        assert_ne!(horizontal_lane, vertical_lane);
        Ok(())
    }

    #[tokio::test]
    async fn nested_concat_child_scopes_include_outer_concat_child() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(keyed_hconcat("inner-left", "inner-right")).key("outer-left"))
            .mark(Subplot::new(keyed_hconcat("inner-left", "inner-right")).key("outer-right"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 320.0, 120.0, &ctx).await?;
        let outer = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("outer HConcat should measure as ConcatCoordMeasurement");
        let left_inner = outer.children()[0]
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("left child should contain an inner concat measurement");
        let right_inner = outer.children()[1]
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("right child should contain an inner concat measurement");

        let left_inner_scope = left_inner
            .child_scope_key(0)
            .expect("left inner child should have a scope key");
        let right_inner_scope = right_inner
            .child_scope_key(0)
            .expect("right inner child should have a scope key");
        assert_eq!(
            left_inner_scope.container_path,
            vec![ContainerPathSegment::concat_child(0, Some("outer-left"))]
        );
        assert_eq!(
            right_inner_scope.container_path,
            vec![ContainerPathSegment::concat_child(1, Some("outer-right"))]
        );
        assert_ne!(left_inner_scope, right_inner_scope);
        Ok(())
    }

    #[tokio::test]
    async fn facet_child_scopes_include_existing_concat_container_path()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<FacetColumn>::new()
            .data(grouped_xy_dataframe(&ctx))
            .mark(
                Subplot::new(Plot::<Cartesian>::new().mark(line_mark(false))).column(col("group")),
            )
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot_with_container_path(
            &compiled,
            320.0,
            120.0,
            &ctx,
            &[ContainerPathSegment::concat_child(0, Some("faceted-child"))],
        )
        .await?;
        let facet = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("FacetColumn should measure as FacetBandCoordMeasurement");
        let first_cell_scope = facet
            .child_scope_key(0)
            .expect("facet cell should have a scope key");
        assert_eq!(
            first_cell_scope.container_path,
            vec![ContainerPathSegment::concat_child(0, Some("faceted-child"))]
        );
        Ok(())
    }

    #[tokio::test]
    async fn concat_inside_facet_child_scopes_include_outer_facet_cell()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let concat_child = keyed_hconcat("inner-left", "inner-right");
        let compiled = Plot::<FacetColumn>::new()
            .data(grouped_xy_dataframe(&ctx))
            .mark(Subplot::new(concat_child).column(col("group")))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 320.0, 120.0, &ctx).await?;
        let facet = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("FacetColumn should measure as FacetBandCoordMeasurement");
        let first_cell = facet
            .cells
            .first()
            .expect("facet should have at least one cell");
        let inner_concat = first_cell
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("facet cell should contain a concat measurement");
        let inner_scope = inner_concat
            .child_scope_key(0)
            .expect("inner concat child should have a scope key");
        assert_eq!(
            inner_scope.container_path,
            vec![ContainerPathSegment::facet_value(
                FacetAxis::Column,
                1,
                first_cell.plan.value.clone(),
            )]
        );
        Ok(())
    }

    #[tokio::test]
    async fn nested_facet_child_scopes_do_not_duplicate_outer_facet_cell()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let inner = Plot::<FacetRow>::new().mark(
            Subplot::new(Plot::<Cartesian>::new().mark(line_mark(false))).row(col("subgroup")),
        );
        let compiled = Plot::<FacetColumn>::new()
            .data(nested_grouped_xy_dataframe(&ctx))
            .mark(Subplot::new(inner).column(col("group")))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 320.0, 160.0, &ctx).await?;
        let outer_facet = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("outer FacetColumn should measure as FacetBandCoordMeasurement");
        let outer_cell = outer_facet
            .cells
            .first()
            .expect("outer facet should have at least one cell");
        let inner_facet = outer_cell
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("outer facet cell should contain an inner facet measurement");
        let inner_scope = inner_facet
            .child_scope_key(0)
            .expect("inner facet cell should have a scope key");

        assert_eq!(
            inner_scope.container_path,
            vec![ContainerPathSegment::facet_value(
                FacetAxis::Column,
                1,
                outer_cell.plan.value.clone(),
            )]
        );
        assert_eq!(
            &inner_scope.child_key,
            &ChildFrameKey::FacetValue {
                axis: FacetAxis::Row,
                level: 2,
                value: inner_facet.cells[0].plan.value.clone(),
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn vconcat_measurement_places_children_on_vertical_band() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = Plot::<VConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("top"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("bottom"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 120.0, 200.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("VConcat should measure as ConcatCoordMeasurement");
        assert_eq!(concat.band_direction(), Some(Orientation::Vertical));

        let container = measurement
            .child_frame_container_view()?
            .expect("concat measurement should expose a child-frame container view");
        assert_eq!(container.placement().placements()[0].origin, [0.0, 0.0]);
        assert_eq!(container.placement().placements()[1].origin[0], 0.0);
        assert!(container.placement().placements()[1].origin[1] > 0.0);
        Ok(())
    }

    #[tokio::test]
    async fn concat_measurement_preserves_sparse_mark_indexes() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("first"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 100.0, 100.0, &ctx).await?;
        let container = measurement
            .child_frame_container_view()?
            .expect("concat measurement should expose a child-frame container view");
        assert!(container.child_measurement(0).is_some());
        assert!(container.child_measurement(1).is_none());
        Ok(())
    }

    #[tokio::test]
    async fn hconcat_renders_child_subplot_groups() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("left"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("right"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let left_path = group_names
            .get("concat_subplot_0_left")
            .expect("left subplot group should render");
        let right_path = group_names
            .get("concat_subplot_1_right")
            .expect("right subplot group should render");
        let left_origin = evaluated
            .scene_graph
            .get_absolute_origin(left_path)
            .expect("left subplot should have an absolute origin");
        let right_origin = evaluated
            .scene_graph
            .get_absolute_origin(right_path)
            .expect("right subplot should have an absolute origin");

        assert_eq!(left_origin[1], right_origin[1]);
        assert!(
            right_origin[0] > left_origin[0],
            "horizontal concat should place the second subplot to the right"
        );
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_rendering_inherits_parent_data() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![3.0, 4.0]);
        let child_plot = Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")));
        let compiled = Plot::<HConcat>::new()
            .data(data)
            .mark(Subplot::new(child_plot).key("points"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let child_path = group_names
            .get("concat_subplot_0_points")
            .expect("child subplot group should render");
        let child_group = evaluated
            .scene_graph
            .get_mark(child_path)
            .expect("child subplot group path should resolve");
        let symbol = find_symbol_mark(child_group).expect("child subplot should render symbols");

        assert_eq!(symbol.len, 2);
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_rendering_preserves_explicit_child_data() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let parent_data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![3.0, 4.0]);
        let child_data = xy_dataframe(&ctx, vec![5.0], vec![6.0]);
        let child_plot = Plot::<Cartesian>::new()
            .data(child_data)
            .mark(Symbol::new().x(col("x")).y(col("y")));
        let compiled = Plot::<HConcat>::new()
            .data(parent_data)
            .mark(Subplot::new(child_plot).key("points"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let child_path = group_names
            .get("concat_subplot_0_points")
            .expect("child subplot group should render");
        let child_group = evaluated
            .scene_graph
            .get_mark(child_path)
            .expect("child subplot group path should resolve");
        let symbol = find_symbol_mark(child_group).expect("child subplot should render symbols");

        assert_eq!(symbol.len, 1);
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_rendering_preserves_nested_facet_child() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let data = grouped_xy_dataframe(&ctx);
        let facet_child = Plot::<FacetColumn>::new().data(data).mark(
            Subplot::new(Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y"))))
                .column(col("group")),
        );
        let compiled = Plot::<HConcat>::new()
            .canvas_size(500.0, 220.0)
            .mark(Subplot::new(facet_child).key("faceted"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 500.0, 220.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let facet = concat.children()[0]
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("child should measure as FacetBandCoordMeasurement");
        assert_eq!(facet.cells.len(), 2);

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let child_path = group_names
            .get("concat_subplot_0_faceted")
            .expect("nested facet subplot group should render");
        let child_group = evaluated
            .scene_graph
            .get_mark(child_path)
            .expect("nested facet group path should resolve");
        let symbol = find_symbol_mark(child_group).expect("nested facet should render symbols");

        assert_eq!(symbol.len, 2);
        assert_eq!(
            evaluated.interaction.scopes.len(),
            2,
            "nested facet coordinate scopes should propagate through concat"
        );
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_domains_are_free_by_default() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let left_data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![1.0, 2.0]);
        let right_data = xy_dataframe(&ctx, vec![100.0, 101.0], vec![1.0, 2.0]);
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(child_scatter_plot(left_data, false)).key("left"))
            .mark(Subplot::new(child_scatter_plot(right_data, false)).key("right"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 400.0, 160.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let left_domain = child_x_domain(&concat.children()[0]);
        let right_domain = child_x_domain(&concat.children()[1]);

        assert_ne!(left_domain, right_domain);
        assert!(left_domain.1 < 50.0);
        assert!(right_domain.1 > 50.0);
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_domains_share_when_channel_requests_sharing()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let left_data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![1.0, 2.0]);
        let right_data = xy_dataframe(&ctx, vec![100.0, 101.0], vec![1.0, 2.0]);
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(child_scatter_plot(left_data, true)).key("left"))
            .mark(Subplot::new(child_scatter_plot(right_data, true)).key("right"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 400.0, 160.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let left_domain = child_x_domain(&concat.children()[0]);
        let right_domain = child_x_domain(&concat.children()[1]);

        assert_eq!(left_domain, right_domain);
        assert!(left_domain.0 <= 1.0);
        assert!(left_domain.1 >= 101.0);
        Ok(())
    }

    #[tokio::test]
    async fn grid_concat_named_domain_group_links_x_and_y_domains() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let x_child = Plot::<Cartesian>::new()
            .data(xy_dataframe(&ctx, vec![1.0, 2.0], vec![0.0, 1.0]))
            .mark(
                Line::new()
                    .x_with(col("x"), |c| {
                        c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                            .with_domain_group("measurement")
                            .share_domain()
                    })
                    .y(col("y")),
            );
        let y_child = Plot::<Cartesian>::new()
            .data(xy_dataframe(&ctx, vec![0.0, 1.0], vec![100.0, 101.0]))
            .mark(Line::new().x(col("x")).y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_group("measurement")
                    .share_domain()
            }));
        let compiled = Plot::<GridConcat>::new()
            .rows(1)
            .columns(2)
            .mark(Subplot::new(x_child).grid_cell(0, 0).key("x-child"))
            .mark(Subplot::new(y_child).grid_cell(0, 1).key("y-child"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 420.0, 180.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("GridConcat should measure as ConcatCoordMeasurement");
        let x_domain = child_x_domain(&concat.children()[0]);
        let y_domain = measurement_y_domain(&concat.children()[1].measurement);

        assert_eq!(x_domain, (1.0, 101.0));
        assert!(
            y_domain.0 < 50.0 && y_domain.1 >= 101.0,
            "named domain group should make the y scale include the remote x-domain extent, got \
             {y_domain:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn concat_and_facet_shared_domains_use_equivalent_unions() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let left_data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![1.0, 2.0]);
        let right_data = xy_dataframe(&ctx, vec![100.0, 101.0], vec![1.0, 2.0]);
        let concat_compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(line_child_plot(left_data, true)).key("left"))
            .mark(Subplot::new(line_child_plot(right_data, true)).key("right"))
            .compile(&ctx)
            .await?;

        let grouped_data = grouped_xy_dataframe(&ctx);
        let facet_compiled = Plot::<FacetColumn>::new()
            .data(grouped_data)
            .mark(Subplot::new(Plot::<Cartesian>::new().mark(line_mark(true))).column(col("group")))
            .compile(&ctx)
            .await?;

        let concat_measurement = measurement_for_plot(&concat_compiled, 400.0, 160.0, &ctx).await?;
        let concat = concat_measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let concat_domains = concat
            .children()
            .iter()
            .map(child_x_domain)
            .collect::<Vec<_>>();

        let facet_measurement = measurement_for_plot(&facet_compiled, 400.0, 160.0, &ctx).await?;
        let facet = facet_measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("FacetColumn should measure as FacetBandCoordMeasurement");
        let facet_domains = facet
            .cells
            .iter()
            .map(|cell| measurement_x_domain(&cell.measurement))
            .collect::<Vec<_>>();

        assert_eq!(concat_domains.len(), 2);
        assert_eq!(facet_domains.len(), 2);
        assert_eq!(concat_domains[0], concat_domains[1]);
        assert_eq!(facet_domains[0], facet_domains[1]);
        assert_eq!(concat_domains[0], facet_domains[0]);
        assert_eq!(concat_domains[0], (1.0, 101.0));
        Ok(())
    }

    #[tokio::test]
    async fn vconcat_renders_child_subplot_groups() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<VConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("top"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("bottom"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let top_path = group_names
            .get("concat_subplot_0_top")
            .expect("top subplot group should render");
        let bottom_path = group_names
            .get("concat_subplot_1_bottom")
            .expect("bottom subplot group should render");
        let top_origin = evaluated
            .scene_graph
            .get_absolute_origin(top_path)
            .expect("top subplot should have an absolute origin");
        let bottom_origin = evaluated
            .scene_graph
            .get_absolute_origin(bottom_path)
            .expect("bottom subplot should have an absolute origin");

        assert_eq!(top_origin[0], bottom_origin[0]);
        assert!(
            bottom_origin[1] > top_origin[1],
            "vertical concat should place the second subplot below the first"
        );
        Ok(())
    }
}
