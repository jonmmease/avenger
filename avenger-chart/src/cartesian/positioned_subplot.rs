//! Coordinate-positioned subplot measurement for Cartesian parents.
//!
//! A Cartesian plot can act as a normal data coordinate system and, when it
//! contains `Subplot<Cartesian>` marks, as a child-frame container. The child
//! frames are measured here so layout, debug overlays, and rendering all use the
//! same saved geometry.

use std::{any::Any, collections::HashMap, future::Future, pin::Pin, sync::Arc};

use avenger_chart_cartesian::{
    CARTESIAN_SUBPLOT_PARTITION_CHANNEL, CARTESIAN_SUBPLOT_X_CHANNEL, CARTESIAN_SUBPLOT_Y_CHANNEL,
};
use avenger_chart_core::{
    DefaultLogicalExprNodeExt, coerce_numeric_channel_with_renderer, scalar_total_cmp,
};
use avenger_chart_scales::DomainExtent;
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::{
    arrow::record_batch::RecordBatch, common::ScalarValue, dataframe::DataFrame, logical_expr::lit,
    prelude::SessionContext,
};

use crate::{
    cartesian::CompiledCartesianSubplot,
    container::{
        ChildFrameKey, ChildFramePlacementResult, ChildFrameRenderPlacement, ChildFrameScopeKey,
        ChildFrameSharingLevel, ContainerPathSegment,
    },
    coords::{CoordMeasurement, EmptyCoordMeasurement, OverflowSpaceRequirement},
    error::AvengerChartError,
    layout::Size2D,
    marks::{CompiledMark, CompiledMarkCore},
    partition::format_partition_value,
    plot::compiled::{
        ChildFrameDataSelection, ChildFrameDomainSharingInput, ChildFrameRuntime, CompiledPlot,
        ComponentsMeasurement, MarkDataRequest, PreparedChildFramePlot,
        child_frame_container_overflow, child_frame_container_view_from_cartesian_positioned,
        compiled_subplot_payload_child_plot, container_path_without_facet_segments,
        coordinated_child_frame_domain_extents, prepare_mark_data_runtime,
    },
    render::{EvaluationContext, RenderContext},
    scales::ConfiguredScaleWithSpec,
};

fn cartesian_subplot_child_plot(subplot: &CompiledCartesianSubplot) -> &CompiledPlot {
    compiled_subplot_payload_child_plot(subplot.payload())
}

pub(crate) fn render_cartesian_subplot_with_context<'a>(
    subplot: &'a CompiledCartesianSubplot,
    context: &'a RenderContext<'a>,
) -> Pin<Box<dyn Future<Output = Result<Vec<SceneMark>, AvengerChartError>> + Send + 'a>> {
    Box::pin(async move {
        let cartesian_measurement = cartesian_positioned_coord_ref(context.coord_measurement())
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Cartesian subplot marks require CartesianPositionedCoordMeasurement"
                        .to_string(),
                )
            })?;
        let child_frame_placement = cartesian_measurement.child_frame_placement();
        let child_count = cartesian_measurement.children().len();
        let mut marks = Vec::new();

        for child in cartesian_measurement.children_for_mark(subplot.mark_index()) {
            let render_placement =
                child_frame_placement
                    .child(child.child_index)
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "Missing Cartesian child-frame placement for child index {}",
                            child.child_index
                        ))
                    })?;

            let child_plot = cartesian_subplot_child_plot(subplot);
            let mut params = child_plot.get_default_params().clone();
            params.extend(context.eval.params.clone());
            let sharing_level = child.sharing_level(child_count);
            let child_eval_ctx = context
                .eval
                .with_params(params)
                .with_child_frame_sharing_level_appended(sharing_level);
            let components = Box::pin(child_plot.build_plot_components(
                &child_eval_ctx,
                &child.measurement,
                child.data_override.as_ref(),
                true,
                context.facet_path,
            ))
            .await?;

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
            }));
        }

        Ok(marks)
    })
}

pub fn compiled_cartesian_subplot(mark: &dyn CompiledMark) -> Option<&CompiledCartesianSubplot> {
    if mark.mark_type() != "subplot" {
        return None;
    }
    mark.as_any().downcast_ref::<CompiledCartesianSubplot>()
}

