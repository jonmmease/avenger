//! Generic coordinate-positioned subplot measurement and rendering.
//!
//! Coordinate crates own the `Subplot<C>` authoring surface and channel names.
//! This module owns the shared child-frame runtime once a compiled subplot mark
//! declares how its placement channels map into the parent coordinate transform.

use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    DefaultLogicalExprNodeExt, PointGeometry, PositionedSubplotMarkCore,
    coerce_numeric_channel_with_renderer, scalar_total_cmp,
};
use avenger_chart_scales::DomainExtent;
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::{
    arrow::record_batch::RecordBatch, common::ScalarValue, dataframe::DataFrame, logical_expr::lit,
    prelude::SessionContext,
};

use crate::{
    container::{
        ChildFrameKey, ChildFrameScopeKey, ChildFrameSharingLevel, ContainerPathSegment,
        PlacedRegion, PlacementSolution,
    },
    coords::{CoordMeasurement, OverflowSpaceRequirement},
    error::AvengerChartError,
    layout::Size2D,
    marks::CompiledMark,
    partition::format_partition_value,
    plot::compiled::{
        ChildFrameDataSelection, ChildFrameDomainSharingInput, ChildFrameRuntime, CompiledPlot,
        ComponentsMeasurement, MarkDataRequest, PreparedChildFramePlot,
        child_frame_container_overflow, child_frame_container_view_from_positioned,
        compiled_subplot_payload_child_plot, container_path_without_facet_segments,
        coordinated_child_frame_domain_extents, prepare_mark_data_runtime,
    },
    render::{EvaluationContext, RenderContext},
    scales::ConfiguredScaleWithSpec,
};

fn positioned_subplot_child_plot(subplot: &dyn PositionedSubplotMarkCore) -> &CompiledPlot {
    compiled_subplot_payload_child_plot(subplot.payload())
}

pub(crate) async fn render_positioned_subplot_with_context(
    subplot: &dyn PositionedSubplotMarkCore,
    context: &RenderContext<'_>,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    let positioned_measurement =
        positioned_coord_ref(context.coord_measurement()).ok_or_else(|| {
            AvengerChartError::InternalError(
                "Positioned subplot marks require PositionedCoordMeasurement".to_string(),
            )
        })?;
    let child_frame_placement = positioned_measurement.child_frame_placement();
    let child_count = positioned_measurement.children().len();
    let mut marks = Vec::new();

    for child in positioned_measurement.children_for_mark(subplot.mark_index()) {
        let render_placement = child_frame_placement
            .child(child.child_index)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing positioned child-frame placement for child index {}",
                    child.child_index
                ))
            })?;

        let child_plot = positioned_subplot_child_plot(subplot);
        let mut params = child_plot.get_default_params().clone();
        params.extend(context.eval.params.clone());
        let sharing_level = child.sharing_level(child_count);
        let child_eval_ctx = context
            .eval
            .with_params(params)
            .with_child_frame_sharing_level_appended(sharing_level);
        let mut components = Box::pin(child_plot.build_plot_components(
            &child_eval_ctx,
            &child.measurement,
            child.data_override.as_ref(),
            true,
            context.facet_path,
        ))
        .await?;
        let group_index = marks.len();
        let child_event_datums = std::mem::take(&mut components.event_datums);
        if !child_event_datums.is_empty() {
            let translated = child_event_datums.into_iter().map(|mut rows| {
                let mut path = Vec::with_capacity(rows.mark_path.len() + 2);
                path.push(group_index);
                path.push(0);
                path.extend(rows.mark_path);
                rows.mark_path = path;
                rows.prepend_subplot_id(subplot.state().id.as_deref());
                rows
            });
            context.eval.push_event_datums(translated);
        }
        let child_chrome_event_datums = std::mem::take(&mut components.chrome_event_datums);
        if !child_chrome_event_datums.is_empty() {
            let translated = child_chrome_event_datums.into_iter().map(|mut rows| {
                let mut path = Vec::with_capacity(rows.mark_path.len() + 1);
                path.push(group_index);
                path.extend(rows.mark_path);
                rows.mark_path = path;
                rows.prepend_subplot_id(subplot.state().id.as_deref());
                rows
            });
            context.eval.push_event_datums(translated);
        }

        let data_marks_group = SceneGroup {
            origin: [0.0, 0.0],
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };
        let mut all_marks = vec![SceneMark::Group(data_marks_group)];
        all_marks.extend(components.guide_marks);
        all_marks.extend(components.legend_marks);
        all_marks.extend(components.title_marks);
        all_marks.extend(components.subtitle_marks);
        all_marks.extend(components.debug_marks);

        marks.push(SceneMark::Group(SceneGroup {
            name: child.group_name(),
            origin: render_placement.origin,
            clip: avenger_scenegraph::marks::group::Clip::None,
            marks: all_marks,
            gradients: Vec::new(),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
            interactive: true,
        }));
    }

    Ok(marks)
}

