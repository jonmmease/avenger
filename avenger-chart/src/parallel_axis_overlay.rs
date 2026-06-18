use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{CompiledMarkCore, CoordMeasurement, SubplotDataSource};
use avenger_chart_parallel::{CompiledParallelAxisOverlay, ParallelTransform};
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
};
use datafusion::{common::ScalarValue, dataframe::DataFrame};

use crate::{
    error::AvengerChartError,
    marks::CompiledMark,
    plot::compiled::{
        ChildFrameDataSelection, ChildFrameRuntime, CompiledPlot, ComponentsMeasurement,
        PreparedChildFramePlot, compiled_subplot_payload_child_plot,
    },
    render::RenderContext,
    scales::ConfiguredScaleWithSpec,
};

pub(crate) fn parallel_axis_overlay_ref(
    mark: &dyn CompiledMark,
) -> Option<&CompiledParallelAxisOverlay> {
    if mark.mark_type() != "parallel_axis_overlay" {
        return None;
    }
    mark.as_any().downcast_ref::<CompiledParallelAxisOverlay>()
}

fn parallel_axis_overlay_child_plot(overlay: &CompiledParallelAxisOverlay) -> &CompiledPlot {
    compiled_subplot_payload_child_plot(overlay.payload())
}

#[derive(Clone, Debug)]
pub(crate) struct ParallelAxisOverlayCoordMeasurement {
    children: Vec<ParallelAxisOverlayChildMeasurement>,
}

impl ParallelAxisOverlayCoordMeasurement {
    fn new(children: Vec<ParallelAxisOverlayChildMeasurement>) -> Self {
        Self { children }
    }

    fn children_for_mark(
        &self,
        mark_index: usize,
    ) -> impl Iterator<Item = &ParallelAxisOverlayChildMeasurement> {
        self.children
            .iter()
            .filter(move |child| child.mark_index == mark_index)
    }
}

impl CoordMeasurement for ParallelAxisOverlayCoordMeasurement {
    fn clone_box(&self) -> Box<dyn CoordMeasurement> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[derive(Clone, Debug)]
struct ParallelAxisOverlayChildMeasurement {
    child_index: usize,
    mark_index: usize,
    dimension_id: String,
    origin: [f32; 2],
    data_override: Option<DataFrame>,
    measurement: ComponentsMeasurement,
}

impl ParallelAxisOverlayChildMeasurement {
    fn group_name(&self) -> String {
        format!(
            "parallel_axis_overlay_{}_{}_{}",
            self.mark_index, self.child_index, self.dimension_id
        )
    }
}

fn data_selection_for_overlay(overlay: &CompiledParallelAxisOverlay) -> ChildFrameDataSelection {
    match overlay.payload().data_source() {
        SubplotDataSource::ExplicitChild => ChildFrameDataSelection::ExplicitChild,
        SubplotDataSource::InheritParent => ChildFrameDataSelection::InheritParent,
    }
}

async fn prepare_overlay_child<'a>(
    overlay: &'a CompiledParallelAxisOverlay,
    data: Option<&DataFrame>,
    eval_ctx: &crate::render::EvaluationContext,
) -> Result<PreparedChildFramePlot<'a>, AvengerChartError> {
    let child_plot = parallel_axis_overlay_child_plot(overlay);
    if child_plot
        .coord_transform
        .as_any()
        .downcast_ref::<crate::cartesian::Cartesian>()
        .is_none()
    {
        return Err(AvengerChartError::InvalidArgument(
            "ParallelAxisOverlay child plots must use Cartesian coordinates".to_string(),
        ));
    }

    let data_selection = data_selection_for_overlay(overlay);
    let inherited_data = match data_selection {
        ChildFrameDataSelection::ExplicitChild => None,
        ChildFrameDataSelection::InheritParent => data,
    };
    Box::pin(ChildFrameRuntime::new().prepare_plot(
        child_plot,
        data_selection,
        inherited_data,
        eval_ctx,
    ))
    .await
}