#[derive(Debug)]
pub(crate) struct CartesianPositionedCoordMeasurement {
    pub(crate) children: Vec<CartesianPositionedChildMeasurement>,
    pub(crate) placement: ChildFramePlacementResult,
}

impl CartesianPositionedCoordMeasurement {
    pub(crate) fn children(&self) -> &[CartesianPositionedChildMeasurement] {
        &self.children
    }

    pub(crate) fn child(&self, child_index: usize) -> Option<&CartesianPositionedChildMeasurement> {
        self.children
            .iter()
            .find(|child| child.child_index == child_index)
    }

    pub(crate) fn children_for_mark(
        &self,
        mark_index: usize,
    ) -> impl Iterator<Item = &CartesianPositionedChildMeasurement> {
        self.children
            .iter()
            .filter(move |child| child.mark_index == mark_index)
    }

    pub(crate) fn child_scope_key(&self, child_index: usize) -> Option<ChildFrameScopeKey> {
        self.child(child_index)
            .map(CartesianPositionedChildMeasurement::scope_key)
    }

    pub(crate) fn child_frame_placement(&self) -> ChildFramePlacementResult {
        self.placement.clone()
    }
}

impl CoordMeasurement for CartesianPositionedCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn positioned_subplot_overflow(
        &self,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Option<OverflowSpaceRequirement>, AvengerChartError> {
        let container = child_frame_container_view_from_cartesian_positioned(self)?;
        child_frame_container_overflow(plot_width, plot_height, &container).map(Some)
    }
}

#[derive(Debug)]
pub(crate) struct CartesianPositionedChildMeasurement {
    pub(crate) child_index: usize,
    pub(crate) mark_index: usize,
    pub(crate) identity: PositionedChildIdentity,
    pub(crate) key: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) container_path: Vec<ContainerPathSegment>,
    pub(crate) data_override: Option<DataFrame>,
    pub(crate) measurement: ComponentsMeasurement,
}

impl CartesianPositionedChildMeasurement {
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
                "cartesian_subplot_{}_{}_{}",
                self.mark_index, self.child_index, key
            ),
            (None, PositionedChildIdentity::Partition { value }) => format!(
                "cartesian_subplot_{}_{}_{}",
                self.mark_index,
                self.child_index,
                format_partition_value(value)
            ),
            (None, PositionedChildIdentity::Row { .. }) => {
                format!("cartesian_subplot_{}_{}", self.mark_index, self.child_index)
            }
        }
    }
}

pub(crate) fn cartesian_positioned_coord_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&CartesianPositionedCoordMeasurement> {
    coord_measurement
        .as_any()
        .downcast_ref::<CartesianPositionedCoordMeasurement>()
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

fn expanded_values(
    values: &ScalarOrArray<f32>,
    len: usize,
    channel: &str,
) -> Result<Vec<f32>, AvengerChartError> {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => Ok(vec![*value; len]),
        ScalarOrArrayValue::Array(values) if values.len() == len => Ok(values.as_ref().clone()),
        ScalarOrArrayValue::Array(values) => Err(AvengerChartError::InternalError(format!(
            "Cartesian subplot channel `{channel}` produced {} values but expected {len}",
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
        "Cartesian subplot channel `{channel}` was not prepared"
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
            "Cartesian subplot channel `{channel}` produced {} values but expected {len}",
            values.len()
        )))
    }
}

fn partitioned_child_specs(
    subplot: &CompiledCartesianSubplot,
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
    subplot: &'a CompiledCartesianSubplot,
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
                        "Missing Cartesian positioned child plot for spec index {spec_index}"
                    ))
                })
            }
        }
    }
}