#[derive(Clone, Debug)]
pub(crate) struct PositionedCoordMeasurement {
    pub(crate) children: Vec<PositionedChildMeasurement>,
    pub(crate) placement: PlacementSolution,
}

impl PositionedCoordMeasurement {
    pub(crate) fn children(&self) -> &[PositionedChildMeasurement] {
        &self.children
    }

    pub(crate) fn child(&self, child_index: usize) -> Option<&PositionedChildMeasurement> {
        self.children
            .iter()
            .find(|child| child.child_index == child_index)
    }

    pub(crate) fn children_for_mark(
        &self,
        mark_index: usize,
    ) -> impl Iterator<Item = &PositionedChildMeasurement> {
        self.children
            .iter()
            .filter(move |child| child.mark_index == mark_index)
    }

    pub(crate) fn child_scope_key(&self, child_index: usize) -> Option<ChildFrameScopeKey> {
        self.child(child_index)
            .map(PositionedChildMeasurement::scope_key)
    }

    pub(crate) fn child_frame_placement(&self) -> PlacementSolution {
        self.placement.clone()
    }
}

impl CoordMeasurement for PositionedCoordMeasurement {
    fn clone_box(&self) -> Box<dyn CoordMeasurement> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn positioned_subplot_overflow(
        &self,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Option<OverflowSpaceRequirement>, AvengerChartError> {
        let container = child_frame_container_view_from_positioned(self)?;
        child_frame_container_overflow(plot_width, plot_height, &container).map(Some)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PositionedChildMeasurement {
    pub(crate) child_index: usize,
    pub(crate) mark_index: usize,
    pub(crate) identity: PositionedChildIdentity,
    pub(crate) group_name_prefix: String,
    pub(crate) key: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) container_path: Vec<ContainerPathSegment>,
    pub(crate) data_override: Option<DataFrame>,
    pub(crate) measurement: ComponentsMeasurement,
}

impl PositionedChildMeasurement {
    pub(crate) fn scope_key(&self) -> ChildFrameScopeKey {
        positioned_child_scope_key(
            &self.container_path,
            self.mark_index,
            &self.identity,
            self.key.as_deref(),
        )
    }

    pub(crate) fn sharing_level(&self, child_count: usize) -> ChildFrameSharingLevel {
        positioned_child_sharing_level(
            self.child_index,
            child_count,
            self.mark_index,
            &self.identity,
            self.key.as_deref(),
        )
    }

    pub(crate) fn group_name(&self) -> String {
        match (self.key.as_deref(), &self.identity) {
            (Some(key), _) => format!(
                "{}_{}_{}_{}",
                self.group_name_prefix, self.mark_index, self.child_index, key
            ),
            (None, PositionedChildIdentity::Partition { value }) => format!(
                "{}_{}_{}_{}",
                self.group_name_prefix,
                self.mark_index,
                self.child_index,
                format_partition_value(value)
            ),
            (None, PositionedChildIdentity::Row { .. }) => {
                format!(
                    "{}_{}_{}",
                    self.group_name_prefix, self.mark_index, self.child_index
                )
            }
        }
    }
}

pub(crate) fn positioned_coord_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&PositionedCoordMeasurement> {
    coord_measurement
        .as_any()
        .downcast_ref::<PositionedCoordMeasurement>()
}

fn positioned_child_scope_key(
    container_path: &[ContainerPathSegment],
    mark_index: usize,
    identity: &PositionedChildIdentity,
    key: Option<&str>,
) -> ChildFrameScopeKey {
    let child_key = match identity {
        PositionedChildIdentity::Row { row_index } => ChildFrameKey::PositionedSubplot {
            mark_index,
            row_index: *row_index,
            key: key.map(ToOwned::to_owned),
        },
        PositionedChildIdentity::Partition { value } => ChildFrameKey::PositionedPartition {
            mark_index,
            value: value.clone(),
            key: key.map(ToOwned::to_owned),
        },
    };
    ChildFrameScopeKey::new(container_path.to_vec(), child_key)
}

fn positioned_child_container_segment(
    mark_index: usize,
    identity: &PositionedChildIdentity,
    key: Option<&str>,
) -> ContainerPathSegment {
    match identity {
        PositionedChildIdentity::Row { row_index } => {
            ContainerPathSegment::positioned_subplot(mark_index, *row_index, key)
        }
        PositionedChildIdentity::Partition { value } => {
            ContainerPathSegment::positioned_partition(mark_index, value.clone(), key)
        }
    }
}

fn positioned_child_sharing_level(
    child_index: usize,
    child_count: usize,
    mark_index: usize,
    identity: &PositionedChildIdentity,
    key: Option<&str>,
) -> ChildFrameSharingLevel {
    match identity {
        PositionedChildIdentity::Row { row_index } => ChildFrameSharingLevel::positioned_subplot(
            child_index,
            child_count,
            mark_index,
            *row_index,
            key,
        ),
        PositionedChildIdentity::Partition { value } => {
            ChildFrameSharingLevel::positioned_partition(
                child_index,
                child_count,
                mark_index,
                value.clone(),
                key,
            )
        }
    }
}

fn scalar_or_array_len(values: &ScalarOrArray<f32>) -> usize {
    match values.value() {
        ScalarOrArrayValue::Scalar(_) => 1,
        ScalarOrArrayValue::Array(values) => values.len(),
    }
}

fn expanded_values(
    values: &ScalarOrArray<f32>,
    len: usize,
    channel: &str,
) -> Result<Vec<f32>, AvengerChartError> {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => Ok(vec![*value; len]),
        ScalarOrArrayValue::Array(values) if values.len() == len => Ok(values.as_ref().clone()),
        ScalarOrArrayValue::Array(values) => Err(AvengerChartError::InternalError(format!(
            "Positioned subplot channel `{channel}` produced {} values but expected {len}",
            values.len()
        ))),
    }
}

fn record_batch_channel_values(
    batch: &RecordBatch,
    channel: &str,
) -> Result<Option<Vec<ScalarValue>>, AvengerChartError> {
    let Some(column) = batch.column_by_name(channel) else {
        return Ok(None);
    };

    let mut values = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        values.push(ScalarValue::try_from_array(column, row)?);
    }
    Ok(Some(values))
}