#[allow(clippy::too_many_arguments)]
async fn measure_overlay_child(
    overlay: &CompiledParallelAxisOverlay,
    prepared: &PreparedChildFramePlot<'_>,
    child_index: usize,
    child_count: usize,
    parent_scale: &ConfiguredScaleWithSpec,
    origin: [f32; 2],
    plot_height: f32,
    eval_ctx: &crate::render::EvaluationContext,
    facet_path: &[ScalarValue],
) -> Result<ParallelAxisOverlayChildMeasurement, AvengerChartError> {
    let runtime = ChildFrameRuntime::new();
    let child_layout_spec = runtime.fixed_plot_area_layout_spec(overlay.width_px(), plot_height);
    let child_eval_ctx = runtime.eval_context(
        eval_ctx,
        crate::container::ChildFrameSharingLevel::positioned_subplot(
            child_index,
            child_count,
            overlay.state().mark_index(),
            child_index,
            Some(overlay.dimension_id()),
        ),
    );
    let scale_overrides = HashMap::from([("y".to_string(), parent_scale.clone())]);
    let measurement = Box::pin(prepared.measure_with_scale_overrides(
        &child_eval_ctx,
        &child_layout_spec,
        facet_path,
        &[],
        scale_overrides,
    ))
    .await?;

    Ok(ParallelAxisOverlayChildMeasurement {
        child_index,
        mark_index: overlay.state().mark_index(),
        dimension_id: overlay.dimension_id().to_string(),
        origin,
        data_override: prepared.data_override().cloned(),
        measurement,
    })
}

pub(crate) async fn measure_parallel_axis_overlays(
    transform: &ParallelTransform,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &crate::render::EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Option<Box<dyn CoordMeasurement>>, AvengerChartError> {
    let overlays = compiled_marks
        .iter()
        .filter_map(|mark| parallel_axis_overlay_ref(mark.as_ref()))
        .collect::<Vec<_>>();
    if overlays.is_empty() {
        return Ok(None);
    }

    let frame = transform.resolve_frame_with_params(plot_width, eval_ctx.params())?;
    let mut prepared = Vec::with_capacity(overlays.len());
    for overlay in &overlays {
        let slot = frame.slot(overlay.dimension_id()).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "ParallelAxisOverlay references unknown dimension '{}'",
                overlay.dimension_id()
            ))
        })?;
        let parent_scale = scales.get(&slot.scale_name).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Missing scale '{}' for parallel axis overlay dimension '{}'",
                slot.scale_name,
                overlay.dimension_id()
            ))
        })?;
        let origin = [
            slot.display_x - overlay.width_px() / 2.0 + overlay.x_offset_px(),
            0.0,
        ];
        prepared.push((
            *overlay,
            Box::pin(prepare_overlay_child(overlay, data, eval_ctx)).await?,
            parent_scale.clone(),
            origin,
        ));
    }

    let child_count = prepared.len();
    let mut children = Vec::with_capacity(child_count);
    for (child_index, (overlay, prepared_child, parent_scale, origin)) in
        prepared.iter().enumerate()
    {
        children.push(
            Box::pin(measure_overlay_child(
                overlay,
                prepared_child,
                child_index,
                child_count,
                parent_scale,
                *origin,
                plot_height,
                eval_ctx,
                facet_path,
            ))
            .await?,
        );
    }

    Ok(Some(Box::new(ParallelAxisOverlayCoordMeasurement::new(
        children,
    ))))
}

pub(crate) async fn render_parallel_axis_overlay_with_context(
    overlay: &CompiledParallelAxisOverlay,
    context: &RenderContext<'_>,
) -> Result<Vec<SceneMark>, AvengerChartError> {
    let measurement = context
        .coord_measurement()
        .as_any()
        .downcast_ref::<ParallelAxisOverlayCoordMeasurement>()
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                "ParallelAxisOverlay marks require ParallelAxisOverlayCoordMeasurement".to_string(),
            )
        })?;
    let child_plot = parallel_axis_overlay_child_plot(overlay);
    let mut marks = Vec::new();

    for child in measurement.children_for_mark(overlay.state().mark_index()) {
        let mut params = child_plot.get_default_params().clone();
        params.extend(context.eval.params.clone());
        let child_eval_ctx = context.eval.with_params(params);
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
                rows.prepend_subplot_id(overlay.state().id.as_deref());
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
                rows.prepend_subplot_id(overlay.state().id.as_deref());
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

        let clip = if overlay.clip() {
            Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: overlay.width_px(),
                height: context.plot_height(),
            }
        } else {
            Clip::None
        };

        marks.push(SceneMark::Group(SceneGroup {
            name: child.group_name(),
            origin: child.origin,
            clip,
            marks: all_marks,
            gradients: Vec::new(),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: overlay.state().zindex,
            interactive: true,
        }));
    }

    Ok(marks)
}