fn mark_inherited_data(
    subplot: &CompiledCartesianSubplot,
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
    subplot: &'a CompiledCartesianSubplot,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    facet_path: &[ScalarValue],
    next_child_index: &mut usize,
) -> Result<Option<PreparedPositionedSubplot<'a>>, AvengerChartError> {
    let ctx = eval_ctx.session_context.as_ref();
    let parent_data = data.ok_or_else(|| {
        AvengerChartError::InvalidArgument(
            "Partitioned Cartesian subplots require parent plot data".to_string(),
        )
    })?;
    let partition_expr = subplot
        .partition_expr()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "Partitioned Cartesian subplot is missing its partition expression".to_string(),
            )
        })?
        .to_expr(ctx)?;

    let prepared_mark = prepare_mark_data_runtime(MarkDataRequest {
        mark: subplot,
        plot_data: None,
        provided_plot_df: data,
        eval_ctx,
        scales,
        plot_width,
        plot_height,
    })
    .await?;
    let Some(prepared_mark) = prepared_mark else {
        return Ok(None);
    };

    let empty_coord = EmptyCoordMeasurement;
    let render_ctx = RenderContext::new(
        eval_ctx,
        &prepared_mark.render_state,
        facet_path,
        &empty_coord,
    );
    let mark_context = render_ctx.core_view();
    let x = coerce_numeric_channel_with_renderer(
        subplot,
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        CARTESIAN_SUBPLOT_X_CHANNEL,
        &mark_context,
        0.0,
    )?;
    let y = coerce_numeric_channel_with_renderer(
        subplot,
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        CARTESIAN_SUBPLOT_Y_CHANNEL,
        &mark_context,
        0.0,
    )?;
    let partition_values = scalar_values_for_channel(
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        CARTESIAN_SUBPLOT_PARTITION_CHANNEL,
    )?;

    let child_count = x.len().max(y.len()).max(partition_values.len()).max(1);
    let xs = expanded_values(&x, child_count, CARTESIAN_SUBPLOT_X_CHANNEL)?;
    let ys = expanded_values(&y, child_count, CARTESIAN_SUBPLOT_Y_CHANNEL)?;
    let partition_values = expanded_scalar_values(
        &partition_values,
        child_count,
        CARTESIAN_SUBPLOT_PARTITION_CHANNEL,
    )?;
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
            runtime
                .prepare_plot(
                    cartesian_subplot_child_plot(subplot),
                    ChildFrameDataSelection::InheritParent,
                    Some(&filtered_data),
                    eval_ctx,
                )
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
    subplot: &'a CompiledCartesianSubplot,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    facet_path: &[ScalarValue],
    next_child_index: &mut usize,
) -> Result<Option<PreparedPositionedSubplot<'a>>, AvengerChartError> {
    if subplot.is_partitioned() {
        return prepare_partitioned_positioned_subplot(
            subplot,
            scales,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            facet_path,
            next_child_index,
        )
        .await;
    }

    let prepared_mark = prepare_mark_data_runtime(MarkDataRequest {
        mark: subplot,
        plot_data: None,
        provided_plot_df: data,
        eval_ctx,
        scales,
        plot_width,
        plot_height,
    })
    .await?;
    let Some(prepared_mark) = prepared_mark else {
        return Ok(None);
    };

    let empty_coord = EmptyCoordMeasurement;
    let render_ctx = RenderContext::new(
        eval_ctx,
        &prepared_mark.render_state,
        facet_path,
        &empty_coord,
    );
    let mark_context = render_ctx.core_view();
    let x = coerce_numeric_channel_with_renderer(
        subplot,
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        CARTESIAN_SUBPLOT_X_CHANNEL,
        &mark_context,
        0.0,
    )?;
    let y = coerce_numeric_channel_with_renderer(
        subplot,
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        CARTESIAN_SUBPLOT_Y_CHANNEL,
        &mark_context,
        0.0,
    )?;
    let child_count = x.len().max(y.len()).max(1);
    let xs = expanded_values(&x, child_count, CARTESIAN_SUBPLOT_X_CHANNEL)?;
    let ys = expanded_values(&y, child_count, CARTESIAN_SUBPLOT_Y_CHANNEL)?;

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
    let child_plot = runtime
        .prepare_plot(
            cartesian_subplot_child_plot(subplot),
            data_selection,
            inherited_data.as_ref(),
            eval_ctx,
        )
        .await?;

    Ok(Some(PreparedPositionedSubplot {
        subplot,
        child_plots: PreparedPositionedChildPlots::Shared(child_plot),
        child_specs,
    }))
}