fn scalar_values_for_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
) -> Result<Vec<ScalarValue>, AvengerChartError> {
    if let Some(data_batch) = data
        && let Some(values) = record_batch_channel_values(data_batch, channel)?
    {
        return Ok(values);
    }

    if let Some(values) = record_batch_channel_values(scalars, channel)? {
        return Ok(values);
    }

    Err(AvengerChartError::InternalError(format!(
        "Positioned subplot channel `{channel}` was not prepared"
    )))
}

fn expanded_scalar_values(
    values: &[ScalarValue],
    len: usize,
    channel: &str,
) -> Result<Vec<ScalarValue>, AvengerChartError> {
    if values.len() == len {
        Ok(values.to_vec())
    } else if values.len() == 1 {
        Ok(vec![values[0].clone(); len])
    } else {
        Err(AvengerChartError::InternalError(format!(
            "Positioned subplot channel `{channel}` produced {} values but expected {len}",
            values.len()
        )))
    }
}

fn transformed_position_points(
    subplot: &dyn PositionedSubplotMarkCore,
    position_values: &HashMap<String, ScalarOrArray<f32>>,
    coord_transform: &dyn avenger_chart_core::CoordinateSystemTransformCore,
    plot_width: f32,
    plot_height: f32,
) -> Result<PointGeometry, AvengerChartError> {
    let mut transform_channels = HashMap::new();
    for channel in &subplot.spec().placement_channels {
        let value = position_values
            .get(&channel.channel)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing positioned subplot placement channel `{}`",
                    channel.channel
                ))
            })?
            .clone();
        transform_channels.insert(channel.transform_channel.as_str(), value);
    }

    let geometry = coord_transform.transform(&transform_channels, None, plot_width, plot_height)?;
    geometry
        .as_any()
        .downcast_ref::<PointGeometry>()
        .cloned()
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "{} subplots require the parent coordinate transform to produce PointGeometry",
                subplot.spec().outer_label
            ))
        })
}

