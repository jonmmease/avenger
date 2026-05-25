//! Coordinate-positioned subplot measurement for Cartesian parents.
//!
//! A Cartesian plot can act as a normal data coordinate system and, when it
//! contains `Subplot<Cartesian>` marks, as a child-frame container. The child
//! frames are measured here so layout, debug overlays, and rendering all use the
//! same saved geometry.

use std::{any::Any, collections::HashMap, future::Future, pin::Pin, sync::Arc};

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::{
    arrow::record_batch::RecordBatch, common::ScalarValue, dataframe::DataFrame,
    logical_expr::Expr, prelude::SessionContext,
};
use serde::{Deserialize, Serialize};

use crate::{
    cartesian::{Cartesian, CartesianPositionConfig},
    channel::{ChannelValue, PositionConfig},
    chart_core::{MarkRuntimeContext, RadiusExpression, coerce_numeric_channel_with_renderer},
    container::{
        ChildFrameKey, ChildFramePlacementResult, ChildFrameRenderPlacement, ChildFrameScopeKey,
        ChildFrameSharingLevel, ContainerPathSegment,
    },
    coords::{
        CoordMeasurement, CoordinateSystemTransformCore, EmptyCoordMeasurement,
        OverflowSpaceRequirement,
    },
    error::AvengerChartError,
    layout::Size2D,
    marks::{
        ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore, CompiledMarkState,
        CompiledSubplotPayload, Mark, Subplot, SubplotContainerCoordinateSystem,
        compile_subplot_payload,
    },
    plot::compiled::{
        ChildFrameDataSelection, ChildFrameDomainSharingInput, ChildFrameRuntime, CompiledPlot,
        ComponentsMeasurement, MarkDataRequest, child_frame_container_overflow,
        child_frame_container_view_from_cartesian_positioned, compiled_subplot_payload_child_plot,
        coordinated_child_frame_domain_extents, prepare_mark_data_runtime,
    },
    render::{EvaluationContext, RenderContext},
    scales::ConfiguredScaleWithSpec,
};

impl Subplot<Cartesian> {
    /// Set the parent x-position for coordinate-positioned child plot frames.
    pub fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    /// Set the parent y-position for coordinate-positioned child plot frames.
    pub fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    /// Configure the parent x-position channel for coordinate-positioned child plot frames.
    pub fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        self.with_position_config("x", value.into(), f)
    }

    /// Configure the parent y-position channel for coordinate-positioned child plot frames.
    pub fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        self.with_position_config("y", value.into(), f)
    }

    /// Set the child plot-area width used for each positioned child frame.
    pub fn plot_width(mut self, width: f32) -> Self {
        self.set_plot_width_config(Some(width));
        self
    }

    /// Set the child plot-area height used for each positioned child frame.
    pub fn plot_height(mut self, height: f32) -> Self {
        self.set_plot_height_config(Some(height));
        self
    }

    /// Set both child plot-area dimensions used for each positioned child frame.
    pub fn plot_size(mut self, width: f32, height: f32) -> Self {
        self.set_plot_width_config(Some(width));
        self.set_plot_height_config(Some(height));
        self
    }

    fn with_position_config<F>(self, channel: &'static str, value: ChannelValue, f: F) -> Self
    where
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        let configured = f(CartesianPositionConfig::new(value));
        let (channel_value, axis_config) = configured.take_axis_config();
        let mut mark = self.with_channel_value(channel, channel_value);
        if let Some(axis_config) = axis_config {
            mark.state_mut()
                .axis_configs
                .insert(channel.to_string(), Arc::new(axis_config));
        }
        mark
    }
}

#[async_trait::async_trait]
impl SubplotContainerCoordinateSystem for Cartesian {
    async fn compile_subplot_mark(
        subplot: &Subplot<Self>,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        subplot.validate_no_facet_channels("Cartesian")?;

        Ok(Arc::new(CompiledCartesianSubplot {
            payload: compile_subplot_payload(subplot, compiled_state, session_context).await?,
            plot_width: subplot.plot_width_config().unwrap_or(80.0).max(1.0),
            plot_height: subplot.plot_height_config().unwrap_or(80.0).max(1.0),
        }))
    }
}

/// Compiled child-plot mark positioned by Cartesian x/y channels.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianSubplot {
    payload: CompiledSubplotPayload,
    plot_width: f32,
    plot_height: f32,
}

impl CompiledCartesianSubplot {
    pub fn compiled_subplot(&self) -> &CompiledPlot {
        compiled_subplot_payload_child_plot(&self.payload)
    }

    pub fn label(&self) -> Option<&str> {
        self.payload.label()
    }

    pub fn key(&self) -> Option<&str> {
        self.payload.key()
    }

    pub fn mark_index(&self) -> usize {
        self.payload.mark_index()
    }

    pub fn plot_width(&self) -> f32 {
        self.plot_width
    }

    pub fn plot_height(&self) -> f32 {
        self.plot_height
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.payload.inherits_parent_data()
    }

    fn group_name(&self, child_index: usize) -> String {
        match self.key() {
            Some(key) => format!(
                "cartesian_subplot_{}_{}_{}",
                self.mark_index(),
                child_index,
                key
            ),
            None => format!("cartesian_subplot_{}_{}", self.mark_index(), child_index),
        }
    }

    pub(crate) fn render_with_context<'a>(
        &'a self,
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

