//! Concatenation coordinate systems.
//!
//! `HConcat` and `VConcat` are container coordinate systems: their marks are
//! child `Subplot` marks, and their coordinate measurement produces child-frame
//! placement metadata for later rendering/debug consumers.

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    channel::value::strip_trailing_numbers,
    coords::{
        CoordMeasurement, CoordinateSystem, CoordinateSystemTransform, PlotGeometry, PointGeometry,
    },
    error::AvengerChartError,
    facet::{evaluated_facet_tree::EvaluatedFacetTree, sharing_level::SharingLevel},
    guide::{CompiledGuide, CoordinateGuide, GuideUpdate, OverflowSpaceRequirement},
    layout::{
        BandChildFrameInput, BandChildFramePlacement, BandDirection, BandSpacing, BoundaryDemand1D,
        ChildFramePlacementResult, EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode,
        LayoutBounds, Size2D,
    },
    marks::{CompiledConcatSubplot, CompiledMark, subplot::compiled_subplot},
    plot::{
        CompiledPlot,
        compiled::{
            ChildFrameDomainRequest, ChildFrameKey, ChildFrameScopeKey, ComponentsMeasurement,
            ContainerLabelChildFrame, ContainerLabelItem, ContainerLabelPlacement,
            ContainerPathSegment, CoordinationKind, CoordinationScopeKey,
            aggregate_domain_requests, child_frame_container_overflow_from_placements,
            container_label_items_from_placements, measure_container_label_slab,
            render_container_labels, scale_provider::DynamicScaleProvider,
            scales::build_scale_builder_from_marks,
        },
    },
    render::EvaluationContext,
    scales::{ConfiguredScaleWithSpec, DomainExtent, ScaleBuilder, ScaleRangeBinding},
    theme::Theme,
};

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

impl CoordinateSystem for HConcat {
    type Guide = ConcatGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystem for VConcat {
    type Guide = ConcatGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

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

    fn set_compiled_marks(
        &mut self,
        _compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
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
        _facet_tree: &EvaluatedFacetTree,
        _facet_path: &[ScalarValue],
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let mut overflow = concat_child_frame_overflow(plot_width, plot_height, coord_measurement)?;
        if let Some(concat) = coord_measurement.and_then(concat_coord_ref) {
            let label_slab = measure_container_label_slab(
                concat_label_placement(concat),
                &concat_label_items(concat)?,
                theme,
                params,
            );
            match concat_label_placement(concat) {
                ContainerLabelPlacement::Top => overflow.top += label_slab,
                ContainerLabelPlacement::Left => overflow.left += label_slab,
            }
        }
        Ok(overflow)
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
        _facet_tree: &EvaluatedFacetTree,
        _facet_path: &[ScalarValue],
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let concat = concat_coord_ref(coord_measurement).ok_or_else(|| {
            AvengerChartError::InternalError(
                "ConcatGuide received non-concat coordinate measurement".to_string(),
            )
        })?;
        Ok(render_container_labels(
            concat_label_placement(concat),
            &concat_label_items(concat)?,
            plot_bounds,
            theme,
            params,
        ))
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

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for HConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    async fn measure(
        &self,
        _scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        eval_ctx: &EvaluationContext,
        data: Option<&DataFrame>,
        compiled_marks: &[Arc<dyn CompiledMark>],
        facet_path: &[ScalarValue],
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        measure_concat_coord_system(
            BandDirection::Horizontal,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
        )
        .await
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

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for VConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    async fn measure(
        &self,
        _scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        eval_ctx: &EvaluationContext,
        data: Option<&DataFrame>,
        compiled_marks: &[Arc<dyn CompiledMark>],
        facet_path: &[ScalarValue],
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        measure_concat_coord_system(
            BandDirection::Vertical,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
        )
        .await
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

#[derive(Debug)]
pub struct ConcatCoordMeasurement {
    pub(crate) children: Vec<ConcatChildMeasurement>,
    pub(crate) child_band_layout: BandChildFramePlacement,
    pub(crate) fallback_content_size: Size2D,
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
        self.child_band_layout
            .to_child_frame_placement_result([0.0, 0.0], self.fallback_content_size)
    }
}

impl CoordMeasurement for ConcatCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Debug)]
pub(crate) struct ConcatChildMeasurement {
    pub(crate) child_index: usize,
    pub(crate) key: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) container_path: Vec<ContainerPathSegment>,
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

fn concat_child_path_segment(child_index: usize, key: Option<&str>) -> ContainerPathSegment {
    ContainerPathSegment::concat_child(child_index, key)
}

pub(crate) fn concat_coord_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&ConcatCoordMeasurement> {
    coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
}

fn concat_child_frame_overflow(
    plot_width: f32,
    plot_height: f32,
    coord_measurement: Option<&dyn CoordMeasurement>,
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    let Some(concat) = coord_measurement.and_then(concat_coord_ref) else {
        return Ok(OverflowSpaceRequirement::default());
    };

    let placement = concat.child_frame_placement();
    child_frame_container_overflow_from_placements(
        plot_width,
        plot_height,
        &placement,
        |child_index| {
            concat
                .child(child_index)
                .map(|child| &child.measurement)
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing concat child measurement for child index {}",
                        child_index
                    ))
                })
        },
    )
}