fn partitioned_child_specs(
    subplot: &dyn PositionedSubplotMarkCore,
    xs: &[f32],
    ys: &[f32],
    partition_values: &[ScalarValue],
    next_child_index: &mut usize,
) -> Vec<PositionedChildSpec> {
    let mut specs = Vec::new();

    for row_index in 0..partition_values.len() {
        let value = partition_values[row_index].clone();
        if specs.iter().any(|spec: &PositionedChildSpec| {
            matches!(
                &spec.identity,
                PositionedChildIdentity::Partition { value: existing } if existing == &value
            )
        }) {
            continue;
        }

        specs.push(PositionedChildSpec {
            child_index: 0,
            mark_index: subplot.mark_index(),
            identity: PositionedChildIdentity::Partition {
                value: value.clone(),
            },
            key: subplot.key().map(ToOwned::to_owned),
            label: subplot
                .label()
                .map(ToOwned::to_owned)
                .or_else(|| Some(format_partition_value(&value))),
            x: xs[row_index],
            y: ys[row_index],
        });
    }

    specs.sort_by(|a, b| match (&a.identity, &b.identity) {
        (
            PositionedChildIdentity::Partition { value: a },
            PositionedChildIdentity::Partition { value: b },
        ) => scalar_total_cmp(a, b),
        _ => std::cmp::Ordering::Equal,
    });

    for spec in &mut specs {
        spec.child_index = *next_child_index;
        *next_child_index += 1;
    }

    specs
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PositionedChildIdentity {
    Row { row_index: usize },
    Partition { value: ScalarValue },
}

#[derive(Debug, Clone)]
struct PositionedChildSpec {
    child_index: usize,
    mark_index: usize,
    identity: PositionedChildIdentity,
    key: Option<String>,
    label: Option<String>,
    x: f32,
    y: f32,
}

enum PreparedPositionedChildPlots<'a> {
    Shared(PreparedChildFramePlot<'a>),
    PerChild(Vec<PreparedChildFramePlot<'a>>),
}

struct PreparedPositionedSubplot<'a> {
    subplot: &'a dyn PositionedSubplotMarkCore,
    child_plots: PreparedPositionedChildPlots<'a>,
    child_specs: Vec<PositionedChildSpec>,
}

impl<'a> PreparedPositionedSubplot<'a> {
    fn child_plot(
        &self,
        spec_index: usize,
    ) -> Result<&PreparedChildFramePlot<'a>, AvengerChartError> {
        match &self.child_plots {
            PreparedPositionedChildPlots::Shared(child_plot) => Ok(child_plot),
            PreparedPositionedChildPlots::PerChild(child_plots) => {
                child_plots.get(spec_index).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing positioned child plot for spec index {spec_index}"
                    ))
                })
            }
        }
    }
}

fn mark_inherited_data(
    subplot: &dyn PositionedSubplotMarkCore,
    ctx: &SessionContext,
    parent_data: Option<&DataFrame>,
) -> Option<DataFrame> {
    subplot
        .data_context()
        .dataframe_with_context(ctx)
        .or_else(|| parent_data.cloned())
}

