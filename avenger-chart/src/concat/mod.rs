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
        BandChildFrameInput, BandChildFramePlacement, BandSpacing, BoundaryDemand1D, ChildFrameKey,
        ChildFramePlacementResult, ChildFrameRenderPlacement, ChildFrameScopeKey,
        ChildFrameSharingLevel, ContainerPathSegment,
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
    layout::{BandDirection, LayoutBounds, Size2D},
    marks::{CompiledMark, CompiledMarkCore},
    plot::compiled::{
        ChildFrameDataSelection, ChildFrameDomainSharingInput, ChildFrameRuntime,
        ComponentsMeasurement, ContainerLabelPlacement, PreparedChildFramePlot,
        child_frame_container_view_from_concat, container_path_without_facet_segments,
        coordinated_child_frame_domain_extents, measure_child_frame_container_guide_overflow,
        render_child_frame_container_guide_labels,
    },
    render::EvaluationContext,
    scales::{DomainExtent, ScaleRangeBinding},
    theme::Theme,
};
use avenger_chart_core::{
    AxisGuideVisibilityConfig, AxisGuideVisibilityPolicy, DefaultLogicalExprNodeExt, ExprHelpers,
    FacetWrapColumnMode, IntoExpr, contains_aggregate, params_to_datafusion,
};
use datafusion_proto::protobuf::LogicalExprNode;

use subplot::GridPlacementConfig;

pub use subplot::{CompiledConcatSubplot, compiled_subplot};

/// Horizontal concatenation of `Subplot` marks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HConcat;

impl HConcat {
    pub fn new() -> Self {
        Self
    }
}

/// Vertical concatenation of `Subplot` marks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VConcat;

impl VConcat {
    pub fn new() -> Self {
        Self
    }
}

/// Explicit two-dimensional concatenation of `Subplot` marks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GridConcat {
    rows: Option<usize>,
    columns: Option<usize>,
    #[serde(default)]
    axis_guide_visibility: AxisGuideVisibilityConfig,
}

/// Row-major wrapped concatenation of `Subplot` marks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapConcat {
    column_mode: FacetWrapColumnMode,
    #[serde(default)]
    axis_guide_visibility: AxisGuideVisibilityConfig,
}

impl Default for WrapConcat {
    fn default() -> Self {
        Self {
            column_mode: FacetWrapColumnMode::Auto,
            axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
        }
    }
}

impl WrapConcat {
    pub fn new() -> Self {
        Self::default()
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

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = Some(rows);
        self
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = Some(columns);
        self
    }