fn positioned_render_origin(
    spec: &PositionedChildSpec,
    subplot: &CompiledCartesianSubplot,
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
) -> Result<CartesianPositionedChildMeasurement, AvengerChartError> {
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
    let measurement = child_plot
        .measure(
            &child_eval_ctx,
            &child_layout_spec,
            facet_path,
            &[domain_extents, facet_scoped_domain_extents],
        )
        .await?;

    Ok(CartesianPositionedChildMeasurement {
        child_index: spec.child_index,
        mark_index: spec.mark_index,
        identity: spec.identity.clone(),
        key: spec.key.clone(),
        label: spec.label.clone(),
        container_path: eval_ctx.child_frame_container_path().to_vec(),
        data_override: child_plot.data_override().cloned(),
        measurement,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn measure_cartesian_positioned_subplots(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    let mut next_child_index = 0usize;
    let mut prepared_subplots = Vec::new();
    for subplot in compiled_marks
        .iter()
        .filter_map(|mark| compiled_cartesian_subplot(mark.as_ref()))
    {
        if let Some(prepared) = prepare_positioned_subplot(
            subplot,
            scales,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            facet_path,
            &mut next_child_index,
        )
        .await?
        {
            prepared_subplots.push(prepared);
        }
    }

    if next_child_index == 0 {
        return Ok(Box::new(EmptyCoordMeasurement));
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
    let mut render_placements = Vec::with_capacity(next_child_index);
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
                measure_positioned_child(
                    prepared,
                    spec_index,
                    spec,
                    next_child_index,
                    eval_ctx,
                    facet_path,
                    &coordinated_domain_extents[domain_index],
                    &facet_scoped_domain_extents,
                )
                .await?,
            );
            render_placements.push(ChildFrameRenderPlacement {
                child_index: spec.child_index,
                origin: positioned_render_origin(spec, prepared.subplot),
            });
            domain_index += 1;
        }
    }

    Ok(Box::new(CartesianPositionedCoordMeasurement {
        children,
        placement: ChildFramePlacementResult::new(
            Size2D::new(plot_width, plot_height),
            render_placements,
        ),
    }))
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
        functions_aggregate::average::avg,
        prelude::*,
    };

    use crate::{
        cartesian::{Cartesian, CartesianSubplotPositionChannels, CartesianSymbolPositionChannels},
        error::AvengerChartError,
        marks::Subplot,
        plot::{Plot, compiled::compiled_subplot_payload_child_plot},
        scales::ScaleChannelConfig,
    };
    use avenger_chart_core::{Linear, ScaleSharing};
    use avenger_chart_marks::Symbol;

    use super::compiled_cartesian_subplot;

    fn partitioned_parent_data(ctx: &SessionContext) -> DataFrame {
        let batch = ArrowRecordBatch::try_from_iter(vec![
            (
                "group",
                Arc::new(StringArray::from(vec!["A", "A", "B", "B", "B"])) as Arc<dyn Array>,
            ),
            (
                "px",
                Arc::new(Float64Array::from(vec![1.0, 1.0, 3.0, 3.0, 3.0])) as Arc<dyn Array>,
            ),
            (
                "py",
                Arc::new(Float64Array::from(vec![1.0, 1.0, 2.0, 2.0, 2.0])) as Arc<dyn Array>,
            ),
            (
                "x",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as Arc<dyn Array>,
            ),
            (
                "y",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 1.5, 2.5, 3.5])) as Arc<dyn Array>,
            ),
        ])
        .expect("partitioned parent batch");
        ctx.read_batch(batch).expect("partitioned parent data")
    }

    fn inherited_child_plot() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(6.0))))
                })
                .y_with(col("y"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(4.0))))
                }),
        )
    }

    fn find_symbol_mark(mark: &SceneMark) -> Option<&SceneSymbolMark> {
        match mark {
            SceneMark::Symbol(symbol) => Some(symbol),
            SceneMark::Group(group) => group.marks.iter().find_map(find_symbol_mark),
            _ => None,
        }
    }

    #[tokio::test]
    async fn partitioned_cartesian_subplots_filter_child_data() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .plot_size(360.0, 240.0)
            .data(partitioned_parent_data(&ctx))
            .mark(
                Subplot::new(inherited_child_plot())
                    .partition_by(col("group"))
                    .subplot_x(avg(col("px")))
                    .subplot_y(avg(col("py")))
                    .plot_size(80.0, 64.0),
            );

        let compiled = plot.compile(&ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let first_path = group_names
            .get("cartesian_subplot_0_0_A")
            .expect("first partitioned subplot group should render");
        let second_path = group_names
            .get("cartesian_subplot_0_1_B")
            .expect("second partitioned subplot group should render");
        let first_group = evaluated
            .scene_graph
            .get_mark(first_path)
            .expect("first partitioned subplot path should resolve");
        let second_group = evaluated
            .scene_graph
            .get_mark(second_path)
            .expect("second partitioned subplot path should resolve");

        assert_eq!(
            find_symbol_mark(first_group)
                .expect("first partition should render symbols")
                .len,
            2
        );
        assert_eq!(
            find_symbol_mark(second_group)
                .expect("second partition should render symbols")
                .len,
            3
        );
        Ok(())
    }

    #[tokio::test]
    async fn cartesian_subplot_placement_scales_use_subplot_channel_names()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .data(partitioned_parent_data(&ctx))
            .mark(
                Subplot::new(inherited_child_plot())
                    .subplot_x_with(col("px"), |c| c.with_scale_sharing(ScaleSharing::Level(2)))
                    .subplot_y_with(col("py"), |c| c.with_scale_sharing(ScaleSharing::Level(1))),
            );

        let compiled = plot.compile(&ctx).await?;
        assert_eq!(
            compiled
                .scale_to_coord_channel
                .get("subplot_x")
                .map(String::as_str),
            Some("x")
        );
        assert_eq!(
            compiled
                .scale_to_coord_channel
                .get("subplot_y")
                .map(String::as_str),
            Some("y")
        );
        assert!(
            !compiled.scale_to_coord_channel.contains_key("x"),
            "parent placement scales should not reuse the child Cartesian x scale name"
        );
        assert!(
            !compiled.scale_to_coord_channel.contains_key("y"),
            "parent placement scales should not reuse the child Cartesian y scale name"
        );

        let parent_channels = compiled.marks[0].data_context().channels();
        assert!(parent_channels.contains_key("subplot_x"));
        assert!(parent_channels.contains_key("subplot_y"));
        assert_eq!(
            parent_channels["subplot_x"]
                .get_scale_name("subplot_x")
                .as_deref(),
            Some("subplot_x")
        );
        assert_eq!(
            parent_channels["subplot_y"]
                .get_scale_name("subplot_y")
                .as_deref(),
            Some("subplot_y")
        );
        assert_eq!(
            parent_channels["subplot_x"].get_share_mode(),
            Some(ScaleSharing::Level(2))
        );
        assert_eq!(
            parent_channels["subplot_y"].get_share_mode(),
            Some(ScaleSharing::Level(1))
        );

        let subplot = compiled_cartesian_subplot(compiled.marks[0].as_ref())
            .expect("compiled mark should be a Cartesian subplot");
        let child_plot = compiled_subplot_payload_child_plot(subplot.payload());
        assert_eq!(
            child_plot
                .scale_to_coord_channel
                .get("x")
                .map(String::as_str),
            Some("x")
        );
        assert_eq!(
            child_plot
                .scale_to_coord_channel
                .get("y")
                .map(String::as_str),
            Some("y")
        );
        assert!(
            !child_plot.scale_to_coord_channel.contains_key("subplot_x"),
            "child Cartesian plot should keep its own x scale name"
        );
        assert!(
            !child_plot.scale_to_coord_channel.contains_key("subplot_y"),
            "child Cartesian plot should keep its own y scale name"
        );

        Ok(())
    }

    #[tokio::test]
    async fn partitioned_cartesian_subplots_reject_raw_position_channels() {
        let ctx = SessionContext::new();
        let plot = Plot::<Cartesian>::new()
            .data(partitioned_parent_data(&ctx))
            .mark(
                Subplot::new(inherited_child_plot())
                    .partition_by(col("group"))
                    .subplot_x(col("px"))
                    .subplot_y(1.0),
            );

        let err = match plot.compile(&ctx).await {
            Ok(_) => panic!("raw row-level placement should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("must be an aggregate, literal/constant, or the partition expression"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn partitioned_cartesian_subplots_reject_explicit_child_data() {
        let ctx = SessionContext::new();
        let child_plot = inherited_child_plot().data(partitioned_parent_data(&ctx));
        let plot = Plot::<Cartesian>::new()
            .data(partitioned_parent_data(&ctx))
            .mark(
                Subplot::new(child_plot)
                    .partition_by(col("group"))
                    .subplot_x(avg(col("px")))
                    .subplot_y(avg(col("py"))),
            );

        let err = match plot.compile(&ctx).await {
            Ok(_) => panic!("partitioned subplot with explicit child data should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("remove plot-level data from the child plot"),
            "unexpected error: {err}"
        );
    }
}
