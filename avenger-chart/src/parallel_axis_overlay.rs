use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{CompiledMarkCore, CoordMeasurement, SubplotDataSource};
use avenger_chart_parallel::{CompiledParallelAxisOverlay, ParallelTransform};
use avenger_scales::scales::linear::LinearScale;
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
    scales::{ConfiguredScaleWithSpec, Linear, Scale, ScaleRangeBinding},
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

fn normalized_overlay_x_scale(width_px: f32) -> ConfiguredScaleWithSpec {
    ConfiguredScaleWithSpec::with_range_binding(
        Scale::<Linear>::new().into_auto(),
        LinearScale::configured((0.0, 1.0), (0.0, width_px.max(1.0))),
        ScaleRangeBinding::Independent,
    )
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
    if child_plot.scale_specs().contains_key("y") {
        return Err(AvengerChartError::InvalidArgument(
            "ParallelAxisOverlay child plots cannot define their own y scale; y is supplied by the selected parallel dimension"
                .to_string(),
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
    let scale_fallbacks = HashMap::from([(
        "x".to_string(),
        normalized_overlay_x_scale(overlay.width_px()),
    )]);
    let scale_overrides = HashMap::from([("y".to_string(), parent_scale.clone())]);
    let measurement = Box::pin(prepared.measure_with_scale_fallbacks_and_overrides(
        &child_eval_ctx,
        &child_layout_spec,
        facet_path,
        &[],
        scale_fallbacks,
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
        if overlay.show_child_chrome() && !child_chrome_event_datums.is_empty() {
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
        if overlay.show_child_chrome() {
            all_marks.extend(components.guide_marks);
            all_marks.extend(components.legend_marks);
            all_marks.extend(components.title_marks);
            all_marks.extend(components.subtitle_marks);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_scenegraph::marks::{
        line::SceneLineMark, rect::SceneRectMark, symbol::SceneSymbolMark,
    };
    use datafusion::prelude::{SessionContext, col, lit};
    use indexmap::IndexMap;

    use crate::event::{ChartEventBinding, ChartEventType};
    use crate::prelude::*;

    fn find_group_by_prefix<'a>(marks: &'a [SceneMark], prefix: &str) -> Option<&'a SceneGroup> {
        for mark in marks {
            if let SceneMark::Group(group) = mark {
                if group.name.starts_with(prefix) {
                    return Some(group);
                }
                if let Some(found) = find_group_by_prefix(&group.marks, prefix) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn first_rect_mark(marks: &[SceneMark]) -> Option<&SceneRectMark> {
        for mark in marks {
            match mark {
                SceneMark::Rect(rect) => return Some(rect),
                SceneMark::Group(group) => {
                    if let Some(rect) = first_rect_mark(&group.marks) {
                        return Some(rect);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn first_symbol_mark(marks: &[SceneMark]) -> Option<&SceneSymbolMark> {
        for mark in marks {
            match mark {
                SceneMark::Symbol(symbol) => return Some(symbol),
                SceneMark::Group(group) => {
                    if let Some(symbol) = first_symbol_mark(&group.marks) {
                        return Some(symbol);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn first_line_mark(marks: &[SceneMark]) -> Option<&SceneLineMark> {
        for mark in marks {
            match mark {
                SceneMark::Line(line) => return Some(line),
                SceneMark::Group(group) => {
                    if let Some(line) = first_line_mark(&group.marks) {
                        return Some(line);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn clip_dimensions(group: &SceneGroup) -> (f32, f32) {
        let Clip::Rect { width, height, .. } = group.clip else {
            panic!("expected rect clip");
        };
        (width, height)
    }

    fn first_rect_x2(rect: &SceneRectMark) -> f32 {
        rect.x2
            .as_ref()
            .expect("x2")
            .as_vec(rect.len as usize, rect.indices.as_ref())[0]
    }

    fn first_rect_y2(rect: &SceneRectMark) -> f32 {
        rect.y2
            .as_ref()
            .expect("y2")
            .as_vec(rect.len as usize, rect.indices.as_ref())[0]
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 0.01,
            "expected {expected}, got {actual}"
        );
    }

    fn parallel_for_overlay_test() -> Parallel {
        Parallel::new().dimension_with("speed", col("speed"), |d| {
            d.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false).zero(false))
                .axis(|axis| axis.visible(false))
        })
    }

    #[tokio::test]
    async fn axis_overlay_defaults_child_x_to_normalized_scale() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx.sql("SELECT 50.0 AS speed").await?;
        let overlay = ParallelAxisOverlay::new(
            "speed",
            Plot::<Cartesian>::new()
                .mark(
                    Rect::new()
                        .x(lit(0.0))
                        .x2(lit(1.0))
                        .y(lit(25.0))
                        .y2(lit(75.0))
                        .fill("rgba(37, 99, 235, 0.20)")
                        .stroke("#2563eb"),
                )
                .mark(
                    Symbol::new()
                        .x(lit(0.5))
                        .y(lit(50.0))
                        .size(25.0)
                        .fill("#2563eb"),
                ),
        )
        .id("speed_overlay")
        .width_px(40.0);

        let compiled = Plot::with_coord(parallel_for_overlay_test())
            .canvas_size(220.0, 160.0)
            .plot_size(120.0, 100.0)
            .data(data)
            .event_binding(
                ChartEventBinding::on(ChartEventType::Click)
                    .filter(crate::event::datum("speed").is_not_null()),
            )
            .mark(overlay)
            .compile(&ctx)
            .await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;
        let overlay_group =
            find_group_by_prefix(&evaluated.scene_graph.marks, "parallel_axis_overlay_")
                .expect("overlay group should render");
        assert_eq!(
            overlay_group.marks.len(),
            1,
            "child guide/title/legend chrome should be suppressed by default"
        );
        let (clip_width, clip_height) = clip_dimensions(overlay_group);
        assert_close(clip_width, 40.0);
        assert_close(clip_height, 100.0);

        let rect = first_rect_mark(&overlay_group.marks).expect("overlay rect should render");
        assert_close(rect.x_vec()[0], 0.0);
        assert_close(first_rect_x2(rect), clip_width);
        assert_close(rect.y_vec()[0], clip_height * 0.75);
        assert_close(first_rect_y2(rect), clip_height * 0.25);

        let symbol =
            first_symbol_mark(&overlay_group.marks).expect("center overlay symbol should render");
        assert_close(symbol.x_vec()[0], clip_width * 0.5);
        assert_close(symbol.y_vec()[0], clip_height * 0.5);

        let overlay_event_rows = evaluated
            .event_datums
            .rows
            .iter()
            .filter(|rows| rows.subplot_id_path == ["speed_overlay"])
            .collect::<Vec<_>>();
        assert!(
            overlay_event_rows.len() >= 2,
            "rect and symbol child rows should be forwarded under the overlay id"
        );
        let source_speed = overlay_event_rows
            .iter()
            .find_map(|rows| {
                rows.rows
                    .column_by_name("speed")
                    .and_then(|column| ScalarValue::try_from_array(column, 0).ok())
            })
            .expect("inherited source row should include speed");
        assert_eq!(source_speed, ScalarValue::Float64(Some(50.0)));
        Ok(())
    }

    #[tokio::test]
    async fn axis_overlay_child_categorical_x_keeps_local_scale() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx
            .sql("SELECT 50.0 AS speed, 'left' AS slot UNION ALL SELECT 60.0, 'right'")
            .await?;
        let overlay = ParallelAxisOverlay::new(
            "speed",
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("slot"))
                    .y(col("speed"))
                    .size(60.0)
                    .fill("#2563eb"),
            ),
        )
        .width_px(48.0);

        let compiled = Plot::with_coord(parallel_for_overlay_test())
            .canvas_size(220.0, 160.0)
            .plot_size(120.0, 100.0)
            .data(data)
            .mark(overlay)
            .compile(&ctx)
            .await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;
        let overlay_group =
            find_group_by_prefix(&evaluated.scene_graph.marks, "parallel_axis_overlay_")
                .expect("overlay group should render");
        let (clip_width, _) = clip_dimensions(overlay_group);
        assert_close(clip_width, 48.0);

        let symbol =
            first_symbol_mark(&overlay_group.marks).expect("overlay symbols should render");
        let xs = symbol.x_vec();
        assert_eq!(xs.len(), 2);
        assert!(xs.iter().all(|x| *x >= 0.0 && *x <= clip_width));
        assert!(
            (xs[0] - xs[1]).abs() > 1.0,
            "categorical child x scale should place distinct categories"
        );
        Ok(())
    }

    #[tokio::test]
    async fn axis_overlay_child_y_values_do_not_expand_parent_dimension_domain()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx
            .sql("SELECT 50.0 AS speed UNION ALL SELECT 60.0 AS speed")
            .await?;
        let compiled = Plot::with_coord(Parallel::new().dimension("speed", col("speed")))
            .canvas_size(220.0, 160.0)
            .plot_size(120.0, 100.0)
            .data(data.clone())
            .mark(ParallelAxisOverlay::new(
                "speed",
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .x(lit(0.0))
                        .x2(lit(1.0))
                        .y(lit(-1000.0))
                        .y2(lit(2000.0)),
                ),
            ))
            .compile(&ctx)
            .await?;
        let scales = compiled
            .build_scales_for_dataframe(&data, 120.0, 100.0, &ctx, &IndexMap::new())
            .await?;
        let speed_domain = scales
            .get("speed")
            .expect("speed scale")
            .configured()
            .numeric_interval_domain()?;

        assert!(
            speed_domain.0 > -500.0 && speed_domain.1 < 500.0,
            "overlay child y/y2 values should not expand parent speed domain: {speed_domain:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn axis_overlay_explicit_child_data_replaces_parent_data() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let parent_data = ctx.sql("SELECT 80.0 AS speed").await?;
        let child_data = ctx.sql("SELECT 25.0 AS speed").await?;
        let overlay = ParallelAxisOverlay::new(
            "speed",
            Plot::<Cartesian>::new().data(child_data).mark(
                Rect::new()
                    .x(lit(0.0))
                    .x2(lit(1.0))
                    .y(col("speed"))
                    .y2(lit(0.0)),
            ),
        )
        .width_px(40.0);
        let compiled = Plot::with_coord(parallel_for_overlay_test())
            .canvas_size(220.0, 160.0)
            .plot_size(120.0, 100.0)
            .data(parent_data)
            .mark(overlay)
            .compile(&ctx)
            .await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;
        let overlay_group =
            find_group_by_prefix(&evaluated.scene_graph.marks, "parallel_axis_overlay_")
                .expect("overlay group should render");
        let rect =
            first_rect_mark(&overlay_group.marks).expect("explicit child rect should render");

        assert_close(rect.y_vec()[0], 75.0);
        assert_close(first_rect_y2(rect), 100.0);
        Ok(())
    }

    #[tokio::test]
    async fn axis_overlay_store_child_data_replaces_parent_data() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let parent_data = ctx.sql("SELECT 80.0 AS speed").await?;
        let store_batch = ctx
            .sql("SELECT 25.0 AS speed")
            .await?
            .collect()
            .await?
            .into_iter()
            .next()
            .expect("store batch");
        let overlay = ParallelAxisOverlay::new(
            "speed",
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .data_store(StoreData::new("axis_overlay_rows"))
                    .x(lit(0.0))
                    .x2(lit(1.0))
                    .y(col("speed"))
                    .y2(lit(0.0)),
            ),
        )
        .width_px(40.0);
        let compiled = Plot::with_coord(parallel_for_overlay_test())
            .canvas_size(220.0, 160.0)
            .plot_size(120.0, 100.0)
            .data(parent_data)
            .add_store(Store::from_record_batch("axis_overlay_rows", store_batch))
            .mark(overlay)
            .compile(&ctx)
            .await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;
        let overlay_group =
            find_group_by_prefix(&evaluated.scene_graph.marks, "parallel_axis_overlay_")
                .expect("overlay group should render");
        let rect = first_rect_mark(&overlay_group.marks).expect("store child rect should render");

        assert_close(rect.y_vec()[0], 75.0);
        assert_close(first_rect_y2(rect), 100.0);
        Ok(())
    }

    #[tokio::test]
    async fn axis_overlay_and_parallel_line_keep_authored_zindex() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let data = ctx.sql("SELECT 50.0 AS speed").await?;
        let overlay = ParallelAxisOverlay::new(
            "speed",
            Plot::<Cartesian>::new().mark(
                Rect::new()
                    .x(lit(0.0))
                    .x2(lit(1.0))
                    .y(lit(25.0))
                    .y2(lit(75.0)),
            ),
        )
        .zindex(10);
        let compiled = Plot::with_coord(parallel_for_overlay_test())
            .canvas_size(220.0, 160.0)
            .plot_size(120.0, 100.0)
            .data(data)
            .mark(ParallelLine::new().zindex(1))
            .mark(overlay)
            .compile(&ctx)
            .await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;
        let line = first_line_mark(&evaluated.scene_graph.marks).expect("parallel line");
        let overlay_group =
            find_group_by_prefix(&evaluated.scene_graph.marks, "parallel_axis_overlay_")
                .expect("overlay group");

        assert_eq!(line.zindex, Some(1));
        assert_eq!(overlay_group.zindex, Some(10));
        Ok(())
    }

    #[tokio::test]
    async fn axis_overlay_unknown_dimension_errors() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx.sql("SELECT 50.0 AS speed").await?;
        let compiled = Plot::with_coord(parallel_for_overlay_test())
            .canvas_size(220.0, 160.0)
            .plot_size(120.0, 100.0)
            .data(data)
            .mark(ParallelAxisOverlay::new(
                "missing",
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .x(lit(0.0))
                        .x2(lit(1.0))
                        .y(lit(25.0))
                        .y2(lit(75.0)),
                ),
            ))
            .compile(&ctx)
            .await?;
        let err = match compiled.evaluate(&ctx, None).await {
            Ok(_) => panic!("unknown overlay dimension should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("ParallelAxisOverlay references unknown dimension 'missing'"),
            "{err}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn axis_overlay_rejects_child_y_scale_override() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = ctx.sql("SELECT 50.0 AS speed").await?;
        let compiled = Plot::with_coord(parallel_for_overlay_test())
            .canvas_size(220.0, 160.0)
            .plot_size(120.0, 100.0)
            .data(data)
            .mark(ParallelAxisOverlay::new(
                "speed",
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .x(lit(0.0))
                        .x2(lit(1.0))
                        .y_with(lit(25.0), |y| {
                            y.scale_with::<Linear>(|s| {
                                s.domain((0.0, 100.0)).nice(false).zero(false)
                            })
                        })
                        .y2(lit(75.0)),
                ),
            ))
            .compile(&ctx)
            .await?;
        let err = match compiled.evaluate(&ctx, None).await {
            Ok(_) => panic!("child y scale override should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("cannot define their own y scale"),
            "{err}"
        );
        Ok(())
    }
}