            for child in cartesian_measurement.children_for_mark(self.mark_index()) {
                let render_placement =
                    child_frame_placement
                        .child(child.child_index)
                        .ok_or_else(|| {
                            AvengerChartError::InternalError(format!(
                                "Missing Cartesian child-frame placement for child index {}",
                                child.child_index
                            ))
                        })?;

                let mut params = self.compiled_subplot().get_default_params().clone();
                params.extend(context.eval.params.clone());
                let sharing_level = ChildFrameSharingLevel::positioned_subplot(
                    child.child_index,
                    child_count,
                    child.mark_index,
                    child.row_index,
                    child.key.as_deref(),
                );
                let child_eval_ctx = context
                    .eval
                    .with_params(params)
                    .with_child_frame_sharing_level_appended(sharing_level);
                let components = Box::pin(self.compiled_subplot().build_plot_components(
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
                    name: self.group_name(child.child_index),
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
}

pub fn compiled_cartesian_subplot(mark: &dyn CompiledMark) -> Option<&CompiledCartesianSubplot> {
    if mark.mark_type() != "subplot" {
        return None;
    }
    mark.as_any().downcast_ref::<CompiledCartesianSubplot>()
}

impl CompiledMarkCore for CompiledCartesianSubplot {
    fn state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        self.payload.compiled_state_mut()
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.payload.compiled_state().data
    }

    fn mark_type(&self) -> &str {
        "subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "x",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianSubplot {
    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Cartesian subplot marks require the top-level layout render dispatcher".to_string(),
        ))
    }
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
    pub(crate) row_index: usize,
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
            self.row_index,
            self.key.as_deref(),
        )
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
    row_index: usize,
    key: Option<&str>,
) -> ChildFrameScopeKey {
    ChildFrameScopeKey::new(
        container_path.to_vec(),
        ChildFrameKey::PositionedSubplot {
            mark_index,
            row_index,
            key: key.map(ToOwned::to_owned),
        },
    )
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

#[derive(Debug, Clone)]
struct PositionedChildSpec {
    child_index: usize,
    mark_index: usize,
    row_index: usize,
    key: Option<String>,
    label: Option<String>,
    x: f32,
    y: f32,
}

struct PreparedPositionedSubplot<'a> {
    subplot: &'a CompiledCartesianSubplot,
    child_plot: crate::plot::compiled::PreparedChildFramePlot<'a>,
    child_specs: Vec<PositionedChildSpec>,
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
        "x",
        &mark_context,
        0.0,
    )?;
    let y = coerce_numeric_channel_with_renderer(
        subplot,
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        "y",
        &mark_context,
        0.0,
    )?;
    let child_count = x.len().max(y.len()).max(1);
    let xs = expanded_values(&x, child_count, "x")?;
    let ys = expanded_values(&y, child_count, "y")?;

    let mut child_specs = Vec::with_capacity(child_count);
    for row_index in 0..child_count {
        let child_index = *next_child_index;
        *next_child_index += 1;
        child_specs.push(PositionedChildSpec {
            child_index,
            mark_index: subplot.mark_index(),
            row_index,
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
            subplot.compiled_subplot(),
            data_selection,
            inherited_data.as_ref(),
            eval_ctx,
        )
        .await?;

    Ok(Some(PreparedPositionedSubplot {
        subplot,
        child_plot,
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
    spec: &PositionedChildSpec,
    total_child_count: usize,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    domain_extents: &HashMap<String, crate::scales::DomainExtent>,
) -> Result<CartesianPositionedChildMeasurement, AvengerChartError> {
    let runtime = ChildFrameRuntime::new();
    let child_layout_spec = runtime.fixed_plot_area_layout_spec(
        prepared.subplot.plot_width(),
        prepared.subplot.plot_height(),
    );
    let sharing_level = ChildFrameSharingLevel::positioned_subplot(
        spec.child_index,
        total_child_count,
        spec.mark_index,
        spec.row_index,
        spec.key.as_deref(),
    );
    let child_eval_ctx = runtime.eval_context(eval_ctx, sharing_level);
    let measurement = prepared
        .child_plot
        .measure(
            &child_eval_ctx,
            &child_layout_spec,
            facet_path,
            &[domain_extents],
        )
        .await?;

    Ok(CartesianPositionedChildMeasurement {
        child_index: spec.child_index,
        mark_index: spec.mark_index,
        row_index: spec.row_index,
        key: spec.key.clone(),
        label: spec.label.clone(),
        container_path: eval_ctx.child_frame_container_path().to_vec(),
        data_override: prepared.child_plot.data_override().cloned(),
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
                spec.row_index,
                spec.key.as_deref(),
            ));
        }
    }

    let mut domain_inputs = Vec::with_capacity(next_child_index);
    let mut scope_index = 0usize;
    for prepared in &prepared_subplots {
        for _spec in &prepared.child_specs {
            domain_inputs.push(ChildFrameDomainSharingInput::new(
                &scope_keys[scope_index],
                prepared.child_plot.local_domain_extents(),
                prepared.child_plot.channel_domain_sharing_levels(),
            ));
            scope_index += 1;
        }
    }
    let coordinated_domain_extents = coordinated_child_frame_domain_extents(&domain_inputs);

    let mut children = Vec::with_capacity(next_child_index);
    let mut render_placements = Vec::with_capacity(next_child_index);
    let mut domain_index = 0usize;
    for prepared in &prepared_subplots {
        for spec in &prepared.child_specs {
            children.push(
                measure_positioned_child(
                    prepared,
                    spec,
                    next_child_index,
                    eval_ctx,
                    facet_path,
                    &coordinated_domain_extents[domain_index],
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