fn concat_label_placement(concat: &ConcatCoordMeasurement) -> ContainerLabelPlacement {
    match concat.child_band_layout.direction {
        BandDirection::Horizontal => ContainerLabelPlacement::Top,
        BandDirection::Vertical => ContainerLabelPlacement::Left,
    }
}

fn concat_label_items(
    concat: &ConcatCoordMeasurement,
) -> Result<Vec<ContainerLabelItem>, AvengerChartError> {
    let placement = concat.child_frame_placement();
    container_label_items_from_placements(
        &placement,
        |child_index| {
            let child = concat.child(child_index).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing concat child measurement for child index {}",
                    child_index
                ))
            })?;
            Ok(ContainerLabelChildFrame {
                plot_bounds: *child.measurement.layout.plot_area_bounds(),
                plot_size: Size2D::new(
                    child.measurement.plot_area_width,
                    child.measurement.plot_area_height,
                ),
                frame_bounds: child.measurement.frame_allocation.rect,
            })
        },
        |child_index| {
            concat
                .child(child_index)
                .and_then(|child| child.label.as_deref())
        },
    )
}

fn fixed_plot_area_layout_spec(width: f32, height: f32) -> EvaluatedLayoutSpec {
    EvaluatedLayoutSpec {
        canvas: EvaluatedSizeMode::Auto,
        plot_area: EvaluatedSizeMode::Fixed {
            width: width.max(1.0),
            height: height.max(1.0),
        },
        margins: EvaluatedMargins {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
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

struct ConcatChannelDomainExtent {
    extent: DomainExtent,
    sharing_level: SharingLevel,
}

struct PreparedConcatChild<'a> {
    subplot: &'a CompiledConcatSubplot,
    container_path: Vec<ContainerPathSegment>,
    child_data_override: Option<DataFrame>,
    scale_builder: ScaleBuilder,
    local_domain_extents: HashMap<String, ConcatChannelDomainExtent>,
    channel_domain_sharing_levels: HashMap<String, SharingLevel>,
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

    fn scope_key(&self) -> ChildFrameScopeKey {
        concat_child_scope_key(&self.container_path, self.child_index(), self.key())
    }

    fn path_segment(&self) -> ContainerPathSegment {
        concat_child_path_segment(self.child_index(), self.key())
    }
}

fn channel_domain_sharing_levels_for_plot(plot: &CompiledPlot) -> HashMap<String, SharingLevel> {
    let mut sharing_levels = HashMap::new();
    for mark in &plot.marks {
        for (channel, channel_value) in mark.data_context().channels() {
            let Some(sharing) = channel_value.get_share_mode() else {
                continue;
            };
            let channel = strip_trailing_numbers(channel).to_string();
            let sharing_level = SharingLevel::from(sharing);
            sharing_levels
                .entry(channel)
                .and_modify(|existing: &mut SharingLevel| {
                    *existing = (*existing).max(sharing_level);
                })
                .or_insert(sharing_level);
        }
    }
    sharing_levels
}

fn extract_concat_domain_extents(
    scale_builder: &ScaleBuilder,
    sharing_levels: &HashMap<String, SharingLevel>,
) -> HashMap<String, ConcatChannelDomainExtent> {
    let shared_channels = sharing_levels
        .iter()
        .filter_map(|(channel, sharing_level)| {
            (!sharing_level.is_free()).then_some(channel.as_str())
        })
        .collect::<Vec<_>>();
    if shared_channels.is_empty() {
        return HashMap::new();
    }

    scale_builder
        .extract_domain_extents(&shared_channels)
        .into_iter()
        .filter_map(|(channel, extent)| {
            sharing_levels.get(&channel).map(|sharing_level| {
                (
                    channel,
                    ConcatChannelDomainExtent {
                        extent,
                        sharing_level: *sharing_level,
                    },
                )
            })
        })
        .collect()
}

fn concat_domain_scope_key(
    child_scope: &ChildFrameScopeKey,
    channel: &str,
    sharing_level: SharingLevel,
) -> CoordinationScopeKey {
    debug_assert!(
        !sharing_level.is_free(),
        "Free concat scale domains should not need a coordination scope"
    );
    CoordinationScopeKey::child_frame_container(CoordinationKind::ScaleDomain, child_scope)
        .with_channel(channel)
}

fn concat_domain_request(
    child_scope: &ChildFrameScopeKey,
    channel: &str,
    annotated: &ConcatChannelDomainExtent,
) -> Option<ChildFrameDomainRequest> {
    (!annotated.sharing_level.is_free()).then(|| {
        ChildFrameDomainRequest::new(
            concat_domain_scope_key(child_scope, channel, annotated.sharing_level),
            annotated.extent.clone(),
        )
    })
}

fn coordinated_domain_extents_for_concat_children(
    children: &[PreparedConcatChild<'_>],
) -> Vec<HashMap<String, DomainExtent>> {
    let unified = aggregate_domain_requests(children.iter().flat_map(|child| {
        let child_scope = child.scope_key();
        child
            .local_domain_extents
            .iter()
            .filter_map(move |(channel, annotated)| {
                concat_domain_request(&child_scope, channel, annotated)
            })
    }));

    children
        .iter()
        .map(|child| {
            let child_scope = child.scope_key();
            let mut coordinated = HashMap::new();
            for (channel, sharing_level) in &child.channel_domain_sharing_levels {
                if sharing_level.is_free() {
                    continue;
                }

                let key = concat_domain_scope_key(&child_scope, channel, *sharing_level);
                if let Some(unified_extent) = unified.get(&key) {
                    coordinated.insert(channel.clone(), unified_extent.clone());
                }
            }
            coordinated
        })
        .collect()
}

async fn prepare_concat_child<'a>(
    subplot: &'a CompiledConcatSubplot,
    eval_ctx: &EvaluationContext,
    inherited_data: Option<&DataFrame>,
) -> Result<PreparedConcatChild<'a>, AvengerChartError> {
    let child_plot = subplot.compiled_subplot();
    let child_data_override = if subplot.inherits_parent_data() {
        inherited_data.cloned()
    } else {
        None
    };
    let child_params: IndexMap<String, ScalarValue> = eval_ctx.params.clone();
    let scale_builder = build_scale_builder_from_marks(
        &child_plot.marks,
        &child_plot.scale_specs,
        &child_plot.coord_transform,
        &child_plot.data,
        child_data_override.clone(),
        eval_ctx.session_context.as_ref(),
        &child_params,
        child_plot.get_theme().as_ref(),
    )
    .await?;

    let channel_domain_sharing_levels = channel_domain_sharing_levels_for_plot(child_plot);
    let local_domain_extents =
        extract_concat_domain_extents(&scale_builder, &channel_domain_sharing_levels);

    Ok(PreparedConcatChild {
        subplot,
        container_path: eval_ctx.child_frame_container_path().to_vec(),
        child_data_override,
        scale_builder,
        local_domain_extents,
        channel_domain_sharing_levels,
    })
}

async fn measure_prepared_concat_child(
    prepared: &PreparedConcatChild<'_>,
    child_plot_area: Size2D,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    coordinated_domain_extents: &HashMap<String, DomainExtent>,
) -> Result<ConcatChildMeasurement, AvengerChartError> {
    let child_plot = prepared.subplot.compiled_subplot();
    let child_layout_spec =
        fixed_plot_area_layout_spec(child_plot_area.width, child_plot_area.height);
    let extended_builder;
    let scale_builder = if coordinated_domain_extents.is_empty() {
        &prepared.scale_builder
    } else {
        extended_builder = {
            let mut builder = prepared.scale_builder.clone();
            builder.extend_with_domain_extents(coordinated_domain_extents);
            builder
        };
        &extended_builder
    };
    let scale_provider = DynamicScaleProvider {
        builder: scale_builder,
        plot: child_plot,
    };
    let child_eval_ctx = eval_ctx.with_child_frame_container_path_appended(prepared.path_segment());
    let measurement = child_plot
        .measure_plot_components(
            &child_eval_ctx,
            &child_layout_spec,
            &scale_provider,
            prepared.child_data_override.as_ref(),
            facet_path,
        )
        .await?;

    Ok(ConcatChildMeasurement {
        child_index: prepared.child_index(),
        key: prepared.key().map(ToOwned::to_owned),
        label: prepared.label().map(ToOwned::to_owned),
        container_path: prepared.container_path.clone(),
        measurement,
    })
}

async fn measure_concat_coord_system(
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
        prepared_children.push(prepare_concat_child(subplot, eval_ctx, data).await?);
    }

    let coordinated_domain_extents =
        coordinated_domain_extents_for_concat_children(&prepared_children);

    let mut children = Vec::with_capacity(prepared_children.len());
    for (prepared, coordinated_extents) in prepared_children
        .iter()
        .zip(coordinated_domain_extents.iter())
    {
        children.push(
            measure_prepared_concat_child(
                prepared,
                child_plot_area,
                eval_ctx,
                facet_path,
                coordinated_extents,
            )
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
        child_band_layout,
        fallback_content_size: Size2D::new(plot_width, plot_height),
    }))
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
        cartesian::Cartesian,
        channel::config_traits::ScaleSharing,
        coords::FacetAxis,
        facet::{
            coord::{FacetBandCoordMeasurement, FacetColumn, FacetRow},
            evaluated_facet_tree::EvaluatedFacetTree,
        },
        layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
        marks::{Subplot, line::Line, symbol::Symbol},
        plot::{
            CompiledPlot, Plot,
            compiled::{
                ChildFrameKey, ContainerPathSegment, CoordinationAxis, CoordinationKind,
                CoordinationScopeKey, scales::build_scale_builder_from_marks,
            },
        },
        render::EvaluationContext,
        scales::Linear,
        zerod::ZeroDCoord,
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
            ctx,
            &params,
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
                .x_with(col("x"), |c| c.with_scale_sharing(ScaleSharing::Shared))
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
                    .with_scale_sharing(ScaleSharing::Shared)
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

    fn zero_plot() -> Plot<ZeroDCoord> {
        Plot::<ZeroDCoord>::new()
    }

    fn keyed_hconcat(left_key: &str, right_key: &str) -> Plot<HConcat> {
        Plot::<HConcat>::new()
            .mark(Subplot::new(zero_plot()).key(left_key))
            .mark(Subplot::new(zero_plot()).key(right_key))
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
        assert_eq!(
            concat.child_band_layout.direction,
            BandDirection::Horizontal
        );
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
        assert_eq!(concat.child_band_layout.direction, BandDirection::Vertical);

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
