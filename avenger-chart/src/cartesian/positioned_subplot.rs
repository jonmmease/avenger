//! Coordinate-positioned subplot measurement for Cartesian parents.
//!
//! A Cartesian plot can act as a normal data coordinate system and, when it
//! contains `Subplot<Cartesian>` marks, as a child-frame container. The child
//! frames are measured here so layout, debug overlays, and rendering all use the
//! same saved geometry.

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};

use crate::{
    container::{
        ChildFrameKey, ChildFramePlacementResult, ChildFrameRenderPlacement, ChildFrameScopeKey,
        ChildFrameSharingLevel, ContainerPathSegment,
    },
    coords::{CoordMeasurement, EmptyCoordMeasurement},
    error::AvengerChartError,
    layout::Size2D,
    marks::{
        CompiledCartesianSubplot, CompiledMark, subplot::compiled_cartesian_subplot,
        util::coerce_numeric_channel_with_renderer,
    },
    plot::compiled::{
        ChildFrameDataSelection, ChildFrameDomainSharingInput, ComponentsMeasurement,
        MarkDataRequest, child_frame_container_view_from_cartesian_positioned,
        child_frame_eval_context, coordinated_child_frame_domain_extents,
        fixed_child_plot_area_layout_spec, prepare_child_frame_plot, prepare_mark_data_runtime,
    },
    render::{EvaluationContext, RenderContext},
    scales::ConfiguredScaleWithSpec,
};

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

    fn child_frame_container_view<'a>(
        &'a self,
        _measurement: &'a ComponentsMeasurement,
    ) -> Result<Option<crate::container::ChildFrameContainerView<'a>>, AvengerChartError> {
        Ok(Some(child_frame_container_view_from_cartesian_positioned(
            self,
        )?))
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
    let x = coerce_numeric_channel_with_renderer(
        subplot,
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        "x",
        &render_ctx,
        0.0,
    )?;
    let y = coerce_numeric_channel_with_renderer(
        subplot,
        prepared_mark.data_batch.as_ref(),
        &prepared_mark.scalar_batch,
        "y",
        &render_ctx,
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
    let child_plot = prepare_child_frame_plot(
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
    let child_layout_spec = fixed_child_plot_area_layout_spec(
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
    let child_eval_ctx = child_frame_eval_context(eval_ctx, sharing_level);
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