#[allow(clippy::too_many_arguments)]
async fn prepare_partitioned_positioned_subplot<'a>(
    subplot: &'a dyn PositionedSubplotMarkCore,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    coord_transform: &dyn avenger_chart_core::CoordinateSystemTransformCore,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    facet_path: &[ScalarValue],
    next_child_index: &mut usize,
) -> Result<Option<PreparedPositionedSubplot<'a>>, AvengerChartError> {
    let ctx = eval_ctx.session_context.as_ref();
    let parent_data = data.ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "Partitioned {} subplots require parent plot data",
            subplot.spec().outer_label
        ))
    })?;
    let partition_expr = subplot
        .partition_expr()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Partitioned positioned subplot is missing its partition expression".to_string(),
            )
        })?
        .to_expr(ctx)?;

    let prepared_mark = Box::pin(prepare_mark_data_runtime(MarkDataRequest {
        mark: subplot.as_compiled_mark(),
        plot_data: None,
        provided_plot_df: data,
        facet_data_scope: Some(crate::facet::data_scope::FacetDataScopeContext::new(
            eval_ctx.facet_tree.as_ref(),
            eval_ctx.facet_data_root(),
            facet_path,
        )),
        prepared_logical: None,
        eval_ctx,
        evaluation_metrics: eval_ctx.evaluation_metrics.clone(),
        scales,
        plot_width,
        plot_height,
    }))
    .await?;
    let Some(prepared_mark) = prepared_mark else {
        return Ok(None);
    };

    let empty_coord = avenger_chart_core::EmptyCoordMeasurement;
    let render_ctx = RenderContext::new(
        eval_ctx,
        &prepared_mark.render_state,
        facet_path,
        &empty_coord,
    );
    let mark_context = render_ctx.core_view();
    let mut position_values = HashMap::new();
    let mut child_count = 1usize;
    for channel in &subplot.spec().placement_channels {
        let value = coerce_numeric_channel_with_renderer(
            subplot.as_compiled_mark(),
            prepared_mark.data_batch.as_ref(),
            &prepared_mark.scalar_batch,
            &channel.channel,
            &mark_context,
            0.0,
        )?;
        child_count = child_count.max(scalar_or_array_len(&value));
        position_values.insert(channel.channel.clone(), value);
    }
    let points = transformed_position_points(
        subplot,
        &position_values,
        coord_transform,
        plot_width,
        plot_height,
    )?;
    child_count = child_count
        .max(scalar_or_array_len(&points.x))
        .max(scalar_or_array_len(&points.y));

    let partition_channel = subplot.spec().partition_channel.as_deref().ok_or_else(|| {
        AvengerChartError::InternalError(
            "Partitioned positioned subplot is missing its partition channel".to_string(),
        )
    })?;
    let partition_values = scalar_values_for_channel(
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        partition_channel,
    )?;
    child_count = child_count.max(partition_values.len());

    let xs = expanded_values(&points.x, child_count, "x")?;
    let ys = expanded_values(&points.y, child_count, "y")?;
    let partition_values =
        expanded_scalar_values(&partition_values, child_count, partition_channel)?;
    let child_specs =
        partitioned_child_specs(subplot, &xs, &ys, &partition_values, next_child_index);

    let runtime = ChildFrameRuntime::new();
    let mut child_plots = Vec::with_capacity(child_specs.len());
    for spec in &child_specs {
        let PositionedChildIdentity::Partition { value } = &spec.identity else {
            continue;
        };
        let filtered_data = parent_data
            .clone()
            .filter(partition_expr.clone().eq(lit(value.clone())))?;
        child_plots.push(
            Box::pin(runtime.prepare_plot(
                positioned_subplot_child_plot(subplot),
                ChildFrameDataSelection::InheritParent,
                Some(&filtered_data),
                eval_ctx,
            ))
            .await?,
        );
    }

    Ok(Some(PreparedPositionedSubplot {
        subplot,
        child_plots: PreparedPositionedChildPlots::PerChild(child_plots),
        child_specs,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn prepare_positioned_subplot<'a>(
    subplot: &'a dyn PositionedSubplotMarkCore,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    coord_transform: &dyn avenger_chart_core::CoordinateSystemTransformCore,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    facet_path: &[ScalarValue],
    next_child_index: &mut usize,
) -> Result<Option<PreparedPositionedSubplot<'a>>, AvengerChartError> {
    if subplot.is_partitioned() {
        return Box::pin(prepare_partitioned_positioned_subplot(
            subplot,
            scales,
            coord_transform,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            facet_path,
            next_child_index,
        ))
        .await;
    }

    let prepared_mark = Box::pin(prepare_mark_data_runtime(MarkDataRequest {
        mark: subplot.as_compiled_mark(),
        plot_data: None,
        provided_plot_df: data,
        facet_data_scope: Some(crate::facet::data_scope::FacetDataScopeContext::new(
            eval_ctx.facet_tree.as_ref(),
            eval_ctx.facet_data_root(),
            facet_path,
        )),
        prepared_logical: None,
        eval_ctx,
        evaluation_metrics: eval_ctx.evaluation_metrics.clone(),
        scales,
        plot_width,
        plot_height,
    }))
    .await?;
    let Some(prepared_mark) = prepared_mark else {
        return Ok(None);
    };

    let empty_coord = avenger_chart_core::EmptyCoordMeasurement;
    let render_ctx = RenderContext::new(
        eval_ctx,
        &prepared_mark.render_state,
        facet_path,
        &empty_coord,
    );
    let mark_context = render_ctx.core_view();
    let mut position_values = HashMap::new();
    let mut child_count = 1usize;
    for channel in &subplot.spec().placement_channels {
        let value = coerce_numeric_channel_with_renderer(
            subplot.as_compiled_mark(),
            prepared_mark.data_batch.as_ref(),
            &prepared_mark.scalar_batch,
            &channel.channel,
            &mark_context,
            0.0,
        )?;
        child_count = child_count.max(scalar_or_array_len(&value));
        position_values.insert(channel.channel.clone(), value);
    }
    let points = transformed_position_points(
        subplot,
        &position_values,
        coord_transform,
        plot_width,
        plot_height,
    )?;
    child_count = child_count
        .max(scalar_or_array_len(&points.x))
        .max(scalar_or_array_len(&points.y));
    let xs = expanded_values(&points.x, child_count, "x")?;
    let ys = expanded_values(&points.y, child_count, "y")?;

    let mut child_specs = Vec::with_capacity(child_count);
    for row_index in 0..child_count {
        let child_index = *next_child_index;
        *next_child_index += 1;
        child_specs.push(PositionedChildSpec {
            child_index,
            mark_index: subplot.mark_index(),
            identity: PositionedChildIdentity::Row { row_index },
            key: subplot.key().map(ToOwned::to_owned),
            label: subplot.label().map(ToOwned::to_owned),
            x: xs[row_index],
            y: ys[row_index],
        });
    }

    let ctx = eval_ctx.session_context.as_ref();
    let inherited_data = mark_inherited_data(subplot, ctx, data);
    let data_selection = if subplot.inherits_parent_data() {
        ChildFrameDataSelection::InheritParent
    } else {
        ChildFrameDataSelection::ExplicitChild
    };
    let runtime = ChildFrameRuntime::new();
    let child_plot = Box::pin(runtime.prepare_plot(
        positioned_subplot_child_plot(subplot),
        data_selection,
        inherited_data.as_ref(),
        eval_ctx,
    ))
    .await?;

    Ok(Some(PreparedPositionedSubplot {
        subplot,
        child_plots: PreparedPositionedChildPlots::Shared(child_plot),
        child_specs,
    }))
}

fn positioned_render_origin(
    spec: &PositionedChildSpec,
    subplot: &dyn PositionedSubplotMarkCore,
) -> [f32; 2] {
    [
        spec.x - subplot.plot_width() / 2.0,
        spec.y - subplot.plot_height() / 2.0,
    ]
}

#[allow(clippy::too_many_arguments)]
async fn measure_positioned_child(
    prepared: &PreparedPositionedSubplot<'_>,
    spec_index: usize,
    spec: &PositionedChildSpec,
    total_child_count: usize,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    domain_extents: &HashMap<String, DomainExtent>,
    facet_scoped_domain_extents: &HashMap<String, DomainExtent>,
) -> Result<PositionedChildMeasurement, AvengerChartError> {
    let runtime = ChildFrameRuntime::new();
    let child_layout_spec = runtime.fixed_plot_area_layout_spec(
        prepared.subplot.plot_width(),
        prepared.subplot.plot_height(),
    );
    let sharing_level = positioned_child_sharing_level(
        spec.child_index,
        total_child_count,
        spec.mark_index,
        &spec.identity,
        spec.key.as_deref(),
    );
    let child_eval_ctx = runtime.eval_context(eval_ctx, sharing_level);
    let child_plot = prepared.child_plot(spec_index)?;
    let measurement = Box::pin(child_plot.measure(
        &child_eval_ctx,
        &child_layout_spec,
        facet_path,
        &[domain_extents, facet_scoped_domain_extents],
    ))
    .await?;

    Ok(PositionedChildMeasurement {
        child_index: spec.child_index,
        mark_index: spec.mark_index,
        identity: spec.identity.clone(),
        group_name_prefix: prepared.subplot.spec().group_name_prefix.clone(),
        key: spec.key.clone(),
        label: spec.label.clone(),
        container_path: eval_ctx.child_frame_container_path().to_vec(),
        data_override: child_plot.data_override().cloned(),
        measurement,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn measure_positioned_subplots(
    coord_transform: &dyn avenger_chart_core::CoordinateSystemTransformCore,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Option<Box<dyn CoordMeasurement>>, AvengerChartError> {
    let mut next_child_index = 0usize;
    let mut prepared_subplots = Vec::new();
    for subplot in compiled_marks
        .iter()
        .filter_map(|mark| mark.as_positioned_subplot())
    {
        if let Some(prepared) = Box::pin(prepare_positioned_subplot(
            subplot,
            scales,
            coord_transform,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            facet_path,
            &mut next_child_index,
        ))
        .await?
        {
            prepared_subplots.push(prepared);
        }
    }

    if next_child_index == 0 {
        return Ok(None);
    }

    let mut scope_keys = Vec::with_capacity(next_child_index);
    for prepared in &prepared_subplots {
        for spec in &prepared.child_specs {
            scope_keys.push(positioned_child_scope_key(
                eval_ctx.child_frame_container_path(),
                spec.mark_index,
                &spec.identity,
                spec.key.as_deref(),
            ));
        }
    }

    let mut domain_inputs = Vec::with_capacity(next_child_index);
    let mut scope_index = 0usize;
    for prepared in &prepared_subplots {
        for (spec_index, _spec) in prepared.child_specs.iter().enumerate() {
            let child_plot = prepared.child_plot(spec_index)?;
            domain_inputs.push(ChildFrameDomainSharingInput::new(
                &scope_keys[scope_index],
                child_plot.local_domain_extents(),
                child_plot.channel_domain_sharing_levels(),
            ));
            scope_index += 1;
        }
    }
    let coordinated_domain_extents = coordinated_child_frame_domain_extents(&domain_inputs);

    let mut children = Vec::with_capacity(next_child_index);
    let mut placements = Vec::with_capacity(next_child_index);
    let mut domain_index = 0usize;
    for prepared in &prepared_subplots {
        for (spec_index, spec) in prepared.child_specs.iter().enumerate() {
            let mut relative_child_frame_path =
                container_path_without_facet_segments(eval_ctx.child_frame_container_path());
            relative_child_frame_path.push(positioned_child_container_segment(
                spec.mark_index,
                &spec.identity,
                spec.key.as_deref(),
            ));
            let facet_scoped_domain_extents = eval_ctx
                .facet_scale_precompute_store()
                .coordinated_child_frame_domain_extents(&relative_child_frame_path, facet_path);
            children.push(
                Box::pin(measure_positioned_child(
                    prepared,
                    spec_index,
                    spec,
                    next_child_index,
                    eval_ctx,
                    facet_path,
                    &coordinated_domain_extents[domain_index],
                    &facet_scoped_domain_extents,
                ))
                .await?,
            );
            placements.push(PlacedRegion::new(
                spec.child_index,
                positioned_render_origin(spec, prepared.subplot),
            ));
            domain_index += 1;
        }
    }

    Ok(Some(Box::new(PositionedCoordMeasurement {
        children,
        placement: PlacementSolution::new(Size2D::new(plot_width, plot_height), placements),
    })))
}