    pub fn axis_guide_visibility(mut self, policy: AxisGuideVisibilityPolicy) -> Self {
        self.axis_guide_visibility = AxisGuideVisibilityConfig::same(policy);
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

#[derive(Clone, Debug)]
pub(crate) enum ConcatChildPlacement {
    Band(BandChildFramePlacement),
    Grid {
        placement: ChildFramePlacementResult,
        shape: GridShape,
    },
}

impl ConcatChildPlacement {
    fn child_frame_placement(&self, fallback_content_size: Size2D) -> ChildFramePlacementResult {
        match self {
            Self::Band(band) => {
                band.to_child_frame_placement_result([0.0, 0.0], fallback_content_size)
            }
            Self::Grid { placement, .. } => placement.clone(),
        }
    }

    fn band_direction(&self) -> Option<BandDirection> {
        match self {
            Self::Band(band) => Some(band.direction),
            Self::Grid { .. } => None,
        }
    }

    fn grid_shape(&self) -> Option<GridShape> {
        match self {
            Self::Band(_) => None,
            Self::Grid { shape, .. } => Some(*shape),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConcatCoordMeasurement {
    pub(crate) children: Vec<ConcatChildMeasurement>,
    pub(crate) placement: ConcatChildPlacement,
    pub(crate) fallback_content_size: Size2D,
    pub(crate) axis_guide_visibility: AxisGuideVisibilityConfig,
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

    pub(crate) fn child_frame_placement(&self) -> ChildFramePlacementResult {
        self.placement
            .child_frame_placement(self.fallback_content_size)
    }

    pub(crate) fn band_direction(&self) -> Option<BandDirection> {
        self.placement.band_direction()
    }

    pub(crate) fn grid_shape(&self) -> Option<GridShape> {
        self.placement.grid_shape()
    }

    pub(crate) fn axis_guide_visibility_config(&self) -> AxisGuideVisibilityConfig {
        self.axis_guide_visibility
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

#[derive(Clone, Debug)]
pub(crate) struct ConcatChildMeasurement {
    pub(crate) child_index: usize,
    pub(crate) key: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) grid_placement: Option<GridPlacementConfig>,
    pub(crate) container_path: Vec<ContainerPathSegment>,
    pub(crate) local_facet_tree: Option<Arc<EvaluatedFacetTree>>,
    pub(crate) facet_data_root: Option<DataFrame>,
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
        BandDirection::Horizontal => Some(ContainerLabelPlacement::Top),
        BandDirection::Vertical => Some(ContainerLabelPlacement::Left),
    }
}

fn child_plot_area_size(
    direction: BandDirection,
    plot_width: f32,
    plot_height: f32,
    child_count: usize,
) -> Size2D {
    let child_count = child_count.max(1) as f32;
    match direction {
        BandDirection::Horizontal => Size2D::new(plot_width / child_count, plot_height),
        BandDirection::Vertical => Size2D::new(plot_width, plot_height / child_count),
    }
}

fn boundary_demand_for_child(
    direction: BandDirection,
    measurement: &ComponentsMeasurement,
) -> BoundaryDemand1D {
    let slabs = measurement.frame_demand().rendered_envelope;
    match direction {
        BandDirection::Horizontal => BoundaryDemand1D {
            before: slabs.left,
            after: slabs.right,
        },
        BandDirection::Vertical => BoundaryDemand1D {
            before: slabs.top,
            after: slabs.bottom,
        },
    }
}

fn band_input_for_child(
    direction: BandDirection,
    child: &ConcatChildMeasurement,
) -> BandChildFrameInput {
    let main_axis_size = match direction {
        BandDirection::Horizontal => child.measurement.plot_area_width,
        BandDirection::Vertical => child.measurement.plot_area_height,
    };
    let cross_axis_size = match direction {
        BandDirection::Horizontal => child.measurement.plot_area_height,
        BandDirection::Vertical => child.measurement.plot_area_width,
    };

    BandChildFrameInput {
        child_index: child.child_index,
        main_axis_size,
        cross_axis_size,
        boundary: boundary_demand_for_child(direction, &child.measurement),
    }
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

    fn sharing_level(
        &self,
        direction: BandDirection,
        child_count: usize,
    ) -> ChildFrameSharingLevel {
        match direction {
            BandDirection::Horizontal => {
                ChildFrameSharingLevel::hconcat_child(self.child_index(), child_count, self.key())
            }
            BandDirection::Vertical => {
                ChildFrameSharingLevel::vconcat_child(self.child_index(), child_count, self.key())
            }
        }
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
    child_plot_area: Size2D,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    coordinated_domain_extents: &HashMap<String, DomainExtent>,
    facet_scoped_domain_extents: &HashMap<String, DomainExtent>,
) -> Result<ConcatChildMeasurement, AvengerChartError> {
    let runtime = ChildFrameRuntime::new();
    let child_layout_spec =
        runtime.fixed_plot_area_layout_spec(child_plot_area.width, child_plot_area.height);
    let mut sharing_levels = sharing_levels.into_iter();
    let first_level = sharing_levels.next().ok_or_else(|| {
        AvengerChartError::InternalError("Concat child measurement requires a sharing level".into())
    })?;
    let mut child_eval_ctx = runtime.eval_context(eval_ctx, first_level);
    for level in sharing_levels {
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
        measurement,
    })
}

pub(crate) async fn measure_concat_coord_system(
    direction: BandDirection,
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
    let child_plot_area = child_plot_area_size(direction, plot_width, plot_height, subplots.len());

    let mut prepared_children = Vec::with_capacity(subplots.len());
    for subplot in subplots {
        prepared_children.push(Box::pin(prepare_concat_child(subplot, eval_ctx, data)).await?);
    }

    let coordinated_domain_extents =
        coordinated_domain_extents_for_concat_children(&prepared_children);

    let mut children = Vec::with_capacity(prepared_children.len());
    let child_count = prepared_children.len();
    for (prepared, coordinated_extents) in prepared_children
        .iter()
        .zip(coordinated_domain_extents.iter())
    {
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

    let inputs = children
        .iter()
        .map(|child| band_input_for_child(direction, child))
        .collect::<Vec<_>>();
    let child_band_layout =
        BandChildFramePlacement::from_sized_children(direction, &inputs, BandSpacing::default());

    Ok(Box::new(ConcatCoordMeasurement {
        children,
        placement: ConcatChildPlacement::Band(child_band_layout),
        fallback_content_size: Size2D::new(plot_width, plot_height),
        axis_guide_visibility: AxisGuideVisibilityConfig::auto(),
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
    let base_child_plot_area = Size2D::new(
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
        let child_plot_area = Size2D::new(
            base_child_plot_area.width * placement.column_span as f32,
            base_child_plot_area.height * placement.row_span as f32,
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
                grid_concat_sharing_levels(
                    prepared.child_index(),
                    prepared.key(),
                    placement,
                    &guide_sharing_slots,
                    grid.axis_guide_visibility_config(),
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

    let placement = grid_child_frame_placement(&children, grid_shape, base_child_plot_area)?;
    Ok(Box::new(ConcatCoordMeasurement {
        children,
        placement: ConcatChildPlacement::Grid {
            placement,
            shape: grid_shape,
        },
        fallback_content_size: Size2D::new(plot_width, plot_height),
        axis_guide_visibility: grid.axis_guide_visibility_config(),
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
    let child_plot_area = Size2D::new(
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
            grid_concat_sharing_levels(
                prepared.child_index(),
                prepared.key(),
                GridPlacementConfig {
                    row: slot_index / columns,
                    column: slot_index % columns,
                    row_span: 1,
                    column_span: 1,
                },
                &guide_sharing_slots,
                wrap.axis_guide_visibility_config(),
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

    let placement = grid_child_frame_placement(&children, grid_shape, child_plot_area)?;
    Ok(Box::new(ConcatCoordMeasurement {
        children,
        placement: ConcatChildPlacement::Grid {
            placement,
            shape: grid_shape,
        },
        fallback_content_size: Size2D::new(plot_width, plot_height),
        axis_guide_visibility: wrap.axis_guide_visibility_config(),
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

pub(crate) fn grid_concat_sharing_levels(
    child_index: usize,
    key: Option<&str>,
    placement: GridPlacementConfig,
    slots: &GridGuideSharingSlots,
    axis_guide_visibility: AxisGuideVisibilityConfig,
) -> Vec<ChildFrameSharingLevel> {
    let row_index = slots.row_slot_index(placement.column, placement.row);
    let row_count = slots.row_slot_count(placement.column);
    let column_index = slots.column_slot_index(placement.row, placement.column);
    let column_count = slots.column_slot_count(placement.row);
    vec![
        ChildFrameSharingLevel::grid_concat_row(child_index, key, row_index, row_count)
            .with_axis_guide_visibility(axis_guide_visibility),
        ChildFrameSharingLevel::grid_concat_column(column_index, column_count)
            .with_axis_guide_visibility(axis_guide_visibility),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GridShape {
    pub(crate) rows: usize,
    pub(crate) columns: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GridGuideSharingSlots {
    rows_by_column: Vec<Vec<usize>>,
    columns_by_row: Vec<Vec<usize>>,
}

impl GridGuideSharingSlots {
    pub(crate) fn from_placements(
        shape: GridShape,
        placements: impl IntoIterator<Item = GridPlacementConfig>,
    ) -> Self {
        let mut rows_by_column = vec![Vec::<usize>::new(); shape.columns];
        let mut columns_by_row = vec![Vec::<usize>::new(); shape.rows];

        for placement in placements {
            if placement.column < shape.columns
                && !rows_by_column[placement.column].contains(&placement.row)
            {
                rows_by_column[placement.column].push(placement.row);
            }
            if placement.row < shape.rows
                && !columns_by_row[placement.row].contains(&placement.column)
            {
                columns_by_row[placement.row].push(placement.column);
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
        if placement.row_span != 1 || placement.column_span != 1 {
            return Err(AvengerChartError::InvalidArgument(
                "GridConcat row/column spans are not supported yet".to_string(),
            ));
        }
        inferred_rows = inferred_rows.max(placement.row + placement.row_span);
        inferred_columns = inferred_columns.max(placement.column + placement.column_span);
        for row in placement.row..placement.row + placement.row_span {
            for column in placement.column..placement.column + placement.column_span {
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
        if placement.row + placement.row_span > rows
            || placement.column + placement.column_span > columns
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "GridConcat subplot at row {}, column {} exceeds configured grid size {rows}x{columns}",
                placement.row, placement.column
            )));
        }
    }

    Ok(GridShape {
        rows: rows.max(1),
        columns: columns.max(1),
    })
}

fn grid_child_frame_placement(
    children: &[ConcatChildMeasurement],
    shape: GridShape,
    base_cell_size: Size2D,
) -> Result<ChildFramePlacementResult, AvengerChartError> {
    let mut column_widths = vec![base_cell_size.width; shape.columns];
    let mut row_heights = vec![base_cell_size.height; shape.rows];
    let mut column_left = vec![0.0f32; shape.columns];
    let mut column_right = vec![0.0f32; shape.columns];
    let mut row_top = vec![0.0f32; shape.rows];
    let mut row_bottom = vec![0.0f32; shape.rows];

    for child in children {
        let placement = child.grid_placement.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Missing grid placement for child {}",
                child.child_index
            ))
        })?;
        let slabs = child.measurement.frame_demand().rendered_envelope;
        column_widths[placement.column] =
            column_widths[placement.column].max(child.measurement.plot_area_width);
        row_heights[placement.row] =
            row_heights[placement.row].max(child.measurement.plot_area_height);
        column_left[placement.column] = column_left[placement.column].max(slabs.left);
        column_right[placement.column] = column_right[placement.column].max(slabs.right);
        row_top[placement.row] = row_top[placement.row].max(slabs.top);
        row_bottom[placement.row] = row_bottom[placement.row].max(slabs.bottom);
    }

    let mut column_starts = vec![0.0f32; shape.columns];
    let mut cursor = 0.0f32;
    for column in 0..shape.columns {
        if column > 0 {
            cursor += column_right[column - 1] + column_left[column];
        }
        column_starts[column] = cursor;
        cursor += column_widths[column];
    }
    let content_width = cursor;

    let mut row_starts = vec![0.0f32; shape.rows];
    let mut cursor = 0.0f32;
    for row in 0..shape.rows {
        if row > 0 {
            cursor += row_bottom[row - 1] + row_top[row];
        }
        row_starts[row] = cursor;
        cursor += row_heights[row];
    }
    let content_height = cursor;

    let render_placements = children
        .iter()
        .map(|child| {
            let placement = child.grid_placement.expect("validated above");
            ChildFrameRenderPlacement {
                child_index: child.child_index,
                origin: [column_starts[placement.column], row_starts[placement.row]],
            }
        })
        .collect();

    Ok(ChildFramePlacementResult::new(
        Size2D::new(content_width, content_height),
        render_placements,
    ))
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
    use std::sync::Arc;

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
        layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
        marks::{Subplot, line::Line, symbol::Symbol},
        plot::{
            CompiledPlot, Plot,
            compiled::{
                CoordinationKind, CoordinationScopeKey, EvaluationRequest,
                child_frame_container_view_from_concat,
                container_label_items_from_child_frame_container,
                scale_provider::DynamicScaleProvider, scales::build_scale_builder_from_marks,
            },
        },
        render::EvaluationContext,
        scales::{Linear, LinearScaleExt, ScaleChannelConfig},
        zerod::ZeroDCoord,
    };
    use avenger_chart_core::{
        ChartEventBinding, ChartEventType, CoordinationAxis, CoordinationScope,
    };

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
        assert_eq!(concat.band_direction(), Some(BandDirection::Horizontal));
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
        assert_eq!(container.placement().render_placements().len(), 2);
        assert_eq!(
            container.placement().render_placements()[0].origin,
            [0.0, 0.0]
        );
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
            .render_placements()
            .iter()
            .map(|placement| (placement.child_index, placement.origin))
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
        assert_eq!(placement.content_size, Size2D::new(200.0, 100.0));
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
            .render_placements()
            .iter()
            .map(|placement| (placement.child_index, placement.origin))
            .collect::<Vec<_>>();
        assert_eq!(origins, vec![(0, [0.0, 0.0]), (1, [200.0, 100.0])]);
        assert_eq!(placement.content_size, Size2D::new(300.0, 200.0));
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
            .render_placements()
            .iter()
            .map(|placement| (placement.child_index, placement.origin))
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
        assert_eq!(placement.content_size, Size2D::new(300.0, 200.0));
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
            .render_placements()
            .iter()
            .map(|placement| (placement.child_index, placement.origin))
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
        assert_eq!(placement.content_size, Size2D::new(200.0, 300.0));
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
            .render_placements()
            .iter()
            .map(|placement| (placement.child_index, placement.origin))
            .collect::<Vec<_>>();
        assert_eq!(origins, vec![(0, [0.0, 0.0])]);
        assert_eq!(placement.content_size, Size2D::new(300.0, 100.0));
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
        assert_eq!(concat.band_direction(), Some(BandDirection::Vertical));

        let container = measurement
            .child_frame_container_view()?
            .expect("concat measurement should expose a child-frame container view");
        assert_eq!(
            container.placement().render_placements()[0].origin,
            [0.0, 0.0]
        );
        assert_eq!(container.placement().render_placements()[1].origin[0], 0.0);
        assert!(container.placement().render_placements()[1].origin[1] > 0.0);
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
