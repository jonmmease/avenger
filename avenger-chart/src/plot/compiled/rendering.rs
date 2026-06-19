//! Rendering pipeline for CompiledPlot

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Instant,
};

use arrow::{
    array::{ArrayRef, Float32Array, Float64Array, UInt32Array},
    compute::take,
    datatypes::Schema,
    record_batch::RecordBatch,
};
use avenger_color::ColorOrGradient;
use avenger_common::{types::LinearScaleAdjustment, value::ScalarOrArray};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
        rect::SceneRectMark,
    },
    scene_graph::SceneGraph,
};
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::lit, prelude::SessionContext,
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use tracing::{Level, debug, trace};

use avenger_chart_core::{
    AxisPosition, DerivedScalarsByChannel, FacetEmptyCellPolicy, FacetWrapColumnMode,
    LegendPosition, ScalarValueHelpers, eval_to_scalars, evaluate_bool_expr, evaluate_f32_expr,
    maybe::Maybe, params_to_datafusion,
};

use crate::{
    concat::compiled_subplot as compiled_concat_subplot,
    coords::{
        CoordMeasureRequest, CoordMeasurement, CoordinatedOverflow, FacetAxis,
        coordinate_overflow_for_guides, coordinate_overflow_for_guides_until,
        measure_coordinate_system_transform,
    },
    error::AvengerChartError,
    facet::{
        coord::{
            FacetBandCoordMeasurement, FacetCellRuntime, facet_band_mut,
            renderable_for_empty_policy, retarget_measurement_plot_area_no_remeasure,
            retarget_measurement_plot_area_policy_no_remeasure,
            retarget_scale_ranges_for_plot_area, sync_measurement_owned_slabs_from_coord,
        },
        debug as facet_debug,
        evaluated_facet_tree::{EvaluatedFacetTree, FacetWrapLayoutContext},
        layout_plan::{FacetBandPaddingFeedback, FacetBandPaddingFeedbackMap},
        marks::facet::{FacetSubplotRef, facet_subplot_ref},
        overflow_projection::{
            FacetOverflowPurpose, FacetOverflowResolutionPhase, FacetOverflowSource,
            boundary_profiles_for_measurement, compute_padding_from_boundary_profiles,
            rendered_subtree_overflow_from_coord_measurement, resolve_facet_overflow,
        },
        subtree_plot_area::{LeafPlotAreaSize, estimate_root_plot_area_from_leaf_size},
    },
    guide::{GuideOverflowPhase, GuideSharingContext, OverflowSpaceRequirement},
    layout::{
        AvengerFrameLayoutSolver, EdgeSlabs, EvaluatedLayoutSpec, EvaluatedMargins,
        EvaluatedSizeMode, FrameAllocation, FrameDimensionSizing, FrameLayout, FrameLayoutInput,
        LayoutBounds, LayoutSpec, Margins, ResolvedLayoutDimensions, Size2D, SizeMode,
    },
    marks::CompiledMark,
    positioned_subplot::{PositionedCoordMeasurement, render_positioned_subplot_with_context},
    render::context::{
        EvaluationMetricsDiagnostics, FacetDimensionSizing, FacetRuntimeSizingMode,
        FacetRuntimeSizingPolicy, FacetSubtreeSnapshotCapture,
    },
    render::{
        CoordinationCheckpoint, EvaluatedEventDatumRows, EvaluatedEventDatumState,
        EvaluatedInteractionScope, EvaluatedInteractionState, EvaluatedPlot, EvaluationContext,
        EvaluationMetrics, EvaluationOptions, FacetSubtreeCheckpoint, FacetSubtreeSelector,
        FacetSubtreeSnapshot, InteractionScopeId, InteractionScopeKind, LayoutDebugOverlayMode,
        LayoutSnapshot, LayoutSolution, PreviewProfileFallbackReason, RefinementCheckpoint,
        RenderContext, RenderState, WholeChartSnapshot,
        debug::{FrameDebugOverlay, create_debug_layout_rects, create_debug_overlay_rects},
    },
    scales::ConfiguredScaleWithSpec,
    serialization::{LogicalExprNodeExt, LogicalPlanNodeExt, serializable_expr_from_expr},
    theme::{Theme, ThemeContext},
};

use super::{
    ChildFrameContainerView, ChildFrameSharingPath, CompiledPlot, ComponentsMeasurement,
    FacetCellProfileIndex, LayoutProfileSnapshot, MarkDataRequest, PlotComponents,
    PreparedMarkData,
    child_frame_coordination::{
        apply_child_frame_layout_alignment, diagnose_child_frame_layout_alignment,
    },
    compiled_subplot_payload_child_plot,
    legends::{HoistedLegendAnchor, HoistedLegendRequest, LegendPlanScope, PreparedLegendPlan},
    prepare_mark_data_runtime,
    scale_provider::{DynamicScaleProvider, ScaleProvider},
    scales::build_scale_builder_from_compiled_plot,
    session::{
        FacetScaleBuilderPrecomputeCacheHandle, FacetSemanticCacheHandle, GuideOverflowCacheHandle,
        LegendMeasurementCacheHandle, ScaleDomainCacheHandle, ScaleDomainCacheScope,
        ScopedParamStore, ScopedSelectionStore, ScopedStoreState, SelectionRevisionFingerprint,
        StoreRevisionFingerprint, TextMeasurementCacheHandle, changed_param_names,
        layout_size_dependency_params, new_plot_session_cache_handles,
        scale_domain_cache_key_for_parts_with_scope,
    },
};

fn plot_contains_responsive_wrap_concat(plot: &CompiledPlot) -> bool {
    if plot
        .coord_transform
        .as_any()
        .downcast_ref::<crate::concat::WrapConcat>()
        .is_some_and(|wrap| matches!(wrap.column_mode(), FacetWrapColumnMode::ResponsiveWidth(_)))
    {
        return true;
    }

    plot.marks.iter().any(|mark| {
        if let Some(subplot) = compiled_concat_subplot(mark.as_ref())
            && plot_contains_responsive_wrap_concat(subplot.compiled_subplot())
        {
            return true;
        }
        if let Some(subplot) = facet_subplot_ref(mark.as_ref())
            && plot_contains_responsive_wrap_concat(subplot.compiled_subplot())
        {
            return true;
        }
        if let Some(subplot) = mark.as_positioned_subplot()
            && plot_contains_responsive_wrap_concat(compiled_subplot_payload_child_plot(
                subplot.payload(),
            ))
        {
            return true;
        }
        false
    })
}

fn derived_scalars_by_channel(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
) -> DerivedScalarsByChannel {
    scales
        .iter()
        .filter_map(|(channel, scale)| {
            if scale.derived_scalars().is_empty() {
                None
            } else {
                Some((channel.clone(), scale.derived_scalars().clone()))
            }
        })
        .collect()
}

fn selection_revision_fingerprint(
    store: Option<&ScopedSelectionStore>,
) -> SelectionRevisionFingerprint {
    store
        .map(ScopedSelectionStore::revision_fingerprint)
        .unwrap_or_default()
}

fn store_revision_fingerprint(store: Option<&ScopedStoreState>) -> StoreRevisionFingerprint {
    store
        .map(ScopedStoreState::revision_fingerprint)
        .unwrap_or_default()
}

fn interaction_scope_content_id(
    coord_node_path: &[usize],
    logical_facet_values: &[ScalarValue],
) -> String {
    let coord_path = coord_node_path
        .iter()
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(".");
    let facet_path = logical_facet_values
        .iter()
        .map(scalar_value_for_scope_id)
        .collect::<Vec<_>>()
        .join("/");
    format!("coord:{coord_path};facet:{facet_path}")
}

fn scalar_value_for_scope_id(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Utf8(Some(value))
        | ScalarValue::LargeUtf8(Some(value))
        | ScalarValue::Utf8View(Some(value)) => value.clone(),
        _ => value.to_string(),
    }
}

struct CachedDataMarks {
    data_marks: Vec<SceneMark>,
    event_datums: Vec<EvaluatedEventDatumRows>,
}

struct RenderedMarkOutput {
    marks: Vec<SceneMark>,
    event_datums: Vec<EvaluatedEventDatumRows>,
}

impl RenderedMarkOutput {
    fn marks_only(marks: Vec<SceneMark>) -> Self {
        Self {
            marks,
            event_datums: Vec::new(),
        }
    }
}

fn set_scene_mark_name(mark: &mut SceneMark, name: &str) {
    match mark {
        SceneMark::Arc(mark) => mark.name = name.to_string(),
        SceneMark::Area(mark) => mark.name = name.to_string(),
        SceneMark::Path(mark) => mark.name = name.to_string(),
        SceneMark::Symbol(mark) => mark.name = name.to_string(),
        SceneMark::Line(mark) => mark.name = name.to_string(),
        SceneMark::Trail(mark) => mark.name = name.to_string(),
        SceneMark::Rect(mark) => mark.name = name.to_string(),
        SceneMark::Rule(mark) => mark.name = name.to_string(),
        SceneMark::Text(mark) => Arc::make_mut(mark).name = name.to_string(),
        SceneMark::Image(mark) => Arc::make_mut(mark).name = name.to_string(),
        SceneMark::Group(mark) => mark.name = name.to_string(),
    }
}

fn prefix_event_datum_rows(
    rows: impl IntoIterator<Item = EvaluatedEventDatumRows>,
    prefix: &[usize],
) -> Vec<EvaluatedEventDatumRows> {
    rows.into_iter()
        .map(|mut rows| {
            let mut path = Vec::with_capacity(prefix.len() + rows.mark_path.len());
            path.extend_from_slice(prefix);
            path.extend(rows.mark_path);
            rows.mark_path = path;
            rows
        })
        .collect()
}

fn offset_flat_event_datum_rows(
    rows: impl IntoIterator<Item = EvaluatedEventDatumRows>,
    start_index: usize,
) -> Vec<EvaluatedEventDatumRows> {
    rows.into_iter()
        .map(|mut rows| {
            if let Some(first) = rows.mark_path.first_mut() {
                *first += start_index;
            } else {
                rows.mark_path.push(start_index);
            }
            rows
        })
        .collect()
}

fn event_datum_rows_for_rendered_marks(
    source_rows: Option<&RecordBatch>,
    mark_count: usize,
    source_row_indices: Option<Vec<Vec<usize>>>,
    generated_rows: Option<Vec<RecordBatch>>,
) -> Result<Vec<EvaluatedEventDatumRows>, AvengerChartError> {
    let source_batches = if let Some(rows) = source_rows {
        let batches = if let Some(source_row_indices) = source_row_indices {
            if source_row_indices.len() != mark_count {
                return Err(AvengerChartError::InternalError(format!(
                    "rendered mark source row index count {} did not match scene mark count {mark_count}",
                    source_row_indices.len()
                )));
            }
            source_row_indices
                .iter()
                .map(|indices| gather_record_batch_rows(rows, indices))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            (0..mark_count).map(|_| rows.clone()).collect()
        };
        Some(batches)
    } else {
        None
    };

    if let Some(generated_rows) = generated_rows.as_ref()
        && generated_rows.len() != mark_count
    {
        return Err(AvengerChartError::InternalError(format!(
            "rendered mark generated event datum row count {} did not match scene mark count {mark_count}",
            generated_rows.len()
        )));
    }

    if source_batches.is_none() && generated_rows.is_none() {
        return Ok(Vec::new());
    }

    let mut rows = Vec::with_capacity(mark_count);
    for mark_index in 0..mark_count {
        let source = source_batches
            .as_ref()
            .and_then(|batches| batches.get(mark_index));
        let generated = generated_rows
            .as_ref()
            .and_then(|batches| batches.get(mark_index));
        let Some(batch) = merge_event_datum_batches(source, generated)? else {
            continue;
        };
        rows.push(EvaluatedEventDatumRows {
            mark_path: vec![mark_index],
            subplot_id_path: Vec::new(),
            rows: batch,
        });
    }
    Ok(rows)
}

fn merge_event_datum_batches(
    source: Option<&RecordBatch>,
    generated: Option<&RecordBatch>,
) -> Result<Option<RecordBatch>, AvengerChartError> {
    let Some(generated) = generated else {
        return Ok(source.cloned());
    };
    let Some(source) = source else {
        return Ok(Some(generated.clone()));
    };
    if source.num_rows() != generated.num_rows() {
        return Err(AvengerChartError::InternalError(format!(
            "source event datum row count {} did not match generated event datum row count {}",
            source.num_rows(),
            generated.num_rows()
        )));
    }

    let generated_schema = generated.schema();
    let generated_names = generated_schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect::<HashSet<_>>();
    let mut fields = Vec::new();
    let mut columns: Vec<ArrayRef> = Vec::new();
    for (field, column) in source.schema().fields().iter().zip(source.columns()) {
        if generated_names.contains(field.name().as_str()) {
            continue;
        }
        fields.push(field.as_ref().clone());
        columns.push(column.clone());
    }
    for (field, column) in generated_schema.fields().iter().zip(generated.columns()) {
        fields.push(field.as_ref().clone());
        columns.push(column.clone());
    }

    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .map(Some)
        .map_err(AvengerChartError::ArrowError)
}

fn gather_record_batch_rows(
    batch: &RecordBatch,
    indices: &[usize],
) -> Result<RecordBatch, AvengerChartError> {
    let take_indices = UInt32Array::from(
        indices
            .iter()
            .map(|index| {
                u32::try_from(*index).map_err(|_| {
                    AvengerChartError::InternalError(format!(
                        "event datum row index {index} exceeded u32::MAX"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    let columns = batch
        .columns()
        .iter()
        .map(|column| {
            take(column.as_ref(), &take_indices, None).map_err(AvengerChartError::ArrowError)
        })
        .collect::<Result<Vec<_>, _>>()?;
    RecordBatch::try_new(batch.schema(), columns).map_err(AvengerChartError::ArrowError)
}

#[derive(Clone, Debug)]
struct RecursiveOverflowSnapshot {
    guide: OverflowSpaceRequirement,
    total: OverflowSpaceRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedContentKind {
    SinglePlot,
    FacetBand,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ResolvedChartSizing {
    SinglePlot,
    FacetBand(FacetRuntimeSizingPolicy),
}

struct EvaluationOutcome {
    evaluated: EvaluatedPlot,
    layout_profile: Option<LayoutProfileSnapshot>,
}

struct FacetLayoutRefreshOutcome {
    plot_bounds: LayoutBounds,
    retarget_count: usize,
}

pub(crate) struct PreviewLayoutProfileAttempt {
    pub(crate) reused: Option<(
        EvaluatedPlot,
        EvaluationMetrics,
        Option<LayoutProfileSnapshot>,
    )>,
    pub(crate) fallback_reasons: Vec<PreviewProfileFallbackReason>,
}

impl PreviewLayoutProfileAttempt {
    fn reused(
        evaluated: EvaluatedPlot,
        metrics: EvaluationMetrics,
        layout_profile: Option<LayoutProfileSnapshot>,
    ) -> Self {
        Self {
            reused: Some((evaluated, metrics, layout_profile)),
            fallback_reasons: Vec::new(),
        }
    }

    fn fallback(reason: PreviewProfileFallbackReason) -> Self {
        Self {
            reused: None,
            fallback_reasons: vec![reason],
        }
    }
}

impl ResolvedChartSizing {
    const DEFAULT_CANVAS_WIDTH: f32 = 400.0;
    const DEFAULT_CANVAS_HEIGHT: f32 = 300.0;

    fn content_kind(self) -> ResolvedContentKind {
        match self {
            Self::SinglePlot => ResolvedContentKind::SinglePlot,
            Self::FacetBand(_) => ResolvedContentKind::FacetBand,
        }
    }

    fn facet_runtime_sizing_mode(self) -> FacetRuntimeSizingMode {
        match self {
            Self::SinglePlot => FacetRuntimeSizingMode::CanvasFit,
            Self::FacetBand(policy) => FacetRuntimeSizingMode::Policy(policy),
        }
    }
}

struct SelectedFacetSubtree<'a> {
    plot: &'a CompiledPlot,
    measurement: &'a ComponentsMeasurement,
    data_override: Option<&'a DataFrame>,
    facet_path: Vec<ScalarValue>,
    dimensions_are_plot_area: bool,
}

fn facet_band_children(
    measurement: &ComponentsMeasurement,
) -> Option<(&Arc<CompiledPlot>, &[FacetCellRuntime])> {
    measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
        .map(|facet_band| (&facet_band.compiled_subplot, facet_band.cells.as_slice()))
}

const OKABE_ITO_DEBUG_COLORS: [&str; 9] = [
    "rgba(230, 159, 0, 0.85)",   // orange
    "rgba(86, 180, 233, 0.85)",  // sky blue
    "rgba(0, 158, 115, 0.85)",   // bluish green
    "rgba(240, 228, 66, 0.85)",  // yellow
    "rgba(0, 114, 178, 0.85)",   // blue
    "rgba(213, 94, 0, 0.85)",    // vermillion
    "rgba(204, 121, 167, 0.85)", // reddish purple
    "rgba(153, 153, 153, 0.85)", // gray
    "rgba(0, 0, 0, 0.85)",       // black
];

fn facet_debug_layout_color(facet_coord_node_path: &[usize]) -> String {
    let cycle_index = facet_coord_node_path
        .iter()
        .enumerate()
        .map(|(depth, child_index)| (depth + 1) * (child_index + 1))
        .sum::<usize>()
        .saturating_sub(1);
    OKABE_ITO_DEBUG_COLORS[cycle_index % OKABE_ITO_DEBUG_COLORS.len()].to_string()
}

fn facet_debug_layout_flip_label_align(facet_coord_node_path: &[usize]) -> bool {
    facet_coord_node_path.len() % 2 == 1
}

fn translate_layout_bounds(bounds: &mut LayoutBounds, dx: f32, dy: f32) {
    bounds.x += dx;
    bounds.y += dy;
}

fn translated_frame_layout(layout: &FrameLayout, origin: [f32; 2]) -> FrameLayout {
    let mut layout = layout.clone();
    translate_layout_bounds(&mut layout.plot_area, origin[0], origin[1]);
    for bounds in layout.guide_overflows.values_mut() {
        translate_layout_bounds(bounds, origin[0], origin[1]);
    }
    for bounds in layout.legends.values_mut() {
        translate_layout_bounds(bounds, origin[0], origin[1]);
    }
    if let Some(bounds) = &mut layout.title {
        translate_layout_bounds(bounds, origin[0], origin[1]);
    }
    if let Some(bounds) = &mut layout.subtitle {
        translate_layout_bounds(bounds, origin[0], origin[1]);
    }
    layout
}

fn translate_scene_mark(mark: SceneMark, dx: f32, dy: f32) -> SceneMark {
    match mark {
        SceneMark::Group(mut group) => {
            group.origin = [group.origin[0] + dx, group.origin[1] + dy];
            SceneMark::Group(group)
        }
        SceneMark::Text(text_mark) => {
            let mut text_mark = (*text_mark).clone();
            text_mark.x = text_mark.x.map(|x| x + dx);
            text_mark.y = text_mark.y.map(|y| y + dy);
            SceneMark::Text(Arc::new(text_mark))
        }
        other => other,
    }
}

fn scale_f32_values(values: &ScalarOrArray<f32>, scale: f32) -> ScalarOrArray<f32> {
    values.map(|value| value * scale)
}

fn adjust_f32_values(
    values: &ScalarOrArray<f32>,
    adjustment: LinearScaleAdjustment,
) -> ScalarOrArray<f32> {
    values.map(|value| adjustment.scale * *value + adjustment.offset)
}

fn retarget_f32_values(
    values: &ScalarOrArray<f32>,
    adjustment: Option<LinearScaleAdjustment>,
    fallback_scale: f32,
) -> ScalarOrArray<f32> {
    if let Some(adjustment) = adjustment {
        adjust_f32_values(values, adjustment)
    } else {
        scale_f32_values(values, fallback_scale)
    }
}

fn scale_optional_f32_values(
    values: &Option<ScalarOrArray<f32>>,
    scale: f32,
) -> Option<ScalarOrArray<f32>> {
    values
        .as_ref()
        .map(|values| scale_f32_values(values, scale))
}

fn retarget_optional_f32_values(
    values: &Option<ScalarOrArray<f32>>,
    adjustment: Option<LinearScaleAdjustment>,
    fallback_scale: f32,
) -> Option<ScalarOrArray<f32>> {
    values
        .as_ref()
        .map(|values| retarget_f32_values(values, adjustment, fallback_scale))
}

fn adjustment_scale(adjustment: Option<LinearScaleAdjustment>, fallback_scale: f32) -> f32 {
    adjustment
        .map(|adjustment| adjustment.scale)
        .unwrap_or(fallback_scale)
}

fn retarget_linear_adjustment(
    existing: &mut Option<LinearScaleAdjustment>,
    adjustment: Option<LinearScaleAdjustment>,
    fallback_scale: f32,
) {
    if let Some(existing) = existing {
        if let Some(adjustment) = adjustment {
            existing.offset = adjustment.scale * existing.offset + adjustment.offset;
            existing.scale *= adjustment.scale;
        } else {
            existing.offset *= fallback_scale;
            existing.scale *= fallback_scale;
        }
    }
}

fn retarget_symbol_position_axis(
    values: &mut ScalarOrArray<f32>,
    existing: &mut Option<LinearScaleAdjustment>,
    adjustment: Option<LinearScaleAdjustment>,
    fallback_scale: f32,
) {
    if let Some(adjustment) = adjustment {
        if existing.is_some() {
            retarget_linear_adjustment(existing, Some(adjustment), fallback_scale);
        } else {
            *existing = Some(adjustment);
        }
    } else if existing.is_some() {
        retarget_linear_adjustment(existing, None, fallback_scale);
    } else {
        *values = scale_f32_values(values, fallback_scale);
    }
}

fn retarget_clip_for_plot_area(
    clip: &Clip,
    scale_x: f32,
    scale_y: f32,
    x_adjustment: Option<LinearScaleAdjustment>,
    y_adjustment: Option<LinearScaleAdjustment>,
) -> Option<Clip> {
    match clip {
        Clip::None => Some(Clip::None),
        Clip::Rect {
            x,
            y,
            width,
            height,
        } => Some(Clip::Rect {
            x: x_adjustment
                .map(|adjustment| adjustment.scale * *x + adjustment.offset)
                .unwrap_or(*x * scale_x),
            y: y_adjustment
                .map(|adjustment| adjustment.scale * *y + adjustment.offset)
                .unwrap_or(*y * scale_y),
            width: width * adjustment_scale(x_adjustment, scale_x),
            height: height * adjustment_scale(y_adjustment, scale_y),
        }),
        Clip::Path(_) => None,
    }
}

/// Resolve non-null raw-domain overrides for a subplot's scales.
///
/// Returns, for each scale whose `raw_domain` evaluates to a finite, non-empty
/// two-element interval under `params`, the resolved `(min, max)`. Scales with no
/// raw_domain, or whose raw_domain is null/degenerate, are omitted (so the cell
/// keeps its inferred domain).
pub(crate) async fn resolve_raw_domain_overrides(
    subplot: &CompiledPlot,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HashMap<String, (f32, f32)>, AvengerChartError> {
    let mut overrides = HashMap::new();
    let datafusion_params = params_to_datafusion(params);
    for (scale_name, spec) in &subplot.scale_specs {
        let avenger_chart_scales::PlotScaleSpec::Local(config) = spec;
        let Some(domain) = config.domain.as_option() else {
            continue;
        };
        let Some(raw_domain) = &domain.raw_domain else {
            continue;
        };
        let expr = raw_domain.to_expr(ctx)?;
        let raw_value = if let Some(value) = direct_param_value(&expr, params) {
            value.clone()
        } else {
            let scalars =
                eval_to_scalars(vec![expr], Some(ctx), datafusion_params.as_ref()).await?;
            let Some(raw_value) = scalars.first() else {
                continue;
            };
            raw_value.clone()
        };
        if let Ok([min, max]) = raw_value.as_f32x2()
            && min.is_finite()
            && max.is_finite()
            && min != max
        {
            overrides.insert(scale_name.clone(), (min, max));
        }
    }
    Ok(overrides)
}

fn direct_param_value<'a>(
    expr: &datafusion::logical_expr::Expr,
    params: &'a IndexMap<String, ScalarValue>,
) -> Option<&'a ScalarValue> {
    let datafusion::logical_expr::Expr::Placeholder(placeholder) = expr else {
        return None;
    };
    params
        .get(placeholder.id.trim_start_matches('$'))
        .or_else(|| params.get(&placeholder.id))
}

pub(crate) fn has_raw_domain_scale(plot: &CompiledPlot) -> bool {
    plot.scale_specs.values().any(|spec| {
        let avenger_chart_scales::PlotScaleSpec::Local(config) = spec;
        config
            .domain
            .as_option()
            .and_then(|domain| domain.raw_domain.as_ref())
            .is_some()
    })
}

/// Apply resolved raw-domain overrides to a scale map, preserving ranges/options.
pub(crate) fn apply_domain_overrides_to_scales(
    scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    overrides: &HashMap<String, (f32, f32)>,
) {
    for (name, &(min, max)) in overrides {
        if let Some(scale) = scales.get_mut(name) {
            let configured = scale.configured().clone().with_domain_interval((min, max));
            scale.set_configured(configured);
        }
    }
}

fn scale_scene_mark_for_plot_area(
    mark: &SceneMark,
    scale_x: f32,
    scale_y: f32,
    x_adjustment: Option<LinearScaleAdjustment>,
    y_adjustment: Option<LinearScaleAdjustment>,
) -> Option<SceneMark> {
    match mark {
        SceneMark::Group(group) => {
            let marks = group
                .marks
                .iter()
                .map(|mark| {
                    scale_scene_mark_for_plot_area(
                        mark,
                        scale_x,
                        scale_y,
                        x_adjustment,
                        y_adjustment,
                    )
                })
                .collect::<Option<Vec<_>>>()?;
            let mut group = group.clone();
            group.origin = [
                x_adjustment
                    .map(|adjustment| adjustment.scale * group.origin[0] + adjustment.offset)
                    .unwrap_or(group.origin[0] * scale_x),
                y_adjustment
                    .map(|adjustment| adjustment.scale * group.origin[1] + adjustment.offset)
                    .unwrap_or(group.origin[1] * scale_y),
            ];
            group.clip = retarget_clip_for_plot_area(
                &group.clip,
                scale_x,
                scale_y,
                x_adjustment,
                y_adjustment,
            )?;
            group.marks = marks;
            Some(SceneMark::Group(group))
        }
        SceneMark::Symbol(mark) => {
            let mut mark = mark.clone();
            retarget_symbol_position_axis(
                &mut mark.x,
                &mut mark.x_adjustment,
                x_adjustment,
                scale_x,
            );
            retarget_symbol_position_axis(
                &mut mark.y,
                &mut mark.y_adjustment,
                y_adjustment,
                scale_y,
            );
            Some(SceneMark::Symbol(mark))
        }
        SceneMark::Rect(mark) => {
            let mut mark = mark.clone();
            mark.x = retarget_f32_values(&mark.x, x_adjustment, scale_x);
            mark.y = retarget_f32_values(&mark.y, y_adjustment, scale_y);
            mark.width =
                scale_optional_f32_values(&mark.width, adjustment_scale(x_adjustment, scale_x));
            mark.height =
                scale_optional_f32_values(&mark.height, adjustment_scale(y_adjustment, scale_y));
            mark.x2 = retarget_optional_f32_values(&mark.x2, x_adjustment, scale_x);
            mark.y2 = retarget_optional_f32_values(&mark.y2, y_adjustment, scale_y);
            Some(SceneMark::Rect(mark))
        }
        SceneMark::Rule(mark) => {
            let mut mark = mark.clone();
            mark.x = retarget_f32_values(&mark.x, x_adjustment, scale_x);
            mark.y = retarget_f32_values(&mark.y, y_adjustment, scale_y);
            mark.x2 = retarget_f32_values(&mark.x2, x_adjustment, scale_x);
            mark.y2 = retarget_f32_values(&mark.y2, y_adjustment, scale_y);
            Some(SceneMark::Rule(mark))
        }
        SceneMark::Line(mark) => {
            let mut mark = mark.clone();
            mark.x = retarget_f32_values(&mark.x, x_adjustment, scale_x);
            mark.y = retarget_f32_values(&mark.y, y_adjustment, scale_y);
            Some(SceneMark::Line(mark))
        }
        SceneMark::Area(mark) => {
            let mut mark = mark.clone();
            mark.x = retarget_f32_values(&mark.x, x_adjustment, scale_x);
            mark.y = retarget_f32_values(&mark.y, y_adjustment, scale_y);
            mark.x2 = retarget_f32_values(&mark.x2, x_adjustment, scale_x);
            mark.y2 = retarget_f32_values(&mark.y2, y_adjustment, scale_y);
            Some(SceneMark::Area(mark))
        }
        SceneMark::Trail(mark) => {
            let mut mark = mark.clone();
            mark.x = retarget_f32_values(&mark.x, x_adjustment, scale_x);
            mark.y = retarget_f32_values(&mark.y, y_adjustment, scale_y);
            Some(SceneMark::Trail(mark))
        }
        SceneMark::Text(mark) => {
            let mut mark = (**mark).clone();
            mark.x = retarget_f32_values(&mark.x, x_adjustment, scale_x);
            mark.y = retarget_f32_values(&mark.y, y_adjustment, scale_y);
            Some(SceneMark::Text(Arc::new(mark)))
        }
        SceneMark::Image(mark) => {
            let mut mark = (**mark).clone();
            mark.x = retarget_f32_values(&mark.x, x_adjustment, scale_x);
            mark.y = retarget_f32_values(&mark.y, y_adjustment, scale_y);
            mark.width = scale_f32_values(&mark.width, adjustment_scale(x_adjustment, scale_x));
            mark.height = scale_f32_values(&mark.height, adjustment_scale(y_adjustment, scale_y));
            Some(SceneMark::Image(Arc::new(mark)))
        }
        SceneMark::Arc(mark) => {
            let mut mark = mark.clone();
            let radius_scale = adjustment_scale(x_adjustment, scale_x)
                .min(adjustment_scale(y_adjustment, scale_y));
            mark.x = retarget_f32_values(&mark.x, x_adjustment, scale_x);
            mark.y = retarget_f32_values(&mark.y, y_adjustment, scale_y);
            mark.outer_radius = scale_f32_values(&mark.outer_radius, radius_scale);
            mark.inner_radius = scale_f32_values(&mark.inner_radius, radius_scale);
            mark.corner_radius = scale_f32_values(&mark.corner_radius, radius_scale);
            Some(SceneMark::Arc(mark))
        }
        SceneMark::Path(_) => None,
    }
}

fn retarget_cached_data_marks_for_plot_area(
    cached_components: &PlotComponents,
    source_measurement: &ComponentsMeasurement,
    target_measurement: &ComponentsMeasurement,
) -> Option<Vec<SceneMark>> {
    let source_plot_area_width = cached_components.plot_bounds.width;
    let source_plot_area_height = cached_components.plot_bounds.height;
    if source_plot_area_width <= 0.01 || source_plot_area_height <= 0.01 {
        return None;
    }
    let scale_x = target_measurement.plot_area_width / source_plot_area_width;
    let scale_y = target_measurement.plot_area_height / source_plot_area_height;
    let x_adjustment =
        scale_adjustment_between_measurements(source_measurement, target_measurement, "x")?;
    let y_adjustment =
        scale_adjustment_between_measurements(source_measurement, target_measurement, "y")?;
    cached_components
        .data_marks
        .iter()
        .map(|mark| {
            scale_scene_mark_for_plot_area(mark, scale_x, scale_y, x_adjustment, y_adjustment)
        })
        .collect()
}

fn same_layout_extent(source: &ComponentsMeasurement, target: &ComponentsMeasurement) -> bool {
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 0.01
    }
    let source_bounds = source.layout.plot_area_bounds();
    let target_bounds = target.layout.plot_area_bounds();
    close(source.plot_area_width, target.plot_area_width)
        && close(source.plot_area_height, target.plot_area_height)
        && close(source.canvas_size.0, target.canvas_size.0)
        && close(source.canvas_size.1, target.canvas_size.1)
        && close(source_bounds.x, target_bounds.x)
        && close(source_bounds.y, target_bounds.y)
        && close(source_bounds.width, target_bounds.width)
        && close(source_bounds.height, target_bounds.height)
}

fn scale_adjustment_between_measurements(
    source_measurement: &ComponentsMeasurement,
    target_measurement: &ComponentsMeasurement,
    channel: &str,
) -> Option<Option<LinearScaleAdjustment>> {
    match (
        source_measurement.scales.get(channel),
        target_measurement.scales.get(channel),
    ) {
        (Some(from_scale), Some(to_scale)) => Some(Some(
            from_scale.configured().adjust(to_scale.configured()).ok()?,
        )),
        (None, None) => Some(None),
        (Some(_), None) | (None, Some(_)) => None,
    }
}

fn set_debug_side_overflow(layout: &mut FrameLayout, side: AxisPosition, total: f32) {
    if total <= 0.01 {
        layout.guide_overflows.remove(&side);
        return;
    }

    let plot_area = layout.plot_area;
    let bounds = match side {
        AxisPosition::Top => LayoutBounds {
            x: plot_area.x,
            y: plot_area.y - total,
            width: plot_area.width,
            height: total,
        },
        AxisPosition::Right => LayoutBounds {
            x: plot_area.x + plot_area.width,
            y: plot_area.y,
            width: total,
            height: plot_area.height,
        },
        AxisPosition::Bottom => LayoutBounds {
            x: plot_area.x,
            y: plot_area.y + plot_area.height,
            width: plot_area.width,
            height: total,
        },
        AxisPosition::Left => LayoutBounds {
            x: plot_area.x - total,
            y: plot_area.y,
            width: total,
            height: plot_area.height,
        },
    };
    layout.guide_overflows.insert(side, bounds);
}

fn add_debug_side_extent(
    extents: &mut EdgeSlabs,
    side: AxisPosition,
    content: LayoutBounds,
    bounds: LayoutBounds,
) {
    let content_right = content.x + content.width;
    let content_bottom = content.y + content.height;
    let bounds_right = bounds.x + bounds.width;
    let bounds_bottom = bounds.y + bounds.height;
    match side {
        AxisPosition::Top => {
            extents.top = extents.top.max(content.y - bounds.y);
        }
        AxisPosition::Right => {
            extents.right = extents.right.max(bounds_right - content_right);
        }
        AxisPosition::Bottom => {
            extents.bottom = extents.bottom.max(bounds_bottom - content_bottom);
        }
        AxisPosition::Left => {
            extents.left = extents.left.max(content.x - bounds.x);
        }
    }
}

fn legend_side(position: LegendPosition) -> AxisPosition {
    match position {
        LegendPosition::Top => AxisPosition::Top,
        LegendPosition::Right => AxisPosition::Right,
        LegendPosition::Bottom => AxisPosition::Bottom,
        LegendPosition::Left => AxisPosition::Left,
    }
}

fn union_layout_bounds(bounds: &mut Option<LayoutBounds>, next: LayoutBounds) {
    let Some(current) = bounds else {
        *bounds = Some(next);
        return;
    };

    let x0 = current.x.min(next.x);
    let y0 = current.y.min(next.y);
    let x1 = (current.x + current.width).max(next.x + next.width);
    let y1 = (current.y + current.height).max(next.y + next.height);
    *current = LayoutBounds {
        x: x0,
        y: y0,
        width: (x1 - x0).max(0.0),
        height: (y1 - y0).max(0.0),
    };
}

fn projected_child_frame_container_plot_area_envelope(
    layout: &FrameLayout,
    container: &ChildFrameContainerView<'_>,
) -> Result<Option<LayoutBounds>, AvengerChartError> {
    let parent_content_origin = [layout.plot_area.x, layout.plot_area.y];
    let mut envelope = None;

    for child_region in container.child_regions() {
        let child_measurement = container
            .child_measurement(child_region.child_index)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing child-frame container child for index {}",
                    child_region.child_index
                ))
            })?;
        let child_plot_bounds = *child_measurement.layout.plot_area_bounds();
        let mut projected_plot_bounds =
            container.project_child_bounds(child_region.child_index, child_plot_bounds)?;
        projected_plot_bounds.x += parent_content_origin[0];
        projected_plot_bounds.y += parent_content_origin[1];
        union_layout_bounds(&mut envelope, projected_plot_bounds);
    }

    Ok(envelope)
}

fn child_frame_container_component_debug_side_extents(
    layout: &FrameLayout,
    container: &ChildFrameContainerView<'_>,
    content: LayoutBounds,
) -> Result<EdgeSlabs, AvengerChartError> {
    let mut extents = EdgeSlabs::default();

    for (side, bounds) in &layout.guide_overflows {
        add_debug_side_extent(&mut extents, *side, content, *bounds);
    }
    for (position, legend_ids) in &layout.legends_by_position {
        let side = legend_side(*position);
        for legend_id in legend_ids {
            if let Some(bounds) = layout.legends.get(legend_id) {
                add_debug_side_extent(&mut extents, side, content, *bounds);
            }
        }
    }

    let parent_content_origin = [layout.plot_area.x, layout.plot_area.y];

    for child_region in container.child_regions() {
        let child_measurement = container
            .child_measurement(child_region.child_index)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing child-frame container child for index {}",
                    child_region.child_index
                ))
            })?;

        for position in [
            LegendPosition::Top,
            LegendPosition::Right,
            LegendPosition::Bottom,
            LegendPosition::Left,
        ] {
            let Some(legend_ids) = child_measurement
                .layout
                .frame_layout
                .legends_by_position
                .get(&position)
            else {
                continue;
            };
            let side = legend_side(position);
            for legend_id in legend_ids {
                let Some(bounds) = child_measurement.layout.frame_layout.legends.get(legend_id)
                else {
                    continue;
                };
                let mut projected =
                    container.project_child_bounds(child_region.child_index, *bounds)?;
                projected.x += parent_content_origin[0];
                projected.y += parent_content_origin[1];
                add_debug_side_extent(&mut extents, side, content, projected);
            }
        }
    }

    Ok(EdgeSlabs {
        top: extents.top.max(0.0),
        right: extents.right.max(0.0),
        bottom: extents.bottom.max(0.0),
        left: extents.left.max(0.0),
    })
}

fn expand_child_frame_container_component_debug_plot_area(
    layout: &mut FrameLayout,
    measurement: &ComponentsMeasurement,
) -> Result<(), AvengerChartError> {
    let Some(container) = measurement.child_frame_container_view()? else {
        return Ok(());
    };
    let Some(content) = projected_child_frame_container_plot_area_envelope(layout, &container)?
    else {
        return Ok(());
    };
    let extents = child_frame_container_component_debug_side_extents(layout, &container, content)?;

    layout.plot_area = content;
    set_debug_side_overflow(layout, AxisPosition::Top, extents.top);
    set_debug_side_overflow(layout, AxisPosition::Right, extents.right);
    set_debug_side_overflow(layout, AxisPosition::Bottom, extents.bottom);
    set_debug_side_overflow(layout, AxisPosition::Left, extents.left);
    Ok(())
}

fn translated_component_debug_frame_layout(
    measurement: &ComponentsMeasurement,
    origin: [f32; 2],
) -> Result<FrameLayout, AvengerChartError> {
    let mut layout = measurement.layout.frame_layout.clone();
    expand_child_frame_container_component_debug_plot_area(&mut layout, measurement)?;
    Ok(translated_frame_layout(&layout, origin))
}

struct RefinementIterationOutcome {
    reached_snapshot_checkpoint: bool,
    overflow_grew: Option<bool>,
    realized_padding_feedback: Arc<FacetBandPaddingFeedbackMap>,
}

#[derive(Clone, Copy)]
enum FacetRefinementMode<'a> {
    Canvas {
        layout_spec: &'a EvaluatedLayoutSpec,
        scale_provider: &'a dyn ScaleProvider,
    },
    Policy {
        layout_spec: &'a EvaluatedLayoutSpec,
    },
}

impl CompiledPlot {
    /// Initial estimate for plot area as ratio of total canvas size
    const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

    /// Default margin in pixels when not specified in theme or expression
    const DEFAULT_MARGIN: f32 = 10.0;

    /// Stop layout-refinement assertions once dimensions are effectively stable.
    #[cfg(test)]
    const LAYOUT_REFINEMENT_EPSILON: f32 = 0.5;

    /// Extra whitespace around standalone facet-subtree snapshot renders.
    const FACET_SUBTREE_SNAPSHOT_PADDING: f32 = 24.0;

    fn overflow_increased(
        previous: &OverflowSpaceRequirement,
        next: &OverflowSpaceRequirement,
        epsilon: f32,
    ) -> bool {
        next.top > previous.top + epsilon
            || next.right > previous.right + epsilon
            || next.bottom > previous.bottom + epsilon
            || next.left > previous.left + epsilon
    }

    fn collect_recursive_overflow_snapshots(
        measurement: &ComponentsMeasurement,
        snapshots: &mut Vec<RecursiveOverflowSnapshot>,
    ) {
        snapshots.push(RecursiveOverflowSnapshot {
            guide: measurement.layout.overflow.clone(),
            total: measurement.layout.total_overflow.clone(),
        });

        if let Some(facet_measurement) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        {
            for child in facet_measurement.child_measurements_iter() {
                Self::collect_recursive_overflow_snapshots(child, snapshots);
            }
        }
    }

    fn recursive_overflow_snapshot(
        measurement: &ComponentsMeasurement,
    ) -> Vec<RecursiveOverflowSnapshot> {
        let mut snapshots = Vec::new();
        Self::collect_recursive_overflow_snapshots(measurement, &mut snapshots);
        snapshots
    }

    fn recursive_overflow_increased(
        previous: &[RecursiveOverflowSnapshot],
        next: &[RecursiveOverflowSnapshot],
        epsilon: f32,
    ) -> bool {
        let empty = RecursiveOverflowSnapshot {
            guide: OverflowSpaceRequirement::default(),
            total: OverflowSpaceRequirement::default(),
        };
        for idx in 0..previous.len().max(next.len()) {
            let previous = previous.get(idx).unwrap_or(&empty);
            let next = next.get(idx).unwrap_or(&empty);
            if Self::overflow_increased(&previous.guide, &next.guide, epsilon)
                || Self::overflow_increased(&previous.total, &next.total, epsilon)
            {
                return true;
            }
        }
        false
    }

    fn collect_realized_padding_feedback(
        measurement: &ComponentsMeasurement,
        node_path: &mut Vec<usize>,
        feedback: &mut FacetBandPaddingFeedbackMap,
    ) {
        let Some(facet_band) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        else {
            return;
        };

        let renderable_cells = facet_band
            .cells
            .iter()
            .map(|cell| {
                renderable_for_empty_policy(facet_band.empty_cell_policy, !cell.plan.has_data_rows)
            })
            .collect::<Vec<_>>();
        let boundary_profiles = facet_band
            .cells
            .iter()
            .map(|cell| boundary_profiles_for_measurement(&cell.measurement))
            .collect::<Vec<_>>();

        let guide_padding_inner_px = compute_padding_from_boundary_profiles(
            facet_band.axis,
            &boundary_profiles,
            &renderable_cells,
            false,
        )
        .unwrap_or(0.0);
        let padding_inner_px = compute_padding_from_boundary_profiles(
            facet_band.axis,
            &boundary_profiles,
            &renderable_cells,
            true,
        )
        .unwrap_or(0.0);
        if guide_padding_inner_px > 0.0 || padding_inner_px > 0.0 {
            feedback.insert(
                node_path.clone(),
                FacetBandPaddingFeedback {
                    padding_inner_px,
                    guide_padding_inner_px,
                },
            );
        }

        for (idx, child) in facet_band.child_measurements_iter().enumerate() {
            node_path.push(idx);
            Self::collect_realized_padding_feedback(child, node_path, feedback);
            node_path.pop();
        }
    }

    fn realized_padding_feedback_snapshot(
        measurement: &ComponentsMeasurement,
    ) -> FacetBandPaddingFeedbackMap {
        let mut feedback = HashMap::new();
        let mut node_path = Vec::new();
        Self::collect_realized_padding_feedback(measurement, &mut node_path, &mut feedback);
        feedback
    }

    fn padding_feedback_increased(
        previous: &FacetBandPaddingFeedbackMap,
        next: &FacetBandPaddingFeedbackMap,
        epsilon: f32,
    ) -> bool {
        for (node_path, next_feedback) in next {
            let previous_feedback = previous.get(node_path).copied().unwrap_or_default();
            if next_feedback.padding_inner_px > previous_feedback.padding_inner_px + epsilon
                || next_feedback.guide_padding_inner_px
                    > previous_feedback.guide_padding_inner_px + epsilon
            {
                return true;
            }
        }
        false
    }

    fn collect_facet_probe_size_overrides(
        measurement: &ComponentsMeasurement,
        overrides: &mut HashMap<Vec<ScalarValue>, (f32, f32)>,
    ) {
        let Some((_compiled_subplot, cells)) = facet_band_children(measurement) else {
            return;
        };

        for cell in cells {
            overrides.insert(
                cell.plan.full_path.clone(),
                (
                    cell.measurement.plot_area_width,
                    cell.measurement.plot_area_height,
                ),
            );
            Self::collect_facet_probe_size_overrides(&cell.measurement, overrides);
        }
    }

    fn collect_facet_leaf_plot_area_sizes(
        measurement: &ComponentsMeasurement,
        leaf_sizes: &mut Vec<(f32, f32)>,
    ) {
        let Some((_compiled_subplot, cells)) = facet_band_children(measurement) else {
            leaf_sizes.push((measurement.plot_area_width, measurement.plot_area_height));
            return;
        };

        for cell in cells {
            Self::collect_facet_leaf_plot_area_sizes(&cell.measurement, leaf_sizes);
        }
    }

    fn legends_within_canvas_recursive(measurement: &ComponentsMeasurement) -> bool {
        let canvas_width = measurement.canvas_size.0;
        let canvas_height = measurement.canvas_size.1;
        for bounds in measurement.layout.frame_layout.legends.values() {
            if bounds.x < -0.5
                || bounds.y < -0.5
                || bounds.x + bounds.width > canvas_width + 0.5
                || bounds.y + bounds.height > canvas_height + 0.5
            {
                return false;
            }
        }

        if let Some(facet_measurement) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        {
            return facet_measurement
                .child_measurements_iter()
                .all(Self::legends_within_canvas_recursive);
        }

        true
    }

    fn marks_use_auto_empty_cell_policy(marks: &[Arc<dyn CompiledMark>]) -> bool {
        marks.iter().any(|mark| {
            if let Some(facet_mark) = facet_subplot_ref(mark.as_ref()) {
                return match facet_mark {
                    FacetSubplotRef::Row(facet_row) => {
                        matches!(
                            facet_row.facet_empty_cell_policy(),
                            FacetEmptyCellPolicy::Auto
                        ) || Self::marks_use_auto_empty_cell_policy(
                            &facet_row.compiled_subplot().marks,
                        )
                    }
                    FacetSubplotRef::Col(facet_col) => {
                        matches!(
                            facet_col.facet_empty_cell_policy(),
                            FacetEmptyCellPolicy::Auto
                        ) || Self::marks_use_auto_empty_cell_policy(
                            &facet_col.compiled_subplot().marks,
                        )
                    }
                    FacetSubplotRef::Wrap(facet_wrap) => {
                        matches!(
                            facet_wrap.facet_empty_cell_policy(),
                            FacetEmptyCellPolicy::Auto
                        ) || Self::marks_use_auto_empty_cell_policy(
                            &facet_wrap.compiled_subplot().marks,
                        )
                    }
                };
            }
            false
        })
    }

    fn marks_contain_facet(marks: &[Arc<dyn CompiledMark>]) -> bool {
        marks
            .iter()
            .any(|mark| facet_subplot_ref(mark.as_ref()).is_some())
    }

    fn validate_no_nested_subplot_plot_size_under_facet(
        marks: &[Arc<dyn CompiledMark>],
        path: &mut Vec<String>,
    ) -> Result<(), AvengerChartError> {
        for (idx, mark) in marks.iter().enumerate() {
            let Some(facet_mark) = facet_subplot_ref(mark.as_ref()) else {
                continue;
            };

            path.push(format!("facet[{idx}]"));
            let subplot = facet_mark.compiled_subplot();
            if !matches!(subplot.layout_spec.plot_area, SizeMode::Auto) {
                let facet_path = if path.is_empty() {
                    "facet-root".to_string()
                } else {
                    path.join(" -> ")
                };
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Nested subplot at {facet_path} cannot set `plot_size(...)` when used under a facet. \
                     Set `plot_size(...)` only on the top-level faceted plot."
                )));
            }
            Self::validate_no_nested_subplot_plot_size_under_facet(&subplot.marks, path)?;
            path.pop();
        }
        Ok(())
    }

    fn evaluated_width(mode: &EvaluatedSizeMode) -> Option<f32> {
        match mode {
            EvaluatedSizeMode::Fixed { width, .. } | EvaluatedSizeMode::Width(width) => {
                Some(*width)
            }
            EvaluatedSizeMode::Height(_) | EvaluatedSizeMode::Auto => None,
        }
    }

    fn evaluated_height(mode: &EvaluatedSizeMode) -> Option<f32> {
        match mode {
            EvaluatedSizeMode::Fixed { height, .. } | EvaluatedSizeMode::Height(height) => {
                Some(*height)
            }
            EvaluatedSizeMode::Width(_) | EvaluatedSizeMode::Auto => None,
        }
    }

    fn resolve_content_kind(&self) -> ResolvedContentKind {
        if Self::marks_contain_facet(&self.marks) {
            ResolvedContentKind::FacetBand
        } else {
            ResolvedContentKind::SinglePlot
        }
    }

    fn resolve_chart_sizing(
        &self,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
    ) -> Result<ResolvedChartSizing, AvengerChartError> {
        match self.resolve_content_kind() {
            ResolvedContentKind::SinglePlot => return Ok(ResolvedChartSizing::SinglePlot),
            ResolvedContentKind::FacetBand => {
                Self::validate_no_nested_subplot_plot_size_under_facet(
                    &self.marks,
                    &mut Vec::new(),
                )?;
            }
        }

        let canvas_width = Self::evaluated_width(&evaluated_layout_spec.canvas);
        let canvas_height = Self::evaluated_height(&evaluated_layout_spec.canvas);
        let plot_width = Self::evaluated_width(&evaluated_layout_spec.plot_area);
        let plot_height = Self::evaluated_height(&evaluated_layout_spec.plot_area);

        if canvas_width.is_some() && plot_width.is_some() {
            return Err(AvengerChartError::InvalidArgument(
                "Faceted charts cannot constrain both canvas width and leaf plot width. \
                 Use either `canvas_size`/`canvas_constraint(width)` or `plot_size`/`plot_constraint(width)` for the width dimension."
                    .to_string(),
            ));
        }
        if canvas_height.is_some() && plot_height.is_some() {
            return Err(AvengerChartError::InvalidArgument(
                "Faceted charts cannot constrain both canvas height and leaf plot height. \
                 Use either `canvas_size`/`canvas_constraint(height)` or `plot_size`/`plot_constraint(height)` for the height dimension."
                    .to_string(),
            ));
        }

        let width_policy = if let Some(leaf_plot_size) = plot_width {
            FacetDimensionSizing::LeafPlotAreaSized { leaf_plot_size }
        } else {
            FacetDimensionSizing::CanvasConstrained {
                canvas_size: canvas_width.unwrap_or(ResolvedChartSizing::DEFAULT_CANVAS_WIDTH),
            }
        };
        let height_policy = if let Some(leaf_plot_size) = plot_height {
            FacetDimensionSizing::LeafPlotAreaSized { leaf_plot_size }
        } else {
            FacetDimensionSizing::CanvasConstrained {
                canvas_size: canvas_height.unwrap_or(ResolvedChartSizing::DEFAULT_CANVAS_HEIGHT),
            }
        };
        let policy = match (width_policy, height_policy) {
            (
                FacetDimensionSizing::CanvasConstrained { canvas_size: width },
                FacetDimensionSizing::CanvasConstrained {
                    canvas_size: height,
                },
            ) => FacetRuntimeSizingPolicy::fully_canvas_constrained(width, height),
            (
                FacetDimensionSizing::LeafPlotAreaSized {
                    leaf_plot_size: width,
                },
                FacetDimensionSizing::LeafPlotAreaSized {
                    leaf_plot_size: height,
                },
            ) => FacetRuntimeSizingPolicy::fully_leaf_plot_area_sized(width, height),
            (width, height) => FacetRuntimeSizingPolicy { width, height },
        };

        Ok(ResolvedChartSizing::FacetBand(policy))
    }

    fn facet_wrap_layout_context(
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        resolved_chart_sizing: ResolvedChartSizing,
    ) -> FacetWrapLayoutContext {
        let ResolvedChartSizing::FacetBand(policy) = resolved_chart_sizing else {
            return FacetWrapLayoutContext::default();
        };
        let root_available_width = match policy.width {
            FacetDimensionSizing::CanvasConstrained { canvas_size } => (canvas_size
                - evaluated_layout_spec.margins.left
                - evaluated_layout_spec.margins.right)
                .max(1.0),
            FacetDimensionSizing::LeafPlotAreaSized { leaf_plot_size } => leaf_plot_size,
        };
        FacetWrapLayoutContext::from_policy(policy, root_available_width)
    }

    fn derive_leaf_subtree_plot_area(
        facet_tree: &EvaluatedFacetTree,
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    ) -> (f32, f32) {
        estimate_root_plot_area_from_leaf_size(
            facet_tree,
            LeafPlotAreaSize::new(leaf_plot_width, leaf_plot_height),
        )
        .dimensions()
    }

    /// Build the layout spec used for the initial measurement pass.
    ///
    /// Media queries and dimension params are resolved before this helper. The
    /// resulting policy assigns each physical dimension to exactly one owner:
    /// canvas-constrained dimensions keep the evaluated canvas constraint, while
    /// leaf-plot-area-sized dimensions contribute an estimated subtree plot area.
    fn layout_spec_for_resolved_chart_sizing(
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        facet_tree: &EvaluatedFacetTree,
        strategy: ResolvedChartSizing,
    ) -> EvaluatedLayoutSpec {
        match strategy {
            ResolvedChartSizing::SinglePlot => evaluated_layout_spec.clone(),
            ResolvedChartSizing::FacetBand(policy) => {
                let leaf_plot_width = policy
                    .leaf_plot_width()
                    .unwrap_or(ResolvedChartSizing::DEFAULT_CANVAS_WIDTH);
                let leaf_plot_height = policy
                    .leaf_plot_height()
                    .unwrap_or(ResolvedChartSizing::DEFAULT_CANVAS_HEIGHT);
                let (plot_area_width, plot_area_height) = Self::derive_leaf_subtree_plot_area(
                    facet_tree,
                    leaf_plot_width,
                    leaf_plot_height,
                );
                Self::layout_spec_for_policy_plot_area_size(
                    evaluated_layout_spec,
                    policy,
                    plot_area_width,
                    plot_area_height,
                )
            }
        }
    }

    fn nested_fixed_plot_area_layout_spec(
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> EvaluatedLayoutSpec {
        EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Auto,
            plot_area: EvaluatedSizeMode::Fixed {
                width: plot_area_width.max(1.0),
                height: plot_area_height.max(1.0),
            },
            margins: EvaluatedMargins {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
        }
    }

    #[doc(hidden)]
    pub fn with_facet_leaf_plot_area_for_testing(
        mut self,
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    ) -> Self {
        self.layout_spec.canvas = SizeMode::Auto;
        self.layout_spec.plot_area = SizeMode::Fixed {
            width: serializable_expr_from_expr(lit(leaf_plot_width), "facet leaf plot width"),
            height: serializable_expr_from_expr(lit(leaf_plot_height), "facet leaf plot height"),
        };
        self
    }

    #[doc(hidden)]
    pub async fn final_facet_leaf_plot_area_sizes_for_testing(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<Vec<(f32, f32)>, AvengerChartError> {
        let merged_params = if let Some(provided) = params {
            let mut merged = self.default_params.clone();
            merged.extend(provided);
            merged
        } else {
            self.default_params.clone()
        };

        let evaluated_layout_spec = evaluate_layout_spec(
            &self.layout_spec,
            ctx,
            &merged_params,
            self.get_theme().as_ref(),
        )
        .await?;
        let resolved_chart_sizing = self.resolve_chart_sizing(&evaluated_layout_spec)?;
        let wrap_layout_context =
            Self::facet_wrap_layout_context(&evaluated_layout_spec, resolved_chart_sizing);
        let facet_tree = Arc::new(
            EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
                self,
                ctx,
                &merged_params,
                wrap_layout_context,
            )
            .await?,
        );
        if !matches!(
            resolved_chart_sizing,
            ResolvedChartSizing::FacetBand(policy) if policy.is_fully_canvas_constrained()
        ) {
            return Err(AvengerChartError::InvalidArgument(
                "Derived plot-size baselines must start from a canvas-sized faceted plot"
                    .to_string(),
            ));
        }

        let measured_layout_spec = Self::layout_spec_for_resolved_chart_sizing(
            &evaluated_layout_spec,
            facet_tree.as_ref(),
            resolved_chart_sizing,
        );

        let scale_eval_ctx = avenger_chart_core::EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            merged_params.clone(),
        )
        .with_time_context(self.time_context.clone());
        let scale_builder = Box::pin(build_scale_builder_from_compiled_plot(
            self,
            None,
            &scale_eval_ctx,
            self.get_theme().as_ref(),
        ))
        .await?;

        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: self,
        };

        let eval_ctx = EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            merged_params,
            facet_tree,
        )
        .with_time_context(self.time_context.clone())
        .with_event_datum_fields(Arc::new(self.event_datum_types()))
        .with_facet_data_root(dataframe_from_compiled_plot_data(&self.data, ctx)?)
        .with_facet_runtime_sizing_mode(resolved_chart_sizing.facet_runtime_sizing_mode())
        .with_facet_layout_refinement(options.facet_layout_refinement)
        .with_debug_layout_overlay(facet_debug::resolve_layout_overlay_mode(
            options.debug_layout_overlay,
        ));

        let mut measurement = self
            .measure_plot_components(&eval_ctx, &measured_layout_spec, &provider, None, &[])
            .await?;

        self.apply_layout_snapshot(
            &LayoutSnapshot::Final,
            &mut measurement,
            &eval_ctx,
            &measured_layout_spec,
            &provider,
            resolved_chart_sizing,
        )
        .await?;

        let mut leaf_sizes = Vec::new();
        Self::collect_facet_leaf_plot_area_sizes(&measurement, &mut leaf_sizes);
        Ok(leaf_sizes)
    }

    #[doc(hidden)]
    pub async fn uniform_facet_leaf_plot_area_from_canvas_for_testing(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<(f32, f32), AvengerChartError> {
        let leaf_sizes = self
            .final_facet_leaf_plot_area_sizes_for_testing(ctx, params, options)
            .await?;
        let Some((first_width, first_height)) = leaf_sizes.first().copied() else {
            return Err(AvengerChartError::InternalError(
                "No facet leaf plot-area sizes were measured".to_string(),
            ));
        };

        let tolerance = 0.5;
        for (width, height) in &leaf_sizes {
            if (width - first_width).abs() > tolerance || (height - first_height).abs() > tolerance
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Canvas solution produced non-uniform leaf plot-area sizes: {:?}",
                    leaf_sizes
                )));
            }
        }

        Ok((first_width, first_height))
    }
}

/// Evaluate a SizeMode to get an EvaluatedSizeMode with concrete f32 values
async fn evaluate_size_mode(
    size_mode: &SizeMode,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<EvaluatedSizeMode, AvengerChartError> {
    match size_mode {
        SizeMode::Fixed { width, height } => {
            let width_node: LogicalExprNode = width.clone().into();
            let height_node: LogicalExprNode = height.clone().into();
            let width_expr = width_node.to_expr(ctx)?;
            let height_expr = height_node.to_expr(ctx)?;
            let w = evaluate_f32_expr(&width_expr, ctx, params).await?;
            let h = evaluate_f32_expr(&height_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Fixed {
                width: w,
                height: h,
            })
        }
        SizeMode::Width(width) => {
            let width_node: LogicalExprNode = width.clone().into();
            let width_expr = width_node.to_expr(ctx)?;
            let w = evaluate_f32_expr(&width_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Width(w))
        }
        SizeMode::Height(height) => {
            let height_node: LogicalExprNode = height.clone().into();
            let height_expr = height_node.to_expr(ctx)?;
            let h = evaluate_f32_expr(&height_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Height(h))
        }
        SizeMode::Auto => Ok(EvaluatedSizeMode::Auto),
    }
}

/// Evaluate Margins to get concrete f32 values (from expression, theme, or default)
async fn evaluate_margins(
    margins: &Margins,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    theme: &Theme,
) -> Result<EvaluatedMargins, AvengerChartError> {
    // Helper to query margin from theme
    let query_margin = |property: &str| -> f32 {
        let canvas_ctx = ThemeContext::new("canvas", params.clone());
        theme
            .query(&canvas_ctx, property)
            .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
            .unwrap_or(CompiledPlot::DEFAULT_MARGIN)
    };

    // Evaluate each margin field, checking expression → theme → default
    let top = match margins.top.as_ref() {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-top"),
    };

    let right = match margins.right.as_ref() {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-right"),
    };

    let bottom = match margins.bottom.as_ref() {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-bottom"),
    };

    let left = match margins.left.as_ref() {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-left"),
    };

    Ok(EvaluatedMargins {
        top,
        right,
        bottom,
        left,
    })
}

fn dataframe_from_compiled_plot_data(
    data: &Option<datafusion_proto::protobuf::LogicalPlanNode>,
    ctx: &SessionContext,
) -> Result<Option<DataFrame>, AvengerChartError> {
    data.as_ref()
        .map(|node| {
            let logical_plan = node.to_logical_plan(ctx)?;
            Ok(DataFrame::new(ctx.state().clone(), logical_plan))
        })
        .transpose()
}

/// Evaluate a LayoutSpec to get an EvaluatedLayoutSpec with concrete f32 values
async fn evaluate_layout_spec(
    layout_spec: &LayoutSpec,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    theme: &Theme,
) -> Result<EvaluatedLayoutSpec, AvengerChartError> {
    let canvas = evaluate_size_mode(&layout_spec.canvas, ctx, params).await?;
    let plot_area = evaluate_size_mode(&layout_spec.plot_area, ctx, params).await?;
    let margins = evaluate_margins(&layout_spec.margins, ctx, params, theme).await?;

    Ok(EvaluatedLayoutSpec {
        canvas,
        plot_area,
        margins,
    })
}

impl CompiledPlot {
    fn record_evaluation_metric(
        evaluation_metrics: &Option<Arc<Mutex<EvaluationMetrics>>>,
        f: impl FnOnce(&mut EvaluationMetrics),
    ) {
        if let Some(metrics) = evaluation_metrics {
            f(&mut metrics.lock().expect("evaluation metrics lock poisoned"));
        }
    }

    /// Evaluate a single mark with an optional provided plot-level DataFrame fallback.
    /// If `provided_plot_df` is Some, it is used when the mark has no explicit data and
    /// the channels reference columns. Otherwise, falls back to this CompiledPlot's plot-level data.
    /// Prepare data batches and context for mark evaluation.
    /// This is shared between measure and render passes.
    async fn prepare_mark_data(
        &self,
        mark: &dyn CompiledMark,
        eval_ctx: &EvaluationContext,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        provided_plot_df: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<Option<PreparedMarkData>, AvengerChartError> {
        let prepared_base = match self.data_group_index_for_mark(mark.state().mark_index()) {
            Some(group_index) => Some(
                Box::pin(self.prepare_mark_group_base_data(
                    group_index,
                    eval_ctx,
                    provided_plot_df,
                    facet_path,
                ))
                .await?,
            ),
            None => None,
        };
        let prepared = prepare_mark_data_runtime(MarkDataRequest {
            mark,
            coord_transform: Some(self.coord_transform.as_ref()),
            plot_data: self.data.as_ref(),
            provided_plot_df,
            facet_data_scope: Some(crate::facet::data_scope::FacetDataScopeContext::new(
                eval_ctx.facet_tree.as_ref(),
                eval_ctx.facet_data_root(),
                facet_path,
            )),
            prepared_logical: None,
            prepared_base: prepared_base.as_deref(),
            eval_ctx,
            evaluation_metrics: eval_ctx.evaluation_metrics.clone(),
            scales,
            plot_width,
            plot_height,
        })
        .await?;
        if let Some(prepared) = &prepared {
            self.validate_positional_channel_types(&prepared.data_batch, &prepared.scalar_batch)?;
        }
        Ok(prepared)
    }

    /// Render a single mark to scene marks.
    async fn render_mark_with_plot_df(
        &self,
        mark: &dyn CompiledMark,
        eval_ctx: &EvaluationContext,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        provided_plot_df: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<RenderedMarkOutput, AvengerChartError> {
        if let Some(visible_expr) = &mark.state().visible {
            let expr = visible_expr.to_expr(eval_ctx.session_context.as_ref())?;
            let visible = if let Some(value) = direct_param_value(&expr, eval_ctx.params()) {
                matches!(value, ScalarValue::Boolean(Some(true)))
            } else {
                evaluate_bool_expr(&expr, eval_ctx.session_context.as_ref(), eval_ctx.params())
                    .await?
            };
            if !visible {
                return Ok(RenderedMarkOutput::marks_only(vec![]));
            }
        }

        if let Some(subplot) = facet_subplot_ref(mark) {
            let render_state = RenderState::new(plot_width, plot_height, scales.clone());
            let render_ctx =
                RenderContext::new(eval_ctx, &render_state, facet_path, coord_measurement);
            return subplot
                .render_with_context(&render_ctx)
                .await
                .map(RenderedMarkOutput::marks_only);
        }

        if let Some(subplot) = compiled_concat_subplot(mark) {
            let prepared = self
                .prepare_mark_data(
                    mark,
                    eval_ctx,
                    scales,
                    plot_width,
                    plot_height,
                    provided_plot_df,
                    facet_path,
                )
                .await?;
            let render_state = prepared
                .as_ref()
                .map(|prepared| prepared.render_state.clone())
                .unwrap_or_else(|| RenderState::new(plot_width, plot_height, scales.clone()));
            let render_ctx =
                RenderContext::new(eval_ctx, &render_state, facet_path, coord_measurement);
            let data_batch = prepared
                .as_ref()
                .and_then(|prepared| prepared.data_batch.as_ref());
            return subplot
                .render_with_context(data_batch, &render_ctx)
                .await
                .map(RenderedMarkOutput::marks_only);
        }

        if let Some(overlay) = crate::parallel_axis_overlay::parallel_axis_overlay_ref(mark) {
            let render_state = RenderState::new(plot_width, plot_height, scales.clone());
            let render_ctx =
                RenderContext::new(eval_ctx, &render_state, facet_path, coord_measurement);
            return crate::parallel_axis_overlay::render_parallel_axis_overlay_with_context(
                overlay,
                &render_ctx,
            )
            .await
            .map(RenderedMarkOutput::marks_only);
        }

        let prepared = self
            .prepare_mark_data(
                mark,
                eval_ctx,
                scales,
                plot_width,
                plot_height,
                provided_plot_df,
                facet_path,
            )
            .await?;

        let Some(prepared) = prepared else {
            return Ok(RenderedMarkOutput::marks_only(vec![]));
        };

        let render_ctx = RenderContext::new(
            eval_ctx,
            &prepared.render_state,
            facet_path,
            coord_measurement,
        );

        if let Some(subplot) = mark.as_positioned_subplot() {
            return render_positioned_subplot_with_context(subplot, &render_ctx)
                .await
                .map(RenderedMarkOutput::marks_only);
        }

        let rendered = mark
            .render_mark_data(
                prepared.data_batch.as_ref(),
                &prepared.scalar_batch,
                &render_ctx,
                self.coord_transform.as_ref(),
            )
            .await?;
        let mut marks = rendered.marks;
        if let Some(id) = mark.state().public_target_path.as_deref() {
            for scene_mark in &mut marks {
                set_scene_mark_name(scene_mark, id);
            }
        }
        let event_datums = prepared
            .event_datum_batch
            .as_ref()
            .map(|rows| rows as &RecordBatch);
        let event_datums = event_datum_rows_for_rendered_marks(
            event_datums,
            marks.len(),
            rendered.source_row_indices,
            rendered.event_datum_rows,
        )?;
        Ok(RenderedMarkOutput {
            marks,
            event_datums,
        })
    }

    /// Create guide marks (axes, grids) for the coordinate system
    pub(super) async fn create_guide_marks(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        guide_overflow: &OverflowSpaceRequirement,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        data_override: Option<&DataFrame>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Use the pre-built guide renderer if available
        if let Some(compiled_guide) = &self.compiled_guide {
            let theme = self.get_theme();
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            let derived_scalars = derived_scalars_by_channel(scales);

            if tracing::enabled!(Level::DEBUG)
                && let Some(y_scale) = configured_scales.get("y")
            {
                let domain = y_scale.domain();
                if let Some(float_arr) = domain.as_any().downcast_ref::<Float32Array>() {
                    let vals: Vec<f32> = float_arr.iter().flatten().collect();
                    debug!(domain = ?vals, "create_guide_marks y scale domain");
                } else if let Some(float_arr) = domain.as_any().downcast_ref::<Float64Array>() {
                    let vals: Vec<f64> = float_arr.iter().flatten().collect();
                    debug!(domain = ?vals, "create_guide_marks y scale domain");
                } else {
                    debug!(domain_type = ?domain.data_type(), "create_guide_marks y scale domain type");
                }
            }

            let sharing_context =
                GuideSharingContext::new(facet_tree, facet_path, child_frame_sharing_path)
                    .with_derived_scalars(&derived_scalars);
            compiled_guide
                .evaluate(
                    &configured_scales,
                    plot_width,
                    plot_height,
                    plot_bounds,
                    guide_overflow,
                    theme.as_ref(),
                    params,
                    ctx,
                    data_override,
                    sharing_context,
                    coord_measurement,
                )
                .await
        } else {
            // No guide renderer available
            Ok(vec![])
        }
    }

    /// Compute layout with an evaluated layout specification.
    ///
    /// This unified method handles both canvas-mode and plot-area-mode layouts
    /// based on the provided EvaluatedLayoutSpec.
    pub(super) async fn compute_layout_with_spec(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
    ) -> Result<(LayoutSolution, PreparedLegendPlan), AvengerChartError> {
        // Check for required positional scales before measuring overflow
        self.validate_positional_scales_exist(scales)?;

        let resolved_dimensions = Self::resolve_dimensions_from_spec(layout_spec);

        // Get dimensions for overflow estimation. Canvas-constrained dimensions
        // use a plot-area estimate, while plot-area-sized dimensions use the
        // exact plot-area size.
        let estimate_width = if resolved_dimensions.width.is_plot_area() {
            resolved_dimensions.width_value()
        } else {
            resolved_dimensions.width_value() * Self::INITIAL_PLOT_AREA_RATIO
        };
        let estimate_height = if resolved_dimensions.height.is_plot_area() {
            resolved_dimensions.height_value()
        } else {
            resolved_dimensions.height_value() * Self::INITIAL_PLOT_AREA_RATIO
        };

        // Measure guide overflow (axis tick labels, titles, etc.)
        let overflow = if let Some(compiled_guide) = &self.compiled_guide {
            let theme = self.get_theme();
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            let derived_scalars = derived_scalars_by_channel(scales);
            let sharing_context =
                GuideSharingContext::new(facet_tree, facet_path, child_frame_sharing_path)
                    .with_derived_scalars(&derived_scalars);
            let cache_lookup = eval_ctx.guide_overflow_cache().map(|cache| {
                let guide_ptr = Arc::as_ptr(compiled_guide) as *const () as usize;
                let key = self.guide_overflow_cache_key(
                    guide_ptr,
                    scales,
                    estimate_width,
                    estimate_height,
                    params,
                    facet_tree,
                    facet_path,
                    child_frame_sharing_path,
                    data_override,
                );
                (cache.clone(), key)
            });
            if let Some((cache, key)) = cache_lookup {
                let cached_overflow = {
                    cache
                        .lock()
                        .expect("guide-overflow cache lock poisoned")
                        .get(&key)
                };
                if let Some(overflow) = cached_overflow {
                    eval_ctx.record_guide_overflow_cache_hit();
                    overflow
                } else {
                    eval_ctx.record_guide_overflow_cache_miss();
                    eval_ctx.record_guide_overflow_measure_call();
                    let guide_start = Instant::now();
                    let overflow = compiled_guide
                        .measure_overflow(
                            &configured_scales,
                            estimate_width,
                            estimate_height,
                            theme.as_ref(),
                            params,
                            data_override,
                            ctx,
                            sharing_context,
                            None, // No coordinate-system measurement is available during initial guide probing.
                        )
                        .await?;
                    let guide_elapsed = guide_start.elapsed();
                    eval_ctx.record_guide_overflow_measure_duration(guide_elapsed);
                    tracing::debug!(
                        target: "avenger_chart::resize",
                        guide_ms = guide_elapsed.as_secs_f64() * 1000.0,
                        phase = "initial",
                        "guide_overflow.measure"
                    );
                    cache
                        .lock()
                        .expect("guide-overflow cache lock poisoned")
                        .insert(key, overflow.clone());
                    overflow
                }
            } else {
                eval_ctx.record_guide_overflow_measure_call();
                let guide_start = Instant::now();
                let overflow = compiled_guide
                    .measure_overflow(
                        &configured_scales,
                        estimate_width,
                        estimate_height,
                        theme.as_ref(),
                        params,
                        data_override,
                        ctx,
                        sharing_context,
                        None, // No coordinate-system measurement is available during initial guide probing.
                    )
                    .await?;
                let guide_elapsed = guide_start.elapsed();
                eval_ctx.record_guide_overflow_measure_duration(guide_elapsed);
                tracing::debug!(
                    target: "avenger_chart::resize",
                    guide_ms = guide_elapsed.as_secs_f64() * 1000.0,
                    phase = "initial",
                    "guide_overflow.measure"
                );
                overflow
            }
        } else {
            OverflowSpaceRequirement::default()
        };

        let available_size = Size2D {
            width: estimate_width,
            height: estimate_height,
        };
        let scope = Self::legend_scope_for_context(facet_path, child_frame_sharing_path);
        Box::pin(self.compute_layout_with_precomputed_overflow(
            eval_ctx,
            &overflow,
            layout_spec,
            scales,
            available_size,
            ctx,
            params,
            facet_tree,
            facet_path,
            child_frame_sharing_path,
            scope,
        ))
        .await
    }

    async fn compute_layout_with_precomputed_overflow(
        &self,
        eval_ctx: &EvaluationContext,
        overflow: &OverflowSpaceRequirement,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        available_size: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
        scope: LegendPlanScope,
    ) -> Result<(LayoutSolution, PreparedLegendPlan), AvengerChartError> {
        Box::pin(self.compute_layout_with_precomputed_overflow_and_coord(
            eval_ctx,
            overflow,
            layout_spec,
            scales,
            available_size,
            ctx,
            params,
            facet_tree,
            facet_path,
            child_frame_sharing_path,
            scope,
            None,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn compute_layout_with_precomputed_overflow_and_coord(
        &self,
        eval_ctx: &EvaluationContext,
        overflow: &OverflowSpaceRequirement,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        available_size: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
        scope: LegendPlanScope,
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<(LayoutSolution, PreparedLegendPlan), AvengerChartError> {
        eval_ctx.record_legend_plan_build();
        let legend_plan = Box::pin(self.prepare_legend_plan(
            eval_ctx,
            scales,
            available_size,
            ctx,
            params,
            facet_tree,
            facet_path,
            child_frame_sharing_path,
            scope,
        ))
        .await?;

        Box::pin(self.compute_layout_from_legend_plan(
            eval_ctx,
            overflow,
            layout_spec,
            available_size,
            ctx,
            params,
            facet_path,
            child_frame_sharing_path,
            coord_measurement,
            legend_plan,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn compute_layout_from_legend_plan(
        &self,
        eval_ctx: &EvaluationContext,
        overflow: &OverflowSpaceRequirement,
        layout_spec: &EvaluatedLayoutSpec,
        available_size: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
        coord_measurement: Option<&dyn CoordMeasurement>,
        mut legend_plan: PreparedLegendPlan,
    ) -> Result<(LayoutSolution, PreparedLegendPlan), AvengerChartError> {
        if let Some(coord_measurement) = coord_measurement {
            Box::pin(self.consume_anchored_hoisted_legends(
                eval_ctx,
                &mut legend_plan,
                coord_measurement,
                facet_path,
                child_frame_sharing_path,
                available_size,
                ctx,
                params,
            ))
            .await?;
        }

        let mut result = Box::pin(AvengerFrameLayoutSolver::solve(FrameLayoutInput {
            overflow,
            layout_spec,
            title: self.get_title(),
            subtitle: self.get_subtitle(),
            theme: self.get_theme().as_ref(),
            legend_measurements: &legend_plan.measurements,
            ctx,
            params,
            eval_ctx: Some(eval_ctx),
        }))
        .await?;

        // The frame solver sets total_overflow to guide-only; add legend dimensions.
        for measurement in legend_plan.measurements.values() {
            match measurement.position {
                LegendPosition::Left => {
                    result.total_overflow.left += measurement.size.width;
                }
                LegendPosition::Right => {
                    result.total_overflow.right += measurement.size.width;
                }
                LegendPosition::Top => {
                    result.total_overflow.top += measurement.size.height;
                }
                LegendPosition::Bottom => {
                    result.total_overflow.bottom += measurement.size.height;
                }
            }
        }

        Ok((result, legend_plan))
    }

    fn collect_child_hoisted_legend_requests(
        coord_measurement: &dyn CoordMeasurement,
    ) -> Vec<HoistedLegendRequest> {
        let mut requests = Vec::new();

        if let Some(facet_band) = coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        {
            Self::extend_child_hoisted_legend_requests(
                &mut requests,
                facet_band.cells.iter().map(|cell| &cell.measurement),
            );
        }

        if let Some(concat) = coord_measurement
            .as_any()
            .downcast_ref::<crate::concat::ConcatCoordMeasurement>()
        {
            Self::extend_child_hoisted_legend_requests(
                &mut requests,
                concat.children.iter().map(|child| &child.measurement),
            );
        }

        if let Some(positioned) = coord_measurement
            .as_any()
            .downcast_ref::<PositionedCoordMeasurement>()
        {
            Self::extend_child_hoisted_legend_requests(
                &mut requests,
                positioned.children.iter().map(|child| &child.measurement),
            );
        }

        requests
    }

    fn extend_child_hoisted_legend_requests<'a>(
        requests: &mut Vec<HoistedLegendRequest>,
        measurements: impl IntoIterator<Item = &'a ComponentsMeasurement>,
    ) {
        for measurement in measurements {
            requests.extend(measurement.legend_plan.hoisted_requests.iter().cloned());
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn consume_anchored_hoisted_legends(
        &self,
        eval_ctx: &EvaluationContext,
        legend_plan: &mut PreparedLegendPlan,
        coord_measurement: &dyn CoordMeasurement,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
        available_size: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<(), AvengerChartError> {
        let mut anchored_here = Vec::new();
        let mut remaining = Vec::new();

        let facet_anchor = HoistedLegendAnchor::FacetPath(facet_path.to_vec());
        let child_frame_anchor =
            HoistedLegendAnchor::ChildFrameContainer(child_frame_sharing_path.container_path());

        for request in Self::collect_child_hoisted_legend_requests(coord_measurement) {
            if request.anchor == facet_anchor || request.anchor == child_frame_anchor {
                anchored_here.push(request);
            } else {
                remaining.push(request);
            }
        }

        legend_plan.hoisted_requests.extend(remaining);
        self.add_measured_hoisted_legends_to_plan(
            eval_ctx,
            legend_plan,
            anchored_here,
            available_size,
            ctx,
            params,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn total_overflow_from_precomputed_guide_overflow(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
        guide_overflow: &OverflowSpaceRequirement,
        facet_path: &[ScalarValue],
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let params_with_dims = eval_ctx.with_dimension_params(plot_area_width, plot_area_height);
        let (layout, _) = Box::pin(self.compute_layout_with_precomputed_overflow(
            &params_with_dims,
            guide_overflow,
            layout_spec,
            scales,
            Size2D {
                width: plot_area_width,
                height: plot_area_height,
            },
            params_with_dims.session_context.as_ref(),
            &params_with_dims.params,
            params_with_dims.facet_tree.as_ref(),
            facet_path,
            params_with_dims.child_frame_sharing_path(),
            Self::legend_scope_for_context(facet_path, params_with_dims.child_frame_sharing_path()),
        ))
        .await?;
        Ok(layout.total_overflow)
    }

    #[inline]
    fn legend_scope_for_context(
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
    ) -> LegendPlanScope {
        if !child_frame_sharing_path.levels().is_empty() {
            LegendPlanScope::ChildFrame {
                sharing_path: child_frame_sharing_path.clone(),
            }
        } else if facet_path.is_empty() {
            LegendPlanScope::TopLevel
        } else {
            LegendPlanScope::FacetCell
        }
    }

    async fn measure_overflow_with_coord(
        &self,
        eval_ctx: &EvaluationContext,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
        coord_measurement: Option<&dyn CoordMeasurement>,
        phase: GuideOverflowPhase,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let coord_subtree_overflow =
            Self::coord_rendered_subtree_total_overflow(coord_measurement, phase);
        let Some(compiled_guide) = &self.compiled_guide else {
            return Ok(coord_subtree_overflow.unwrap_or_default());
        };

        let configured_scales: HashMap<String, ConfiguredScale> = scales
            .iter()
            .map(|(k, v)| (k.clone(), v.configured().clone()))
            .collect();
        let derived_scalars = derived_scalars_by_channel(scales);

        let sharing_context =
            GuideSharingContext::new(facet_tree, facet_path, child_frame_sharing_path)
                .with_derived_scalars(&derived_scalars);
        let cache_lookup = if phase == GuideOverflowPhase::Final && coord_subtree_overflow.is_none()
        {
            compiled_guide
                .overflow_cache_discriminator(sharing_context, phase)
                .and_then(|discriminator| {
                    eval_ctx.guide_overflow_cache().map(|cache| {
                        let guide_ptr = Arc::as_ptr(compiled_guide) as *const () as usize;
                        let key = self.guide_overflow_cache_key_for_discriminator(
                            guide_ptr,
                            scales,
                            plot_width,
                            plot_height,
                            params,
                            discriminator,
                        );
                        (cache.clone(), key)
                    })
                })
        } else {
            None
        };

        let guide_overflow = if let Some((cache, key)) = cache_lookup {
            let cached_overflow = {
                cache
                    .lock()
                    .expect("guide-overflow cache lock poisoned")
                    .get(&key)
            };
            if let Some(overflow) = cached_overflow {
                eval_ctx.record_guide_overflow_cache_hit();
                overflow
            } else {
                eval_ctx.record_guide_overflow_cache_miss();
                eval_ctx.record_guide_overflow_measure_call();
                let guide_start = Instant::now();
                let overflow = compiled_guide
                    .measure_overflow_for_phase(
                        &configured_scales,
                        plot_width,
                        plot_height,
                        self.get_theme().as_ref(),
                        params,
                        data_override,
                        ctx,
                        sharing_context,
                        coord_measurement,
                        phase,
                    )
                    .await?;
                let guide_elapsed = guide_start.elapsed();
                eval_ctx.record_guide_overflow_measure_duration(guide_elapsed);
                tracing::debug!(
                    target: "avenger_chart::resize",
                    guide_ms = guide_elapsed.as_secs_f64() * 1000.0,
                    ?phase,
                    cache_scope = "discriminator",
                    "guide_overflow.measure"
                );
                cache
                    .lock()
                    .expect("guide-overflow cache lock poisoned")
                    .insert(key, overflow.clone());
                overflow
            }
        } else {
            eval_ctx.record_guide_overflow_measure_call();
            let guide_start = Instant::now();
            let guide_overflow = compiled_guide
                .measure_overflow_for_phase(
                    &configured_scales,
                    plot_width,
                    plot_height,
                    self.get_theme().as_ref(),
                    params,
                    data_override,
                    ctx,
                    sharing_context,
                    coord_measurement,
                    phase,
                )
                .await?;
            let guide_elapsed = guide_start.elapsed();
            eval_ctx.record_guide_overflow_measure_duration(guide_elapsed);
            tracing::debug!(
                target: "avenger_chart::resize",
                guide_ms = guide_elapsed.as_secs_f64() * 1000.0,
                ?phase,
                "guide_overflow.measure"
            );
            guide_overflow
        };

        Ok(coord_subtree_overflow
            .map(|coord_overflow| guide_overflow.max_components(&coord_overflow))
            .unwrap_or(guide_overflow))
    }

    fn coord_rendered_subtree_total_overflow(
        coord_measurement: Option<&dyn CoordMeasurement>,
        phase: GuideOverflowPhase,
    ) -> Option<OverflowSpaceRequirement> {
        let measurement = coord_measurement?;
        let phase = match phase {
            GuideOverflowPhase::Measurement => FacetOverflowResolutionPhase::Measurement,
            GuideOverflowPhase::Final => FacetOverflowResolutionPhase::Final,
        };
        let resolved_overflow =
            resolve_facet_overflow(measurement, phase, FacetOverflowPurpose::RenderedSubtree)
                .map(|resolved| resolved.overflow);
        let measured_overflow = if matches!(phase, FacetOverflowResolutionPhase::Final) {
            rendered_subtree_overflow_from_coord_measurement(
                measurement,
                FacetOverflowSource::MeasuredLocal,
            )
        } else {
            None
        };

        let overflow = match (resolved_overflow, measured_overflow) {
            (Some(resolved), Some(measured)) => CoordinatedOverflow {
                guide: resolved.guide.max_components(&measured.guide),
                total: resolved.total.max_components(&measured.total),
            },
            (Some(resolved), None) => resolved,
            (None, Some(measured)) => measured,
            (None, None) => return None,
        };
        Some(overflow.total)
    }

    async fn rebuild_layout_with_coord_overflow(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
        params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
        coord_measurement: Option<&dyn CoordMeasurement>,
        phase: GuideOverflowPhase,
    ) -> Result<(OverflowSpaceRequirement, LayoutSolution, PreparedLegendPlan), AvengerChartError>
    {
        let overflow = self
            .measure_overflow_with_coord(
                eval_ctx,
                scales,
                plot_area_width,
                plot_area_height,
                params,
                data_override,
                ctx,
                facet_tree,
                facet_path,
                child_frame_sharing_path,
                coord_measurement,
                phase,
            )
            .await?;

        let (layout, legend_plan) =
            Box::pin(self.compute_layout_with_precomputed_overflow_and_coord(
                eval_ctx,
                &overflow,
                layout_spec,
                scales,
                Size2D {
                    width: plot_area_width,
                    height: plot_area_height,
                },
                ctx,
                params,
                facet_tree,
                facet_path,
                child_frame_sharing_path,
                Self::legend_scope_for_context(facet_path, child_frame_sharing_path),
                coord_measurement,
            ))
            .await?;

        Ok((overflow, layout, legend_plan))
    }

    fn install_rebuilt_layout(
        measurement: &mut ComponentsMeasurement,
        layout: LayoutSolution,
        legend_plan: PreparedLegendPlan,
    ) -> LayoutBounds {
        let plot_bounds = *layout.plot_area_bounds();
        measurement.layout = layout;
        measurement.legend_plan = legend_plan;
        measurement.sync_canvas_size_from_layout();
        plot_bounds
    }

    fn finalize_measurement_layout_state(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        facet_path: &[ScalarValue],
        refresh_geometry: bool,
    ) -> Result<(), AvengerChartError> {
        measurement.clip = self.resolved_clip_region(
            eval_ctx,
            facet_path,
            &measurement.scales,
            measurement.plot_area_width,
            measurement.plot_area_height,
        );
        measurement.legend_plan.retarget_scales(&measurement.scales);
        if refresh_geometry {
            crate::facet::coordination_apply::refresh_current_facet_geometry(
                measurement,
                eval_ctx.facet_runtime_sizing_mode(),
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn rebuild_layout_from_current_coord_overflow(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        phase: GuideOverflowPhase,
    ) -> Result<LayoutBounds, AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let facet_tree = eval_ctx.facet_tree.as_ref();
        let (_, layout, legend_plan) = Box::pin(self.rebuild_layout_with_coord_overflow(
            eval_ctx,
            layout_spec,
            &measurement.scales,
            measurement.plot_area_width,
            measurement.plot_area_height,
            &measurement.params,
            data_override,
            ctx,
            facet_tree,
            facet_path,
            eval_ctx.child_frame_sharing_path(),
            Some(measurement.coord_measurement.as_ref()),
            phase,
        ))
        .await?;

        Ok(Self::install_rebuilt_layout(
            measurement,
            layout,
            legend_plan,
        ))
    }

    async fn refresh_final_child_layouts_bottom_up(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        facet_path: &[ScalarValue],
    ) -> Result<(), AvengerChartError> {
        if let Some(facet_band) = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
        {
            let compiled_subplot = facet_band.compiled_subplot.clone();
            for cell in &mut facet_band.cells {
                let child_layout_spec = Self::nested_fixed_plot_area_layout_spec(
                    cell.measurement.plot_area_width,
                    cell.measurement.plot_area_height,
                );
                Box::pin(compiled_subplot.refresh_final_layout_bottom_up(
                    &mut cell.measurement,
                    eval_ctx,
                    &child_layout_spec,
                    Some(&cell.data_override),
                    &cell.plan.full_path,
                ))
                .await?;
            }

            facet_band.realize_coordinated_child_frame_allocations();
            return Ok(());
        }

        if let Some(concat) = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<crate::concat::ConcatCoordMeasurement>()
        {
            for child in &mut concat.children {
                let compiled_subplot = child.compiled_subplot.clone();
                let data_override = child.data_override.clone();
                let local_facet_tree = child.local_facet_tree.clone();
                let facet_data_root = child.facet_data_root.clone();
                let mut sharing_levels = child.sharing_levels.iter().cloned();
                let first_level = sharing_levels.next().ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "Concat final layout refresh requires a sharing level".to_string(),
                    )
                })?;
                let mut child_eval_ctx =
                    eval_ctx.with_child_frame_sharing_level_appended(first_level);
                for level in sharing_levels {
                    child_eval_ctx = child_eval_ctx.with_child_frame_sharing_level_appended(level);
                }
                let local_facet_path;
                let child_facet_path = if let Some(facet_tree) = local_facet_tree {
                    child_eval_ctx = child_eval_ctx
                        .with_facet_tree(facet_tree)
                        .with_facet_data_root(facet_data_root);
                    local_facet_path = Vec::new();
                    local_facet_path.as_slice()
                } else {
                    facet_path
                };
                let child_layout_spec = Self::nested_fixed_plot_area_layout_spec(
                    child.measurement.plot_area_width,
                    child.measurement.plot_area_height,
                );
                Box::pin(compiled_subplot.refresh_final_layout_bottom_up(
                    &mut child.measurement,
                    &child_eval_ctx,
                    &child_layout_spec,
                    data_override.as_ref(),
                    child_facet_path,
                ))
                .await?;
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn refresh_final_layout_bottom_up(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<LayoutBounds, AvengerChartError> {
        Box::pin(self.refresh_final_child_layouts_bottom_up(measurement, eval_ctx, facet_path))
            .await?;

        let plot_bounds = Box::pin(self.rebuild_layout_from_current_coord_overflow(
            measurement,
            eval_ctx,
            layout_spec,
            data_override,
            facet_path,
            GuideOverflowPhase::Final,
        ))
        .await?;
        self.finalize_measurement_layout_state(measurement, eval_ctx, facet_path, false)?;

        Ok(plot_bounds)
    }

    #[allow(clippy::too_many_arguments)]
    async fn refresh_policy_layout_from_current_coord_overflow(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        policy: FacetRuntimeSizingPolicy,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        phase: GuideOverflowPhase,
    ) -> Result<FacetLayoutRefreshOutcome, AvengerChartError> {
        let mut retarget_count = 0usize;
        for _ in 0..2 {
            let plot_area_width = measurement.plot_area_width.max(1.0);
            let plot_area_height = measurement.plot_area_height.max(1.0);
            let layout_spec = if facet_path.is_empty() {
                Self::layout_spec_for_policy_plot_area_size(
                    evaluated_layout_spec,
                    policy,
                    plot_area_width,
                    plot_area_height,
                )
            } else {
                Self::nested_fixed_plot_area_layout_spec(plot_area_width, plot_area_height)
            };
            let plot_bounds = Box::pin(self.rebuild_layout_from_current_coord_overflow(
                measurement,
                eval_ctx,
                &layout_spec,
                data_override,
                facet_path,
                phase,
            ))
            .await?;

            let current_width = measurement.plot_area_width.max(1.0);
            let current_height = measurement.plot_area_height.max(1.0);
            let target_width = if policy.width.is_canvas_constrained() {
                plot_bounds.width.max(1.0)
            } else {
                current_width
            };
            let target_height = if policy.height.is_canvas_constrained() {
                plot_bounds.height.max(1.0)
            } else {
                current_height
            };
            if (target_width - current_width).abs() <= 0.01
                && (target_height - current_height).abs() <= 0.01
            {
                self.finalize_measurement_layout_state(measurement, eval_ctx, facet_path, true)?;
                return Ok(FacetLayoutRefreshOutcome {
                    plot_bounds,
                    retarget_count,
                });
            }

            retarget_measurement_plot_area_policy_no_remeasure(
                measurement,
                self,
                eval_ctx,
                facet_path,
                target_width,
                target_height,
            )?;
            retarget_count += 1;
        }

        let plot_area_width = measurement.plot_area_width.max(1.0);
        let plot_area_height = measurement.plot_area_height.max(1.0);
        let layout_spec = if facet_path.is_empty() {
            Self::layout_spec_for_policy_plot_area_size(
                evaluated_layout_spec,
                policy,
                plot_area_width,
                plot_area_height,
            )
        } else {
            Self::nested_fixed_plot_area_layout_spec(plot_area_width, plot_area_height)
        };
        let plot_bounds = Box::pin(self.rebuild_layout_from_current_coord_overflow(
            measurement,
            eval_ctx,
            &layout_spec,
            data_override,
            facet_path,
            phase,
        ))
        .await?;
        self.finalize_measurement_layout_state(measurement, eval_ctx, facet_path, true)?;

        Ok(FacetLayoutRefreshOutcome {
            plot_bounds,
            retarget_count,
        })
    }

    async fn refresh_facet_snapshot_layout_from_coord_overflow(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        resolved_chart_sizing: ResolvedChartSizing,
        phase: GuideOverflowPhase,
    ) -> Result<(), AvengerChartError> {
        let ResolvedChartSizing::FacetBand(policy) = resolved_chart_sizing else {
            return Ok(());
        };
        if measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .is_none()
        {
            return Ok(());
        }

        let outcome = Box::pin(self.refresh_policy_layout_from_current_coord_overflow(
            measurement,
            eval_ctx,
            evaluated_layout_spec,
            policy,
            None,
            &[],
            phase,
        ))
        .await?;
        trace!(
            plot_area_width = measurement.plot_area_width,
            plot_area_height = measurement.plot_area_height,
            plot_bounds_width = outcome.plot_bounds.width,
            plot_bounds_height = outcome.plot_bounds.height,
            retarget_count = outcome.retarget_count,
            "facet snapshot layout refreshed from current coord overflow"
        );

        Ok(())
    }

    pub(crate) async fn refresh_reused_profile_layout(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<(), AvengerChartError> {
        tracing::debug!(
            target: "avenger_chart::resize",
            facet_depth = facet_path.len(),
            "refresh_reused_profile_layout start"
        );
        let refresh_start = Instant::now();
        eval_ctx.record_facet_cell_measurement_profile_chrome_refresh();
        let layout_spec = Self::nested_fixed_plot_area_layout_spec(
            measurement.plot_area_width,
            measurement.plot_area_height,
        );
        Box::pin(self.refresh_final_layout_bottom_up(
            measurement,
            eval_ctx,
            &layout_spec,
            data_override,
            facet_path,
        ))
        .await?;
        sync_measurement_owned_slabs_from_coord(measurement);
        crate::facet::coordination_apply::refresh_current_facet_geometry(
            measurement,
            eval_ctx.facet_runtime_sizing_mode(),
        )?;
        let refresh_elapsed = refresh_start.elapsed();
        eval_ctx.record_refresh_reused_profile_layout_duration(refresh_elapsed);
        tracing::debug!(
            target: "avenger_chart::resize",
            chrome_ms = refresh_elapsed.as_secs_f64() * 1000.0,
            facet_depth = facet_path.len(),
            "refresh_reused_profile_layout"
        );
        Ok(())
    }

    fn realize_canvas_plot_area_no_overflow_remeasure(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        facet_path: &[ScalarValue],
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Result<(), AvengerChartError> {
        let plot_area_width = plot_area_width.max(1.0);
        let plot_area_height = plot_area_height.max(1.0);

        retarget_scale_ranges_for_plot_area(
            &mut measurement.scales,
            plot_area_width,
            plot_area_height,
        );

        if let Some(facet_band) = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
        {
            facet_band.retarget_parent_plot_area_no_remeasure(
                eval_ctx,
                plot_area_width,
                plot_area_height,
            )?;
        }

        measurement.plot_area_width = plot_area_width;
        measurement.plot_area_height = plot_area_height;
        // Canvas-mode params carry the evaluated canvas dimensions for media
        // queries and user expressions. Retargeting the plot area must not
        // turn `width`/`height` into inner plot dimensions.
        self.finalize_measurement_layout_state(measurement, eval_ctx, facet_path, true)?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn measure_canvas_candidate_layout_from_current_measurement(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        iter: usize,
        trace_label: &'static str,
    ) -> Result<LayoutBounds, AvengerChartError> {
        let candidate_bounds = Box::pin(self.refresh_final_layout_bottom_up(
            measurement,
            eval_ctx,
            layout_spec,
            data_override,
            facet_path,
        ))
        .await?;

        let delta_w = (candidate_bounds.width - measurement.plot_area_width).abs();
        let delta_h = (candidate_bounds.height - measurement.plot_area_height).abs();
        trace!(
            iter,
            current_width = measurement.plot_area_width,
            current_height = measurement.plot_area_height,
            candidate_width = candidate_bounds.width,
            candidate_height = candidate_bounds.height,
            delta_w,
            delta_h,
            trace_label
        );

        Ok(candidate_bounds)
    }

    async fn remeasure_canvas_coord_at_current_plot_area(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<(), AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let next_width = measurement.plot_area_width.max(1.0);
        let next_height = measurement.plot_area_height.max(1.0);
        let mut probe_size_overrides = HashMap::new();
        Self::collect_facet_probe_size_overrides(measurement, &mut probe_size_overrides);
        let params_with_canvas_dims = eval_ctx
            .with_params(measurement.params.clone())
            .with_facet_probe_size_overrides(Arc::new(probe_size_overrides));

        let final_scales = scale_provider
            .build_scales(
                next_width,
                next_height,
                ctx,
                &params_with_canvas_dims.params,
            )
            .await?;
        let coord_measurement = Box::pin(self.measure_coord_system(
            &final_scales,
            next_width,
            next_height,
            &params_with_canvas_dims,
            data_override,
            facet_path,
            ctx,
        ))
        .await?;
        measurement.plot_area_width = next_width;
        measurement.plot_area_height = next_height;
        measurement.scales = final_scales;
        measurement.coord_measurement = coord_measurement;
        self.finalize_measurement_layout_state(measurement, eval_ctx, facet_path, false)?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_canvas_refinement_iteration(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        padding_feedback: Option<Arc<FacetBandPaddingFeedbackMap>>,
        iteration: usize,
        target_checkpoint: Option<RefinementCheckpoint>,
        trace_label: &'static str,
    ) -> Result<RefinementIterationOutcome, AvengerChartError> {
        let feedback_eval_ctx = padding_feedback
            .as_ref()
            .map(|feedback| eval_ctx.with_facet_padding_feedback(feedback.clone()));
        let active_eval_ctx = feedback_eval_ctx.as_ref().unwrap_or(eval_ctx);
        let before = if iteration == 0 {
            None
        } else {
            Some(Self::recursive_overflow_snapshot(measurement))
        };

        if iteration > 0 {
            Box::pin(self.remeasure_canvas_coord_at_current_plot_area(
                measurement,
                active_eval_ctx,
                scale_provider,
                data_override,
                facet_path,
            ))
            .await?;
            coordinate_overflow_for_guides(measurement, active_eval_ctx).await?;

            if target_checkpoint == Some(RefinementCheckpoint::Recoordinated) {
                return Ok(RefinementIterationOutcome {
                    reached_snapshot_checkpoint: true,
                    overflow_grew: None,
                    realized_padding_feedback: Arc::new(HashMap::new()),
                });
            }
        }

        let candidate_bounds = Box::pin(
            self.measure_canvas_candidate_layout_from_current_measurement(
                measurement,
                active_eval_ctx,
                layout_spec,
                data_override,
                facet_path,
                iteration,
                trace_label,
            ),
        )
        .await?;

        if target_checkpoint == Some(RefinementCheckpoint::CandidateLayoutMeasured) {
            return Ok(RefinementIterationOutcome {
                reached_snapshot_checkpoint: true,
                overflow_grew: None,
                realized_padding_feedback: Arc::new(HashMap::new()),
            });
        }

        self.realize_canvas_plot_area_no_overflow_remeasure(
            measurement,
            active_eval_ctx,
            facet_path,
            candidate_bounds.width.max(1.0),
            candidate_bounds.height.max(1.0),
        )?;

        if target_checkpoint == Some(RefinementCheckpoint::PlotAreaRetargeted) {
            return Ok(RefinementIterationOutcome {
                reached_snapshot_checkpoint: true,
                overflow_grew: None,
                realized_padding_feedback: Arc::new(HashMap::new()),
            });
        }

        let realized_padding_feedback =
            Arc::new(Self::realized_padding_feedback_snapshot(measurement));
        let padding_feedback_grew = padding_feedback.as_ref().map(|previous| {
            Self::padding_feedback_increased(
                previous.as_ref(),
                realized_padding_feedback.as_ref(),
                active_eval_ctx
                    .facet_layout_refinement()
                    .overflow_growth_epsilon,
            )
        });
        let overflow_grew = before.map(|before| {
            let after = Self::recursive_overflow_snapshot(measurement);
            let recursive_overflow_grew = Self::recursive_overflow_increased(
                &before,
                &after,
                active_eval_ctx
                    .facet_layout_refinement()
                    .overflow_growth_epsilon,
            );
            recursive_overflow_grew || padding_feedback_grew.unwrap_or(false)
        });

        Ok(RefinementIterationOutcome {
            reached_snapshot_checkpoint: false,
            overflow_grew,
            realized_padding_feedback,
        })
    }

    async fn refine_canvas_measurement_after_coordination(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
    ) -> Result<(), AvengerChartError> {
        Box::pin(self.refine_measurement_after_coordination(
            FacetRefinementMode::Canvas {
                layout_spec,
                scale_provider,
            },
            measurement,
            eval_ctx,
            data_override,
            facet_path,
            max_refinement_passes,
            "canvas layout",
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn refine_canvas_measurement_after_coordination_until(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
        target_iteration: usize,
        target_checkpoint: RefinementCheckpoint,
    ) -> Result<(), AvengerChartError> {
        Box::pin(self.refine_measurement_after_coordination_until(
            FacetRefinementMode::Canvas {
                layout_spec,
                scale_provider,
            },
            measurement,
            eval_ctx,
            data_override,
            facet_path,
            max_refinement_passes,
            target_iteration,
            target_checkpoint,
            "canvas layout",
        ))
        .await
    }

    fn facet_subtree_realized_plot_area_extent(
        measurement: &ComponentsMeasurement,
    ) -> Option<(f32, f32)> {
        if let Some(facet_band) = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .filter(|facet_band| facet_band.content_driven_main_axis())
        {
            if facet_band.cells.is_empty() {
                return Some((measurement.plot_area_width, measurement.plot_area_height));
            }
            return Some(facet_band.plot_area_extent());
        }

        let facet_band = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()?;
        if facet_band.cells.is_empty() {
            return Some((measurement.plot_area_width, measurement.plot_area_height));
        }

        let child_extents = facet_band
            .child_measurements_iter()
            .map(|child| {
                Self::facet_subtree_realized_plot_area_extent(child)
                    .unwrap_or((child.plot_area_width, child.plot_area_height))
            })
            .collect::<Vec<_>>();

        let (width, height) = match facet_band.axis {
            FacetAxis::Column => {
                let height = child_extents
                    .iter()
                    .map(|(_, height)| *height)
                    .fold(0.0_f32, f32::max);
                (measurement.plot_area_width, height.max(1.0))
            }
            FacetAxis::Row => {
                let width = child_extents
                    .iter()
                    .map(|(width, _)| *width)
                    .fold(0.0_f32, f32::max);
                (width.max(1.0), measurement.plot_area_height)
            }
        };
        Some((width, height))
    }

    /// Build a layout spec from a known policy and concrete root plot-area size.
    ///
    /// The same ownership rules apply to final realization and refinement:
    /// canvas-constrained dimensions remain canvas constraints, and
    /// leaf-plot-area-sized dimensions become plot-area constraints.
    fn layout_spec_for_policy_plot_area_size(
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        policy: FacetRuntimeSizingPolicy,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> EvaluatedLayoutSpec {
        let mut adjusted = evaluated_layout_spec.clone();
        adjusted.canvas = match (policy.width, policy.height) {
            (
                FacetDimensionSizing::CanvasConstrained { canvas_size: width },
                FacetDimensionSizing::CanvasConstrained {
                    canvas_size: height,
                },
            ) => EvaluatedSizeMode::Fixed { width, height },
            (
                FacetDimensionSizing::CanvasConstrained { canvas_size: width },
                FacetDimensionSizing::LeafPlotAreaSized { .. },
            ) => EvaluatedSizeMode::Width(width),
            (
                FacetDimensionSizing::LeafPlotAreaSized { .. },
                FacetDimensionSizing::CanvasConstrained {
                    canvas_size: height,
                },
            ) => EvaluatedSizeMode::Height(height),
            (
                FacetDimensionSizing::LeafPlotAreaSized { .. },
                FacetDimensionSizing::LeafPlotAreaSized { .. },
            ) => EvaluatedSizeMode::Auto,
        };
        adjusted.plot_area = match (policy.width, policy.height) {
            (
                FacetDimensionSizing::LeafPlotAreaSized { .. },
                FacetDimensionSizing::LeafPlotAreaSized { .. },
            ) => EvaluatedSizeMode::Fixed {
                width: plot_area_width.max(1.0),
                height: plot_area_height.max(1.0),
            },
            (
                FacetDimensionSizing::LeafPlotAreaSized { .. },
                FacetDimensionSizing::CanvasConstrained { .. },
            ) => EvaluatedSizeMode::Width(plot_area_width.max(1.0)),
            (
                FacetDimensionSizing::CanvasConstrained { .. },
                FacetDimensionSizing::LeafPlotAreaSized { .. },
            ) => EvaluatedSizeMode::Height(plot_area_height.max(1.0)),
            (
                FacetDimensionSizing::CanvasConstrained { .. },
                FacetDimensionSizing::CanvasConstrained { .. },
            ) => EvaluatedSizeMode::Auto,
        };
        adjusted
    }

    async fn build_scale_builder_for_render_context(
        &self,
        eval_ctx: &EvaluationContext,
        data_override: Option<DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<avenger_chart_scales::ScaleBuilder, AvengerChartError> {
        let cache_lookup = eval_ctx.scale_domain_cache().map(|cache| {
            let scope = if facet_path.is_empty() && data_override.is_none() {
                ScaleDomainCacheScope::TopLevel
            } else {
                ScaleDomainCacheScope::FacetPath(
                    facet_path
                        .iter()
                        .map(|value| format!("{value:?}"))
                        .collect(),
                )
            };
            let key = scale_domain_cache_key_for_parts_with_scope(
                &self.marks,
                &self.scale_specs,
                &self.data,
                data_override.as_ref(),
                eval_ctx.session_context.as_ref(),
                eval_ctx.params(),
                scope,
            );
            (cache.clone(), key)
        });
        if let Some((cache, key)) = &cache_lookup {
            let cached_builder = {
                cache
                    .lock()
                    .expect("scale-domain cache lock poisoned")
                    .get(key)
            };
            if let Some(builder) = cached_builder {
                eval_ctx.record_scale_domain_cache_hit();
                return Ok((*builder).clone());
            }
            eval_ctx.record_scale_domain_cache_miss();
        }
        eval_ctx.record_scale_builder_build();
        let scale_builder = Box::pin(build_scale_builder_from_compiled_plot(
            self,
            data_override,
            eval_ctx,
            self.get_theme().as_ref(),
        ))
        .await?;
        if let Some((cache, key)) = cache_lookup {
            cache
                .lock()
                .expect("scale-domain cache lock poisoned")
                .insert(key, scale_builder.clone());
        }
        Ok(scale_builder)
    }

    async fn remeasure_policy_coord_at_current_plot_area(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<(), AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let plot_area_width = measurement.plot_area_width.max(1.0);
        let plot_area_height = measurement.plot_area_height.max(1.0);

        let policy = eval_ctx.facet_runtime_sizing_mode().policy();
        let realized_layout_spec = if facet_path.is_empty() {
            Self::layout_spec_for_policy_plot_area_size(
                evaluated_layout_spec,
                policy,
                plot_area_width,
                plot_area_height,
            )
        } else {
            Self::nested_fixed_plot_area_layout_spec(plot_area_width, plot_area_height)
        };
        let mut probe_size_overrides = HashMap::new();
        Self::collect_facet_probe_size_overrides(measurement, &mut probe_size_overrides);
        let params_with_current_dims = eval_ctx
            .with_params(measurement.params.clone())
            .with_facet_probe_size_overrides(Arc::new(probe_size_overrides))
            .without_layout_profile();
        let scale_builder = Box::pin(self.build_scale_builder_for_render_context(
            &params_with_current_dims,
            data_override.cloned(),
            facet_path,
        ))
        .await?;
        let scale_provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: self,
        };

        if measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .is_none()
        {
            *measurement = Box::pin(self.measure_plot_components(
                &params_with_current_dims,
                &realized_layout_spec,
                &scale_provider,
                data_override,
                facet_path,
            ))
            .await?;
            return Ok(());
        }

        let final_scales = scale_provider
            .build_scales(
                plot_area_width,
                plot_area_height,
                ctx,
                &params_with_current_dims.params,
            )
            .await?;
        let coord_measurement = Box::pin(self.measure_coord_system(
            &final_scales,
            plot_area_width,
            plot_area_height,
            &params_with_current_dims,
            data_override,
            facet_path,
            ctx,
        ))
        .await?;
        measurement.scales = final_scales;
        measurement.coord_measurement = coord_measurement;

        Box::pin(self.rebuild_layout_from_current_coord_overflow(
            measurement,
            eval_ctx,
            &realized_layout_spec,
            data_override,
            facet_path,
            GuideOverflowPhase::Final,
        ))
        .await?;
        self.finalize_measurement_layout_state(measurement, eval_ctx, facet_path, false)?;

        Ok(())
    }

    async fn realize_policy_extents_no_remeasure(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<(), AvengerChartError> {
        let policy = eval_ctx.facet_runtime_sizing_mode().policy();
        if let Some(facet_band) = measurement
            .coord_measurement
            .as_any_mut()
            .downcast_mut::<FacetBandCoordMeasurement>()
        {
            let compiled_subplot = facet_band.compiled_subplot.clone();
            for cell in &mut facet_band.cells {
                let child_layout_spec = Self::nested_fixed_plot_area_layout_spec(
                    cell.measurement.plot_area_width,
                    cell.measurement.plot_area_height,
                );
                Box::pin(compiled_subplot.realize_policy_extents_no_remeasure(
                    &mut cell.measurement,
                    eval_ctx,
                    &child_layout_spec,
                    Some(&cell.data_override),
                    &cell.plan.full_path,
                ))
                .await?;
            }

            facet_band.realize_coordinated_child_frame_allocations();
        } else {
            return Ok(());
        }

        let (subtree_width, subtree_height) =
            Self::facet_subtree_realized_plot_area_extent(measurement)
                .unwrap_or((measurement.plot_area_width, measurement.plot_area_height));
        let mut plot_area_width = measurement.plot_area_width;
        let mut plot_area_height = measurement.plot_area_height;
        if policy.width.is_leaf_plot_area_sized() {
            plot_area_width = subtree_width.max(1.0);
        }
        if policy.height.is_leaf_plot_area_sized() {
            plot_area_height = subtree_height.max(1.0);
        }

        retarget_scale_ranges_for_plot_area(
            &mut measurement.scales,
            plot_area_width,
            plot_area_height,
        );

        measurement.plot_area_width = plot_area_width;
        measurement.plot_area_height = plot_area_height;

        let realized_layout_spec = if facet_path.is_empty() {
            Self::layout_spec_for_policy_plot_area_size(
                evaluated_layout_spec,
                policy,
                plot_area_width,
                plot_area_height,
            )
        } else {
            Self::nested_fixed_plot_area_layout_spec(plot_area_width, plot_area_height)
        };
        let plot_bounds = Box::pin(self.rebuild_layout_from_current_coord_overflow(
            measurement,
            eval_ctx,
            &realized_layout_spec,
            data_override,
            facet_path,
            GuideOverflowPhase::Final,
        ))
        .await?;

        if policy.width.is_canvas_constrained() {
            plot_area_width = plot_bounds.width.max(1.0);
        }
        if policy.height.is_canvas_constrained() {
            plot_area_height = plot_bounds.height.max(1.0);
        }
        let retargeted = if (measurement.plot_area_width - plot_area_width).abs() > 0.01
            || (measurement.plot_area_height - plot_area_height).abs() > 0.01
        {
            retarget_measurement_plot_area_policy_no_remeasure(
                measurement,
                self,
                eval_ctx,
                facet_path,
                plot_area_width,
                plot_area_height,
            )?;
            true
        } else {
            self.finalize_measurement_layout_state(measurement, eval_ctx, facet_path, false)?;
            false
        };

        if retargeted {
            let post_retarget_width = measurement.plot_area_width.max(1.0);
            let post_retarget_height = measurement.plot_area_height.max(1.0);
            let post_retarget_layout_spec = if facet_path.is_empty() {
                Self::layout_spec_for_policy_plot_area_size(
                    evaluated_layout_spec,
                    policy,
                    post_retarget_width,
                    post_retarget_height,
                )
            } else {
                Self::nested_fixed_plot_area_layout_spec(post_retarget_width, post_retarget_height)
            };
            let post_retarget_bounds = Box::pin(self.rebuild_layout_from_current_coord_overflow(
                measurement,
                eval_ctx,
                &post_retarget_layout_spec,
                data_override,
                facet_path,
                GuideOverflowPhase::Final,
            ))
            .await?;

            let mut realized_width = post_retarget_width;
            let mut realized_height = post_retarget_height;
            if policy.width.is_canvas_constrained() {
                realized_width = post_retarget_bounds.width.max(1.0);
            }
            if policy.height.is_canvas_constrained() {
                realized_height = post_retarget_bounds.height.max(1.0);
            }

            if (measurement.plot_area_width - realized_width).abs() > 0.01
                || (measurement.plot_area_height - realized_height).abs() > 0.01
            {
                retarget_measurement_plot_area_policy_no_remeasure(
                    measurement,
                    self,
                    eval_ctx,
                    facet_path,
                    realized_width,
                    realized_height,
                )?;
            } else {
                self.finalize_measurement_layout_state(measurement, eval_ctx, facet_path, true)?;
            }
        }

        trace!(
            facet_path = ?facet_path,
            plot_area_width = measurement.plot_area_width,
            plot_area_height = measurement.plot_area_height,
            canvas_width = measurement.canvas_size.0,
            canvas_height = measurement.canvas_size.1,
            "policy realization computed facet extent"
        );

        if !retargeted {
            crate::facet::coordination_apply::refresh_current_facet_geometry(
                measurement,
                eval_ctx.facet_runtime_sizing_mode(),
            )?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_policy_refinement_iteration(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        padding_feedback: Option<Arc<FacetBandPaddingFeedbackMap>>,
        iteration: usize,
        target_checkpoint: Option<RefinementCheckpoint>,
        trace_label: &'static str,
    ) -> Result<RefinementIterationOutcome, AvengerChartError> {
        let feedback_eval_ctx = padding_feedback
            .as_ref()
            .map(|feedback| eval_ctx.with_facet_padding_feedback(feedback.clone()));
        let active_eval_ctx = feedback_eval_ctx.as_ref().unwrap_or(eval_ctx);
        let before = if iteration == 0 {
            None
        } else {
            Some(Self::recursive_overflow_snapshot(measurement))
        };

        if iteration > 0 {
            Box::pin(self.remeasure_policy_coord_at_current_plot_area(
                measurement,
                active_eval_ctx,
                evaluated_layout_spec,
                data_override,
                facet_path,
            ))
            .await?;
            coordinate_overflow_for_guides(measurement, active_eval_ctx).await?;

            if target_checkpoint == Some(RefinementCheckpoint::Recoordinated) {
                return Ok(RefinementIterationOutcome {
                    reached_snapshot_checkpoint: true,
                    overflow_grew: None,
                    realized_padding_feedback: Arc::new(HashMap::new()),
                });
            }
        }

        Box::pin(self.realize_policy_extents_no_remeasure(
            measurement,
            active_eval_ctx,
            evaluated_layout_spec,
            data_override,
            facet_path,
        ))
        .await?;

        if matches!(
            target_checkpoint,
            Some(RefinementCheckpoint::CandidateLayoutMeasured)
                | Some(RefinementCheckpoint::PlotAreaRetargeted)
        ) {
            return Ok(RefinementIterationOutcome {
                reached_snapshot_checkpoint: true,
                overflow_grew: None,
                realized_padding_feedback: Arc::new(HashMap::new()),
            });
        }

        let realized_padding_feedback =
            Arc::new(Self::realized_padding_feedback_snapshot(measurement));
        let padding_feedback_grew = padding_feedback.as_ref().map(|previous| {
            Self::padding_feedback_increased(
                previous.as_ref(),
                realized_padding_feedback.as_ref(),
                active_eval_ctx
                    .facet_layout_refinement()
                    .overflow_growth_epsilon,
            )
        });
        let overflow_grew = before.map(|before| {
            let after = Self::recursive_overflow_snapshot(measurement);
            let recursive_overflow_grew = Self::recursive_overflow_increased(
                &before,
                &after,
                active_eval_ctx
                    .facet_layout_refinement()
                    .overflow_growth_epsilon,
            );
            recursive_overflow_grew || padding_feedback_grew.unwrap_or(false)
        });

        trace!(
            iteration,
            overflow_grew,
            canvas_width = measurement.canvas_size.0,
            canvas_height = measurement.canvas_size.1,
            trace_label
        );

        Ok(RefinementIterationOutcome {
            reached_snapshot_checkpoint: false,
            overflow_grew,
            realized_padding_feedback,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_refinement_iteration(
        &self,
        mode: FacetRefinementMode<'_>,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        padding_feedback: Option<Arc<FacetBandPaddingFeedbackMap>>,
        iteration: usize,
        target_checkpoint: Option<RefinementCheckpoint>,
        trace_label: &'static str,
    ) -> Result<RefinementIterationOutcome, AvengerChartError> {
        match mode {
            FacetRefinementMode::Canvas {
                layout_spec,
                scale_provider,
            } => {
                Box::pin(self.run_canvas_refinement_iteration(
                    measurement,
                    eval_ctx,
                    layout_spec,
                    scale_provider,
                    data_override,
                    facet_path,
                    padding_feedback,
                    iteration,
                    target_checkpoint,
                    trace_label,
                ))
                .await
            }
            FacetRefinementMode::Policy { layout_spec } => {
                Box::pin(self.run_policy_refinement_iteration(
                    measurement,
                    eval_ctx,
                    layout_spec,
                    data_override,
                    facet_path,
                    padding_feedback,
                    iteration,
                    target_checkpoint,
                    trace_label,
                ))
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn refine_measurement_after_coordination(
        &self,
        mode: FacetRefinementMode<'_>,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
        trace_prefix: &'static str,
    ) -> Result<(), AvengerChartError> {
        let mut padding_feedback = Box::pin(self.run_refinement_iteration(
            mode,
            measurement,
            eval_ctx,
            data_override,
            facet_path,
            None,
            0,
            None,
            "mandatory realization",
        ))
        .await?
        .realized_padding_feedback;

        if max_refinement_passes == 0 {
            eval_ctx.record_facet_refinement_converged();
            trace!(
                trace_prefix,
                "refinement disabled after mandatory realization"
            );
            return Ok(());
        }

        for pass in 1..=max_refinement_passes {
            let outcome = Box::pin(self.run_refinement_iteration(
                mode,
                measurement,
                eval_ctx,
                data_override,
                facet_path,
                Some(padding_feedback.clone()),
                pass,
                None,
                "refinement realization",
            ))
            .await?;

            eval_ctx.record_facet_refinement_pass();
            let overflow_grew = outcome.overflow_grew.unwrap_or(false);
            trace!(
                trace_prefix,
                pass, overflow_grew, "refinement pass completed"
            );

            if !overflow_grew {
                eval_ctx.record_facet_refinement_converged();
                return Ok(());
            }
            padding_feedback = outcome.realized_padding_feedback;
        }

        eval_ctx.record_facet_refinement_hit_max_passes();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn refine_measurement_after_coordination_until(
        &self,
        mode: FacetRefinementMode<'_>,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
        target_iteration: usize,
        target_checkpoint: RefinementCheckpoint,
        trace_prefix: &'static str,
    ) -> Result<(), AvengerChartError> {
        if target_iteration > max_refinement_passes {
            return Err(AvengerChartError::InternalError(format!(
                "Requested refinement snapshot iteration {} exceeds configured max iteration {}",
                target_iteration, max_refinement_passes
            )));
        }

        let target = (target_iteration == 0).then_some(target_checkpoint);
        let outcome = Box::pin(self.run_refinement_iteration(
            mode,
            measurement,
            eval_ctx,
            data_override,
            facet_path,
            None,
            0,
            target,
            "refinement snapshot",
        ))
        .await?;

        if outcome.reached_snapshot_checkpoint {
            return Ok(());
        }

        if max_refinement_passes == 0 {
            return Err(AvengerChartError::InternalError(format!(
                "Requested refinement snapshot {:?} at iteration {} was not reached ({trace_prefix} refinement snapshot)",
                target_checkpoint, target_iteration
            )));
        }

        let mut padding_feedback = outcome.realized_padding_feedback;
        for pass in 1..=max_refinement_passes {
            let target = (pass == target_iteration).then_some(target_checkpoint);
            let outcome = Box::pin(self.run_refinement_iteration(
                mode,
                measurement,
                eval_ctx,
                data_override,
                facet_path,
                Some(padding_feedback.clone()),
                pass,
                target,
                "refinement snapshot",
            ))
            .await?;
            if outcome.reached_snapshot_checkpoint {
                return Ok(());
            }
            if !outcome.overflow_grew.unwrap_or(false) {
                break;
            }
            padding_feedback = outcome.realized_padding_feedback;
        }

        Err(AvengerChartError::InternalError(format!(
            "Requested refinement snapshot {:?} at iteration {} was not reached ({trace_prefix} refinement snapshot)",
            target_checkpoint, target_iteration
        )))
    }

    async fn refine_policy_measurement_after_coordination(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
    ) -> Result<(), AvengerChartError> {
        Box::pin(self.refine_measurement_after_coordination(
            FacetRefinementMode::Policy {
                layout_spec: evaluated_layout_spec,
            },
            measurement,
            eval_ctx,
            data_override,
            facet_path,
            max_refinement_passes,
            "policy",
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn refine_policy_measurement_after_coordination_until(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
        target_iteration: usize,
        target_checkpoint: RefinementCheckpoint,
    ) -> Result<(), AvengerChartError> {
        if target_iteration == 0 && target_checkpoint == RefinementCheckpoint::Recoordinated {
            return Ok(());
        }
        Box::pin(self.refine_measurement_after_coordination_until(
            FacetRefinementMode::Policy {
                layout_spec: evaluated_layout_spec,
            },
            measurement,
            eval_ctx,
            data_override,
            facet_path,
            max_refinement_passes,
            target_iteration,
            target_checkpoint,
            "policy",
        ))
        .await
    }

    async fn realize_policy_layout_after_coordination(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        max_refinement_passes: usize,
    ) -> Result<(), AvengerChartError> {
        Box::pin(self.refine_policy_measurement_after_coordination(
            measurement,
            eval_ctx,
            layout_spec,
            data_override,
            facet_path,
            max_refinement_passes,
        ))
        .await?;

        debug_assert!(
            Self::legends_within_canvas_recursive(measurement),
            "policy invariant: legends must be within canvas after extent realization"
        );
        Ok(())
    }

    /// Extract width and height independently from the evaluated layout spec.
    ///
    /// Each physical dimension can be sourced from the canvas constraint or the
    /// plot-area constraint. This is what lets faceted charts combine a fixed
    /// canvas width with a fixed per-leaf plot height.
    fn resolve_dimensions_from_spec(layout_spec: &EvaluatedLayoutSpec) -> ResolvedLayoutDimensions {
        ResolvedLayoutDimensions::from_spec(
            layout_spec,
            ResolvedChartSizing::DEFAULT_CANVAS_WIDTH,
            ResolvedChartSizing::DEFAULT_CANVAS_HEIGHT,
        )
    }

    /// Determine clip region from guide or default to plot area rect.
    fn get_clip_region(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Clip {
        if let Some(ref guide) = self.compiled_guide {
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            guide.get_clip(plot_area_width, plot_area_height, &configured_scales)
        } else {
            Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: plot_area_width,
                height: plot_area_height,
            }
        }
    }

    pub(crate) fn resolved_clip_region(
        &self,
        eval_ctx: &EvaluationContext,
        facet_path: &[ScalarValue],
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Clip {
        if facet_path.is_empty()
            && matches!(
                eval_ctx.facet_runtime_sizing_mode(),
                FacetRuntimeSizingMode::Policy(_)
            )
            && eval_ctx
                .facet_runtime_sizing_mode()
                .has_leaf_plot_area_sized_dimension()
            && Self::marks_contain_facet(&self.marks)
        {
            // Plot-area-sized top-level facet content can legitimately extend past the
            // synthesized root plot-area rectangle; avoid clipping at the root.
            Clip::None
        } else {
            self.get_clip_region(scales, plot_area_width, plot_area_height)
        }
    }

    fn placed_child_frame_debug_rects(
        measurement: &ComponentsMeasurement,
        content_rect: LayoutBounds,
    ) -> Result<Option<Vec<LayoutBounds>>, AvengerChartError> {
        let Some(container) = measurement.child_frame_container_view()? else {
            return Ok(None);
        };

        let mut rects = Vec::with_capacity(container.child_regions().len());
        for child_region in container.child_regions() {
            let child_measurement = container
                .child_measurement(child_region.child_index)
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing debug child frame for child index {}",
                        child_region.child_index
                    ))
                })?;
            let child_rect = child_measurement.frame_allocation.rect;
            let mut projected =
                container.project_child_bounds(child_region.child_index, child_rect)?;
            projected.x += content_rect.x;
            projected.y += content_rect.y;
            rects.push(projected);
        }

        Ok(Some(rects))
    }

    fn debug_layout_marks_for_measurement(
        measurement: &ComponentsMeasurement,
        origin: [f32; 2],
        color: Option<String>,
        zindex: i32,
        flip_label_align: bool,
        overlay_mode: LayoutDebugOverlayMode,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();
        if overlay_mode.components_enabled() {
            let frame_layout = translated_component_debug_frame_layout(measurement, origin)?;
            marks.extend(create_debug_layout_rects(
                &frame_layout,
                color.clone(),
                Some(1.0),
                Some(zindex),
                flip_label_align,
            ));
        }
        if overlay_mode.allocation_demand_enabled() {
            let content_layout = measurement.content_layout()?;
            let child_rects = Self::placed_child_frame_debug_rects(
                measurement,
                content_layout.allocation.content_rect,
            )?
            .unwrap_or_else(|| {
                content_layout
                    .child_frame_allocations
                    .iter()
                    .map(|allocation| allocation.rect)
                    .collect()
            });
            let overlay = FrameDebugOverlay::from_content_layout_with_child_rects(
                &content_layout,
                origin,
                child_rects,
            );
            marks.extend(create_debug_overlay_rects(
                &overlay,
                color,
                Some(1.0),
                Some(zindex + 5),
                flip_label_align,
            ));
        }
        Ok(marks)
    }

    /// Compute layout and determine plot area dimensions.
    ///
    /// Handles two modes:
    /// - **Plot area mode** (`is_plot_area_mode=true`): Uses provided dimensions as plot area,
    ///   computes layout to determine canvas size
    /// - **Canvas mode** (`is_plot_area_mode=false`): Uses provided dimensions as canvas,
    ///   computes layout to determine plot area from overflow
    ///
    /// Returns (plot_area_width, plot_area_height, canvas_size, layout, legend_plan)
    async fn compute_layout_and_dimensions(
        &self,
        eval_ctx: &EvaluationContext,
        dimensions: ResolvedLayoutDimensions,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        ctx: &SessionContext,
        merged_params: &IndexMap<String, ScalarValue>,
        data_override: Option<&DataFrame>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        child_frame_sharing_path: &ChildFrameSharingPath,
    ) -> Result<(f32, f32, (f32, f32), LayoutSolution, PreparedLegendPlan), AvengerChartError> {
        if dimensions.dimensions_are_plot_area() {
            // Plot area mode: dimensions specify the plot area size.
            let plot_area_width = dimensions.width_value();
            let plot_area_height = dimensions.height_value();

            let initial_scales = scale_provider
                .build_scales(plot_area_width, plot_area_height, ctx, merged_params)
                .await?;

            let (layout, legend_plan) = Box::pin(self.compute_layout_with_spec(
                eval_ctx,
                layout_spec,
                &initial_scales,
                ctx,
                merged_params,
                data_override,
                facet_tree,
                facet_path,
                child_frame_sharing_path,
            ))
            .await?;

            Ok((
                plot_area_width,
                plot_area_height,
                layout.canvas_size,
                layout,
                legend_plan,
            ))
        } else {
            // Canvas or mixed mode: canvas-sourced dimensions are converted to a
            // plot-area estimate, while plot-area-sourced dimensions are exact.
            let initial_plot_width = if dimensions.width.is_plot_area() {
                dimensions.width_value()
            } else {
                dimensions.width_value() * Self::INITIAL_PLOT_AREA_RATIO
            };
            let initial_plot_height = if dimensions.height.is_plot_area() {
                dimensions.height_value()
            } else {
                dimensions.height_value() * Self::INITIAL_PLOT_AREA_RATIO
            };

            let initial_scales = scale_provider
                .build_scales(initial_plot_width, initial_plot_height, ctx, merged_params)
                .await?;

            let (layout, legend_plan) = Box::pin(self.compute_layout_with_spec(
                eval_ctx,
                layout_spec,
                &initial_scales,
                ctx,
                merged_params,
                data_override,
                facet_tree,
                facet_path,
                child_frame_sharing_path,
            ))
            .await?;

            let plot_bounds = layout.plot_area_bounds();
            Ok((
                if dimensions.width.is_plot_area() {
                    dimensions.width_value()
                } else {
                    plot_bounds.width
                },
                if dimensions.height.is_plot_area() {
                    dimensions.height_value()
                } else {
                    plot_bounds.height
                },
                layout.canvas_size,
                layout,
                legend_plan,
            ))
        }
    }

    /// Measure coordinate system layout (e.g., facet cell positioning).
    ///
    /// This allows coordinate systems to compute layout data that's available
    /// to both guides and marks during rendering.
    ///
    /// Facet coordinate measurement returns chart-owned geometry state; scales
    /// are retargeted only for ordinary plot scale ranges.
    async fn measure_coord_system(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
        params_with_dims: &EvaluationContext,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
        ctx: &SessionContext,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        // Get data for coordinate-system measurement: use data_override if provided (nested facets),
        // otherwise use the plot's own data (top-level).
        let plot_data = if data_override.is_some() {
            None
        } else {
            self.data.as_ref().and_then(|node| {
                node.to_logical_plan(ctx)
                    .ok()
                    .map(|plan| DataFrame::new(ctx.state().clone(), plan))
            })
        };
        let coord_data = data_override.or(plot_data.as_ref());

        Box::pin(measure_coordinate_system_transform(
            self.coord_transform.as_ref(),
            CoordMeasureRequest::new(
                scales,
                plot_area_width,
                plot_area_height,
                params_with_dims,
                coord_data,
                &self.marks,
                facet_path,
            ),
        ))
        .await
    }

    /// Measure plot components without rendering (for layout coordination).
    ///
    /// This method performs all setup and measurement needed for layout coordination:
    /// - Resolves plot area dimensions from layout_spec (see Layout Modes below)
    /// - Builds scales with the provided scale_provider
    /// - Calls coordinate system measure (for facet cell layout)
    /// - Computes overflow requirements for guide elements
    ///
    /// Returns `ComponentsMeasurement` for parent layout coordination and rendering.
    ///
    /// # Layout Modes
    /// The `layout_spec` determines how dimensions are resolved:
    /// - **Canvas mode** (`canvas: Fixed`, `plot_area: Auto`): Plot area is computed
    ///   by subtracting legend/title overflow from canvas dimensions
    /// - **Plot area mode** (`canvas: Auto`, `plot_area: Fixed`): Plot area dimensions
    ///   are used directly; facet subplots always use this mode
    ///
    /// # Arguments
    /// * `eval_ctx` - Evaluation context with theme, session, params, and facet tree
    /// * `layout_spec` - Evaluated layout specification determining canvas vs plot area mode
    /// * `scale_provider` - Provider for building scales (prebuilt scale data or from-data)
    /// * `data_override` - Optional data override for faceted subplots (filtered data)
    /// * `facet_path` - Path of values identifying current cell in facet hierarchy (e.g., `["East", "Eng"]`).
    ///   Used for tree navigation, data filtering, and axis visibility checks.
    pub(crate) async fn measure_plot_components(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &EvaluatedLayoutSpec,
        scale_provider: &dyn ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[ScalarValue],
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        eval_ctx.record_plot_component_measure_call(facet_path.len());

        let ctx = &*eval_ctx.session_context;
        let facet_tree = &*eval_ctx.facet_tree;

        // Phase 1: Extract dimensions from layout spec
        let dimensions = Self::resolve_dimensions_from_spec(layout_spec);

        debug!(
            width = dimensions.width_value(),
            height = dimensions.height_value(),
            width_is_plot_area = dimensions.width.is_plot_area(),
            height_is_plot_area = dimensions.height.is_plot_area(),
            "measure_plot_components dimensions"
        );

        // Add dimensions to params for media queries
        let params_with_dims =
            eval_ctx.with_dimension_params(dimensions.width_value(), dimensions.height_value());
        let merged_params = params_with_dims.params.clone();

        // Phase 2: Compute layout and determine plot area dimensions
        let (plot_area_width, plot_area_height, mut canvas_size, mut layout, mut legend_plan) =
            Box::pin(self.compute_layout_and_dimensions(
                eval_ctx,
                dimensions,
                layout_spec,
                scale_provider,
                ctx,
                &merged_params,
                data_override,
                facet_tree,
                facet_path,
                eval_ctx.child_frame_sharing_path(),
            ))
            .await?;

        // Phase 3: Build final scales with actual plot area dimensions
        let final_scales = scale_provider
            .build_scales(plot_area_width, plot_area_height, ctx, &merged_params)
            .await?;

        // Phase 4: Coordinate system measurement
        let coord_measurement = Box::pin(self.measure_coord_system(
            &final_scales,
            plot_area_width,
            plot_area_height,
            &params_with_dims,
            data_override,
            facet_path,
            ctx,
        ))
        .await?;

        if dimensions.has_plot_area_dimension() {
            let initial_plot_bounds = *layout.plot_area_bounds();
            let initial_overflow = layout.overflow;
            let initial_total_overflow = layout.total_overflow;
            let (_, refined_layout, refined_legend_plan) =
                Box::pin(self.rebuild_layout_with_coord_overflow(
                    eval_ctx,
                    layout_spec,
                    &final_scales,
                    plot_area_width,
                    plot_area_height,
                    &merged_params,
                    data_override,
                    ctx,
                    facet_tree,
                    facet_path,
                    eval_ctx.child_frame_sharing_path(),
                    Some(coord_measurement.as_ref()),
                    GuideOverflowPhase::Measurement,
                ))
                .await?;
            let refined_plot_bounds = refined_layout.plot_area_bounds();
            trace!(
                initial_width = initial_plot_bounds.width,
                initial_height = initial_plot_bounds.height,
                refined_width = refined_plot_bounds.width,
                refined_height = refined_plot_bounds.height,
                delta_w = (refined_plot_bounds.width - initial_plot_bounds.width).abs(),
                delta_h = (refined_plot_bounds.height - initial_plot_bounds.height).abs(),
                initial_overflow_top = initial_overflow.top,
                initial_overflow_right = initial_overflow.right,
                initial_overflow_bottom = initial_overflow.bottom,
                initial_overflow_left = initial_overflow.left,
                refined_overflow_top = refined_layout.overflow.top,
                refined_overflow_right = refined_layout.overflow.right,
                refined_overflow_bottom = refined_layout.overflow.bottom,
                refined_overflow_left = refined_layout.overflow.left,
                initial_total_overflow_top = initial_total_overflow.top,
                initial_total_overflow_right = initial_total_overflow.right,
                initial_total_overflow_bottom = initial_total_overflow.bottom,
                initial_total_overflow_left = initial_total_overflow.left,
                refined_total_overflow_top = refined_layout.total_overflow.top,
                refined_total_overflow_right = refined_layout.total_overflow.right,
                refined_total_overflow_bottom = refined_layout.total_overflow.bottom,
                refined_total_overflow_left = refined_layout.total_overflow.left,
                "plot-area measurement refined with coord-aware overflow"
            );
            layout = refined_layout;
            legend_plan = refined_legend_plan;
            canvas_size = layout.canvas_size;
        }

        // Phase 5: Get clip region from guide
        let clip = self.resolved_clip_region(
            eval_ctx,
            facet_path,
            &final_scales,
            plot_area_width,
            plot_area_height,
        );

        // Note: Overflow info is available via layout.overflow (guide only) and
        // layout.total_overflow (guide + legends), computed during layout phase.
        let frame_allocation = FrameAllocation {
            rect: LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: canvas_size.0,
                height: canvas_size.1,
            },
            sizing: dimensions.frame_sizing_policy(),
            owned_slabs: EdgeSlabs::default(),
        };

        let mut measurement = ComponentsMeasurement {
            coord_measurement,
            scales: final_scales,
            plot_area_width,
            plot_area_height,
            canvas_size,
            clip,
            layout,
            frame_allocation,
            params: merged_params,
            legend_plan,
        };
        crate::facet::coordination_apply::refresh_current_facet_geometry(
            &mut measurement,
            eval_ctx.facet_runtime_sizing_mode(),
        )?;
        if tracing::enabled!(target: "avenger_chart::layout_coordination", tracing::Level::DEBUG) {
            let _ = diagnose_child_frame_layout_alignment(&measurement)?;
        }
        Ok(measurement)
    }

    /// Build plot components with explicit dimensions and scale provider (recursive entry point)
    ///
    /// This method supports both top-level plots and subplots by accepting:
    /// - Explicit dimensions (canvas size or plot area size, controlled by `dimensions_are_plot_area`)
    /// - A scale provider (build new scales or use prebuilt scale data from parent)
    /// - Evaluation mode (measure overflow or full render)
    /// - Optional data override (for faceted subplots)
    ///
    /// When `dimensions_are_plot_area` is false (canvas mode), the dimensions represent the full
    /// canvas and layout is computed to determine the plot area. When true (plot area mode), the
    /// dimensions represent the already-determined plot area size.
    ///
    /// This enables true recursive rendering where the same logic works at all nesting levels.
    /// Build plot components using pre-computed measurement
    ///
    /// This method renders all marks using the provided measurement results.
    /// Call `measure_plot_components()` first to get the measurement.
    ///
    /// # Arguments
    /// * `eval_ctx` - Evaluation context
    /// * `measurement` - Pre-computed measurement from `measure_plot_components()`
    /// * `data_override` - Optional data override for faceted subplots
    /// * `dimensions_are_plot_area` - If true, dimensions are plot area; if false, canvas
    /// * `facet_path` - Current cell path in facet hierarchy as values (for axis visibility).
    ///   Empty slice when not in a facet cell.
    pub async fn build_plot_components(
        &self,
        eval_ctx: &EvaluationContext,
        measurement: &ComponentsMeasurement,
        data_override: Option<&DataFrame>,
        dimensions_are_plot_area: bool,
        facet_path: &[ScalarValue],
    ) -> Result<PlotComponents, AvengerChartError> {
        self.build_plot_components_internal(
            eval_ctx,
            measurement,
            data_override,
            dimensions_are_plot_area,
            facet_path,
            None,
        )
        .await
    }

    pub(crate) async fn build_plot_components_reusing_data_marks(
        &self,
        eval_ctx: &EvaluationContext,
        source_measurement: &ComponentsMeasurement,
        measurement: &ComponentsMeasurement,
        data_override: Option<&DataFrame>,
        dimensions_are_plot_area: bool,
        facet_path: &[ScalarValue],
        cached_components: &PlotComponents,
    ) -> Result<Option<PlotComponents>, AvengerChartError> {
        // Child-frame containers synthesize interaction scopes by rendering
        // their children. Reusing only the cached data marks would preserve the
        // visuals but skip that child traversal, leaving the evaluated plot
        // without coordinate scopes for nested concat/repeat/facet tools.
        if measurement.child_frame_container_view()?.is_some() {
            return Ok(None);
        }
        let Some(data_marks) = retarget_cached_data_marks_for_plot_area(
            cached_components,
            source_measurement,
            measurement,
        ) else {
            return Ok(None);
        };
        self.build_plot_components_internal(
            eval_ctx,
            measurement,
            data_override,
            dimensions_are_plot_area,
            facet_path,
            Some(CachedDataMarks {
                data_marks,
                event_datums: cached_components.event_datums.clone(),
            }),
        )
        .await
        .map(Some)
    }

    pub(crate) fn build_plot_components_reusing_data_marks_and_chrome(
        &self,
        eval_ctx: &EvaluationContext,
        source_measurement: &ComponentsMeasurement,
        measurement: &ComponentsMeasurement,
        dimensions_are_plot_area: bool,
        facet_path: &[ScalarValue],
        cached_components: &PlotComponents,
    ) -> Result<Option<PlotComponents>, AvengerChartError> {
        let build_start = Instant::now();
        if !self.can_reuse_plot_components_data_marks_and_chrome(
            source_measurement,
            measurement,
            dimensions_are_plot_area,
        )? {
            return Ok(None);
        }

        let Some(data_marks) = retarget_cached_data_marks_for_plot_area(
            cached_components,
            source_measurement,
            measurement,
        ) else {
            return Ok(None);
        };
        let event_datums = cached_components.event_datums.clone();

        let local_scope_bounds = LayoutBounds {
            x: 0.0,
            y: 0.0,
            width: measurement.plot_area_width,
            height: measurement.plot_area_height,
        };
        let sharing_owner_paths = self.interaction_sharing_owner_paths(eval_ctx, facet_path);
        let logical_facet_values = self.interaction_logical_facet_values(eval_ctx, facet_path);
        let interaction_scopes = self.build_coordinate_interaction_scopes(
            local_scope_bounds,
            measurement.plot_area_width,
            measurement.plot_area_height,
            &measurement.scales,
            facet_path,
            logical_facet_values,
            eval_ctx.facet_coord_node_path().to_vec(),
            sharing_owner_paths,
        );

        let components = PlotComponents {
            data_marks,
            guide_marks: cached_components.guide_marks.clone(),
            legend_marks: cached_components.legend_marks.clone(),
            title_marks: cached_components.title_marks.clone(),
            subtitle_marks: cached_components.subtitle_marks.clone(),
            plot_bounds: local_scope_bounds,
            clip: measurement.clip.clone(),
            size: measurement.canvas_size,
            size_is_canvas: false,
            debug_marks: cached_components.debug_marks.clone(),
            interaction_scopes,
            event_datums,
            chrome_event_datums: cached_components.chrome_event_datums.clone(),
        };
        eval_ctx.record_build_plot_components_duration(build_start.elapsed());
        Ok(Some(components))
    }

    pub(crate) fn can_reuse_plot_components_data_marks_and_chrome(
        &self,
        source_measurement: &ComponentsMeasurement,
        measurement: &ComponentsMeasurement,
        dimensions_are_plot_area: bool,
    ) -> Result<bool, AvengerChartError> {
        Ok(dimensions_are_plot_area
            && measurement.child_frame_container_view()?.is_none()
            && same_layout_extent(source_measurement, measurement))
    }

    async fn build_plot_components_internal(
        &self,
        eval_ctx: &EvaluationContext,
        measurement: &ComponentsMeasurement,
        data_override: Option<&DataFrame>,
        dimensions_are_plot_area: bool,
        facet_path: &[ScalarValue],
        cached_data_marks: Option<CachedDataMarks>,
    ) -> Result<PlotComponents, AvengerChartError> {
        tracing::debug!(
            target: "avenger_chart::resize",
            facet_depth = facet_path.len(),
            dimensions_are_plot_area,
            "build_plot_components start"
        );
        let build_start = Instant::now();
        let ctx = &*eval_ctx.session_context;

        debug!(
            plot_area_width = measurement.plot_area_width,
            plot_area_height = measurement.plot_area_height,
            dimensions_are_plot_area,
            "build_plot_components start"
        );

        // Render components using measurement
        let layout_solution = measurement.layout.clone();

        // Extract values from measurement for convenience
        let plot_area_width = measurement.plot_area_width;
        let plot_area_height = measurement.plot_area_height;
        let canvas_size = measurement.canvas_size;
        let clip = measurement.clip.clone();
        let merged_params = measurement.params.clone();
        let merged_scales = measurement.scales.clone();
        let legend_plan_initial = measurement.legend_plan.clone();

        // Create context with measurement's params (which include dimensions).
        // Install a fresh interaction-scope sink so child cells (facets, concat)
        // collect their translated scopes here, isolated from any parent sink.
        let interaction_scope_sink: Arc<Mutex<Vec<EvaluatedInteractionScope>>> =
            Arc::new(Mutex::new(Vec::new()));
        let event_datum_sink: Arc<Mutex<Vec<EvaluatedEventDatumRows>>> =
            Arc::new(Mutex::new(Vec::new()));
        let mark_eval_ctx = eval_ctx
            .with_params(merged_params.clone())
            .with_interaction_scope_sink(Some(interaction_scope_sink.clone()))
            .with_event_datum_sink(Some(event_datum_sink.clone()));

        // Render marks using pre-computed measurements from measurement
        let coord_measurement_ref: &dyn CoordMeasurement = measurement.coord_measurement.as_ref();

        let data_marks_start = Instant::now();
        let data_marks_reused = cached_data_marks.is_some();
        let (data_marks, event_datums) = if let Some(cached) = cached_data_marks {
            (cached.data_marks, cached.event_datums)
        } else {
            let mut data_marks = Vec::new();
            let mut event_datums = Vec::new();
            for mark in &self.marks {
                let output = Box::pin(self.render_mark_with_plot_df(
                    mark.as_ref(),
                    &mark_eval_ctx,
                    &merged_scales,
                    plot_area_width,
                    plot_area_height,
                    data_override,
                    facet_path,
                    coord_measurement_ref,
                ))
                .await?;
                let start_index = data_marks.len();
                event_datums.extend(offset_flat_event_datum_rows(
                    output.event_datums,
                    start_index,
                ));
                let pushed_event_datums: Vec<_> = event_datum_sink
                    .lock()
                    .expect("event datum sink poisoned")
                    .drain(..)
                    .collect();
                event_datums.extend(offset_flat_event_datum_rows(
                    pushed_event_datums,
                    start_index,
                ));
                data_marks.extend(output.marks);
            }
            let leftover_event_datums: Vec<_> = event_datum_sink
                .lock()
                .expect("event datum sink poisoned")
                .drain(..)
                .collect();
            event_datums.extend(leftover_event_datums);
            (data_marks, event_datums)
        };
        let data_marks_elapsed = data_marks_start.elapsed();

        // Create guide marks and other components
        let chrome_marks_start = Instant::now();
        let (
            plot_bounds_struct,
            guide_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
            debug_marks,
            chrome_event_datums,
            legend_interaction_scopes,
        ) = {
            let layout_initial = layout_solution;
            // Check if this is a top-level plot (canvas mode) or subplot (plot area mode)
            if !dimensions_are_plot_area {
                // Canvas mode: measurement already carries the finalized layout/overflow.
                let plot_bounds = layout_initial.plot_area_bounds();
                let plot_bounds_struct = LayoutBounds {
                    x: plot_bounds.x,
                    y: plot_bounds.y,
                    width: plot_area_width,
                    height: plot_area_height,
                };

                let guide_marks = self
                    .create_guide_marks(
                        &merged_scales,
                        plot_area_width,
                        plot_area_height,
                        &plot_bounds_struct,
                        &layout_initial.overflow,
                        &merged_params,
                        ctx,
                        data_override,
                        eval_ctx.facet_tree.as_ref(),
                        facet_path,
                        eval_ctx.child_frame_sharing_path(),
                        coord_measurement_ref,
                    )
                    .await?;
                let mut chrome_event_datums =
                    crate::parallel_guide_event::parallel_guide_event_datums(
                        self.compiled_guide.as_ref(),
                        &guide_marks,
                        plot_area_width,
                        &merged_params,
                        ctx,
                    )?;

                let rendered_legends = self
                    .render_legends_from_plan(
                        &eval_ctx,
                        &legend_plan_initial,
                        &layout_initial.frame_layout,
                        ctx,
                        &merged_params,
                    )
                    .await?;
                let legend_event_datums = offset_flat_event_datum_rows(
                    rendered_legends.event_datums,
                    1 + guide_marks.len(),
                );
                chrome_event_datums.extend(legend_event_datums);
                let legend_marks = rendered_legends.marks;
                let legend_interaction_scopes = rendered_legends.interaction_scopes;

                let title_marks = if let Some(title_bounds) = &layout_initial.frame_layout.title {
                    self.create_title(Some(*title_bounds), ctx, &merged_params)
                        .await?
                } else {
                    Vec::new()
                };

                let subtitle_marks =
                    if let Some(subtitle_bounds) = &layout_initial.frame_layout.subtitle {
                        self.create_subtitle(Some(*subtitle_bounds), ctx, &merged_params)
                            .await?
                    } else {
                        Vec::new()
                    };

                let mut debug_marks = vec![];
                let debug_overlay = eval_ctx.debug_layout_overlay();
                if debug_overlay.enabled() {
                    debug_marks.extend(Self::debug_layout_marks_for_measurement(
                        measurement,
                        [0.0, 0.0],
                        None,
                        200,
                        false,
                        debug_overlay,
                    )?);
                }

                (
                    plot_bounds_struct,
                    guide_marks,
                    legend_marks,
                    title_marks,
                    subtitle_marks,
                    debug_marks,
                    chrome_event_datums,
                    legend_interaction_scopes,
                )
            } else {
                // Plot area mode (subplots): plot_bounds.y = 0
                // For nested facets, the FacetColGuide computes adjusted_plot_bounds.y as
                // a NEGATIVE value, placing labels ABOVE the subplot origin (in the overflow
                // region). Data marks render at y = 0 to plot_height.
                // The subplot's overflow region is at negative y, not positive.
                let plot_bounds_struct = LayoutBounds {
                    x: 0.0,
                    y: 0.0,
                    width: plot_area_width,
                    height: plot_area_height,
                };

                // Create guide marks
                // Pass data_override so nested facets use filtered data
                let guide_marks = self
                    .create_guide_marks(
                        &merged_scales,
                        plot_area_width,
                        plot_area_height,
                        &plot_bounds_struct,
                        &layout_initial.overflow,
                        &merged_params,
                        ctx,
                        data_override,
                        eval_ctx.facet_tree.as_ref(),
                        facet_path,
                        eval_ctx.child_frame_sharing_path(),
                        coord_measurement_ref,
                    )
                    .await?;
                let mut chrome_event_datums =
                    crate::parallel_guide_event::parallel_guide_event_datums(
                        self.compiled_guide.as_ref(),
                        &guide_marks,
                        plot_area_width,
                        &merged_params,
                        ctx,
                    )?;

                // Create legend marks from the computed layout
                // Legend positions from layout include the plot area offset, but we need them at (0,0)
                let plot_bounds = layout_initial.plot_area_bounds();
                let rendered_legends = self
                    .render_legends_from_plan(
                        &eval_ctx,
                        &legend_plan_initial,
                        &layout_initial.frame_layout,
                        ctx,
                        &merged_params,
                    )
                    .await?;
                let legend_event_datums = offset_flat_event_datum_rows(
                    rendered_legends.event_datums,
                    1 + guide_marks.len(),
                );
                chrome_event_datums.extend(legend_event_datums);
                let legend_interaction_scopes = rendered_legends.interaction_scopes;

                // Translate frame chrome to be relative to the child plot-area origin.
                let legend_marks: Vec<_> = rendered_legends
                    .marks
                    .into_iter()
                    .map(|mark| translate_scene_mark(mark, -plot_bounds.x, -plot_bounds.y))
                    .collect();

                let title_marks_raw = if let Some(title_bounds) = &layout_initial.frame_layout.title
                {
                    self.create_title(Some(*title_bounds), ctx, &merged_params)
                        .await?
                } else {
                    Vec::new()
                };
                let title_marks = title_marks_raw
                    .into_iter()
                    .map(|mark| translate_scene_mark(mark, -plot_bounds.x, -plot_bounds.y))
                    .collect();

                let subtitle_marks_raw =
                    if let Some(subtitle_bounds) = &layout_initial.frame_layout.subtitle {
                        self.create_subtitle(Some(*subtitle_bounds), ctx, &merged_params)
                            .await?
                    } else {
                        Vec::new()
                    };
                let subtitle_marks = subtitle_marks_raw
                    .into_iter()
                    .map(|mark| translate_scene_mark(mark, -plot_bounds.x, -plot_bounds.y))
                    .collect();

                let mut debug_marks = vec![];
                let debug_overlay = eval_ctx.debug_layout_overlay();
                if debug_overlay.enabled() {
                    let plot_bounds = layout_initial.plot_area_bounds();
                    let origin = [-plot_bounds.x, -plot_bounds.y];
                    let subplot_color_string =
                        facet_debug_layout_color(eval_ctx.facet_coord_node_path());
                    let flip_label_align =
                        facet_debug_layout_flip_label_align(eval_ctx.facet_coord_node_path());

                    debug_marks.extend(Self::debug_layout_marks_for_measurement(
                        measurement,
                        origin,
                        Some(subplot_color_string),
                        100,
                        flip_label_align,
                        debug_overlay,
                    )?);
                }

                (
                    plot_bounds_struct,
                    guide_marks,
                    legend_marks,
                    title_marks,
                    subtitle_marks,
                    debug_marks,
                    chrome_event_datums,
                    legend_interaction_scopes,
                )
            }
        };
        let chrome_marks_elapsed = chrome_marks_start.elapsed();

        // Debug marks are kept separate - they're in absolute canvas coordinates
        // and should not be translated with the data marks group
        let data_mark_count = data_marks.len();
        let guide_mark_count = guide_marks.len();
        let legend_mark_count = legend_marks.len();
        let title_mark_count = title_marks.len();
        let subtitle_mark_count = subtitle_marks.len();
        let debug_mark_count = debug_marks.len();

        // Collect interaction scopes for this plot:
        // 1. child cell scopes pushed into the sink by facet rendering (already
        //    translated into this plot's data-marks-local space), plus
        // 2. this plot's own coordinate scope when its coordinate system can
        //    invert pointer positions.
        //
        // All scope bounds are in data-marks-local space; the consumer (the
        // root `components_to_evaluated_plot`, or the parent facet render for a
        // nested container) translates them by the origin it applies to the
        // data-marks group.
        let mut interaction_scopes = interaction_scope_sink
            .lock()
            .expect("interaction scope sink poisoned")
            .drain(..)
            .collect::<Vec<_>>();
        interaction_scopes.extend(legend_interaction_scopes);
        let local_scope_bounds = LayoutBounds {
            x: 0.0,
            y: 0.0,
            width: plot_area_width,
            height: plot_area_height,
        };
        let sharing_owner_paths = self.interaction_sharing_owner_paths(eval_ctx, facet_path);
        let logical_facet_values = self.interaction_logical_facet_values(eval_ctx, facet_path);
        interaction_scopes.extend(self.build_coordinate_interaction_scopes(
            local_scope_bounds,
            plot_area_width,
            plot_area_height,
            &merged_scales,
            facet_path,
            logical_facet_values,
            eval_ctx.facet_coord_node_path().to_vec(),
            sharing_owner_paths,
        ));

        let components = PlotComponents {
            data_marks,
            guide_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
            plot_bounds: plot_bounds_struct,
            clip,
            size: canvas_size,
            size_is_canvas: !dimensions_are_plot_area,
            debug_marks,
            interaction_scopes,
            event_datums,
            chrome_event_datums,
        };
        let build_elapsed = build_start.elapsed();
        eval_ctx.record_build_plot_components_duration(build_elapsed);
        tracing::debug!(
            target: "avenger_chart::resize",
            build_plot_components_ms = build_elapsed.as_secs_f64() * 1000.0,
            facet_depth = facet_path.len(),
            "build_plot_components"
        );
        tracing::debug!(
            target: "avenger_chart::resize",
            facet_depth = facet_path.len(),
            data_marks_ms = data_marks_elapsed.as_secs_f64() * 1000.0,
            chrome_marks_ms = chrome_marks_elapsed.as_secs_f64() * 1000.0,
            data_mark_count,
            guide_mark_count,
            legend_mark_count,
            title_mark_count,
            subtitle_mark_count,
            debug_mark_count,
            data_marks_reused,
            "build_plot_components phases"
        );
        Ok(components)
    }

    /// CoordinationScope-owner paths for an interaction scope at `facet_path`.
    ///
    /// Provides an owner path for every logical sharing level the cell could use
    /// (0 = Free .. logical depth = fully shared), so an event-binding assignment
    /// can resolve the owner path for whatever sharing level its param declares.
    /// The root scope (empty facet path) returns an empty map, which resolves
    /// every level to the root owner path `[]`.
    fn interaction_sharing_owner_paths(
        &self,
        eval_ctx: &EvaluationContext,
        facet_path: &[ScalarValue],
    ) -> HashMap<u8, Vec<ScalarValue>> {
        let mut owner_paths = HashMap::new();
        if facet_path.is_empty() {
            return owner_paths;
        }
        let facet_tree = eval_ctx.facet_tree.as_ref();
        let logical_depth = facet_tree.logical_depth_for_path(facet_path);
        for level in 0..=logical_depth.min(u8::MAX as usize) {
            let level_u8 = level as u8;
            owner_paths.insert(
                level_u8,
                facet_tree.sharing_owner_path(facet_path, level_u8),
            );
        }
        owner_paths
    }

    fn interaction_logical_facet_values(
        &self,
        eval_ctx: &EvaluationContext,
        facet_path: &[ScalarValue],
    ) -> Vec<ScalarValue> {
        if facet_path.is_empty() {
            Vec::new()
        } else {
            eval_ctx.facet_tree.logical_values_for_path(facet_path)
        }
    }

    /// Build coordinate interaction scopes for this plot's coordinate system.
    ///
    /// Returns an empty vector unless the coordinate transform declares
    /// interaction-invertible channels and a configured scale exists for every
    /// requested channel. `bounds` are in this plot's data-marks-local scene
    /// coordinates; the consumer translates them by the data-marks group origin.
    #[allow(clippy::too_many_arguments)]
    fn build_coordinate_interaction_scopes(
        &self,
        bounds: LayoutBounds,
        plot_area_width: f32,
        plot_area_height: f32,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        facet_path: &[ScalarValue],
        logical_facet_values: Vec<ScalarValue>,
        coord_node_path: Vec<usize>,
        sharing_owner_paths: HashMap<u8, Vec<ScalarValue>>,
    ) -> Vec<EvaluatedInteractionScope> {
        let channels = self.coord_transform.interaction_invertible_channels();
        if channels.is_empty() {
            return Vec::new();
        }
        let mut channel_scales = HashMap::new();
        for (scale_name, scale) in scales {
            let coord = self
                .scale_to_coord_channel
                .get(scale_name)
                .map(String::as_str)
                .unwrap_or_else(|| avenger_chart_core::channel::strip_trailing_numbers(scale_name));
            if channels.iter().any(|channel| channel == coord) {
                channel_scales.insert(coord.to_string(), scale.configured().clone());
            }
            if channels.iter().any(|channel| channel == scale_name) {
                channel_scales.insert(scale_name.clone(), scale.configured().clone());
            }
        }
        // Only export a scope when every scale-backed interaction channel has
        // a configured scale. Coordinate-local pixel channels can be inverted
        // without scale state.
        if !channels.iter().all(|ch| {
            !self.coord_transform.channel_uses_scale(ch) || channel_scales.contains_key(ch)
        }) {
            return Vec::new();
        }
        let scope_id = interaction_scope_content_id(&coord_node_path, &logical_facet_values);
        vec![EvaluatedInteractionScope {
            id: InteractionScopeId(0),
            kind: InteractionScopeKind::Coordinate,
            scope_id,
            bounds,
            plot_area_width,
            plot_area_height,
            facet_path: facet_path.to_vec(),
            logical_facet_values,
            coord_node_path,
            subplot_id_path: Vec::new(),
            child_frame_path: Vec::new(),
            coord_transform: self.coord_transform.clone(),
            channels,
            scales: channel_scales,
            sharing_owner_paths,
        }]
    }

    pub(crate) fn components_to_evaluated_plot(
        &self,
        eval_ctx: &EvaluationContext,
        components: PlotComponents,
        build_scene_rtree: bool,
    ) -> EvaluatedPlot {
        let _span = tracing::debug_span!("components_to_evaluated_plot").entered();
        let convert_start = Instant::now();
        let plot_bounds = components.plot_bounds;
        let (final_width, final_height) = components.size;

        // Scopes are collected in data-marks-local space; translate them by the
        // data-marks group origin so their bounds are in final scene coordinates,
        // then assign stable ids.
        let mut interaction_scopes = components.interaction_scopes;
        for (index, scope) in interaction_scopes.iter_mut().enumerate() {
            scope.bounds.x += plot_bounds.x;
            scope.bounds.y += plot_bounds.y;
            scope.id = InteractionScopeId(index);
        }
        let interaction = EvaluatedInteractionState {
            scopes: interaction_scopes,
        };

        let data_marks_group = SceneGroup {
            origin: [plot_bounds.x, plot_bounds.y],
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };

        let mut all_marks = Vec::new();

        let theme = self.get_theme();
        let canvas_ctx = ThemeContext::new("canvas", eval_ctx.params.clone());
        if let Some(color) = theme
            .query(&canvas_ctx, "background-color")
            .and_then(|v| v.as_color_array())
        {
            let background_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(final_width.into()),
                height: Some(final_height.into()),
                fill: ColorOrGradient::Color(color).into(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke_width: 0.0.into(),
                zindex: Some(-100),
                ..Default::default()
            };
            all_marks.push(SceneMark::Rect(background_rect));
        }

        let data_group_index = all_marks.len();
        all_marks.push(SceneMark::Group(data_marks_group));
        all_marks.extend(components.guide_marks);
        all_marks.extend(components.legend_marks);
        all_marks.extend(components.title_marks);
        all_marks.extend(components.subtitle_marks);
        all_marks.extend(components.debug_marks);

        let root_group = SceneGroup {
            marks: all_marks,
            ..Default::default()
        };

        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(root_group)],
            width: final_width,
            height: final_height,
            origin: [0.0, 0.0],
        };

        let rtree = build_scene_rtree.then(|| SceneGraphRTree::from_scene_graph(&scene_graph));
        let mut event_datum_rows =
            prefix_event_datum_rows(components.event_datums, &[0, data_group_index]);
        event_datum_rows.extend(prefix_event_datum_rows(
            offset_flat_event_datum_rows(components.chrome_event_datums, data_group_index),
            &[0],
        ));
        let event_datums = EvaluatedEventDatumState {
            rows: event_datum_rows,
        };

        let evaluated = EvaluatedPlot {
            scene_graph,
            rtree,
            interaction,
            event_datums,
        };
        let convert_elapsed = convert_start.elapsed();
        eval_ctx.record_components_to_evaluated_plot_duration(convert_elapsed);
        tracing::debug!(
            target: "avenger_chart::resize",
            components_to_evaluated_plot_ms = convert_elapsed.as_secs_f64() * 1000.0,
            scene_width = final_width,
            scene_height = final_height,
            "components_to_evaluated_plot"
        );
        evaluated
    }

    fn select_facet_subtree_by_facet_path<'a>(
        &'a self,
        measurement: &'a ComponentsMeasurement,
        data_override: Option<&'a DataFrame>,
        remaining_path: &[ScalarValue],
        current_path: Vec<ScalarValue>,
        dimensions_are_plot_area: bool,
    ) -> Result<SelectedFacetSubtree<'a>, AvengerChartError> {
        if remaining_path.is_empty() {
            return Ok(SelectedFacetSubtree {
                plot: self,
                measurement,
                data_override,
                facet_path: current_path,
                dimensions_are_plot_area,
            });
        }

        let Some((compiled_subplot, cells)) = facet_band_children(measurement) else {
            return Err(AvengerChartError::InternalError(format!(
                "Facet subtree path {:?} continues through a non-facet measurement",
                remaining_path
            )));
        };

        let target = &remaining_path[0];
        let cell = cells
            .iter()
            .find(|cell| &cell.plan.value == target)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Facet subtree path value {:?} not found",
                    target
                ))
            })?;

        compiled_subplot.select_facet_subtree_by_facet_path(
            &cell.measurement,
            Some(&cell.data_override),
            &remaining_path[1..],
            cell.plan.full_path.clone(),
            true,
        )
    }

    fn select_facet_subtree_by_coord_node_path<'a>(
        &'a self,
        measurement: &'a ComponentsMeasurement,
        data_override: Option<&'a DataFrame>,
        remaining_path: &[usize],
        current_path: Vec<ScalarValue>,
        dimensions_are_plot_area: bool,
    ) -> Result<SelectedFacetSubtree<'a>, AvengerChartError> {
        if remaining_path.is_empty() {
            return Ok(SelectedFacetSubtree {
                plot: self,
                measurement,
                data_override,
                facet_path: current_path,
                dimensions_are_plot_area,
            });
        }

        let Some((compiled_subplot, cells)) = facet_band_children(measurement) else {
            return Err(AvengerChartError::InternalError(format!(
                "Facet subtree coord-node path {:?} continues through a non-facet measurement",
                remaining_path
            )));
        };

        let child_index = remaining_path[0];
        let cell = cells.get(child_index).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Facet subtree coord-node child index {} out of range",
                child_index
            ))
        })?;

        compiled_subplot.select_facet_subtree_by_coord_node_path(
            &cell.measurement,
            Some(&cell.data_override),
            &remaining_path[1..],
            cell.plan.full_path.clone(),
            true,
        )
    }

    async fn render_facet_subtree_snapshot(
        &self,
        eval_ctx: &EvaluationContext,
        measurement: &ComponentsMeasurement,
        snapshot: &FacetSubtreeSnapshot,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        let selected = match &snapshot.selector {
            FacetSubtreeSelector::ByFacetPath(path) => {
                self.select_facet_subtree_by_facet_path(measurement, None, path, Vec::new(), false)?
            }
            FacetSubtreeSelector::ByCoordinationNodePath(path) => self
                .select_facet_subtree_by_coord_node_path(
                    measurement,
                    None,
                    path,
                    Vec::new(),
                    false,
                )?,
        };

        let components = selected
            .plot
            .build_plot_components(
                eval_ctx,
                selected.measurement,
                selected.data_override,
                selected.dimensions_are_plot_area,
                &selected.facet_path,
            )
            .await?;
        let evaluated = selected
            .plot
            .components_to_evaluated_plot(eval_ctx, components, true);
        Ok(Self::pad_facet_subtree_snapshot(evaluated))
    }

    fn pad_facet_subtree_snapshot(evaluated: EvaluatedPlot) -> EvaluatedPlot {
        let mut interaction = evaluated.interaction;
        let event_datums = EvaluatedEventDatumState {
            rows: prefix_event_datum_rows(evaluated.event_datums.rows, &[0, 0]),
        };
        let original_scene = evaluated.scene_graph;
        let original_width = original_scene.width.max(1.0);
        let original_height = original_scene.height.max(1.0);
        let envelope = evaluated
            .rtree
            .as_ref()
            .map(|rtree| *rtree.envelope())
            .unwrap_or_else(|| *SceneGraphRTree::from_scene_graph(&original_scene).envelope());

        let min_x = envelope.lower()[0].min(0.0);
        let min_y = envelope.lower()[1].min(0.0);
        let max_x = envelope.upper()[0].max(original_width);
        let max_y = envelope.upper()[1].max(original_height);
        let padding = Self::FACET_SUBTREE_SNAPSHOT_PADDING;
        let shift_x = padding - min_x;
        let shift_y = padding - min_y;
        let final_width = (max_x - min_x + 2.0 * padding).ceil().max(1.0);
        let final_height = (max_y - min_y + 2.0 * padding).ceil().max(1.0);

        let content_group = SceneGroup {
            name: "facet_subtree_snapshot_content".to_string(),
            origin: [shift_x, shift_y],
            marks: original_scene.marks,
            ..Default::default()
        };

        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(SceneGroup {
                marks: vec![SceneMark::Group(content_group)],
                ..Default::default()
            })],
            width: final_width,
            height: final_height,
            origin: [0.0, 0.0],
        };
        let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);
        // Shift interaction scope bounds to match the repositioned snapshot content.
        for scope in interaction.scopes.iter_mut() {
            scope.bounds.x += shift_x;
            scope.bounds.y += shift_y;
        }
        EvaluatedPlot {
            scene_graph,
            rtree: Some(rtree),
            interaction,
            event_datums,
        }
    }

    async fn apply_layout_snapshot(
        &self,
        snapshot: &LayoutSnapshot,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        provider: &dyn ScaleProvider,
        resolved_chart_sizing: ResolvedChartSizing,
    ) -> Result<(), AvengerChartError> {
        match snapshot {
            LayoutSnapshot::Final => {
                Box::pin(self.apply_final_layout_snapshot(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    provider,
                    resolved_chart_sizing,
                ))
                .await
            }
            LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured) => {
                if matches!(resolved_chart_sizing, ResolvedChartSizing::FacetBand(_)) {
                    Box::pin(self.refresh_facet_snapshot_layout_from_coord_overflow(
                        measurement,
                        eval_ctx,
                        evaluated_layout_spec,
                        resolved_chart_sizing,
                        GuideOverflowPhase::Measurement,
                    ))
                    .await
                } else {
                    Ok(())
                }
            }
            LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(checkpoint)) => {
                if matches!(resolved_chart_sizing, ResolvedChartSizing::FacetBand(_)) {
                    coordinate_overflow_for_guides_until(measurement, eval_ctx, *checkpoint)
                        .await?;
                    Box::pin(self.refresh_facet_snapshot_layout_from_coord_overflow(
                        measurement,
                        eval_ctx,
                        evaluated_layout_spec,
                        resolved_chart_sizing,
                        GuideOverflowPhase::Final,
                    ))
                    .await
                } else {
                    Ok(())
                }
            }
            LayoutSnapshot::Whole(WholeChartSnapshot::Refinement {
                iteration,
                checkpoint,
            }) => {
                if matches!(resolved_chart_sizing, ResolvedChartSizing::FacetBand(_)) {
                    coordinate_overflow_for_guides_until(
                        measurement,
                        eval_ctx,
                        CoordinationCheckpoint::Adopted,
                    )
                    .await?;
                }
                Box::pin(self.apply_refinement_snapshot(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    provider,
                    resolved_chart_sizing,
                    *iteration,
                    *checkpoint,
                ))
                .await
            }
            LayoutSnapshot::FacetSubtree(_) => Ok(()),
        }
    }

    fn validate_root_content_layout(
        measurement: &ComponentsMeasurement,
    ) -> Result<(), AvengerChartError> {
        let content_layout = measurement.content_layout()?;
        if content_layout.allocation.content_rect != *measurement.layout.plot_area_bounds() {
            return Err(AvengerChartError::InternalError(
                "root content rect diverged from the measured plot area".to_string(),
            ));
        }
        if let Some(container) = measurement.child_frame_container_view()? {
            let expected_count = container.child_regions().len();
            if content_layout.child_frame_allocations.len() != expected_count {
                return Err(AvengerChartError::InternalError(format!(
                    "root child-frame allocation count mismatch: expected {}, got {}",
                    expected_count,
                    content_layout.child_frame_allocations.len()
                )));
            }
        } else if !content_layout.child_frame_allocations.is_empty() {
            return Err(AvengerChartError::InternalError(
                "root content layout unexpectedly produced child frame allocations without a child-frame container"
                    .to_string(),
            ));
        }
        Ok(())
    }

    async fn apply_final_layout_snapshot(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        provider: &dyn ScaleProvider,
        resolved_chart_sizing: ResolvedChartSizing,
    ) -> Result<(), AvengerChartError> {
        match resolved_chart_sizing {
            ResolvedChartSizing::SinglePlot => {
                let dimensions = Self::resolve_dimensions_from_spec(evaluated_layout_spec);
                if !dimensions.dimensions_are_plot_area() {
                    let refinement = eval_ctx.facet_layout_refinement();
                    Box::pin(self.refine_canvas_measurement_after_coordination(
                        measurement,
                        eval_ctx,
                        evaluated_layout_spec,
                        provider,
                        None,
                        &[],
                        refinement.max_refinement_passes,
                    ))
                    .await?;
                }
                Self::validate_root_content_layout(measurement)?;
            }
            ResolvedChartSizing::FacetBand(_) => {
                // Facet content uses the global coordination cycle:
                // snapshot -> solve -> install channels -> adopt geometry.
                coordinate_overflow_for_guides(measurement, eval_ctx).await?;
                let refinement = eval_ctx.facet_layout_refinement();
                Box::pin(self.realize_policy_layout_after_coordination(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    None,
                    &[],
                    refinement.max_refinement_passes,
                ))
                .await?;
            }
        }

        let alignment_trace = apply_child_frame_layout_alignment(measurement)?;
        if alignment_trace.changed() {
            Box::pin(self.refresh_final_layout_bottom_up(
                measurement,
                eval_ctx,
                evaluated_layout_spec,
                None,
                &[],
            ))
            .await?;
            Self::validate_root_content_layout(measurement)?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_refinement_snapshot(
        &self,
        measurement: &mut ComponentsMeasurement,
        eval_ctx: &EvaluationContext,
        evaluated_layout_spec: &EvaluatedLayoutSpec,
        provider: &dyn ScaleProvider,
        resolved_chart_sizing: ResolvedChartSizing,
        iteration: usize,
        checkpoint: RefinementCheckpoint,
    ) -> Result<(), AvengerChartError> {
        let refinement = eval_ctx.facet_layout_refinement();
        match resolved_chart_sizing {
            ResolvedChartSizing::SinglePlot => {
                let dimensions = Self::resolve_dimensions_from_spec(evaluated_layout_spec);
                if dimensions.dimensions_are_plot_area() {
                    return Err(AvengerChartError::InternalError(
                        "Refinement snapshots are not available for non-faceted plot-area layouts"
                            .to_string(),
                    ));
                }
                Box::pin(self.refine_canvas_measurement_after_coordination_until(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    provider,
                    None,
                    &[],
                    refinement.max_refinement_passes,
                    iteration,
                    checkpoint,
                ))
                .await
            }
            ResolvedChartSizing::FacetBand(_) => {
                Box::pin(self.refine_policy_measurement_after_coordination_until(
                    measurement,
                    eval_ctx,
                    evaluated_layout_spec,
                    None,
                    &[],
                    refinement.max_refinement_passes,
                    iteration,
                    checkpoint,
                ))
                .await
            }
        }
    }

    /// Evaluate the plot to a scene graph
    pub async fn evaluate(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        Box::pin(self.evaluate_with_options(ctx, params, EvaluationOptions::default())).await
    }

    /// Evaluate the plot to a scene graph with explicit layout snapshot and debug options.
    pub async fn evaluate_with_options(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<EvaluatedPlot, AvengerChartError> {
        let (evaluated, _) =
            Box::pin(self.evaluate_with_options_and_metrics(ctx, params, options)).await?;
        Ok(evaluated)
    }

    /// Evaluate the plot while collecting focused performance diagnostics.
    #[doc(hidden)]
    pub async fn evaluate_with_options_and_metrics(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
    ) -> Result<(EvaluatedPlot, EvaluationMetrics), AvengerChartError> {
        let (
            scale_domain_cache,
            facet_semantic_cache,
            facet_scale_builder_precompute_cache,
            guide_overflow_cache,
            legend_measurement_cache,
            text_measurement_cache,
        ) = new_plot_session_cache_handles();
        let (evaluated, metrics, _) = Box::pin(
            self.evaluate_with_options_and_metrics_with_scale_domain_cache(
                ctx,
                params,
                options,
                scale_domain_cache,
                facet_semantic_cache,
                facet_scale_builder_precompute_cache,
                Some(guide_overflow_cache),
                Some(legend_measurement_cache),
                Some(text_measurement_cache),
                None,
                None,
                Some(Arc::new(ScopedStoreState::new(self.store_specs.clone()))),
            ),
        )
        .await?;
        Ok((evaluated, metrics))
    }

    pub(crate) async fn evaluate_with_options_and_metrics_with_scale_domain_cache(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
        scale_domain_cache: ScaleDomainCacheHandle,
        facet_semantic_cache: FacetSemanticCacheHandle,
        facet_scale_builder_precompute_cache: FacetScaleBuilderPrecomputeCacheHandle,
        guide_overflow_cache: Option<GuideOverflowCacheHandle>,
        legend_measurement_cache: Option<LegendMeasurementCacheHandle>,
        text_measurement_cache: Option<TextMeasurementCacheHandle>,
        scoped_param_store: Option<Arc<ScopedParamStore>>,
        scoped_selection_store: Option<Arc<ScopedSelectionStore>>,
        scoped_store_state: Option<Arc<ScopedStoreState>>,
    ) -> Result<
        (
            EvaluatedPlot,
            EvaluationMetrics,
            Option<LayoutProfileSnapshot>,
        ),
        AvengerChartError,
    > {
        let metrics = Arc::new(Mutex::new(EvaluationMetrics::default()));
        let outcome = Box::pin(self.evaluate_with_options_internal(
            ctx,
            params,
            options,
            Some(metrics.clone()),
            Some(scale_domain_cache),
            Some(facet_semantic_cache),
            Some(facet_scale_builder_precompute_cache),
            guide_overflow_cache,
            legend_measurement_cache,
            text_measurement_cache,
            None,
            scoped_param_store,
            scoped_selection_store,
            scoped_store_state,
        ))
        .await?;
        let metrics = metrics
            .lock()
            .expect("evaluation metrics lock poisoned")
            .clone();
        Ok((outcome.evaluated, metrics, outcome.layout_profile))
    }

    async fn evaluate_with_options_internal(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
        evaluation_metrics: Option<Arc<Mutex<EvaluationMetrics>>>,
        scale_domain_cache: Option<ScaleDomainCacheHandle>,
        facet_semantic_cache: Option<FacetSemanticCacheHandle>,
        facet_scale_builder_precompute_cache: Option<FacetScaleBuilderPrecomputeCacheHandle>,
        guide_overflow_cache: Option<GuideOverflowCacheHandle>,
        legend_measurement_cache: Option<LegendMeasurementCacheHandle>,
        text_measurement_cache: Option<TextMeasurementCacheHandle>,
        layout_profile: Option<Arc<LayoutProfileSnapshot>>,
        scoped_param_store: Option<Arc<ScopedParamStore>>,
        scoped_selection_store: Option<Arc<ScopedSelectionStore>>,
        scoped_store_state: Option<Arc<ScopedStoreState>>,
    ) -> Result<EvaluationOutcome, AvengerChartError> {
        // Merge provided params with defaults
        let merged_params = if let Some(provided) = params {
            let mut merged = self.default_params.clone();
            merged.extend(provided);
            merged
        } else {
            self.default_params.clone()
        };

        // Evaluate layout spec to get concrete dimensions
        let evaluated_layout_spec = evaluate_layout_spec(
            &self.layout_spec,
            ctx,
            &merged_params,
            self.get_theme().as_ref(),
        )
        .await?;
        let resolved_chart_sizing = self.resolve_chart_sizing(&evaluated_layout_spec)?;

        // Build evaluated facet tree (pre-pass to discover partition structure).
        // Responsive wrap columns need the evaluated sizing policy, so this runs
        // after layout expressions are resolved but before measurement.
        let facet_tree_start = Instant::now();
        Self::record_evaluation_metric(&evaluation_metrics, |metrics| {
            metrics.record_facet_tree_build();
        });
        let wrap_layout_context =
            Self::facet_wrap_layout_context(&evaluated_layout_spec, resolved_chart_sizing);
        let facet_tree = if let Some(cache) = &facet_semantic_cache {
            let mut slot_cache = cache
                .lock()
                .expect("facet semantic cache lock poisoned")
                .clone();
            let before = slot_cache.stats();
            let tree =
                EvaluatedFacetTree::from_compiled_plot_with_params_wrap_layout_context_and_slot_cache(
                    self,
                    ctx,
                    &merged_params,
                    wrap_layout_context,
                    &mut slot_cache,
                )
                .await?;
            let after = slot_cache.stats();
            Self::record_evaluation_metric(&evaluation_metrics, |metrics| {
                metrics.record_facet_semantic_cache_hits(after.hits.saturating_sub(before.hits));
                metrics
                    .record_facet_semantic_cache_misses(after.misses.saturating_sub(before.misses));
            });
            cache
                .lock()
                .expect("facet semantic cache lock poisoned")
                .merge_from(slot_cache);
            Arc::new(tree)
        } else {
            Arc::new(
                EvaluatedFacetTree::from_compiled_plot_with_params_and_wrap_layout_context(
                    self,
                    ctx,
                    &merged_params,
                    wrap_layout_context,
                )
                .await?,
            )
        };
        debug!(
            elapsed_ms = facet_tree_start.elapsed().as_secs_f64() * 1000.0,
            depth = facet_tree.depth(),
            "evaluated facet tree construction completed"
        );
        let facet_scale_builder_precompute_store = facet_scale_builder_precompute_cache
            .as_ref()
            .filter(|_| facet_tree.depth() > 0)
            .map(|cache| {
                let (store, hit) = self.facet_scale_builder_precompute_store_from_session_cache(
                    cache,
                    ctx,
                    &merged_params,
                    facet_tree.as_ref(),
                );
                Self::record_evaluation_metric(&evaluation_metrics, |metrics| {
                    if hit {
                        metrics.record_facet_scale_builder_precompute_cache_hit();
                    } else {
                        metrics.record_facet_scale_builder_precompute_cache_miss();
                    }
                });
                store
            });

        let measured_layout_spec = Self::layout_spec_for_resolved_chart_sizing(
            &evaluated_layout_spec,
            facet_tree.as_ref(),
            resolved_chart_sizing,
        );

        let mut scale_eval_ctx = avenger_chart_core::EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            merged_params.clone(),
        )
        .with_time_context(self.time_context.clone());
        if let Some(metrics) = &evaluation_metrics {
            scale_eval_ctx = scale_eval_ctx
                .with_diagnostics(Arc::new(EvaluationMetricsDiagnostics::new(metrics.clone())));
        }
        let scale_builder = if let Some(scale_domain_cache) = &scale_domain_cache {
            let cache_key = self.top_level_scale_domain_cache_key(ctx, &merged_params);
            let cached_builder = {
                scale_domain_cache
                    .lock()
                    .expect("scale-domain cache lock poisoned")
                    .get(&cache_key)
            };
            if let Some(builder) = cached_builder {
                Self::record_evaluation_metric(&evaluation_metrics, |metrics| {
                    metrics.record_scale_domain_cache_hit();
                });
                builder
            } else {
                Self::record_evaluation_metric(&evaluation_metrics, |metrics| {
                    metrics.record_scale_domain_cache_miss();
                    metrics.record_scale_builder_build();
                });
                let builder = Box::pin(build_scale_builder_from_compiled_plot(
                    self,
                    None,
                    &scale_eval_ctx,
                    self.get_theme().as_ref(),
                ))
                .await?;
                scale_domain_cache
                    .lock()
                    .expect("scale-domain cache lock poisoned")
                    .insert(cache_key, builder)
            }
        } else {
            Self::record_evaluation_metric(&evaluation_metrics, |metrics| {
                metrics.record_scale_builder_build();
            });
            Arc::new(
                Box::pin(build_scale_builder_from_compiled_plot(
                    self,
                    None,
                    &scale_eval_ctx,
                    self.get_theme().as_ref(),
                ))
                .await?,
            )
        };

        let provider = DynamicScaleProvider {
            builder: scale_builder.as_ref(),
            plot: self,
        };

        let inherited_facet_cell_profiles = layout_profile
            .as_ref()
            .map(|profile| profile.facet_cell_profiles.clone());
        let facet_cell_rendered_components_capture =
            Some(Arc::new(Mutex::new(FacetCellProfileIndex::default())));

        // Create EvaluationContext for the entire evaluation
        let mut eval_ctx = EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            merged_params,
            facet_tree.clone(),
        )
        .with_time_context(self.time_context.clone())
        .with_event_datum_fields(Arc::new(self.event_datum_types()))
        .with_facet_data_root(dataframe_from_compiled_plot_data(&self.data, ctx)?)
        .with_facet_runtime_sizing_mode(resolved_chart_sizing.facet_runtime_sizing_mode())
        .with_facet_layout_refinement(options.facet_layout_refinement)
        .with_debug_layout_overlay(facet_debug::resolve_layout_overlay_mode(
            options.debug_layout_overlay,
        ));
        if let Some(capture) = &facet_cell_rendered_components_capture {
            eval_ctx = eval_ctx.with_facet_cell_rendered_components_capture(capture.clone());
        }
        if let LayoutSnapshot::FacetSubtree(snapshot) = &options.layout_snapshot
            && snapshot.checkpoint == FacetSubtreeCheckpoint::EstimatedOverflowProbe
        {
            eval_ctx = eval_ctx.with_facet_subtree_snapshot_capture(Arc::new(Mutex::new(
                FacetSubtreeSnapshotCapture {
                    request: snapshot.clone(),
                    result: None,
                },
            )));
        }
        if let Some(cache) = &scale_domain_cache {
            eval_ctx = eval_ctx.with_scale_domain_cache(cache.clone());
        }
        if let Some(cache) = &guide_overflow_cache {
            eval_ctx = eval_ctx.with_guide_overflow_cache(cache.clone());
        }
        if let Some(cache) = &legend_measurement_cache {
            eval_ctx = eval_ctx.with_legend_measurement_cache(cache.clone());
        }
        if let Some(cache) = &text_measurement_cache {
            eval_ctx = eval_ctx.with_text_measurement_cache(cache.clone());
        }
        if let Some(profile) = layout_profile {
            eval_ctx = eval_ctx.with_layout_profile(profile);
        }
        if let Some(store) = facet_scale_builder_precompute_store {
            eval_ctx = eval_ctx.with_facet_scale_builder_precompute_store(store);
        }
        if let Some(store) = scoped_param_store {
            eval_ctx = eval_ctx.with_scoped_param_store(store);
        }
        let selection_revision_fingerprint =
            selection_revision_fingerprint(scoped_selection_store.as_deref());
        let store_revision_fingerprint = store_revision_fingerprint(scoped_store_state.as_deref());
        if let Some(store) = &scoped_selection_store {
            eval_ctx = eval_ctx.with_scoped_selection_store(store.clone());
        }
        if let Some(store) = &scoped_store_state {
            eval_ctx = eval_ctx.with_scoped_store_state(store.clone());
        }
        if let Some(metrics) = evaluation_metrics {
            eval_ctx = eval_ctx.with_evaluation_metrics(metrics);
        }

        if Self::marks_use_auto_empty_cell_policy(&self.marks) {
            trace!("Facet empty-cell policy `auto` resolved to `hole` for this evaluation");
        }

        // Measure plot components
        let measurement = Box::pin(self.measure_plot_components(
            &eval_ctx,
            &measured_layout_spec,
            &provider,
            None, // No data override for top-level plots
            &[],  // Empty facet path for top-level plots
        ))
        .await?;

        let mut measurement = measurement;
        if let LayoutSnapshot::FacetSubtree(snapshot) = &options.layout_snapshot {
            match snapshot.checkpoint {
                FacetSubtreeCheckpoint::EstimatedOverflowProbe => {
                    let evaluated = eval_ctx.take_facet_subtree_snapshot().ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "Requested estimated overflow probe snapshot was not captured: {:?}",
                            snapshot
                        ))
                    })?;
                    return Ok(EvaluationOutcome {
                        evaluated: Self::pad_facet_subtree_snapshot(evaluated),
                        layout_profile: None,
                    });
                }
                FacetSubtreeCheckpoint::LocalRetargetedLayout => {}
                FacetSubtreeCheckpoint::CoordinatedLayout => {
                    Box::pin(self.apply_layout_snapshot(
                        &LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                            CoordinationCheckpoint::Adopted,
                        )),
                        &mut measurement,
                        &eval_ctx,
                        &measured_layout_spec,
                        &provider,
                        resolved_chart_sizing,
                    ))
                    .await?;
                }
                FacetSubtreeCheckpoint::FinalLayout => {
                    Box::pin(self.apply_layout_snapshot(
                        &LayoutSnapshot::Final,
                        &mut measurement,
                        &eval_ctx,
                        &measured_layout_spec,
                        &provider,
                        resolved_chart_sizing,
                    ))
                    .await?;
                }
            }
            let evaluated = self
                .render_facet_subtree_snapshot(&eval_ctx, &measurement, snapshot)
                .await?;
            return Ok(EvaluationOutcome {
                evaluated,
                layout_profile: None,
            });
        }

        Box::pin(self.apply_layout_snapshot(
            &options.layout_snapshot,
            &mut measurement,
            &eval_ctx,
            &measured_layout_spec,
            &provider,
            resolved_chart_sizing,
        ))
        .await?;

        let content_layout = measurement.content_layout()?;
        trace!(
            content_kind = ?resolved_chart_sizing.content_kind(),
            child_frame_allocation_count = content_layout.child_frame_allocations.len(),
            content_x = content_layout.allocation.content_rect.x,
            content_y = content_layout.allocation.content_rect.y,
            content_width = content_layout.allocation.content_rect.width,
            content_height = content_layout.allocation.content_rect.height,
            "resolved content layout"
        );

        // Build plot components using measurement
        let components = Box::pin(self.build_plot_components(
            &eval_ctx,
            &measurement,
            None,  // No data override for top-level plots
            false, // Canvas mode: dimensions are canvas size
            &[],   // Empty path for top-level plots (not in a facet cell)
        ))
        .await?;

        let rendered_components = if measurement.child_frame_container_view()?.is_none() {
            Some(components.clone())
        } else {
            None
        };
        let evaluated =
            self.components_to_evaluated_plot(&eval_ctx, components, options.build_scene_rtree);
        let captured_facet_cell_profiles = facet_cell_rendered_components_capture
            .as_ref()
            .map(|capture| {
                capture
                    .lock()
                    .expect("facet cell rendered components profile lock poisoned")
                    .clone()
            })
            .unwrap_or_default();
        let facet_cell_profiles = if captured_facet_cell_profiles.is_empty() {
            inherited_facet_cell_profiles.unwrap_or_default()
        } else {
            captured_facet_cell_profiles
        };
        let layout_profile = LayoutProfileSnapshot::new_with_components(
            self,
            measurement,
            Some(facet_tree.clone()),
            ctx,
            &eval_ctx.params,
            selection_revision_fingerprint,
            store_revision_fingerprint,
            rendered_components,
            facet_cell_profiles,
        );
        Ok(EvaluationOutcome {
            evaluated,
            layout_profile: Some(layout_profile),
        })
    }

    #[allow(clippy::too_many_arguments)]
    /// Re-apply active raw-domain overrides to faceted cell scales during Preview.
    ///
    /// Preview clones the last measurement and refreshes only the top-level
    /// scales. For faceted charts the per-cell scales live inside the coord
    /// measurement and otherwise keep their pre-pan domains. This walks the facet
    /// band tree, evaluates each band's leaf raw-domain expressions with the
    /// current params, and applies any non-null resolved domain to every cell's
    /// matching scale (ranges are left untouched). Only scales that declare a
    /// raw_domain are touched, so non-pan faceted charts are unaffected.
    async fn apply_preview_facet_domain_overrides(
        measurement: &mut ComponentsMeasurement,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        tree: &EvaluatedFacetTree,
        store: Option<&Arc<ScopedParamStore>>,
    ) -> Result<(), AvengerChartError> {
        let Some(band) = facet_band_mut(measurement.coord_measurement.as_mut()) else {
            return Ok(());
        };
        let subplot = band.compiled_subplot.clone();
        // Shared fast path: when no per-cell (`Free`/`Level`) assignment exists,
        // resolve the raw-domain overrides once from the global params and reuse
        // them for every cell. This preserves the original single-eval behavior.
        let per_cell = matches!(store, Some(store) if store.has_scoped_overrides());
        let shared_overrides = if per_cell {
            None
        } else {
            Some(resolve_raw_domain_overrides(subplot.as_ref(), ctx, params).await?)
        };
        for cell in &mut band.cells {
            // Per-cell path resolves this cell owner's effective params (via its
            // absolute `full_path`) and evaluates the raw-domain against those.
            let per_cell_overrides;
            let overrides: &HashMap<String, (f32, f32)> = if let Some(shared) = &shared_overrides {
                shared
            } else {
                let store = store.expect("per-cell override path implies a scoped store");
                let cell_params = store.effective_params_for_cell(tree, &cell.plan.full_path);
                per_cell_overrides =
                    resolve_raw_domain_overrides(subplot.as_ref(), ctx, &cell_params).await?;
                &per_cell_overrides
            };
            if !overrides.is_empty() {
                apply_domain_overrides_to_scales(&mut cell.measurement.scales, overrides);
            }
            Box::pin(Self::apply_preview_facet_domain_overrides(
                &mut cell.measurement,
                ctx,
                params,
                tree,
                store,
            ))
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn evaluate_preview_with_layout_profile_and_metrics(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, ScalarValue>>,
        options: EvaluationOptions,
        layout_profile: &LayoutProfileSnapshot,
        scale_domain_cache: ScaleDomainCacheHandle,
        facet_semantic_cache: FacetSemanticCacheHandle,
        facet_scale_builder_precompute_cache: FacetScaleBuilderPrecomputeCacheHandle,
        guide_overflow_cache: Option<GuideOverflowCacheHandle>,
        legend_measurement_cache: Option<LegendMeasurementCacheHandle>,
        text_measurement_cache: Option<TextMeasurementCacheHandle>,
        scoped_param_store: Option<Arc<ScopedParamStore>>,
        scoped_selection_store: Option<Arc<ScopedSelectionStore>>,
        scoped_store_state: Option<Arc<ScopedStoreState>>,
    ) -> Result<PreviewLayoutProfileAttempt, AvengerChartError> {
        tracing::debug!(target: "avenger_chart::resize", "plot_session.preview_attempt start");
        if options.layout_snapshot != LayoutSnapshot::Final {
            return Ok(PreviewLayoutProfileAttempt::fallback(
                PreviewProfileFallbackReason::NonFinalSnapshot,
            ));
        }

        let metrics = Arc::new(Mutex::new(EvaluationMetrics::default()));
        let layout_setup_start = Instant::now();
        let current_selection_revision_fingerprint =
            selection_revision_fingerprint(scoped_selection_store.as_deref());
        let current_store_revision_fingerprint =
            store_revision_fingerprint(scoped_store_state.as_deref());
        let merged_params = if let Some(provided) = params {
            let mut merged = self.default_params.clone();
            merged.extend(provided);
            merged
        } else {
            self.default_params.clone()
        };

        let evaluated_layout_spec = evaluate_layout_spec(
            &self.layout_spec,
            ctx,
            &merged_params,
            self.get_theme().as_ref(),
        )
        .await?;
        let resolved_chart_sizing = self.resolve_chart_sizing(&evaluated_layout_spec)?;

        let wrap_layout_context =
            Self::facet_wrap_layout_context(&evaluated_layout_spec, resolved_chart_sizing);
        let mut profile_comparison_params = merged_params.clone();
        for dimension_param in ["width", "height"] {
            if !profile_comparison_params.contains_key(dimension_param)
                && let Some(previous_value) = layout_profile.measurement.params.get(dimension_param)
            {
                profile_comparison_params
                    .insert(dimension_param.to_string(), previous_value.clone());
            }
        }
        let changed_params = changed_param_names(
            &layout_profile.measurement.params,
            &profile_comparison_params,
        );
        let layout_size_params = layout_size_dependency_params(self, ctx, &merged_params);
        let changed_params_are_layout_size_only = changed_params
            .iter()
            .all(|name| layout_size_params.contains(name));
        let changed_params_touch_layout_size = changed_params
            .iter()
            .any(|name| layout_size_params.contains(name));
        let profile_dependencies_match =
            layout_profile.profile_dependencies_match(&profile_comparison_params);
        let can_reuse_profile_facet_tree = scoped_param_store.is_none()
            && !changed_params_touch_layout_size
            && profile_dependencies_match;
        Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
            metrics.record_preview_layout_setup_duration(layout_setup_start.elapsed());
        });
        let facet_tree = if can_reuse_profile_facet_tree {
            if let Some(facet_tree) = layout_profile.facet_tree.clone() {
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics.record_facet_tree_profile_reuse();
                });
                facet_tree
            } else {
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics.record_facet_tree_build();
                });
                let mut slot_cache = facet_semantic_cache
                    .lock()
                    .expect("facet semantic cache lock poisoned")
                    .clone();
                let before = slot_cache.stats();
                let tree = EvaluatedFacetTree::from_compiled_plot_with_params_wrap_layout_context_and_slot_cache(
                    self,
                    ctx,
                    &merged_params,
                    wrap_layout_context,
                    &mut slot_cache,
                )
                .await?;
                let after = slot_cache.stats();
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics
                        .record_facet_semantic_cache_hits(after.hits.saturating_sub(before.hits));
                    metrics.record_facet_semantic_cache_misses(
                        after.misses.saturating_sub(before.misses),
                    );
                });
                facet_semantic_cache
                    .lock()
                    .expect("facet semantic cache lock poisoned")
                    .merge_from(slot_cache);
                Arc::new(tree)
            }
        } else {
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_facet_tree_build();
            });
            let mut slot_cache = facet_semantic_cache
                .lock()
                .expect("facet semantic cache lock poisoned")
                .clone();
            let before = slot_cache.stats();
            let tree = EvaluatedFacetTree::from_compiled_plot_with_params_wrap_layout_context_and_slot_cache(
                self,
                ctx,
                &merged_params,
                wrap_layout_context,
                &mut slot_cache,
            )
            .await?;
            let after = slot_cache.stats();
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_facet_semantic_cache_hits(after.hits.saturating_sub(before.hits));
                metrics
                    .record_facet_semantic_cache_misses(after.misses.saturating_sub(before.misses));
            });
            facet_semantic_cache
                .lock()
                .expect("facet semantic cache lock poisoned")
                .merge_from(slot_cache);
            Arc::new(tree)
        };

        if !layout_profile.physical_structure_matches(facet_tree.as_ref()) {
            if facet_tree.has_wrap_levels()
                && layout_profile.logical_structure_matches(facet_tree.as_ref())
            {
                if layout_profile.facet_cell_profile_count() == 0 {
                    Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                        metrics.record_preview_structure_reflow_miss();
                    });
                    return Ok(PreviewLayoutProfileAttempt::fallback(
                        PreviewProfileFallbackReason::MissingTerminalProfile,
                    ));
                }
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics.record_preview_structure_reflow_reuse();
                });
                let reflow_start = Instant::now();
                let mut reflow_options = options.clone();
                reflow_options.facet_layout_refinement.max_refinement_passes = reflow_options
                    .facet_layout_refinement
                    .max_refinement_passes
                    .max(crate::render::FacetLayoutRefinement::default().max_refinement_passes);
                let outcome = Box::pin(self.evaluate_with_options_internal(
                    ctx,
                    Some(merged_params),
                    reflow_options,
                    Some(metrics.clone()),
                    Some(scale_domain_cache),
                    Some(facet_semantic_cache),
                    Some(facet_scale_builder_precompute_cache),
                    guide_overflow_cache,
                    legend_measurement_cache,
                    text_measurement_cache,
                    Some(Arc::new(layout_profile.clone())),
                    scoped_param_store.clone(),
                    scoped_selection_store.clone(),
                    scoped_store_state.clone(),
                ))
                .await?;
                let reflow_elapsed = reflow_start.elapsed();
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics.record_preview_structure_reflow_duration(reflow_elapsed);
                });
                tracing::debug!(
                    target: "avenger_chart::resize",
                    reflow_ms = reflow_elapsed.as_secs_f64() * 1000.0,
                    "facet_wrap.structure_reflow"
                );
                let skipped_cell_measurements = metrics
                    .lock()
                    .expect("evaluation metrics lock poisoned")
                    .pipeline
                    .facet_cell_measurement_profile_reuses;
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics.record_preview_profile_reuse();
                    metrics.record_skipped_component_measure_calls(skipped_cell_measurements);
                });
                let metrics = metrics
                    .lock()
                    .expect("evaluation metrics lock poisoned")
                    .clone();
                return Ok(PreviewLayoutProfileAttempt::reused(
                    outcome.evaluated,
                    metrics,
                    outcome.layout_profile,
                ));
            }
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_preview_structure_reflow_miss();
            });
            let reason = if facet_tree.has_wrap_levels() {
                PreviewProfileFallbackReason::LogicalStructureMismatch
            } else {
                PreviewProfileFallbackReason::PhysicalStructureMismatch
            };
            return Ok(PreviewLayoutProfileAttempt::fallback(reason));
        }

        if changed_params_touch_layout_size && plot_contains_responsive_wrap_concat(self) {
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_preview_structure_reflow_miss();
            });
            return Ok(PreviewLayoutProfileAttempt::fallback(
                PreviewProfileFallbackReason::PhysicalStructureMismatch,
            ));
        }

        let measured_layout_spec = Self::layout_spec_for_resolved_chart_sizing(
            &evaluated_layout_spec,
            facet_tree.as_ref(),
            resolved_chart_sizing,
        );
        let dimensions = Self::resolve_dimensions_from_spec(&measured_layout_spec);
        let scale_domain_params = merged_params.clone();
        let mut eval_ctx = EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            merged_params,
            facet_tree.clone(),
        )
        .with_time_context(self.time_context.clone())
        .with_event_datum_fields(Arc::new(self.event_datum_types()))
        .with_facet_data_root(dataframe_from_compiled_plot_data(&self.data, ctx)?)
        .with_facet_runtime_sizing_mode(resolved_chart_sizing.facet_runtime_sizing_mode())
        .with_facet_layout_refinement(options.facet_layout_refinement)
        .with_debug_layout_overlay(facet_debug::resolve_layout_overlay_mode(
            options.debug_layout_overlay,
        ))
        .with_scale_domain_cache(scale_domain_cache.clone())
        .with_evaluation_metrics(metrics.clone());
        let facet_cell_rendered_components_capture =
            Arc::new(Mutex::new(FacetCellProfileIndex::default()));
        eval_ctx = eval_ctx
            .with_layout_profile(Arc::new(layout_profile.clone()))
            .with_facet_cell_rendered_components_capture(
                facet_cell_rendered_components_capture.clone(),
            );
        if let Some(store) = scoped_param_store {
            eval_ctx = eval_ctx.with_scoped_param_store(store);
        }
        if let Some(store) = &scoped_selection_store {
            eval_ctx = eval_ctx.with_scoped_selection_store(store.clone());
        }
        if let Some(store) = &scoped_store_state {
            eval_ctx = eval_ctx.with_scoped_store_state(store.clone());
        }

        let measurement_clone_start = Instant::now();
        let mut measurement = layout_profile.measurement.clone();
        Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
            metrics.record_preview_measurement_clone_duration(measurement_clone_start.elapsed());
        });
        let old_canvas_size = measurement.canvas_size;
        // Plot-area-owned dimensions in a faceted layout spec are nominal
        // content-size constraints. If the constraint itself is unchanged,
        // preserve the measured profile extent because coordination/refinement
        // can make it differ from the nominal estimate.
        let target_plot_area_width = if dimensions.width.is_plot_area() {
            match measurement.frame_allocation.sizing.width {
                FrameDimensionSizing::ContentSized { content_size }
                    if (content_size - dimensions.width_value()).abs() <= 0.01 =>
                {
                    measurement.plot_area_width
                }
                _ => dimensions.width_value(),
            }
        } else {
            measurement.plot_area_width + dimensions.width_value() - old_canvas_size.0
        }
        .max(1.0);
        let target_plot_area_height = if dimensions.height.is_plot_area() {
            match measurement.frame_allocation.sizing.height {
                FrameDimensionSizing::ContentSized { content_size }
                    if (content_size - dimensions.height_value()).abs() <= 0.01 =>
                {
                    measurement.plot_area_height
                }
                _ => dimensions.height_value(),
            }
        } else {
            measurement.plot_area_height + dimensions.height_value() - old_canvas_size.1
        }
        .max(1.0);

        eval_ctx =
            eval_ctx.with_dimension_params(dimensions.width_value(), dimensions.height_value());
        if matches!(resolved_chart_sizing, ResolvedChartSizing::FacetBand(_)) {
            if !dimensions.width.is_plot_area() {
                measurement.canvas_size.0 = dimensions.width_value().max(1.0);
            }
            if !dimensions.height.is_plot_area() {
                measurement.canvas_size.1 = dimensions.height_value().max(1.0);
            }
            measurement.layout.canvas_size = measurement.canvas_size;
            measurement.refresh_frame_allocation_rect();
        }
        measurement.params = eval_ctx.params.clone();

        let root_raw_domain_overrides = if has_raw_domain_scale(self) {
            resolve_raw_domain_overrides(self, ctx, &eval_ctx.params).await?
        } else {
            HashMap::new()
        };
        let has_active_root_raw_domain_overrides = !root_raw_domain_overrides.is_empty();
        let can_reuse_single_plot_raw_domain_scales =
            matches!(resolved_chart_sizing, ResolvedChartSizing::SinglePlot)
                && !changed_params_touch_layout_size
                && has_active_root_raw_domain_overrides;
        let can_reuse_root_facet_scales =
            matches!(resolved_chart_sizing, ResolvedChartSizing::FacetBand(_))
                && can_reuse_profile_facet_tree
                && !changed_params_touch_layout_size
                && (!has_raw_domain_scale(self) || has_active_root_raw_domain_overrides);
        if can_reuse_single_plot_raw_domain_scales {
            apply_domain_overrides_to_scales(&mut measurement.scales, &root_raw_domain_overrides);
        } else if !can_reuse_root_facet_scales {
            let scale_refresh_start = Instant::now();
            let scale_context_setup_start = Instant::now();
            let mut scale_eval_ctx = avenger_chart_core::EvaluationContext::new(
                self.get_theme(),
                Arc::new(ctx.clone()),
                scale_domain_params.clone(),
            )
            .with_time_context(self.time_context.clone())
            .with_diagnostics(Arc::new(EvaluationMetricsDiagnostics::new(metrics.clone())));
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_preview_scale_context_setup_duration(
                    scale_context_setup_start.elapsed(),
                );
            });
            let scale_cache_key_start = Instant::now();
            let cache_key = self.top_level_scale_domain_cache_key(ctx, &scale_domain_params);
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_preview_scale_cache_key_duration(scale_cache_key_start.elapsed());
            });
            let scale_cache_lookup_start = Instant::now();
            let cached_builder = {
                scale_domain_cache
                    .lock()
                    .expect("scale-domain cache lock poisoned")
                    .get(&cache_key)
            };
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics
                    .record_preview_scale_cache_lookup_duration(scale_cache_lookup_start.elapsed());
            });
            let scale_builder = if let Some(builder) = cached_builder {
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics.record_scale_domain_cache_hit();
                });
                builder
            } else {
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics.record_scale_domain_cache_miss();
                    metrics.record_scale_builder_build();
                });
                let builder = Box::pin(build_scale_builder_from_compiled_plot(
                    self,
                    None,
                    &scale_eval_ctx,
                    self.get_theme().as_ref(),
                ))
                .await?;
                scale_domain_cache
                    .lock()
                    .expect("scale-domain cache lock poisoned")
                    .insert(cache_key, builder)
            };
            scale_eval_ctx = scale_eval_ctx.with_params(eval_ctx.params.clone());
            let scale_provider = DynamicScaleProvider {
                builder: scale_builder.as_ref(),
                plot: self,
            };
            let scale_build_start = Instant::now();
            let refreshed_scales = scale_provider
                .build_scales(
                    target_plot_area_width,
                    target_plot_area_height,
                    ctx,
                    &scale_eval_ctx.params,
                )
                .await?;
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_preview_scale_build_duration(scale_build_start.elapsed());
            });
            measurement.scales = refreshed_scales;
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_preview_scale_refresh_duration(scale_refresh_start.elapsed());
            });
        }

        let measurement_retarget_start = Instant::now();
        match resolved_chart_sizing {
            ResolvedChartSizing::SinglePlot => {
                retarget_measurement_plot_area_no_remeasure(
                    &mut measurement,
                    self,
                    &eval_ctx,
                    &[],
                    target_plot_area_width,
                    target_plot_area_height,
                )?;
            }
            ResolvedChartSizing::FacetBand(_) => {
                retarget_measurement_plot_area_policy_no_remeasure(
                    &mut measurement,
                    self,
                    &eval_ctx,
                    &[],
                    target_plot_area_width,
                    target_plot_area_height,
                )?;
            }
        }
        Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
            metrics
                .record_preview_measurement_retarget_duration(measurement_retarget_start.elapsed());
        });
        measurement.params = eval_ctx.params.clone();

        // The clone-and-retarget path above refreshes only the top-level scales.
        // Re-apply active raw-domain overrides to faceted cell scales so a pan
        // updates per-cell domains live in Preview (no-op for non-pan facets).
        // For Free/Level pans, each cell resolves its owner's param via the
        // scoped store + facet tree; for Shared (or no store) it reuses the
        // single global resolution.
        let facet_domain_override_start = Instant::now();
        Self::apply_preview_facet_domain_overrides(
            &mut measurement,
            ctx,
            &eval_ctx.params,
            eval_ctx.facet_tree.as_ref(),
            eval_ctx.scoped_param_store.as_ref(),
        )
        .await?;
        Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
            metrics.record_preview_facet_domain_override_duration(
                facet_domain_override_start.elapsed(),
            );
        });

        Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
            metrics.record_preview_profile_reuse();
            metrics.record_skipped_component_measure_calls(1);
        });

        // The profile dependency key intentionally excludes `raw_domain`
        // placeholders. If those are the only non-size dependencies that changed,
        // the cached data marks can be moved by affine scale adjustments instead
        // of rebuilding mark data.
        let changed_params_are_profile_retarget_only =
            !changed_params_touch_layout_size && profile_dependencies_match;
        let selection_revisions_match =
            layout_profile.selection_revision_fingerprint == current_selection_revision_fingerprint;
        let store_revisions_match =
            layout_profile.store_revision_fingerprint == current_store_revision_fingerprint;
        let can_reuse_top_level_data_marks = measurement.child_frame_container_view()?.is_none()
            && selection_revisions_match
            && store_revisions_match
            && (changed_params_are_layout_size_only || changed_params_are_profile_retarget_only);
        let components = if can_reuse_top_level_data_marks {
            if let Some(cached_components) = &layout_profile.rendered_components {
                match Box::pin(self.build_plot_components_reusing_data_marks(
                    &eval_ctx,
                    &layout_profile.measurement,
                    &measurement,
                    None,
                    false,
                    &[],
                    cached_components,
                ))
                .await?
                {
                    Some(components) => {
                        Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                            metrics.record_preview_data_mark_reuse();
                        });
                        components
                    }
                    None => {
                        Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                            metrics.record_preview_data_mark_reuse_miss();
                        });
                        Box::pin(self.build_plot_components(
                            &eval_ctx,
                            &measurement,
                            None,
                            false,
                            &[],
                        ))
                        .await?
                    }
                }
            } else {
                Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                    metrics.record_preview_data_mark_reuse_miss();
                });
                Box::pin(self.build_plot_components(&eval_ctx, &measurement, None, false, &[]))
                    .await?
            }
        } else {
            debug!(
                ?changed_params,
                ?layout_size_params,
                "preview data-mark reuse skipped because changed params require data rebuild or the plot has child frames"
            );
            Self::record_evaluation_metric(&Some(metrics.clone()), |metrics| {
                metrics.record_preview_data_mark_reuse_miss();
            });
            Box::pin(self.build_plot_components(&eval_ctx, &measurement, None, false, &[])).await?
        };
        let facet_cell_profiles = {
            let captured = facet_cell_rendered_components_capture
                .lock()
                .expect("facet cell rendered components profile lock poisoned")
                .clone();
            if captured.is_empty() {
                layout_profile.facet_cell_profiles.clone()
            } else {
                captured
            }
        };
        let rendered_components = if measurement.child_frame_container_view()?.is_none() {
            Some(components.clone())
        } else {
            None
        };
        let preview_layout_profile = if can_reuse_profile_facet_tree {
            None
        } else {
            Some(LayoutProfileSnapshot::new_with_components(
                self,
                measurement.clone(),
                Some(eval_ctx.facet_tree.clone()),
                ctx,
                &eval_ctx.params,
                current_selection_revision_fingerprint,
                current_store_revision_fingerprint,
                rendered_components,
                facet_cell_profiles,
            ))
        };
        let evaluated =
            self.components_to_evaluated_plot(&eval_ctx, components, options.build_scene_rtree);
        let metrics = metrics
            .lock()
            .expect("evaluation metrics lock poisoned")
            .clone();
        Ok(PreviewLayoutProfileAttempt::reused(
            evaluated,
            metrics,
            preview_layout_profile,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coords::CoordinatedOverflow,
        facet::{
            coord::{FacetBandCoordMeasurement, facet_band_ref as facet_band_ref_from_coord},
            coordination::coordinate_facet_measurement_tree,
        },
        layout::{CanvasConstraint, FrameDimensionSizing, PlotConstraint},
        prelude::*,
        render::FacetLayoutRefinement,
    };
    use avenger_chart_polar::PolarSubplotPositionChannels;
    use datafusion::{
        arrow::{
            array::{Float64Array, Int64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        dataframe::DataFrame,
        functions_aggregate::average::avg,
        prelude::SessionContext,
    };

    fn facet_band_ref(measurement: &ComponentsMeasurement) -> Option<&FacetBandCoordMeasurement> {
        facet_band_ref_from_coord(measurement.coord_measurement.as_ref())
    }

    fn fully_leaf_sizing_policy(
        leaf_plot_width: f32,
        leaf_plot_height: f32,
    ) -> ResolvedChartSizing {
        ResolvedChartSizing::FacetBand(FacetRuntimeSizingPolicy::fully_leaf_plot_area_sized(
            leaf_plot_width,
            leaf_plot_height,
        ))
    }

    #[test]
    fn event_datum_rows_for_rendered_marks_gathers_source_indices() -> Result<(), AvengerChartError>
    {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "group",
                DataType::Utf8,
                false,
            )])),
            vec![Arc::new(StringArray::from(vec!["A", "B", "C"]))],
        )?;

        let rows = event_datum_rows_for_rendered_marks(
            Some(&batch),
            2,
            Some(vec![vec![0, 2], vec![1]]),
            None,
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].mark_path, vec![0]);
        assert_eq!(rows[1].mark_path, vec![1]);

        let first = rows[0]
            .rows
            .column_by_name("group")
            .expect("group column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("string group");
        assert_eq!(first.value(0), "A");
        assert_eq!(first.value(1), "C");

        let second = rows[1]
            .rows
            .column_by_name("group")
            .expect("group column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("string group");
        assert_eq!(second.value(0), "B");
        Ok(())
    }

    #[test]
    fn event_datum_rows_for_rendered_marks_merges_generated_rows() -> Result<(), AvengerChartError>
    {
        let source = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "group",
                DataType::Utf8,
                false,
            )])),
            vec![Arc::new(StringArray::from(vec!["A", "B", "C"]))],
        )?;
        let generated = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "dimension_index",
                DataType::Int64,
                false,
            )])),
            vec![Arc::new(Int64Array::from(vec![7, 7]))],
        )?;

        let rows = event_datum_rows_for_rendered_marks(
            Some(&source),
            1,
            Some(vec![vec![2, 0]]),
            Some(vec![generated]),
        )?;
        assert_eq!(rows.len(), 1);

        let groups = rows[0]
            .rows
            .column_by_name("group")
            .expect("group column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("string group");
        assert_eq!(groups.value(0), "C");
        assert_eq!(groups.value(1), "A");

        let dimension_indices = rows[0]
            .rows
            .column_by_name("dimension_index")
            .expect("dimension index column")
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("int64 dimension index");
        assert_eq!(dimension_indices.value(0), 7);
        assert_eq!(dimension_indices.value(1), 7);
        Ok(())
    }

    #[test]
    fn padding_feedback_increased_tracks_growth_only() {
        let mut previous = HashMap::new();
        previous.insert(
            vec![0, 1],
            FacetBandPaddingFeedback {
                padding_inner_px: 40.0,
                guide_padding_inner_px: 20.0,
            },
        );

        let mut unchanged_or_smaller = HashMap::new();
        unchanged_or_smaller.insert(
            vec![0, 1],
            FacetBandPaddingFeedback {
                padding_inner_px: 39.0,
                guide_padding_inner_px: 20.25,
            },
        );
        assert!(!CompiledPlot::padding_feedback_increased(
            &previous,
            &unchanged_or_smaller,
            0.5
        ));

        let mut grown = unchanged_or_smaller;
        grown.insert(
            vec![0, 1],
            FacetBandPaddingFeedback {
                padding_inner_px: 40.6,
                guide_padding_inner_px: 20.25,
            },
        );
        assert!(CompiledPlot::padding_feedback_increased(
            &previous, &grown, 0.5
        ));
    }

    fn deeply_nested_dataframe(ctx: &SessionContext) -> DataFrame {
        let outer_groups = StringArray::from(vec![
            "G1", "G1", "G1", "G1", // S1: A,B; S2: B,C
            "G2", "G2", "G2", "G2", // S1: C,D; S2: A,D
        ]);
        let sub_groups = StringArray::from(vec![
            "S1", "S1", "S2", "S2", // G1
            "S1", "S1", "S2", "S2", // G2
        ]);
        let categories = StringArray::from(vec![
            "A", "B", "B", "C", // G1 (S1: A,B; S2: B,C)
            "C", "D", "A", "D", // G2 (S1: C,D; S2: A,D)
        ]);
        let values = Float64Array::from(vec![
            10.0, 20.0, 15.0, 25.0, // G1
            30.0, 40.0, 35.0, 45.0, // G2
        ]);

        let schema = Arc::new(Schema::new(vec![
            Field::new("outer_group", DataType::Utf8, false),
            Field::new("sub_group", DataType::Utf8, false),
            Field::new("category", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(outer_groups),
                Arc::new(sub_groups),
                Arc::new(categories),
                Arc::new(values),
            ],
        )
        .expect("create deeply nested categorical sharing batch");

        ctx.read_batch(batch)
            .expect("read deeply nested test batch")
    }

    fn sparse_fixed_column_hole_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_from_iter(vec![
            (
                "division",
                Arc::new(StringArray::from(vec!["Eng", "Eng", "Ops", "Ops"]))
                    as Arc<dyn arrow::array::Array>,
            ),
            (
                "department",
                Arc::new(StringArray::from(vec![
                    "Frontend", "Backend", "Support", "DevOps",
                ])) as Arc<dyn arrow::array::Array>,
            ),
            (
                "team",
                Arc::new(StringArray::from(vec!["Alpha", "Gamma", "Echo", "Golf"]))
                    as Arc<dyn arrow::array::Array>,
            ),
            (
                "subteam",
                Arc::new(StringArray::from(vec!["X", "X", "X", "X"]))
                    as Arc<dyn arrow::array::Array>,
            ),
            (
                "x_val",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 1.0, 2.0]))
                    as Arc<dyn arrow::array::Array>,
            ),
            (
                "y_val",
                Arc::new(Float64Array::from(vec![90.0, 110.0, 40.0, 60.0]))
                    as Arc<dyn arrow::array::Array>,
            ),
        ])
        .expect("create sparse fixed column hole batch");

        ctx.read_batch(batch)
            .expect("read sparse fixed column hole batch")
    }

    fn build_deeply_nested_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(700, 500)
            .mark(
                Subplot::new(
                    Plot::<FacetRow>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new().mark(
                                Rect::new()
                                    .x_with(col("category"), |c| {
                                        c.scale_with::<Band>(|s| s)
                                            .with_domain_scope(CoordinationScope::Shared)
                                            .axis(|a| a.title("Category"))
                                    })
                                    .x2_with(col(":x"), |c| c.band(1.0))
                                    .y(0.0)
                                    .y2_with(col("value"), |c| {
                                        c.with_domain_scope(CoordinationScope::Shared)
                                            .axis(|a| a.title("Value"))
                                    })
                                    .fill("#4682b4"),
                            ),
                        )
                        .row_with(col("sub_group"), |c| c.guide(|g| g.title("Sub Group"))),
                    ),
                )
                .col_with(col("outer_group"), |c| c.guide(|g| g.title("Outer Group"))),
            )
    }

    fn build_sparse_fixed_column_hole_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(80.0, 60.0)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<FacetColumn>::new().mark(
                                Subplot::new(
                                    Plot::<FacetColumn>::new().mark(
                                        Subplot::new(
                                            Plot::<Cartesian>::new().mark(
                                                Symbol::new()
                                                    .x_with(col("x_val"), |c| {
                                                        c.with_domain_scope(
                                                            CoordinationScope::Level(4),
                                                        )
                                                    })
                                                    .y_with(col("y_val"), |c| {
                                                        c.with_domain_scope(
                                                            CoordinationScope::Level(4),
                                                        )
                                                    })
                                                    .size(24.0)
                                                    .fill("#3498db"),
                                            ),
                                        )
                                        .col_with(col("subteam"), |c| c.guide(|g| g.title("Sub"))),
                                    ),
                                )
                                .col_with(col("team"), |c| {
                                    c.with_slot_sharing(CoordinationScope::Level(1))
                                        .guide(|g| g.title("Team"))
                                }),
                            ),
                        )
                        .col_with(col("department"), |c| {
                            c.free_slots().guide(|g| g.title("Dept"))
                        }),
                    ),
                )
                .col_with(col("division"), |c| c.guide(|g| g.title("Division"))),
            )
    }

    fn build_simple_facet_col_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new().data(df).mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("value"))
                        .y(col("value"))
                        .size(24.0)
                        .fill("#4682b4"),
                ),
            )
            .column(col("outer_group")),
        )
    }

    fn build_simple_facet_wrap_plot(df: DataFrame) -> Plot<FacetWrap> {
        Plot::<FacetWrap>::new()
            .data(df)
            .canvas_constraint(CanvasConstraint::width(360.0))
            .plot_constraint(PlotConstraint::height(90.0))
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("value"))
                            .y(col("value"))
                            .size(24.0)
                            .fill("#4682b4"),
                    ),
                )
                .wrap_with(col("category"), |c| {
                    c.columns(2).guide(|g| g.title("Category"))
                }),
            )
    }

    fn build_canvas_sized_facet_wrap_plot(df: DataFrame) -> Plot<FacetWrap> {
        Plot::<FacetWrap>::new()
            .data(df)
            .canvas_size(360.0, 260.0)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(
                        Symbol::new()
                            .x(col("value"))
                            .y(col("value"))
                            .size(24.0)
                            .fill("#4682b4"),
                    ),
                )
                .wrap_with(col("category"), |c| c.columns(2)),
            )
    }

    async fn legend_sharing_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE legend_sharing AS VALUES
            ('DivA', 'Dept1', 1.0, 1.0, 'Low'),
            ('DivA', 'Dept1', 1.4, 1.3, 'High'),
            ('DivA', 'Dept2', 2.0, 1.1, 'Low'),
            ('DivA', 'Dept2', 2.3, 1.4, 'High'),
            ('DivB', 'Dept1', 3.0, 1.0, 'Low'),
            ('DivB', 'Dept1', 3.4, 1.3, 'High'),
            ('DivB', 'Dept2', 4.0, 1.1, 'Low'),
            ('DivB', 'Dept2', 4.3, 1.4, 'High')",
        )
        .await
        .expect("create legend sharing test data");

        ctx.sql(
            "SELECT
                column1 AS division,
                column2 AS department,
                column3 AS x_val,
                column4 AS y_val,
                column5 AS category
             FROM legend_sharing",
        )
        .await
        .expect("read legend sharing test data")
    }

    async fn legend_sharing_three_level_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE legend_sharing_three_level AS VALUES
            ('DivA', 'Dept1', 'Team1', 1.0, 1.0, 'Low'),
            ('DivA', 'Dept1', 'Team1', 1.4, 1.3, 'High'),
            ('DivA', 'Dept1', 'Team2', 2.0, 1.1, 'Low'),
            ('DivA', 'Dept1', 'Team2', 2.3, 1.4, 'High'),
            ('DivA', 'Dept2', 'Team1', 1.1, 2.0, 'Low'),
            ('DivA', 'Dept2', 'Team1', 1.5, 2.3, 'High'),
            ('DivA', 'Dept2', 'Team2', 2.1, 2.1, 'Low'),
            ('DivA', 'Dept2', 'Team2', 2.4, 2.4, 'High'),
            ('DivB', 'Dept1', 'Team1', 3.0, 1.0, 'Low'),
            ('DivB', 'Dept1', 'Team1', 3.4, 1.3, 'High'),
            ('DivB', 'Dept1', 'Team2', 4.0, 1.1, 'Low'),
            ('DivB', 'Dept1', 'Team2', 4.3, 1.4, 'High'),
            ('DivB', 'Dept2', 'Team1', 3.1, 2.0, 'Low'),
            ('DivB', 'Dept2', 'Team1', 3.5, 2.3, 'High'),
            ('DivB', 'Dept2', 'Team2', 4.1, 2.1, 'Low'),
            ('DivB', 'Dept2', 'Team2', 4.4, 2.4, 'High')",
        )
        .await
        .expect("create three-level legend sharing test data");

        ctx.sql(
            "SELECT
                column1 AS division,
                column2 AS department,
                column3 AS team,
                column4 AS x_val,
                column5 AS y_val,
                column6 AS category
             FROM legend_sharing_three_level",
        )
        .await
        .expect("read three-level legend sharing test data")
    }

    async fn positioned_legend_sharing_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "SELECT
                column1 AS slot,
                column2 AS parent_r,
                column3 AS parent_theta,
                column4 AS child_x,
                column5 AS child_y,
                column6 AS category
             FROM (VALUES
                ('near', 0.34, 0.70, 0.10, 0.20, 'Alpha'),
                ('near', 0.34, 0.70, 0.26, 0.48, 'Alpha'),
                ('near', 0.34, 0.70, 0.42, 0.78, 'Alpha'),
                ('far',  0.70, 3.80, 0.58, 0.28, 'Beta'),
                ('far',  0.70, 3.80, 0.76, 0.56, 'Beta'),
                ('far',  0.70, 3.80, 0.92, 0.84, 'Beta')
             )",
        )
        .await
        .expect("create positioned legend sharing test data")
    }

    async fn two_level_col_col_refinement_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE two_level_col_col_refinement AS VALUES
            ('Eng', 'Frontend', 1.0, 90.0),
            ('Eng', 'Frontend', 2.0, 95.0),
            ('Eng', 'Backend', 1.0, 110.0),
            ('Eng', 'Backend', 2.0, 115.0),
            ('Ops', 'Support', 1.0, 40.0),
            ('Ops', 'Support', 2.0, 45.0),
            ('Ops', 'DevOps', 1.0, 60.0),
            ('Ops', 'DevOps', 2.0, 65.0)",
        )
        .await
        .expect("create two-level col-col refinement test data");

        ctx.sql(
            "SELECT
                column1 AS division,
                column2 AS department,
                column3 AS x_val,
                column4 AS y_val
             FROM two_level_col_col_refinement",
        )
        .await
        .expect("read two-level col-col refinement test data")
    }

    async fn nested_sparse_row_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE nested_sparse_row AS VALUES
            ('Iris-setosa', 'narrow', 4.8, 3.1),
            ('Iris-setosa', 'narrow', 5.1, 3.4),
            ('Iris-versicolor', 'medium', 5.8, 2.8),
            ('Iris-versicolor', 'wide', 7.2, 3.0),
            ('Iris-virginica', 'medium', 6.3, 2.9),
            ('Iris-virginica', 'wide', 7.8, 3.1)",
        )
        .await
        .expect("create nested sparse row test data");

        ctx.sql(
            "SELECT
                column1 AS species,
                column2 AS petal_width_bin,
                column3 AS sepal_length,
                column4 AS sepal_width
             FROM nested_sparse_row",
        )
        .await
        .expect("read nested sparse row test data")
    }

    async fn shared_row_basic_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE shared_row_basic AS VALUES
            ('C1', 'R1', 1.0, 1.0),
            ('C1', 'R2', 1.2, 1.3),
            ('C2', 'R1', 2.0, 1.1),
            ('C2', 'R3', 2.3, 1.4),
            ('C3', 'R2', 3.0, 1.2),
            ('C3', 'R3', 3.4, 1.5)",
        )
        .await
        .expect("create shared row basic test data");

        ctx.sql(
            "SELECT
                column1 AS col_group,
                column2 AS row_group,
                column3 AS x_val,
                column4 AS y_val
             FROM shared_row_basic",
        )
        .await
        .expect("read shared row basic test data")
    }

    async fn jagged_group_local_dataframe(ctx: &SessionContext) -> DataFrame {
        ctx.sql(
            "CREATE TABLE jagged_group_local AS VALUES
            ('A', 'C1', 'A1', 1.0, 1.0),
            ('A', 'C1', 'A2', 1.2, 1.2),
            ('A', 'C1', 'A3', 1.4, 1.4),
            ('A', 'C2', 'A1', 2.0, 1.1),
            ('A', 'C2', 'A2', 2.2, 1.3),
            ('A', 'C3', 'A2', 3.0, 1.2),
            ('A', 'C3', 'A3', 3.2, 1.4),
            ('B', 'C1', 'B1', 1.0, 2.0),
            ('B', 'C1', 'B2', 1.3, 2.2),
            ('B', 'C2', 'B1', 2.0, 2.1),
            ('B', 'C2', 'B2', 2.3, 2.3)",
        )
        .await
        .expect("create jagged group-local test data");

        ctx.sql(
            "SELECT
                column1 AS outer_group,
                column2 AS inner_col,
                column3 AS row_group,
                column4 AS x_val,
                column5 AS y_val
             FROM jagged_group_local",
        )
        .await
        .expect("read jagged group-local test data")
    }

    fn build_level1_fill_legend_symbol(position: LegendPosition) -> Symbol<Cartesian> {
        Symbol::new()
            .x_with(col("x_val"), |c| {
                c.with_domain_scope(CoordinationScope::Shared)
            })
            .y_with(col("y_val"), |c| {
                c.with_domain_scope(CoordinationScope::Shared)
            })
            .fill_with(col("category"), move |c| {
                c.with_domain_scope(CoordinationScope::Level(1))
                    .legend(|legend| legend.title("Category").position(position))
            })
            .size(70.0)
    }

    fn build_level2_fill_legend_symbol(position: LegendPosition) -> Symbol<Cartesian> {
        Symbol::new()
            .x_with(col("x_val"), |c| {
                c.with_domain_scope(CoordinationScope::Shared)
            })
            .y_with(col("y_val"), |c| {
                c.with_domain_scope(CoordinationScope::Shared)
            })
            .fill_with(col("category"), move |c| {
                c.with_domain_scope(CoordinationScope::Level(2))
                    .legend(|legend| legend.title("Category").position(position))
            })
            .size(70.0)
    }

    fn build_unshared_fill_legend_symbol(position: LegendPosition) -> Symbol<Cartesian> {
        Symbol::new()
            .x(col("x_val"))
            .y(col("y_val"))
            .fill_with(col("category"), move |c| {
                c.legend(|legend| legend.title("Category").position(position))
            })
            .size(70.0)
    }

    fn build_two_level_col_legend_sharing_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(960.0, 420.0)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new()
                                .mark(build_level1_fill_legend_symbol(position)),
                        )
                        .column(col("department")),
                    ),
                )
                .column(col("division")),
            )
    }

    fn build_two_level_col_free_legend_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(960.0, 420.0)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("x_val"), |c| {
                                        c.with_domain_scope(CoordinationScope::Shared)
                                    })
                                    .y_with(col("y_val"), |c| {
                                        c.with_domain_scope(CoordinationScope::Shared)
                                    })
                                    .fill_with(col("category"), move |c| {
                                        c.with_domain_scope(CoordinationScope::Free).legend(
                                            |legend| legend.title("Category").position(position),
                                        )
                                    })
                                    .size(70.0),
                            ),
                        )
                        .column(col("department")),
                    ),
                )
                .column(col("division")),
            )
    }

    fn build_three_level_col_legend_sharing_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(1500.0, 380.0)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<FacetColumn>::new().mark(
                                Subplot::new(
                                    Plot::<Cartesian>::new()
                                        .mark(build_level2_fill_legend_symbol(position)),
                                )
                                .column(col("team")),
                            ),
                        )
                        .column(col("department")),
                    ),
                )
                .column(col("division")),
            )
    }

    fn build_three_level_col_legend_sharing_plot_area_sized(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(120.0, 90.0)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<FacetColumn>::new().mark(
                                Subplot::new(
                                    Plot::<Cartesian>::new()
                                        .mark(build_level2_fill_legend_symbol(position)),
                                )
                                .column(col("team")),
                            ),
                        )
                        .column(col("department")),
                    ),
                )
                .column(col("division")),
            )
    }

    fn build_nested_col_row_col_continuous_legend_plot_area_sized(
        df: DataFrame,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(110.0, 80.0)
            .mark(
                Subplot::new(
                    Plot::<FacetRow>::new().mark(
                        Subplot::new(
                            Plot::<FacetColumn>::new().mark(
                                Subplot::new(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_domain_scope(CoordinationScope::Shared)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_domain_scope(CoordinationScope::Shared)
                                            })
                                            .fill_with(col("y_val"), |c| {
                                                c.scale_with::<avenger_chart_scales::Linear>(|s| s)
                                                    .legend(|legend| {
                                                        legend
                                                            .title("Score")
                                                            .position(LegendPosition::Right)
                                                    })
                                            })
                                            .size(58.0),
                                    ),
                                )
                                .column(col("team")),
                            ),
                        )
                        .row(col("department")),
                    ),
                )
                .column(col("division")),
            )
    }

    fn build_shared_positioned_legend_child() -> Plot<Cartesian> {
        Plot::<Cartesian>::new().mark(
            Symbol::new()
                .x_with(col("child_x"), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .axis(|a| a.show_title(false))
                })
                .y_with(col("child_y"), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                    .axis(|a| a.show_title(false))
                })
                .fill_with(col("category"), |c| {
                    c.with_domain_scope(CoordinationScope::Shared)
                        .legend(|legend| legend.title("Category").position(LegendPosition::Right))
                })
                .stroke("#ffffff")
                .stroke_width(1.0)
                .size(72.0),
        )
    }

    fn build_polar_positioned_legend_sharing_plot(df: DataFrame) -> Plot<Polar> {
        Plot::<Polar>::new().data(df).plot_size(480.0, 360.0).mark(
            Subplot::<Polar>::new(build_shared_positioned_legend_child())
                .partition_by(col("slot"))
                .r_with(avg(col("parent_r")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(1.0))).nice(false).zero(false)
                    })
                })
                .theta_with(avg(col("parent_theta")), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(0.0), lit(std::f64::consts::TAU)))
                            .nice(false)
                            .zero(false)
                    })
                })
                .plot_size(118.0, 92.0),
        )
    }

    fn build_nested_sparse_row_plot(df: DataFrame) -> Plot<FacetRow> {
        Plot::<FacetRow>::new().data(df).canvas_size(600, 600).mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), |c| {
                                    c.with_domain_scope(CoordinationScope::Shared)
                                })
                                .y_with(col("sepal_width"), |c| {
                                    c.with_domain_scope(CoordinationScope::Level(1))
                                })
                                .size(25.0)
                                .fill("#4682b4"),
                        ),
                    )
                    .column(col("petal_width_bin")),
                ),
            )
            .row(col("species")),
        )
    }

    fn build_nested_shared_row_basic_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(760.0, 560.0)
            .mark(
                Subplot::new(
                    Plot::<FacetRow>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new()
                                .mark(Symbol::new().x(col("x_val")).y(col("y_val")).size(35.0)),
                        )
                        .row_with(col("row_group"), |c| {
                            c.with_slot_sharing(CoordinationScope::Shared)
                                .guide(|g| g.position("right"))
                        }),
                    ),
                )
                .column(col("col_group")),
            )
    }

    fn build_nested_shared_row_shared_both_plot(df: DataFrame) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(760.0, 560.0)
            .mark(
                Subplot::new(
                    Plot::<FacetRow>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("x_val"), |c| {
                                        c.with_domain_scope(CoordinationScope::Shared)
                                    })
                                    .y_with(col("y_val"), |c| {
                                        c.with_domain_scope(CoordinationScope::Shared)
                                    })
                                    .size(35.0),
                            ),
                        )
                        .row_with(col("row_group"), |c| {
                            c.with_slot_sharing(CoordinationScope::Shared)
                        }),
                    ),
                )
                .column(col("col_group")),
            )
    }

    fn build_nested_shared_row_shared_both_plot_with_empty_policy(
        df: DataFrame,
        policy: FacetEmptyCellPolicy,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(760.0, 560.0)
            .mark(
                Subplot::new(
                    Plot::<FacetRow>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("x_val"), |c| {
                                        c.with_domain_scope(CoordinationScope::Shared)
                                    })
                                    .y_with(col("y_val"), |c| {
                                        c.with_domain_scope(CoordinationScope::Shared)
                                    })
                                    .size(35.0),
                            ),
                        )
                        .row_with(col("row_group"), move |c| {
                            c.with_slot_sharing(CoordinationScope::Shared)
                                .empty_cell_policy(policy)
                        }),
                    ),
                )
                .column(col("col_group")),
            )
    }

    fn build_jagged_group_local_shared_row_plot(df: DataFrame) -> Plot<FacetRow> {
        Plot::<FacetRow>::new()
            .data(df)
            .canvas_size(980.0, 700.0)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<FacetRow>::new().mark(
                                Subplot::new(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x(col("x_val"))
                                            .y(col("y_val"))
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                )
                                .row_with(col("row_group"), |c| {
                                    c.with_slot_sharing(CoordinationScope::Level(1))
                                        .guide(|g| g.position("right"))
                                }),
                            ),
                        )
                        .column(col("inner_col")),
                    ),
                )
                .row(col("outer_group")),
            )
    }

    fn build_single_level_row_legend_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetRow> {
        Plot::<FacetRow>::new()
            .data(df)
            .canvas_size(960.0, 420.0)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(build_unshared_fill_legend_symbol(position)),
                )
                .row(col("department")),
            )
    }

    fn build_single_level_col_legend_plot(
        df: DataFrame,
        position: LegendPosition,
    ) -> Plot<FacetColumn> {
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(960.0, 420.0)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new().mark(build_unshared_fill_legend_symbol(position)),
                )
                .column(col("division")),
            )
    }

    async fn compile_two_level_col_legend_sharing_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_dataframe(ctx).await;
        build_two_level_col_legend_sharing_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_two_level_col_free_legend_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_three_level_dataframe(ctx).await;
        build_two_level_col_free_legend_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_three_level_col_legend_sharing_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_three_level_dataframe(ctx).await;
        build_three_level_col_legend_sharing_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_three_level_col_legend_sharing_plot_area_sized(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_three_level_dataframe(ctx).await;
        build_three_level_col_legend_sharing_plot_area_sized(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_two_level_col_col_refinement_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = two_level_col_col_refinement_dataframe(ctx).await;
        Plot::<FacetColumn>::new()
            .data(df)
            .canvas_size(1800, 500)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<Cartesian>::new().mark(
                                Symbol::new()
                                    .x_with(col("x_val"), |c| {
                                        c.with_domain_scope(CoordinationScope::Level(2))
                                    })
                                    .y_with(col("y_val"), |c| {
                                        c.with_domain_scope(CoordinationScope::Level(2))
                                    })
                                    .size(40.0)
                                    .fill("#9b59b6"),
                            ),
                        )
                        .col_with(col("department"), |c| c.guide(|g| g.title("Dept"))),
                    ),
                )
                .col_with(col("division"), |c| c.guide(|g| g.title("Division"))),
            )
            .compile(ctx)
            .await
    }

    async fn compile_nested_col_row_col_continuous_legend_plot_area_sized(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_three_level_dataframe(ctx).await;
        build_nested_col_row_col_continuous_legend_plot_area_sized(df)
            .compile(ctx)
            .await
    }

    async fn compile_nested_row_col_row_mixed_sharing_plot_area_sized(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_three_level_dataframe(ctx).await;
        Plot::<FacetRow>::new()
            .data(df)
            .plot_size(110.0, 80.0)
            .mark(
                Subplot::new(
                    Plot::<FacetColumn>::new().mark(
                        Subplot::new(
                            Plot::<FacetRow>::new().mark(
                                Subplot::new(
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x_with(col("x_val"), |c| {
                                                c.with_domain_scope(CoordinationScope::Shared)
                                            })
                                            .y_with(col("y_val"), |c| {
                                                c.with_domain_scope(CoordinationScope::Free)
                                            })
                                            .fill_with(col("category"), |c| {
                                                c.with_domain_scope(CoordinationScope::Level(1))
                                            })
                                            .size(58.0),
                                    ),
                                )
                                .row(col("team")),
                            ),
                        )
                        .column(col("department")),
                    ),
                )
                .row(col("division")),
            )
            .compile(ctx)
            .await
    }

    async fn compile_nested_sparse_row_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = nested_sparse_row_dataframe(ctx).await;
        build_nested_sparse_row_plot(df).compile(ctx).await
    }

    async fn compile_nested_shared_row_basic_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = shared_row_basic_dataframe(ctx).await;
        build_nested_shared_row_basic_plot(df).compile(ctx).await
    }

    async fn compile_nested_shared_row_shared_both_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = shared_row_basic_dataframe(ctx).await;
        build_nested_shared_row_shared_both_plot(df)
            .compile(ctx)
            .await
    }

    async fn compile_nested_shared_row_shared_both_plot_with_empty_policy(
        ctx: &SessionContext,
        policy: FacetEmptyCellPolicy,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = shared_row_basic_dataframe(ctx).await;
        build_nested_shared_row_shared_both_plot_with_empty_policy(df, policy)
            .compile(ctx)
            .await
    }

    async fn compile_jagged_group_local_shared_row_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = jagged_group_local_dataframe(ctx).await;
        build_jagged_group_local_shared_row_plot(df)
            .compile(ctx)
            .await
    }

    async fn compile_single_level_row_legend_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_dataframe(ctx).await;
        build_single_level_row_legend_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_single_level_col_legend_plot(
        ctx: &SessionContext,
        position: LegendPosition,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = legend_sharing_dataframe(ctx).await;
        build_single_level_col_legend_plot(df, position)
            .compile(ctx)
            .await
    }

    async fn compile_deeply_nested_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = deeply_nested_dataframe(ctx);
        let plot = build_deeply_nested_plot(df);
        plot.compile(ctx).await
    }

    async fn compile_sparse_fixed_column_hole_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = sparse_fixed_column_hole_dataframe(ctx);
        build_sparse_fixed_column_hole_plot(df).compile(ctx).await
    }

    async fn compile_simple_facet_plot_with_plot_size(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = deeply_nested_dataframe(ctx);
        build_simple_facet_col_plot(df)
            .plot_size(120.0, 90.0)
            .compile(ctx)
            .await
    }

    async fn compile_simple_facet_wrap_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = deeply_nested_dataframe(ctx);
        build_simple_facet_wrap_plot(df).compile(ctx).await
    }

    async fn compile_simple_regular_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = deeply_nested_dataframe(ctx);
        Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .size(24.0)
                    .fill("#4682b4"),
            )
            .compile(ctx)
            .await
    }

    fn collect_parallel_mark_names(
        mark: &SceneMark,
        line_names: &mut Vec<String>,
        symbol_names: &mut Vec<String>,
    ) {
        match mark {
            SceneMark::Group(group) => {
                for child in &group.marks {
                    collect_parallel_mark_names(child, line_names, symbol_names);
                }
            }
            SceneMark::Line(line) => line_names.push(line.name.clone()),
            SceneMark::Symbol(symbol) => symbol_names.push(symbol.name.clone()),
            _ => {}
        }
    }

    #[tokio::test]
    async fn parallel_mark_public_ids_name_all_emitted_scene_marks() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("speed", DataType::Float64, false),
                Field::new("cost", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Float64Array::from(vec![42.0, 64.0])),
                Arc::new(Float64Array::from(vec![95.0, 145.0])),
            ],
        )?;
        let df = ctx.read_batch(batch)?;
        let coord = Parallel::new()
            .dimension("speed", col("speed"))
            .dimension("cost", col("cost"));
        let compiled = Plot::with_coord(coord)
            .data(df)
            .plot_size(220.0, 140.0)
            .mark(ParallelLine::new().id("paths"))
            .mark(ParallelSymbol::new().id("points"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let mut line_names = Vec::new();
        let mut symbol_names = Vec::new();
        for mark in &evaluated.scene_graph.marks {
            collect_parallel_mark_names(mark, &mut line_names, &mut symbol_names);
        }

        assert_eq!(line_names, vec!["paths".to_string(), "paths".to_string()]);
        assert_eq!(
            symbol_names,
            vec!["points".to_string(), "points".to_string()]
        );
        Ok(())
    }

    #[tokio::test]
    async fn discrete_legend_items_register_event_datum_rows() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = legend_sharing_dataframe(&ctx).await;
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .canvas_size(520.0, 360.0)
            .event_binding(
                ChartEventBinding::on(crate::event::ChartEventType::Click)
                    .filter(crate::event::is_legend_item())
                    .filter(crate::event::legend_value().is_not_null())
                    .filter(crate::event::legend_channel().eq(lit("fill"))),
            )
            .mark(build_unshared_fill_legend_symbol(LegendPosition::Right))
            .compile(&ctx)
            .await?;
        let event_datum_types = compiled.event_datum_types();
        assert_eq!(
            event_datum_types.get("__legend_value"),
            Some(&DataType::Utf8)
        );
        assert_eq!(
            event_datum_types.get("__legend_channel"),
            Some(&DataType::Utf8)
        );

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let legend_rows = evaluated
            .event_datums
            .rows
            .iter()
            .filter(|rows| rows.rows.column_by_name("__legend_channel").is_some())
            .collect::<Vec<_>>();
        assert_eq!(legend_rows.len(), 2);

        let first = legend_rows[0];
        assert!(first.mark_path.len() >= 3);
        let value =
            ScalarValue::try_from_array(first.rows.column_by_name("__legend_value").unwrap(), 0)?;
        let label =
            ScalarValue::try_from_array(first.rows.column_by_name("__legend_label").unwrap(), 0)?;
        let channel =
            ScalarValue::try_from_array(first.rows.column_by_name("__legend_channel").unwrap(), 0)?;
        let index =
            ScalarValue::try_from_array(first.rows.column_by_name("__legend_index").unwrap(), 0)?;
        assert_eq!(value, ScalarValue::Utf8(Some("High".to_string())));
        assert_eq!(label, ScalarValue::Utf8(Some("High".to_string())));
        assert_eq!(channel, ScalarValue::Utf8(Some("fill".to_string())));
        assert_eq!(index, ScalarValue::Int64(Some(0)));
        Ok(())
    }

    #[tokio::test]
    async fn colorbar_legend_bindings_register_continuous_surface() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .canvas_size(420.0, 320.0)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .size(16.0)
                    .fill("#aeb6c4"),
            )
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .size(24.0)
                    .fill_with(col("value"), |c| {
                        c.legend(|l| {
                            l.event_binding(
                                ChartEventBinding::on(crate::event::ChartEventType::Click)
                                    .filter(
                                        crate::event::datum("surface_kind")
                                            .eq(lit("continuous-colorbar")),
                                    )
                                    .filter(crate::event::datum("value_channel").eq(lit("y"))),
                            )
                        })
                    }),
            )
            .compile(&ctx)
            .await?;

        let event_datum_types = compiled.event_datum_types();
        assert_eq!(
            event_datum_types.get("__legend_surface_kind"),
            Some(&DataType::Utf8)
        );
        assert_eq!(
            event_datum_types.get("__legend_value_channel"),
            Some(&DataType::Utf8)
        );

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let colorbar_rows = evaluated
            .event_datums
            .rows
            .iter()
            .filter(|rows| {
                rows.rows
                    .column_by_name("__legend_surface_kind")
                    .and_then(|array| ScalarValue::try_from_array(array, 0).ok())
                    == Some(ScalarValue::Utf8(Some("continuous-colorbar".to_string())))
            })
            .collect::<Vec<_>>();
        assert_eq!(colorbar_rows.len(), 1);
        let value_channel = ScalarValue::try_from_array(
            colorbar_rows[0]
                .rows
                .column_by_name("__legend_value_channel")
                .unwrap(),
            0,
        )?;
        assert_eq!(value_channel, ScalarValue::Utf8(Some("y".to_string())));
        let colorbar_scopes = evaluated
            .interaction
            .scopes
            .iter()
            .filter(|scope| scope.kind == InteractionScopeKind::LegendColorbar)
            .collect::<Vec<_>>();
        assert_eq!(colorbar_scopes.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn colorbar_overlay_marks_render_in_legend_group() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let overlay = crate::legend::ColorbarOverlay::new().mark(
            Rect::<Cartesian>::new()
                .unit_data()
                .exclude_from_scale_domains()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(lit(2.0))
                .y2(lit(7.0))
                .fill("rgba(37, 99, 235, 0.20)")
                .stroke("#2563eb")
                .stroke_width(1.5),
        );
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .canvas_size(420.0, 320.0)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .size(24.0)
                    .fill_with(col("value"), |c| {
                        c.scale_with::<Linear>(|s| s.domain((0.0, 10.0)).nice(false).zero(false))
                            .legend(|l| l.title("Value").colorbar_overlay(overlay))
                    }),
            )
            .compile(&ctx)
            .await?;
        let overlay_bytes = bincode::serialize(&compiled.legend_colorbar_overlays)
            .expect("serialize colorbar overlay marks");
        let _roundtripped: Vec<crate::plot::compiled::CompiledColorbarOverlayMarks> =
            bincode::deserialize(&overlay_bytes).expect("deserialize colorbar overlay marks");
        let legend_bytes = bincode::serialize(&compiled.legends).expect("serialize legends");
        let _legend_roundtripped: IndexMap<String, Legend> =
            bincode::deserialize(&legend_bytes).expect("deserialize legends");
        let compiled_bytes = bincode::serialize(&compiled).expect("serialize compiled plot");
        let _compiled_roundtripped: CompiledPlot =
            bincode::deserialize(&compiled_bytes).expect("deserialize compiled plot");

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let overlay_group =
            find_scene_group_by_name(&evaluated.scene_graph.marks, "fill-colorbar-overlays")
                .expect("colorbar overlay group should render");
        assert!(!overlay_group.interactive);
        assert!(matches!(
            overlay_group.clip,
            avenger_scenegraph::marks::group::Clip::Rect { .. }
        ));
        assert!(
            overlay_group
                .marks
                .iter()
                .any(|mark| matches!(mark, avenger_scenegraph::marks::mark::SceneMark::Rect(_)))
        );
        let (clip_width, clip_height) = clip_dimensions(overlay_group);
        let rect = first_rect_mark(overlay_group);
        assert_close(rect.x_vec()[0], 0.0);
        assert_close(rect.x2_vec()[0], clip_width);
        assert_close(rect.y_vec()[0], clip_height * 0.8);
        assert_close(rect.y2_vec()[0], clip_height * 0.3);
        Ok(())
    }

    #[tokio::test]
    async fn horizontal_colorbar_overlay_rect_maps_data_values_to_pixels()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let overlay = crate::legend::ColorbarOverlay::new().mark(
            Rect::<Cartesian>::new()
                .unit_data()
                .exclude_from_scale_domains()
                .x(lit(2.0))
                .x2(lit(7.0))
                .y(lit(0.0))
                .y2(lit(1.0))
                .fill("rgba(37, 99, 235, 0.20)")
                .stroke("#2563eb")
                .stroke_width(1.5),
        );
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .canvas_size(420.0, 320.0)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .size(24.0)
                    .fill_with(col("value"), |c| {
                        c.scale_with::<Linear>(|s| s.domain((0.0, 10.0)).nice(false).zero(false))
                            .legend(|l| {
                                l.title("Value")
                                    .position(LegendPosition::Bottom)
                                    .colorbar_overlay(overlay)
                            })
                    }),
            )
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let overlay_group =
            find_scene_group_by_name(&evaluated.scene_graph.marks, "fill-colorbar-overlays")
                .expect("colorbar overlay group should render");
        let (clip_width, clip_height) = clip_dimensions(overlay_group);
        let rect = first_rect_mark(overlay_group);
        assert_close(rect.x_vec()[0], clip_width * 0.2);
        assert_close(rect.x2_vec()[0], clip_width * 0.7);
        assert_close(rect.y_vec()[0], 0.0);
        assert_close(rect.y2_vec()[0], clip_height);
        Ok(())
    }

    #[tokio::test]
    async fn colorbar_overlay_channels_do_not_affect_parent_domains_or_legends()
    -> Result<(), AvengerChartError> {
        use avenger_chart_core::{ConfiguredScaleLegendExt, DomainValues};

        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let overlay = crate::legend::ColorbarOverlay::new().mark(
            Rect::<Cartesian>::new()
                .unit_data()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(lit(-1_000.0))
                .y2(lit(1_000.0))
                .fill("#2563eb"),
        );
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .canvas_size(420.0, 320.0)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .size(24.0)
                    .fill_with(col("value"), |c| {
                        c.legend(|l| l.title("Value").colorbar_overlay(overlay))
                    }),
            )
            .compile(&ctx)
            .await?;
        assert_eq!(compiled.legends.len(), 1);
        assert!(compiled.legends.contains_key("fill"));

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let colorbar_scope = evaluated
            .interaction
            .scopes
            .iter()
            .find(|scope| scope.kind == InteractionScopeKind::LegendColorbar)
            .expect("colorbar interaction scope");
        let value_scale = colorbar_scope
            .scales
            .get("y")
            .expect("vertical colorbar value scale");
        let DomainValues::Interval(min, max) = value_scale.domain_values()? else {
            panic!("colorbar scale should have interval domain");
        };
        let min = min.as_f64().expect("numeric colorbar domain min");
        let max = max.as_f64().expect("numeric colorbar domain max");
        assert!(
            min > -100.0 && max < 100.0,
            "overlay interval values should not affect parent colorbar domain: {min}..{max}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn colorbar_overlay_rejects_visible_overlay_legends() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let overlay = crate::legend::ColorbarOverlay::new().mark(
            Rect::<Cartesian>::new()
                .unit_data()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(lit(2.0))
                .y2(lit(7.0))
                .fill_with(lit("overlay"), |c| c.legend(|l| l.title("Overlay"))),
        );
        let err = match Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .fill_with(col("value"), |c| c.legend(|l| l.colorbar_overlay(overlay))),
            )
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("overlay legends should be rejected"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("cannot create a legend"));
    }

    #[tokio::test]
    async fn colorbar_overlay_rejects_position_scale_configs() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let overlay = crate::legend::ColorbarOverlay::new().mark(
            Rect::<Cartesian>::new()
                .unit_data()
                .x_with(lit(0.0), |c| c.scale_with::<Linear>(|s| s))
                .x2(lit(1.0))
                .y(lit(2.0))
                .y2(lit(7.0))
                .fill("#2563eb"),
        );
        let err = match Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .fill_with(col("value"), |c| c.legend(|l| l.colorbar_overlay(overlay))),
            )
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("overlay position scale configs should be rejected"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("cannot define its own scale"));
    }

    #[tokio::test]
    async fn colorbar_overlay_rejects_positioned_subplots() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let child = Plot::<Cartesian>::new().mark(Symbol::new().x(lit(0.0)).y(lit(0.0)));
        let overlay = crate::legend::ColorbarOverlay::new()
            .mark(Subplot::new(child).subplot_x(lit(0.5)).subplot_y(lit(5.0)));
        let err = match Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .fill_with(col("value"), |c| c.legend(|l| l.colorbar_overlay(overlay))),
            )
            .compile(&ctx)
            .await
        {
            Ok(_) => panic!("positioned subplots should be rejected"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("positioned subplots"));
    }

    #[tokio::test]
    async fn colorbar_overlay_rejects_non_colorbar_legend() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let overlay = crate::legend::ColorbarOverlay::new().mark(
            Rect::<Cartesian>::new()
                .unit_data()
                .x(lit(0.0))
                .x2(lit(1.0))
                .y(lit(2.0))
                .y2(lit(7.0))
                .fill("#2563eb"),
        );
        let compiled = Plot::<Cartesian>::new()
            .data(df)
            .mark(
                Symbol::new()
                    .x(col("value"))
                    .y(col("value"))
                    .fill_with(col("category"), |c| {
                        c.legend(|l| l.colorbar_overlay(overlay))
                    }),
            )
            .compile(&ctx)
            .await?;
        let err = match compiled.evaluate(&ctx, None).await {
            Ok(_) => panic!("overlay on a discrete legend should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("did not render a colorbar surface")
        );
        Ok(())
    }

    async fn compile_domain_param_regular_plot(
        ctx: &SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        let df = deeply_nested_dataframe(ctx);
        let x0 = Param::new("x0", ScalarValue::Float64(Some(0.0)));
        let x1 = Param::new("x1", ScalarValue::Float64(Some(10.0)));
        let x0_expr = x0.expr();
        let x1_expr = x1.expr();
        Plot::<Cartesian>::new()
            .data(df)
            .add_params([x0.clone(), x1.clone()])
            .mark(
                Symbol::new()
                    .x_with(col("value"), move |c| {
                        let x0_expr = x0_expr.clone();
                        let x1_expr = x1_expr.clone();
                        c.scale(move |s| s.domain_interval(x0_expr.clone(), x1_expr.clone()))
                    })
                    .y(col("value"))
                    .size(24.0)
                    .fill("#4682b4"),
            )
            .compile(ctx)
            .await
    }

    async fn prepare_top_level_measurement(
        compiled: &CompiledPlot,
        ctx: &SessionContext,
    ) -> Result<
        (
            EvaluationContext,
            EvaluatedLayoutSpec,
            avenger_chart_scales::ScaleBuilder,
            ComponentsMeasurement,
        ),
        AvengerChartError,
    > {
        let merged_params = compiled.get_default_params().clone();
        let facet_tree = Arc::new(EvaluatedFacetTree::from_compiled_plot(compiled, ctx).await?);
        let evaluated_layout_spec = evaluate_layout_spec(
            compiled.get_layout_spec(),
            ctx,
            &merged_params,
            compiled.get_theme().as_ref(),
        )
        .await?;
        let resolved_chart_sizing = compiled.resolve_chart_sizing(&evaluated_layout_spec)?;
        let measured_layout_spec = CompiledPlot::layout_spec_for_resolved_chart_sizing(
            &evaluated_layout_spec,
            facet_tree.as_ref(),
            resolved_chart_sizing,
        );

        let scale_eval_ctx = avenger_chart_core::EvaluationContext::new(
            compiled.get_theme(),
            Arc::new(ctx.clone()),
            merged_params.clone(),
        )
        .with_time_context(compiled.time_context.clone());
        let scale_builder = build_scale_builder_from_compiled_plot(
            compiled,
            None,
            &scale_eval_ctx,
            compiled.get_theme().as_ref(),
        )
        .await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: compiled,
        };

        let eval_ctx = EvaluationContext::new(
            compiled.get_theme(),
            Arc::new(ctx.clone()),
            merged_params,
            facet_tree,
        )
        .with_time_context(compiled.time_context.clone())
        .with_event_datum_fields(Arc::new(compiled.event_datum_types()))
        .with_facet_data_root(dataframe_from_compiled_plot_data(&compiled.data, ctx)?)
        .with_facet_runtime_sizing_mode(resolved_chart_sizing.facet_runtime_sizing_mode());

        let measurement = compiled
            .measure_plot_components(&eval_ctx, &measured_layout_spec, &provider, None, &[])
            .await?;

        Ok((eval_ctx, measured_layout_spec, scale_builder, measurement))
    }

    async fn prepare_refined_top_level_measurement(
        compiled: &CompiledPlot,
        ctx: &SessionContext,
    ) -> Result<
        (
            EvaluationContext,
            EvaluatedLayoutSpec,
            ComponentsMeasurement,
        ),
        AvengerChartError,
    > {
        let (eval_ctx, evaluated_layout_spec, scale_builder, mut measurement) =
            prepare_top_level_measurement(compiled, ctx).await?;

        let resolved_chart_sizing = compiled.resolve_chart_sizing(&evaluated_layout_spec)?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: compiled,
        };
        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Final,
                &mut measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                resolved_chart_sizing,
            )
            .await?;

        Ok((eval_ctx, evaluated_layout_spec, measurement))
    }

    fn max_overflow_abs_delta(a: &OverflowSpaceRequirement, b: &OverflowSpaceRequirement) -> f32 {
        let top = (a.top - b.top).abs();
        let right = (a.right - b.right).abs();
        let bottom = (a.bottom - b.bottom).abs();
        let left = (a.left - b.left).abs();
        top.max(right).max(bottom).max(left)
    }

    fn legend_slab_for_position(overflow: &CoordinatedOverflow, position: LegendPosition) -> f32 {
        match position {
            LegendPosition::Top => (overflow.total.top - overflow.guide.top).max(0.0),
            LegendPosition::Right => (overflow.total.right - overflow.guide.right).max(0.0),
            LegendPosition::Bottom => (overflow.total.bottom - overflow.guide.bottom).max(0.0),
            LegendPosition::Left => (overflow.total.left - overflow.guide.left).max(0.0),
        }
    }

    fn total_overflow_for_position(
        overflow: &CoordinatedOverflow,
        position: LegendPosition,
    ) -> f32 {
        match position {
            LegendPosition::Top => overflow.total.top,
            LegendPosition::Right => overflow.total.right,
            LegendPosition::Bottom => overflow.total.bottom,
            LegendPosition::Left => overflow.total.left,
        }
    }

    #[test]
    fn resolved_dimensions_encode_frame_sizing_policy() {
        let layout_spec = EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Width(640.0),
            plot_area: EvaluatedSizeMode::Height(120.0),
            margins: EvaluatedMargins {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
        };

        let dimensions = CompiledPlot::resolve_dimensions_from_spec(&layout_spec);
        let policy = dimensions.frame_sizing_policy();

        match policy.width {
            FrameDimensionSizing::CanvasConstrained { canvas_size } => {
                assert_eq!(canvas_size, 640.0);
            }
            other => panic!("expected canvas-constrained width, got {other:?}"),
        }
        match policy.height {
            FrameDimensionSizing::ContentSized { content_size } => {
                assert_eq!(content_size, 120.0);
            }
            other => panic!("expected content-sized height, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn regular_plot_resolves_single_plot_content_kind() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_regular_plot(&ctx).await?;
        let params = compiled.get_default_params().clone();
        let evaluated_layout_spec = evaluate_layout_spec(
            compiled.get_layout_spec(),
            &ctx,
            &params,
            compiled.get_theme().as_ref(),
        )
        .await?;

        let sizing = compiled.resolve_chart_sizing(&evaluated_layout_spec)?;
        assert_eq!(sizing.content_kind(), ResolvedContentKind::SinglePlot);
        assert_eq!(
            sizing.facet_runtime_sizing_mode(),
            FacetRuntimeSizingMode::CanvasFit
        );
        Ok(())
    }

    #[tokio::test]
    async fn single_plot_measurement_content_layout_has_no_child_allocations()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_regular_plot(&ctx).await?;
        let (_, _, _, measurement) = prepare_top_level_measurement(&compiled, &ctx).await?;
        let content_layout = measurement.content_layout()?;

        assert!(content_layout.child_frame_allocations.is_empty());
        assert_eq!(
            content_layout.allocation.frame,
            measurement.frame_allocation
        );
        assert_eq!(
            content_layout.allocation.content_rect,
            *measurement.layout.plot_area_bounds()
        );
        Ok(())
    }

    #[tokio::test]
    async fn single_plot_final_layout_remains_one_node_content() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_regular_plot(&ctx).await?;
        let (eval_ctx, evaluated_layout_spec, scale_builder, mut measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        let sizing = compiled.resolve_chart_sizing(&evaluated_layout_spec)?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled,
        };

        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Final,
                &mut measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                sizing,
            )
            .await?;

        let content_layout = measurement.content_layout()?;
        assert!(content_layout.child_frame_allocations.is_empty());
        assert_eq!(
            content_layout.allocation.content_rect,
            *measurement.layout.plot_area_bounds()
        );
        assert_eq!(content_layout.frame_demand, measurement.frame_demand());
        assert_eq!(
            content_layout
                .frame_demand
                .residual_overflow(measurement.frame_allocation.owned_slabs),
            measurement.frame_demand().rendered_envelope
        );
        Ok(())
    }

    #[tokio::test]
    async fn single_plot_coordination_snapshot_is_noop() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_regular_plot(&ctx).await?;
        let (eval_ctx, evaluated_layout_spec, scale_builder, mut measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        let before = measurement.content_layout()?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled,
        };

        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                    CoordinationCheckpoint::Adopted,
                )),
                &mut measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                ResolvedChartSizing::SinglePlot,
            )
            .await?;

        assert_eq!(measurement.content_layout()?, before);
        Ok(())
    }

    #[tokio::test]
    async fn facet_measurement_content_layout_exposes_child_allocations()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_plot_with_plot_size(&ctx).await?;
        let (_, _, _, measurement) = prepare_top_level_measurement(&compiled, &ctx).await?;
        let facet_band = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("fixture should produce a top-level facet measurement");
        let content_layout = measurement.content_layout()?;

        assert_eq!(
            content_layout.child_frame_allocations.len(),
            facet_band.cells.len()
        );
        assert_eq!(
            content_layout.child_frame_allocations[0],
            facet_band.cells[0].measurement.frame_allocation
        );
        Ok(())
    }

    #[tokio::test]
    async fn child_frame_container_view_exposes_facet_children() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_plot_with_plot_size(&ctx).await?;
        let (eval_ctx, _, _, mut measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        coordinate_facet_measurement_tree(&mut measurement, &eval_ctx).await?;
        let facet_band = facet_band_ref(&measurement)
            .expect("fixture should produce a top-level facet measurement");
        let container = measurement
            .child_frame_container_view()?
            .expect("facet measurement should expose a child-frame container view");

        assert_eq!(container.child_regions().len(), facet_band.cells.len());
        for child_region in container.child_regions() {
            let child_measurement = container
                .child_measurement(child_region.child_index)
                .expect("child region should resolve to a child measurement");
            assert_eq!(
                child_measurement.frame_allocation,
                facet_band.cells[child_region.child_index]
                    .measurement
                    .frame_allocation
            );
        }
        Ok(())
    }

    fn measured_legend_slab_for_position(
        measurement: &ComponentsMeasurement,
        position: LegendPosition,
    ) -> f32 {
        measurement
            .legend_plan
            .measurements
            .values()
            .filter(|legend| legend.position == position)
            .map(|legend| match position {
                LegendPosition::Top | LegendPosition::Bottom => legend.size.height,
                LegendPosition::Left | LegendPosition::Right => legend.size.width,
            })
            .fold(0.0, f32::max)
    }

    fn absolute_origins_for_named_groups(
        scene_graph: &SceneGraph,
        prefix: &str,
    ) -> Vec<(String, [f32; 2])> {
        let mut groups = scene_graph
            .group_paths()
            .into_iter()
            .filter_map(|path| {
                let SceneMark::Group(group) = scene_graph.get_mark(&path)? else {
                    return None;
                };
                if !group.name.starts_with(prefix) || group.name.ends_with("_empty") {
                    return None;
                }
                let origin = scene_graph.get_absolute_origin(&path)?;
                Some((group.name.clone(), origin))
            })
            .collect::<Vec<_>>();
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        groups
    }

    fn count_groups_with_name(scene_graph: &SceneGraph, prefix: &str, ends_with: &str) -> usize {
        scene_graph
            .group_paths()
            .into_iter()
            .filter_map(|path| scene_graph.get_mark(&path))
            .filter_map(|mark| match mark {
                SceneMark::Group(group) => Some(group),
                _ => None,
            })
            .filter(|group| group.name.starts_with(prefix) && group.name.ends_with(ends_with))
            .count()
    }

    fn count_groups_with_prefix_excluding_suffix(
        scene_graph: &SceneGraph,
        prefix: &str,
        excluded_suffix: &str,
    ) -> usize {
        scene_graph
            .group_paths()
            .into_iter()
            .filter_map(|path| scene_graph.get_mark(&path))
            .filter_map(|mark| match mark {
                SceneMark::Group(group) => Some(group),
                _ => None,
            })
            .filter(|group| {
                group.name.starts_with(prefix) && !group.name.ends_with(excluded_suffix)
            })
            .count()
    }

    fn find_scene_group_by_name<'a>(
        marks: &'a [SceneMark],
        name: &str,
    ) -> Option<&'a avenger_scenegraph::marks::group::SceneGroup> {
        for mark in marks {
            if let SceneMark::Group(group) = mark {
                if group.name == name {
                    return Some(group);
                }
                if let Some(found) = find_scene_group_by_name(&group.marks, name) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn clip_dimensions(group: &avenger_scenegraph::marks::group::SceneGroup) -> (f32, f32) {
        let Clip::Rect { width, height, .. } = group.clip else {
            panic!("expected rect clip");
        };
        (width, height)
    }

    fn first_rect_mark(group: &avenger_scenegraph::marks::group::SceneGroup) -> &SceneRectMark {
        group
            .marks
            .iter()
            .find_map(|mark| {
                if let SceneMark::Rect(rect) = mark {
                    Some(rect)
                } else {
                    None
                }
            })
            .expect("expected rect mark")
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1.0,
            "expected {actual} to be within 1px of {expected}"
        );
    }

    fn collect_text_x_positions(scene_graph: &SceneGraph, text: &str) -> Vec<f32> {
        fn collect_from_mark(mark: &SceneMark, origin: [f32; 2], text: &str, xs: &mut Vec<f32>) {
            match mark {
                SceneMark::Group(group) => {
                    let next_origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];
                    for child in &group.marks {
                        collect_from_mark(child, next_origin, text, xs);
                    }
                }
                SceneMark::Text(text_mark) => {
                    let matches = text_mark.text_iter().any(|value| value == text);
                    if !matches {
                        return;
                    }
                    if let Some(x) = text_mark.x_iter().next() {
                        xs.push(origin[0] + *x);
                    }
                }
                _ => {}
            }
        }

        let mut xs = Vec::new();
        for mark in scene_graph.children() {
            collect_from_mark(mark, [0.0, 0.0], text, &mut xs);
        }
        xs.sort_by(f32::total_cmp);
        xs
    }

    fn collect_leaf_plot_areas(
        measurement: &ComponentsMeasurement,
        leaf_widths: &mut Vec<f32>,
        leaf_heights: &mut Vec<f32>,
    ) {
        if let Some(facet_band) = facet_band_ref(measurement) {
            for child in facet_band.child_measurements_iter() {
                collect_leaf_plot_areas(child, leaf_widths, leaf_heights);
            }
        } else {
            leaf_widths.push(measurement.plot_area_width);
            leaf_heights.push(measurement.plot_area_height);
        }
    }

    fn collect_empty_plot_area_sized_facet_plot_areas(
        measurement: &ComponentsMeasurement,
        out: &mut Vec<(f32, f32)>,
    ) {
        if let Some(plot_area_sized_facet) =
            facet_band_ref(measurement).filter(|facet_band| facet_band.content_driven_main_axis())
        {
            if plot_area_sized_facet.cells.is_empty() {
                out.push((measurement.plot_area_width, measurement.plot_area_height));
            }

            for child in plot_area_sized_facet.child_measurements_iter() {
                collect_empty_plot_area_sized_facet_plot_areas(child, out);
            }
        }
    }

    fn assert_no_plot_area_sized_main_axis_overlap(measurement: &ComponentsMeasurement) {
        if let Some(facet_band) =
            facet_band_ref(measurement).filter(|facet_band| facet_band.content_driven_main_axis())
        {
            let geometry = facet_band.content_driven_geometry();
            for window in geometry.cells.windows(2) {
                let current = &window[0];
                let next = &window[1];
                assert!(
                    next.main_start + 0.01 >= current.main_start + current.main_size,
                    "plot-area-sized main-axis overlap at axis {:?}, cell_index {}, current_start={}, current_span={}, next_start={}",
                    facet_band.axis,
                    current.cell_index,
                    current.main_start,
                    current.main_size,
                    next.main_start
                );
            }

            for child in facet_band.child_measurements_iter() {
                assert_no_plot_area_sized_main_axis_overlap(child);
            }
        }
    }

    fn count_coordinated_layout_patches(measurement: &ComponentsMeasurement) -> usize {
        if let Some(facet_band) = facet_band_ref(measurement) {
            let local = usize::from(facet_band.has_coordinated_layout());
            local
                + facet_band
                    .child_measurements_iter()
                    .map(count_coordinated_layout_patches)
                    .sum::<usize>()
        } else {
            0
        }
    }

    fn assert_legends_within_canvas(measurement: &ComponentsMeasurement) {
        let canvas_width = measurement.canvas_size.0;
        let canvas_height = measurement.canvas_size.1;
        for (legend_key, bounds) in &measurement.layout.frame_layout.legends {
            assert!(
                bounds.x >= -0.5,
                "legend {legend_key} starts left of canvas: x={}, canvas_width={canvas_width}",
                bounds.x
            );
            assert!(
                bounds.y >= -0.5,
                "legend {legend_key} starts above canvas: y={}, canvas_height={canvas_height}",
                bounds.y
            );
            assert!(
                bounds.x + bounds.width <= canvas_width + 0.5,
                "legend {legend_key} exceeds canvas width: right={}, canvas_width={canvas_width}",
                bounds.x + bounds.width
            );
            assert!(
                bounds.y + bounds.height <= canvas_height + 0.5,
                "legend {legend_key} exceeds canvas height: bottom={}, canvas_height={canvas_height}",
                bounds.y + bounds.height
            );
        }

        if let Some(facet_band) = facet_band_ref(measurement) {
            for child in facet_band.child_measurements_iter() {
                assert_legends_within_canvas(child);
            }
        }
    }

    fn assert_legends_within_root_canvas(
        measurement: &ComponentsMeasurement,
        root_canvas: (f32, f32),
        origin: (f32, f32),
    ) {
        for (legend_key, bounds) in &measurement.layout.frame_layout.legends {
            let left = origin.0 + bounds.x;
            let top = origin.1 + bounds.y;
            let right = left + bounds.width;
            let bottom = top + bounds.height;
            assert!(
                left >= -0.5,
                "legend {legend_key} starts left of root canvas: left={left}, root_width={}",
                root_canvas.0
            );
            assert!(
                top >= -0.5,
                "legend {legend_key} starts above root canvas: top={top}, root_height={}",
                root_canvas.1
            );
            assert!(
                right <= root_canvas.0 + 0.5,
                "legend {legend_key} exceeds root canvas width: right={right}, root_width={}",
                root_canvas.0
            );
            assert!(
                bottom <= root_canvas.1 + 0.5,
                "legend {legend_key} exceeds root canvas height: bottom={bottom}, root_height={}",
                root_canvas.1
            );
        }

        if let Some(facet_band) = facet_band_ref(measurement) {
            let container = measurement
                .child_frame_container_view()
                .expect("resolve child-frame container")
                .expect("facet child-frame container");
            for (idx, child) in facet_band.child_measurements_iter().enumerate() {
                let child_region = container.child_region(idx).expect("child-frame region");
                let child_origin = (
                    origin.0 + child_region.content.x,
                    origin.1 + child_region.content.y,
                );
                assert_legends_within_root_canvas(child, root_canvas, child_origin);
            }
        }
    }

    fn count_legend_measurements_recursive(measurement: &ComponentsMeasurement) -> usize {
        let mut count = measurement.legend_plan.measurements.len();
        if let Some(facet_band) = facet_band_ref(measurement) {
            for child in facet_band.child_measurements_iter() {
                count += count_legend_measurements_recursive(child);
            }
        }
        count
    }

    fn assert_plot_area_sized_content_geometry_count_and_order(
        measurement: &ComponentsMeasurement,
    ) {
        if let Some(plot_area_sized_facet) =
            facet_band_ref(measurement).filter(|facet_band| facet_band.content_driven_main_axis())
        {
            let resolved = plot_area_sized_facet.content_driven_geometry();
            assert_eq!(
                resolved.cell_count(),
                plot_area_sized_facet.cells.len(),
                "resolved plot-area-sized geometry count should match facet cell count"
            );
            for (idx, cell_geometry) in resolved.cells.iter().enumerate() {
                assert_eq!(
                    cell_geometry.cell_index, idx,
                    "resolved plot-area-sized geometry should preserve cell ordering"
                );
            }

            for child in plot_area_sized_facet.child_measurements_iter() {
                assert_plot_area_sized_content_geometry_count_and_order(child);
            }
        }
    }

    fn assert_plot_area_sized_facet_plot_area_matches_geometry(
        measurement: &ComponentsMeasurement,
    ) {
        if let Some(plot_area_sized_facet) = facet_band_ref(measurement)
            .filter(|facet_band| facet_band.content_driven_main_axis())
            // Empty content-driven bands derive their extent from the containing
            // measurement's plot area, making this check an identity.
            .filter(|facet_band| !facet_band.cells.is_empty())
        {
            let (expected_width, expected_height) = plot_area_sized_facet.plot_area_extent();
            assert!(
                (measurement.plot_area_width - expected_width).abs() <= 0.01,
                "plot-area-sized facet plot width should match computed geometry: measurement={}, geometry={}",
                measurement.plot_area_width,
                expected_width
            );
            assert!(
                (measurement.plot_area_height - expected_height).abs() <= 0.01,
                "plot-area-sized facet plot height should match computed geometry: measurement={}, geometry={}",
                measurement.plot_area_height,
                expected_height
            );

            let bounds = measurement.layout.plot_area_bounds();
            assert!(
                (bounds.width - measurement.plot_area_width).abs() <= 0.01,
                "plot-area-sized facet layout bounds width should match realized plot area: bounds={}, measurement={}",
                bounds.width,
                measurement.plot_area_width
            );
            assert!(
                (bounds.height - measurement.plot_area_height).abs() <= 0.01,
                "plot-area-sized facet layout bounds height should match realized plot area: bounds={}, measurement={}",
                bounds.height,
                measurement.plot_area_height
            );

            for child in plot_area_sized_facet.child_measurements_iter() {
                assert_plot_area_sized_facet_plot_area_matches_geometry(child);
            }
        }
    }

    fn collect_team_level_plot_area_sized_apply_signals(
        measurement: &ComponentsMeasurement,
        out: &mut Vec<f32>,
    ) -> Result<(), AvengerChartError> {
        if let Some(facet_band) = facet_band_ref(measurement) {
            if facet_band.coordination_field_identity == "team" {
                let slabs = crate::facet::overflow_projection::FacetOverflowSlabs::from_coordinated(
                    facet_band.active_overflow(),
                );
                out.push(slabs.legend.right.max(0.0));
            }

            for child in facet_band.child_measurements_iter() {
                collect_team_level_plot_area_sized_apply_signals(child, out)?;
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn evaluate_default_matches_with_options_final() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;

        let default_eval = compiled.evaluate(&ctx, None).await?;
        let options_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert_eq!(
            default_eval.scene_graph.width,
            options_eval.scene_graph.width
        );
        assert_eq!(
            default_eval.scene_graph.height,
            options_eval.scene_graph.height
        );

        let default_col_origins =
            absolute_origins_for_named_groups(&default_eval.scene_graph, "facet_col_");
        let options_col_origins =
            absolute_origins_for_named_groups(&options_eval.scene_graph, "facet_col_");
        assert_eq!(default_col_origins, options_col_origins);

        let default_row_origins =
            absolute_origins_for_named_groups(&default_eval.scene_graph, "facet_row_");
        let options_row_origins =
            absolute_origins_for_named_groups(&options_eval.scene_graph, "facet_row_");
        assert_eq!(default_row_origins, options_row_origins);

        Ok(())
    }

    #[tokio::test]
    async fn evaluate_with_options_initial_and_coordinated_execute_for_nested_facets()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;

        let initial_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured),
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let coordinated_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                        CoordinationCheckpoint::Adopted,
                    )),
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert!(initial_eval.scene_graph.width > 0.0);
        assert!(initial_eval.scene_graph.height > 0.0);
        assert!(coordinated_eval.scene_graph.width > 0.0);
        assert!(coordinated_eval.scene_graph.height > 0.0);

        let initial_col_origins =
            absolute_origins_for_named_groups(&initial_eval.scene_graph, "facet_col_");
        let coordinated_col_origins =
            absolute_origins_for_named_groups(&coordinated_eval.scene_graph, "facet_col_");
        let initial_row_origins =
            absolute_origins_for_named_groups(&initial_eval.scene_graph, "facet_row_");
        let coordinated_row_origins =
            absolute_origins_for_named_groups(&coordinated_eval.scene_graph, "facet_row_");

        assert!(!initial_col_origins.is_empty());
        assert!(!coordinated_col_origins.is_empty());
        assert_eq!(initial_col_origins.len(), coordinated_col_origins.len());
        assert_eq!(initial_row_origins.len(), coordinated_row_origins.len());

        Ok(())
    }

    #[tokio::test]
    async fn facet_subtree_snapshots_render_probe_and_local_layout() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let selector = FacetSubtreeSelector::ByFacetPath(vec![
            ScalarValue::Utf8(Some("G1".to_string())),
            ScalarValue::Utf8(Some("S1".to_string())),
        ]);

        let probe_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector: selector.clone(),
                        checkpoint: FacetSubtreeCheckpoint::EstimatedOverflowProbe,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let probe_by_index_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector: FacetSubtreeSelector::ByCoordinationNodePath(vec![0, 0]),
                        checkpoint: FacetSubtreeCheckpoint::EstimatedOverflowProbe,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let local_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector,
                        checkpoint: FacetSubtreeCheckpoint::LocalRetargetedLayout,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let by_index_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector: FacetSubtreeSelector::ByCoordinationNodePath(vec![0, 0]),
                        checkpoint: FacetSubtreeCheckpoint::LocalRetargetedLayout,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert!(probe_eval.scene_graph.width > 0.0);
        assert!(probe_eval.scene_graph.height > 0.0);
        assert_eq!(
            probe_eval.scene_graph.width,
            probe_by_index_eval.scene_graph.width
        );
        assert_eq!(
            probe_eval.scene_graph.height,
            probe_by_index_eval.scene_graph.height
        );
        assert!(local_eval.scene_graph.width > 0.0);
        assert!(local_eval.scene_graph.height > 0.0);
        assert_eq!(
            local_eval.scene_graph.width,
            by_index_eval.scene_graph.width
        );
        assert_eq!(
            local_eval.scene_graph.height,
            by_index_eval.scene_graph.height
        );

        Ok(())
    }

    #[tokio::test]
    async fn evaluation_metrics_capture_facet_recursive_counts() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
        let (_evaluated, metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured),
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let facet_metrics = &metrics.facet_layout;

        assert_eq!(
            facet_metrics
                .plot_component_measure_calls_by_facet_depth
                .first()
                .copied(),
            Some(1),
            "expected one top-level measurement: {metrics:?}"
        );
        assert!(
            facet_metrics
                .plot_component_measure_calls_by_facet_depth
                .len()
                >= 4,
            "expected top-level plus three facet path depths: {metrics:?}"
        );
        assert!(
            facet_metrics.plot_component_measure_calls_by_facet_depth[3] > 0,
            "expected leaf-depth measurements: {metrics:?}"
        );
        assert!(
            facet_metrics.facet_band_measure_runs > 0,
            "expected facet-band measurement pipelines: {metrics:?}"
        );
        assert!(
            facet_metrics.estimated_overflow_non_leaf_aggregate_count > 0,
            "expected non-leaf estimated-overflow probe aggregates: {metrics:?}"
        );
        assert!(
            facet_metrics.estimated_overflow_non_leaf_full_measure_count > 0,
            "expected non-leaf estimated-overflow probes to require full subtree measurement: {metrics:?}"
        );
        assert!(
            facet_metrics.plot_component_measure_calls <= 128,
            "unexpected recursive measurement regression: {metrics:?}"
        );
        assert_eq!(
            metrics.pipeline.facet_tree_builds, 1,
            "evaluation should build one top-level facet tree: {metrics:?}"
        );
        assert!(
            metrics.pipeline.scale_builder_builds > 1,
            "faceted layout should build top-level and child scale builders: {metrics:?}"
        );
        assert!(
            metrics.pipeline.scale_domain_collects > 0,
            "faceted layout should collect scale-domain data: {metrics:?}"
        );
        assert!(
            metrics.pipeline.guide_overflow_measure_calls > 0,
            "facet evaluation should measure guide overflow: {metrics:?}"
        );
        assert!(
            metrics.pipeline.legend_plan_builds > 0,
            "facet evaluation should build legend plans: {metrics:?}"
        );
        assert!(
            metrics.pipeline.legend_measurements > 0,
            "legend-sharing fixture should measure legends: {metrics:?}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn evaluation_metrics_capture_regular_repeated_pipeline_work()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_regular_plot(&ctx).await?;
        let (_first, first_metrics) = compiled
            .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
            .await?;
        let (_second, second_metrics) = compiled
            .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
            .await?;

        for metrics in [&first_metrics, &second_metrics] {
            assert_eq!(
                metrics.pipeline.facet_tree_builds, 1,
                "one-shot evaluation should rebuild the facet tree each time: {metrics:?}"
            );
            assert_eq!(
                metrics.pipeline.scale_builder_builds, 1,
                "one-shot regular evaluation should rebuild its top-level scale builder: {metrics:?}"
            );
            assert!(
                metrics.pipeline.guide_overflow_measure_calls > 0,
                "regular evaluation should measure axis overflow: {metrics:?}"
            );
            assert!(
                metrics.pipeline.scale_domain_collects > 0,
                "regular evaluation should collect scale-domain data: {metrics:?}"
            );
            assert!(
                metrics.pipeline.legend_plan_builds > 0,
                "regular evaluation should still build an empty legend plan: {metrics:?}"
            );
            assert!(
                metrics.pipeline.mark_data_collects > 0,
                "regular evaluation should collect mark data during render: {metrics:?}"
            );
            assert_eq!(
                metrics.facet_layout.plot_component_measure_calls, 1,
                "regular evaluation should measure one plot component: {metrics:?}"
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn evaluation_metrics_capture_facet_plot_repeated_pipeline_work()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_plot_with_plot_size(&ctx).await?;
        let (_first, first_metrics) = compiled
            .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
            .await?;
        let (_second, second_metrics) = compiled
            .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
            .await?;

        for metrics in [&first_metrics, &second_metrics] {
            assert_eq!(
                metrics.pipeline.facet_tree_builds, 1,
                "one-shot facet evaluation should rebuild the facet tree each time: {metrics:?}"
            );
            assert!(
                metrics.pipeline.scale_builder_builds > 1,
                "facet evaluation should build top-level and child scale builders: {metrics:?}"
            );
            assert!(
                metrics.pipeline.scale_domain_collects > 0,
                "facet evaluation should collect scale-domain data: {metrics:?}"
            );
            assert!(
                metrics.pipeline.guide_overflow_measure_calls > 1,
                "facet evaluation should measure guide overflow for parent and child plots: {metrics:?}"
            );
            assert!(
                metrics.facet_layout.plot_component_measure_calls > 1,
                "facet evaluation should recursively measure child plots: {metrics:?}"
            );
            assert!(
                metrics.facet_layout.facet_band_measure_runs > 0,
                "facet evaluation should run facet-band measurement: {metrics:?}"
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn evaluation_metrics_capture_wrap_resize_baseline_work() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_wrap_plot(&ctx).await?;
        let (_evaluated, metrics) = compiled
            .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
            .await?;

        assert_eq!(
            metrics.pipeline.facet_tree_builds, 1,
            "wrap evaluation should build one semantic/physical facet tree: {metrics:?}"
        );
        assert!(
            metrics.pipeline.scale_builder_builds > 1,
            "wrap evaluation should build top-level and child scale builders: {metrics:?}"
        );
        assert!(
            metrics.pipeline.scale_domain_collects > 0,
            "wrap evaluation should collect scale-domain data: {metrics:?}"
        );
        assert!(
            metrics.facet_layout.plot_component_measure_calls > 1,
            "wrap evaluation should recursively measure wrapped cells: {metrics:?}"
        );
        assert!(
            metrics.facet_layout.facet_band_measure_runs > 0,
            "wrap evaluation should run facet-band measurement: {metrics:?}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn evaluation_metrics_capture_domain_param_update_work() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_domain_param_regular_plot(&ctx).await?;
        let (_default_eval, default_metrics) = compiled
            .evaluate_with_options_and_metrics(&ctx, None, EvaluationOptions::default())
            .await?;

        let mut updated_params = indexmap::IndexMap::new();
        updated_params.insert("x0".to_string(), ScalarValue::Float64(Some(2.0)));
        updated_params.insert("x1".to_string(), ScalarValue::Float64(Some(6.0)));
        let (_updated_eval, updated_metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                Some(updated_params),
                EvaluationOptions::default(),
            )
            .await?;

        for metrics in [&default_metrics, &updated_metrics] {
            assert_eq!(
                metrics.pipeline.facet_tree_builds, 1,
                "domain-param evaluation should still build the top-level tree: {metrics:?}"
            );
            assert_eq!(
                metrics.pipeline.scale_builder_builds, 1,
                "domain-param evaluation should rebuild the scale builder in the one-shot path: {metrics:?}"
            );
            assert!(
                metrics.pipeline.scale_domain_collects > 0,
                "domain-param evaluation should collect scale-domain data: {metrics:?}"
            );
            assert!(
                metrics.pipeline.guide_overflow_measure_calls > 0,
                "domain-param evaluation should remeasure guide overflow: {metrics:?}"
            );
            assert!(
                metrics.pipeline.mark_data_collects > 0,
                "domain-param evaluation should collect mark data during render: {metrics:?}"
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn final_snapshot_fast_measure_once_has_no_refinement_passes()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
        let (_evaluated, metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    facet_layout_refinement: FacetLayoutRefinement {
                        max_refinement_passes: 0,
                        overflow_growth_epsilon: 0.5,
                    },
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let facet_metrics = &metrics.facet_layout;

        assert_eq!(
            facet_metrics.refinement_pass_count, 0,
            "max_refinement_passes=0 should not run extra measurement passes: {metrics:?}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn canvas_refinement_remeasures_after_plot_area_retarget() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_two_level_col_col_refinement_plot(&ctx).await?;
        let selector = FacetSubtreeSelector::ByFacetPath(vec![
            ScalarValue::Utf8(Some("Eng".to_string())),
            ScalarValue::Utf8(Some("Frontend".to_string())),
        ]);

        let (fast_eval, fast_metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector: selector.clone(),
                        checkpoint: FacetSubtreeCheckpoint::FinalLayout,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    facet_layout_refinement: FacetLayoutRefinement {
                        max_refinement_passes: 0,
                        overflow_growth_epsilon: 0.5,
                    },
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let (refined_eval, refined_metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector,
                        checkpoint: FacetSubtreeCheckpoint::FinalLayout,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    facet_layout_refinement: FacetLayoutRefinement {
                        max_refinement_passes: 1,
                        overflow_growth_epsilon: 0.5,
                    },
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert_eq!(fast_metrics.facet_layout.refinement_pass_count, 0);
        assert!(
            refined_metrics.facet_layout.refinement_pass_count >= 1,
            "expected an additional canvas refinement pass: {refined_metrics:?}"
        );
        assert!(
            refined_metrics.facet_layout.plot_component_measure_calls
                > fast_metrics.facet_layout.plot_component_measure_calls,
            "refinement should remeasure at the retargeted subplot size; fast={fast_metrics:?}, refined={refined_metrics:?}"
        );
        // The fast zero-refinement snapshot renders from the current geometry
        // available at that point. The late right overflow is only guaranteed
        // after the refinement remeasure below.
        let _ = fast_eval;
        assert!(
            !collect_text_x_positions(&refined_eval.scene_graph, "of-right").is_empty(),
            "refined subtree should allocate the late right overflow"
        );

        Ok(())
    }

    #[tokio::test]
    async fn plot_area_sized_refinement_remeasures_after_domain_coordination()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_nested_row_col_row_mixed_sharing_plot_area_sized(&ctx).await?;
        let selector = FacetSubtreeSelector::ByFacetPath(vec![
            ScalarValue::Utf8(Some("DivB".to_string())),
            ScalarValue::Utf8(Some("Dept1".to_string())),
            ScalarValue::Utf8(Some("Team2".to_string())),
        ]);

        let (fast_eval, fast_metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector: selector.clone(),
                        checkpoint: FacetSubtreeCheckpoint::FinalLayout,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    facet_layout_refinement: FacetLayoutRefinement {
                        max_refinement_passes: 0,
                        overflow_growth_epsilon: 0.5,
                    },
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let (refined_eval, refined_metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::FacetSubtree(FacetSubtreeSnapshot {
                        selector,
                        checkpoint: FacetSubtreeCheckpoint::FinalLayout,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    facet_layout_refinement: FacetLayoutRefinement {
                        max_refinement_passes: 1,
                        overflow_growth_epsilon: 0.5,
                    },
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert_eq!(fast_metrics.facet_layout.refinement_pass_count, 0);
        assert_eq!(
            refined_metrics.facet_layout.refinement_pass_count, 1,
            "expected one plot-area-sized refinement pass: {refined_metrics:?}"
        );
        assert!(
            refined_metrics.facet_layout.plot_component_measure_calls
                > fast_metrics.facet_layout.plot_component_measure_calls,
            "plot-area-sized refinement should remeasure at the realized subplot size; fast={fast_metrics:?}, refined={refined_metrics:?}"
        );
        let _ = (fast_eval, refined_eval);

        Ok(())
    }

    #[tokio::test]
    async fn canvas_refinement_snapshot_reuses_iteration_remeasure_path()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_two_level_col_col_refinement_plot(&ctx).await?;
        let (_evaluated, metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::Refinement {
                        iteration: 1,
                        checkpoint: RefinementCheckpoint::Recoordinated,
                    }),
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    facet_layout_refinement: FacetLayoutRefinement {
                        max_refinement_passes: 1,
                        overflow_growth_epsilon: 0.5,
                    },
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert!(
            metrics.facet_layout.plot_component_measure_calls > 7,
            "iteration-1 snapshot should run the refinement remeasure path: {metrics:?}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn initial_snapshot_ignores_facet_refinement_budget() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
        let (_evaluated, metrics) = compiled
            .evaluate_with_options_and_metrics(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured),
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    facet_layout_refinement: FacetLayoutRefinement {
                        max_refinement_passes: 3,
                        overflow_growth_epsilon: 0.5,
                    },
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert_eq!(
            metrics.facet_layout.refinement_pass_count, 0,
            "Initial snapshot should stop before final refinement: {metrics:?}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn evaluate_with_options_coordinated_vs_final_canvas_mode()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let coordinated_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                        CoordinationCheckpoint::Adopted,
                    )),
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let final_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        assert!(coordinated_eval.scene_graph.width > 0.0);
        assert!(coordinated_eval.scene_graph.height > 0.0);
        assert!(final_eval.scene_graph.width > 0.0);
        assert!(final_eval.scene_graph.height > 0.0);

        let (eval_ctx, evaluated_layout_spec, scale_builder, mut coordinated_measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        let resolved_chart_sizing = compiled.resolve_chart_sizing(&evaluated_layout_spec)?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled,
        };
        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                    CoordinationCheckpoint::Adopted,
                )),
                &mut coordinated_measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                resolved_chart_sizing,
            )
            .await?;

        let coordinated_bounds = coordinated_measurement.layout.plot_area_bounds();
        let coordinated_delta_w =
            (coordinated_measurement.plot_area_width - coordinated_bounds.width).abs();
        let coordinated_delta_h =
            (coordinated_measurement.plot_area_height - coordinated_bounds.height).abs();

        let (eval_ctx, evaluated_layout_spec, scale_builder, mut final_measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled,
        };
        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Final,
                &mut final_measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                resolved_chart_sizing,
            )
            .await?;

        let final_bounds = final_measurement.layout.plot_area_bounds();
        let final_delta_w = (final_measurement.plot_area_width - final_bounds.width).abs();
        let final_delta_h = (final_measurement.plot_area_height - final_bounds.height).abs();

        assert!(final_delta_w <= CompiledPlot::LAYOUT_REFINEMENT_EPSILON);
        assert!(final_delta_h <= CompiledPlot::LAYOUT_REFINEMENT_EPSILON);
        assert!(final_delta_w <= coordinated_delta_w + 1e-6);
        assert!(final_delta_h <= coordinated_delta_h + 1e-6);

        Ok(())
    }

    #[tokio::test]
    async fn facet_plot_size_mode_detected_on_top_level_facet_root() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_plot_with_plot_size(&ctx).await?;
        let params = compiled.get_default_params().clone();
        let evaluated_layout_spec = evaluate_layout_spec(
            compiled.get_layout_spec(),
            &ctx,
            &params,
            compiled.get_theme().as_ref(),
        )
        .await?;

        let strategy = compiled.resolve_chart_sizing(&evaluated_layout_spec)?;
        assert!(
            matches!(
                strategy,
                ResolvedChartSizing::FacetBand(FacetRuntimeSizingPolicy {
                    width: FacetDimensionSizing::LeafPlotAreaSized {
                        leaf_plot_size: 120.0
                    },
                    height: FacetDimensionSizing::LeafPlotAreaSized {
                        leaf_plot_size: 90.0
                    },
                })
            ),
            "expected plot-area-sized strategy for faceted plot_size"
        );
        Ok(())
    }

    #[tokio::test]
    async fn facet_plot_size_mode_evaluates_successfully() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_plot_with_plot_size(&ctx).await?;
        let evaluated = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        assert!(evaluated.scene_graph.width > 0.0);
        assert!(evaluated.scene_graph.height > 0.0);
        Ok(())
    }

    #[tokio::test]
    async fn facet_canvas_and_plot_size_combination_errors() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let compiled = build_simple_facet_col_plot(df)
            .canvas_size(640.0, 420.0)
            .plot_size(120.0, 90.0)
            .compile(&ctx)
            .await
            .expect("compile facet plot with both canvas_size and plot_size");

        let err = match compiled
            .evaluate_with_options(&ctx, None, EvaluationOptions::default())
            .await
        {
            Ok(_) => panic!("facet chart with canvas_size + plot_size should error"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            message.contains("cannot constrain both canvas width and leaf plot width"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn facet_partial_plot_width_constraint_evaluates() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let compiled = build_simple_facet_col_plot(df)
            .plot_constraint(PlotConstraint::width(200.0))
            .compile(&ctx)
            .await
            .expect("compile facet plot with plot_constraint");

        let evaluated = compiled
            .evaluate_with_options(&ctx, None, EvaluationOptions::default())
            .await
            .expect("faceted partial plot constraint should evaluate");
        assert!(
            evaluated.scene_graph.width > 200.0,
            "canvas should include leaf plot width plus guide overflow"
        );
    }

    #[tokio::test]
    async fn facet_wrap_without_public_guide_reserves_child_overflow()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_simple_facet_wrap_plot(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let wrap_measurement =
            facet_band_ref(&measurement).expect("FacetWrap should measure as a facet band");
        let measured_subtree_overflow = wrap_measurement
            .measured_overflow_value()
            .expect("FacetWrap should carry measured subtree overflow");

        assert!(
            measured_subtree_overflow.total.top > 20.0,
            "test setup should measure visible child facet guide overflow: {:?}",
            measured_subtree_overflow
        );
        assert!(
            measurement.layout.total_overflow.top > 20.0,
            "FacetWrap has no public guide, but its measured child facet guide overflow must still reserve root canvas space: {:?}",
            measurement.layout.total_overflow
        );
        assert!(
            measurement.layout.frame_layout.plot_area.y
                >= measurement.layout.total_overflow.top - 1.0,
            "root plot area should be shifted below propagated FacetWrap child overflow: plot_y={} overflow_top={}",
            measurement.layout.frame_layout.plot_area.y,
            measurement.layout.total_overflow.top
        );
        Ok(())
    }

    #[tokio::test]
    async fn canvas_sized_facet_wrap_keeps_main_axis_canvas_constrained()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let compiled = build_canvas_sized_facet_wrap_plot(df).compile(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let wrap_measurement =
            facet_band_ref(&measurement).expect("FacetWrap should measure as a facet band");

        assert!(
            !wrap_measurement.content_driven_main_axis(),
            "canvas-sized wrap rows must distribute available canvas height instead of growing from nested content"
        );

        Ok(())
    }

    #[tokio::test]
    async fn facet_mixed_canvas_width_leaf_plot_height_preserves_policy()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let compiled = build_deeply_nested_plot(df)
            .canvas_constraint(CanvasConstraint::width(640.0))
            .plot_constraint(PlotConstraint::height(72.0))
            .compile(&ctx)
            .await
            .expect("compile mixed dimension facet plot");

        let evaluated = compiled
            .evaluate_with_options(&ctx, None, EvaluationOptions::default())
            .await?;
        assert!(
            (evaluated.scene_graph.width - 640.0).abs() <= 0.5,
            "mixed chart should preserve constrained canvas width: {}",
            evaluated.scene_graph.width
        );

        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let mut leaf_widths = Vec::new();
        let mut leaf_heights = Vec::new();
        collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
        assert!(
            !leaf_heights.is_empty(),
            "expected at least one leaf measurement"
        );
        assert!(
            leaf_widths.iter().any(|width| (*width - 72.0).abs() > 0.5),
            "widths should be solved from canvas, not fixed to the requested leaf height"
        );
        for height in leaf_heights {
            assert!(
                (height - 72.0).abs() <= 0.5,
                "leaf plot height should remain fixed in mixed mode: {height}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn nested_subplot_plot_size_under_facet_errors() {
        let ctx = SessionContext::new();
        let df = deeply_nested_dataframe(&ctx);
        let plot = Plot::<FacetColumn>::new()
            .data(df)
            .plot_size(120.0, 90.0)
            .mark(
                Subplot::new(
                    Plot::<Cartesian>::new()
                        .plot_size(80.0, 60.0)
                        .mark(Symbol::new().x(col("value")).y(col("value")).size(24.0)),
                )
                .column(col("outer_group")),
            );

        let compiled = plot
            .compile(&ctx)
            .await
            .expect("compile nested subplot plot_size test");
        let err = match compiled
            .evaluate_with_options(&ctx, None, EvaluationOptions::default())
            .await
        {
            Ok(_) => panic!("nested subplot plot_size under facet should error"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            message.contains("cannot set `plot_size(...)` when used under a facet"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn plot_area_sized_mode_leaf_plot_area_uniform_for_three_level_col_col_col_with_level2_right_legend()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot_area_sized(&ctx, LegendPosition::Top)
                .await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let mut leaf_widths = Vec::new();
        let mut leaf_heights = Vec::new();
        collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
        assert!(
            !leaf_widths.is_empty(),
            "expected at least one leaf measurement"
        );
        assert_eq!(leaf_widths.len(), leaf_heights.len());

        let first_width = leaf_widths[0];
        let first_height = leaf_heights[0];
        for (idx, width) in leaf_widths.iter().enumerate() {
            assert!(
                (width - first_width).abs() <= 0.01,
                "leaf width mismatch at index {idx}: width={width}, baseline={first_width}"
            );
            assert!(
                (*width - 120.0).abs() <= 0.01,
                "leaf width should match fixed plot_size width: width={width}"
            );
        }
        for (idx, height) in leaf_heights.iter().enumerate() {
            assert!(
                (height - first_height).abs() <= 0.01,
                "leaf height mismatch at index {idx}: height={height}, baseline={first_height}"
            );
            assert!(
                (*height - 90.0).abs() <= 0.01,
                "leaf height should match fixed plot_size height: height={height}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn fixed_level2_right_hoists_legend_without_team_node_slab()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot_area_sized(&ctx, LegendPosition::Right)
                .await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let mut team_signals = Vec::new();
        collect_team_level_plot_area_sized_apply_signals(&measurement, &mut team_signals)?;
        assert!(
            !team_signals.is_empty(),
            "expected at least one team-level facet node in level2-right scenario"
        );

        let nodes_with_right_legend_slab = team_signals
            .iter()
            .filter(|right_slab| **right_slab > 0.0)
            .count();
        assert_eq!(
            nodes_with_right_legend_slab, 0,
            "level-2 shared legends should be hoisted to their sharing group instead of owned by team-level nodes"
        );
        assert!(
            count_legend_measurements_recursive(&measurement) > 0,
            "hoisted right legend should still be measured in the final layout"
        );
        assert_legends_within_canvas(&measurement);
        Ok(())
    }

    #[tokio::test]
    async fn legend_disposition_polar_positioned_child_requests_are_collected()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = build_polar_positioned_legend_sharing_plot(
            positioned_legend_sharing_dataframe(&ctx).await,
        )
        .compile(&ctx)
        .await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        assert_eq!(
            measurement.legend_plan.measurements.len(),
            1,
            "the shared positioned-subplot legend should be measured by the polar parent"
        );
        assert!(
            measurement.legend_plan.hoisted_requests.is_empty(),
            "the polar parent should consume child legend requests anchored to its frame"
        );
        Ok(())
    }

    #[tokio::test]
    async fn plot_area_sized_mode_collection_only_applies_layout_patches_without_resizing_leaves()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot_area_sized(&ctx, LegendPosition::Right)
                .await?;
        let (eval_ctx, evaluated_layout_spec, scale_builder, mut measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled,
        };
        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                    CoordinationCheckpoint::Adopted,
                )),
                &mut measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                fully_leaf_sizing_policy(120.0, 90.0),
            )
            .await?;

        let patch_count = count_coordinated_layout_patches(&measurement);
        assert!(
            patch_count > 0,
            "expected plot-area-sized full-cycle mode to apply at least one coordinated layout patch"
        );

        let mut leaf_widths = Vec::new();
        let mut leaf_heights = Vec::new();
        collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
        assert!(
            leaf_widths.iter().all(|w| (*w - 120.0).abs() <= 0.01),
            "plot-area-sized full-cycle path must keep leaf plot widths locked"
        );
        assert!(
            leaf_heights.iter().all(|h| (*h - 90.0).abs() <= 0.01),
            "plot-area-sized full-cycle path must keep leaf plot heights locked"
        );
        Ok(())
    }

    #[tokio::test]
    async fn plot_area_sized_mode_final_realization_applies_layout_patches_without_resizing_leaves()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot_area_sized(&ctx, LegendPosition::Right)
                .await?;
        let (eval_ctx, evaluated_layout_spec, scale_builder, mut measurement) =
            prepare_top_level_measurement(&compiled, &ctx).await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: &compiled,
        };
        compiled
            .apply_layout_snapshot(
                &LayoutSnapshot::Final,
                &mut measurement,
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                fully_leaf_sizing_policy(120.0, 90.0),
            )
            .await?;

        let patch_count = count_coordinated_layout_patches(&measurement);
        assert!(
            patch_count > 0,
            "expected plot-area-sized final realization to preserve coordinated layout patches"
        );

        let mut leaf_widths = Vec::new();
        let mut leaf_heights = Vec::new();
        collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
        assert!(
            leaf_widths.iter().all(|w| (*w - 120.0).abs() <= 0.01),
            "plot-area-sized final realization must keep leaf plot widths locked"
        );
        assert!(
            leaf_heights.iter().all(|h| (*h - 90.0).abs() <= 0.01),
            "plot-area-sized final realization must keep leaf plot heights locked"
        );
        Ok(())
    }

    #[tokio::test]
    async fn plot_area_sized_mode_has_no_main_axis_overlap_for_three_level_col_col_col_with_level2_right_legend()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot_area_sized(&ctx, LegendPosition::Right)
                .await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        assert_no_plot_area_sized_main_axis_overlap(&measurement);
        Ok(())
    }

    #[tokio::test]
    async fn fixed_level2_right_legends_within_canvas() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot_area_sized(&ctx, LegendPosition::Right)
                .await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        assert_legends_within_canvas(&measurement);
        Ok(())
    }

    #[tokio::test]
    async fn plot_area_sized_mode_realizes_plot_area_from_computed_geometry()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot_area_sized(&ctx, LegendPosition::Right)
                .await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        assert_plot_area_sized_facet_plot_area_matches_geometry(&measurement);
        Ok(())
    }

    #[tokio::test]
    async fn plot_area_sized_mode_preserves_empty_nested_slot_extent()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_sparse_fixed_column_hole_plot(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let mut empty_slot_sizes = Vec::new();
        collect_empty_plot_area_sized_facet_plot_areas(&measurement, &mut empty_slot_sizes);
        assert!(
            !empty_slot_sizes.is_empty(),
            "test fixture should contain empty plot-area-sized facet slots"
        );
        for (width, height) in empty_slot_sizes {
            assert!(
                (width - 80.0).abs() <= 0.01,
                "empty plot-area-sized facet slot width should preserve the incoming leaf plot area: {width}"
            );
            assert!(
                (height - 60.0).abs() <= 0.01,
                "empty plot-area-sized facet slot height should preserve the incoming leaf plot area: {height}"
            );
        }
        assert_plot_area_sized_facet_plot_area_matches_geometry(&measurement);
        assert_plot_area_sized_content_geometry_count_and_order(&measurement);
        Ok(())
    }

    #[tokio::test]
    async fn plot_area_sized_mode_continuous_legend_is_visible_with_uniform_leaf_sizes()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_nested_col_row_col_continuous_legend_plot_area_sized(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let legend_measurement_count = count_legend_measurements_recursive(&measurement);
        assert!(
            legend_measurement_count > 0,
            "expected at least one legend measurement in plot-area-sized continuous legend scenario"
        );
        assert_legends_within_canvas(&measurement);

        let mut leaf_widths = Vec::new();
        let mut leaf_heights = Vec::new();
        collect_leaf_plot_areas(&measurement, &mut leaf_widths, &mut leaf_heights);
        assert!(
            !leaf_widths.is_empty(),
            "expected non-empty leaf measurements"
        );
        for width in leaf_widths {
            assert!(
                (width - 110.0).abs() <= 0.01,
                "plot-area-sized width drifted from configured width: {width}"
            );
        }
        for height in leaf_heights {
            assert!(
                (height - 80.0).abs() <= 0.01,
                "plot-area-sized height drifted from configured height: {height}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn debug_layout_overlay_option_enables_components_without_env()
    -> Result<(), AvengerChartError> {
        if crate::facet::debug::env_layout_overlay_enabled() {
            return Ok(());
        }

        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let base_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_overlay: LayoutDebugOverlayMode::Off,
                    ..EvaluationOptions::default()
                },
            )
            .await?;
        let debug_eval = compiled
            .evaluate_with_options(
                &ctx,
                None,
                EvaluationOptions {
                    layout_snapshot: LayoutSnapshot::Final,
                    debug_layout_overlay: LayoutDebugOverlayMode::Components,
                    ..EvaluationOptions::default()
                },
            )
            .await?;

        let base_plot_area_labels = collect_text_x_positions(&base_eval.scene_graph, "plot-area");
        let debug_plot_area_labels = collect_text_x_positions(&debug_eval.scene_graph, "plot-area");

        assert_eq!(
            base_plot_area_labels.len(),
            0,
            "baseline evaluation unexpectedly has layout debug labels"
        );
        assert!(
            !debug_plot_area_labels.is_empty(),
            "debug option should enable layout overlay labels"
        );
        assert!(
            debug_plot_area_labels.len() > 1,
            "debug option should include nested facet/subplot layout overlay labels"
        );

        Ok(())
    }

    #[tokio::test]
    async fn canvas_mode_measurement_matches_final_layout_bounds() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let bounds = measurement.layout.plot_area_bounds();
        let delta_w = (measurement.plot_area_width - bounds.width).abs();
        let delta_h = (measurement.plot_area_height - bounds.height).abs();
        assert!(
            delta_w <= CompiledPlot::LAYOUT_REFINEMENT_EPSILON,
            "plot area width mismatch after refinement: measurement={} layout={} delta={}",
            measurement.plot_area_width,
            bounds.width,
            delta_w
        );
        assert!(
            delta_h <= CompiledPlot::LAYOUT_REFINEMENT_EPSILON,
            "plot area height mismatch after refinement: measurement={} layout={} delta={}",
            measurement.plot_area_height,
            bounds.height,
            delta_h
        );
        Ok(())
    }

    #[tokio::test]
    async fn level1_left_legend_slab_contributes_to_rendered_subtree()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Left).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level legend sharing test should measure as FacetBandCoordMeasurement");
        let outer_rendered_overflow = outer_facet
            .measured_rendered_subtree_overflow_value()
            .expect("outer facet should expose measured rendered-subtree overflow");
        let outer_left_overflow =
            total_overflow_for_position(&outer_rendered_overflow, LegendPosition::Left);

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected non-empty outer facet cell");
        facet_band_ref(&first_non_empty.measurement)
            .expect("expected nested child facet measurement");
        let child_left_slab =
            measured_legend_slab_for_position(&first_non_empty.measurement, LegendPosition::Left);

        assert!(
            outer_left_overflow > 1.0,
            "outer rendered-subtree overflow should include descendant left-side demand (found {})",
            outer_left_overflow
        );
        assert!(
            child_left_slab > 1.0,
            "child facet layout should retain its own left legend slab (found {})",
            child_left_slab
        );
        Ok(())
    }

    #[tokio::test]
    async fn level1_right_legend_slab_contributes_to_rendered_subtree()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level legend sharing test should measure as FacetBandCoordMeasurement");
        let outer_rendered_overflow = outer_facet
            .measured_rendered_subtree_overflow_value()
            .expect("outer facet should expose measured rendered-subtree overflow");
        let outer_right_overflow =
            total_overflow_for_position(&outer_rendered_overflow, LegendPosition::Right);

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected non-empty outer facet cell");
        facet_band_ref(&first_non_empty.measurement)
            .expect("expected nested child facet measurement");
        let child_right_slab =
            measured_legend_slab_for_position(&first_non_empty.measurement, LegendPosition::Right);

        assert!(
            outer_right_overflow > 1.0,
            "outer rendered-subtree overflow should include descendant right-side demand (found {})",
            outer_right_overflow
        );
        assert!(
            child_right_slab > 1.0,
            "child facet layout should retain its own right legend slab (found {})",
            child_right_slab
        );
        Ok(())
    }

    #[tokio::test]
    async fn level0_right_free_legends_stay_inside_canvas() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_two_level_col_free_legend_plot(&ctx, LegendPosition::Right).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        assert_legends_within_canvas(&measurement);
        assert_legends_within_root_canvas(&measurement, measurement.canvas_size, (0.0, 0.0));
        Ok(())
    }

    #[tokio::test]
    async fn level1_bottom_legend_slab_contributes_to_rendered_subtree()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Bottom).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level legend sharing test should measure as FacetBandCoordMeasurement");
        let outer_rendered_overflow = outer_facet
            .measured_rendered_subtree_overflow_value()
            .expect("outer facet should expose measured rendered-subtree overflow");
        let outer_bottom_overflow =
            total_overflow_for_position(&outer_rendered_overflow, LegendPosition::Bottom);

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected non-empty outer facet cell");
        facet_band_ref(&first_non_empty.measurement)
            .expect("expected nested child facet measurement");
        let child_bottom_slab =
            measured_legend_slab_for_position(&first_non_empty.measurement, LegendPosition::Bottom);

        assert!(
            outer_bottom_overflow > 1.0,
            "outer rendered-subtree overflow should include descendant bottom-side demand (found {})",
            outer_bottom_overflow
        );
        assert!(
            child_bottom_slab > 1.0,
            "child facet layout should retain its own bottom legend slab (found {})",
            child_bottom_slab
        );
        Ok(())
    }

    #[tokio::test]
    async fn level1_top_legend_slab_contributes_to_rendered_subtree()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Top).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level legend sharing test should measure as FacetBandCoordMeasurement");
        let outer_rendered_overflow = outer_facet
            .measured_rendered_subtree_overflow_value()
            .expect("outer facet should expose measured rendered-subtree overflow");
        let outer_top_overflow =
            total_overflow_for_position(&outer_rendered_overflow, LegendPosition::Top);

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected non-empty outer facet cell");
        let child_facet = facet_band_ref(&first_non_empty.measurement)
            .expect("expected nested child facet measurement");
        let child_top_slab =
            measured_legend_slab_for_position(&first_non_empty.measurement, LegendPosition::Top);
        assert!(
            outer_top_overflow > 1.0,
            "outer rendered-subtree overflow should include descendant top-side demand (found {})",
            outer_top_overflow
        );
        assert!(
            child_top_slab > 1.0,
            "child facet layout should retain its own top legend slab (found {})",
            child_top_slab
        );
        assert!(
            child_facet.cells.iter().any(|cell| !cell.plan.is_empty),
            "expected top-legend child facet to retain non-empty subplot cells"
        );
        Ok(())
    }

    #[tokio::test]
    async fn row_facet_group_origins_include_main_axis_legend_start_slab()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_single_level_row_legend_plot(&ctx, LegendPosition::Left).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let row_facet = facet_band_ref(&measurement)
            .expect("row facet origin invariant expects FacetBandCoordMeasurement");
        let legend_start =
            legend_slab_for_position(row_facet.active_overflow(), LegendPosition::Left);
        let base_x = measurement.layout.plot_area_bounds().x;
        let base_y = measurement.layout.plot_area_bounds().y;
        let container = measurement
            .child_frame_container_view()?
            .expect("row facet measurement should expose current child-frame geometry");
        let expected_y_starts: Vec<f32> = container
            .child_regions()
            .iter()
            .map(|region| base_y + region.content.y)
            .collect();
        let row_origins = absolute_origins_for_named_groups(&evaluated.scene_graph, "facet_row_");
        assert!(
            !row_origins.is_empty(),
            "expected non-empty row facet groups in evaluated scene"
        );
        assert_eq!(
            row_origins.len(),
            expected_y_starts.len(),
            "row facet groups should match row band count"
        );

        for ((name, origin), expected_y) in row_origins.iter().zip(expected_y_starts.iter()) {
            assert!(
                (origin[0] - (base_x + legend_start)).abs() <= 1.0,
                "row facet group {} should include left legend start slab at x (origin_x={}, expected={})",
                name,
                origin[0],
                base_x + legend_start
            );
            assert!(
                (origin[1] - *expected_y).abs() <= 1.0,
                "row facet group {} should align to row band start y (origin_y={}, expected={})",
                name,
                origin[1],
                expected_y
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn col_facet_group_origins_include_main_axis_legend_start_slab()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_single_level_col_legend_plot(&ctx, LegendPosition::Top).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let col_facet = facet_band_ref(&measurement)
            .expect("col facet origin invariant expects FacetBandCoordMeasurement");
        let legend_start =
            legend_slab_for_position(col_facet.active_overflow(), LegendPosition::Top);
        let base_x = measurement.layout.plot_area_bounds().x;
        let base_y = measurement.layout.plot_area_bounds().y;
        let container = measurement
            .child_frame_container_view()?
            .expect("column facet measurement should expose current child-frame geometry");
        let expected_x_starts: Vec<f32> = container
            .child_regions()
            .iter()
            .map(|region| base_x + region.content.x)
            .collect();
        let col_origins = absolute_origins_for_named_groups(&evaluated.scene_graph, "facet_col_");
        assert!(
            !col_origins.is_empty(),
            "expected non-empty col facet groups in evaluated scene"
        );
        assert_eq!(
            col_origins.len(),
            expected_x_starts.len(),
            "col facet groups should match column band count"
        );

        for ((name, origin), expected_x) in col_origins.iter().zip(expected_x_starts.iter()) {
            assert!(
                (origin[1] - (base_y + legend_start)).abs() <= 1.0,
                "col facet group {} should include top legend start slab at y (origin_y={}, expected={})",
                name,
                origin[1],
                base_y + legend_start
            );
            assert!(
                (origin[0] - *expected_x).abs() <= 1.0,
                "col facet group {} should align to column band start x (origin_x={}, expected={})",
                name,
                origin[0],
                expected_x
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn level0_right_outer_labels_align_with_child_department_titles()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_two_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let mut division_label_centers = Vec::new();
        for label in ["DivA", "DivB"] {
            let xs = collect_text_x_positions(&evaluated.scene_graph, label);
            assert_eq!(xs.len(), 1, "expected exactly one {:?} label", label);
            division_label_centers.push(xs[0]);
        }
        division_label_centers.sort_by(f32::total_cmp);

        let mut department_title_centers =
            collect_text_x_positions(&evaluated.scene_graph, "department");
        assert_eq!(
            department_title_centers.len(),
            2,
            "expected one inner 'department' title per outer division"
        );
        department_title_centers.sort_by(f32::total_cmp);

        for (division_x, department_x) in division_label_centers
            .iter()
            .zip(department_title_centers.iter())
        {
            assert!(
                (division_x - department_x).abs() <= 2.0,
                "outer division label should align to child department title center (division_x={}, department_x={})",
                division_x,
                department_x
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn level2_right_outer_labels_align_with_child_department_titles()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled =
            compile_three_level_col_legend_sharing_plot(&ctx, LegendPosition::Right).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let mut division_label_centers = Vec::new();
        for label in ["DivA", "DivB"] {
            let xs = collect_text_x_positions(&evaluated.scene_graph, label);
            assert_eq!(xs.len(), 1, "expected exactly one {:?} label", label);
            division_label_centers.push(xs[0]);
        }
        division_label_centers.sort_by(f32::total_cmp);

        let mut department_title_centers =
            collect_text_x_positions(&evaluated.scene_graph, "department");
        assert_eq!(
            department_title_centers.len(),
            2,
            "expected one inner 'department' title per outer division"
        );
        department_title_centers.sort_by(f32::total_cmp);

        for (division_x, department_x) in division_label_centers
            .iter()
            .zip(department_title_centers.iter())
        {
            assert!(
                (division_x - department_x).abs() <= 2.0,
                "outer division label should align to child department title center (division_x={}, department_x={})",
                division_x,
                department_x
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn shared_row_basic_right_owner_renders_each_row_label_once_globally()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_nested_shared_row_basic_plot(&ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        for label in ["R1", "R2", "R3"] {
            let xs = collect_text_x_positions(&evaluated.scene_graph, label);
            assert_eq!(
                xs.len(),
                1,
                "expected shared-row label {:?} to render exactly once on owning column",
                label
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn empty_cell_policy_controls_whether_empty_slots_render_subplots()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let hole_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::Hole,
        )
        .await?;
        let hole_scene = hole_plot.evaluate(&ctx, None).await?;

        let ctx = SessionContext::new();
        let empty_subplot_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::EmptySubplot,
        )
        .await?;
        let empty_subplot_scene = empty_subplot_plot.evaluate(&ctx, None).await?;

        let ctx = SessionContext::new();
        let auto_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::Auto,
        )
        .await?;
        let auto_scene = auto_plot.evaluate(&ctx, None).await?;

        let hole_empty_groups =
            count_groups_with_name(&hole_scene.scene_graph, "facet_row_", "_empty");
        let empty_subplot_empty_groups =
            count_groups_with_name(&empty_subplot_scene.scene_graph, "facet_row_", "_empty");
        let auto_empty_groups =
            count_groups_with_name(&auto_scene.scene_graph, "facet_row_", "_empty");

        assert!(
            hole_empty_groups > 0,
            "hole policy should render explicit empty groups for empty slots"
        );
        assert_eq!(
            empty_subplot_empty_groups, 0,
            "empty subplot policy should render full subplot groups instead of *_empty placeholders"
        );
        assert_eq!(
            auto_empty_groups, hole_empty_groups,
            "auto policy should resolve to hole in this release"
        );

        Ok(())
    }

    #[tokio::test]
    async fn empty_subplot_policy_renders_structural_subplot_groups_for_empty_slots()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let hole_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::Hole,
        )
        .await?;
        let hole_scene = hole_plot.evaluate(&ctx, None).await?;

        let ctx = SessionContext::new();
        let empty_subplot_plot = compile_nested_shared_row_shared_both_plot_with_empty_policy(
            &ctx,
            FacetEmptyCellPolicy::EmptySubplot,
        )
        .await?;
        let empty_subplot_scene = empty_subplot_plot.evaluate(&ctx, None).await?;

        let hole_empty_groups =
            count_groups_with_name(&hole_scene.scene_graph, "facet_row_", "_empty");
        let hole_subplot_groups = count_groups_with_prefix_excluding_suffix(
            &hole_scene.scene_graph,
            "facet_row_",
            "_empty",
        );
        let empty_subplot_groups = count_groups_with_prefix_excluding_suffix(
            &empty_subplot_scene.scene_graph,
            "facet_row_",
            "_empty",
        );

        assert!(
            hole_empty_groups > 0,
            "fixture should include hole placeholders so policy replacement can be validated"
        );
        assert!(
            empty_subplot_groups > hole_subplot_groups,
            "empty subplot policy should add subplot groups in slots that are holes under hole policy"
        );
        assert_eq!(
            empty_subplot_groups,
            hole_subplot_groups + hole_empty_groups,
            "empty subplot policy should replace each hole placeholder with a structural subplot group"
        );

        Ok(())
    }

    #[tokio::test]
    async fn data_empty_shared_cells_receive_coordinated_domain_extents()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_nested_shared_row_shared_both_plot(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("expected top-level FacetBandCoordMeasurement for shared-both probe");

        let mut covered_cell_count = 0usize;
        for outer_cell in &outer_facet.cells {
            let inner_row = facet_band_ref(&outer_cell.measurement)
                .expect("expected nested row FacetBandCoordMeasurement");

            for cell in &inner_row.cells {
                if !cell.plan.is_empty {
                    covered_cell_count += 1;
                    assert!(
                        cell.local_domain_extents.contains_key("x")
                            || cell.coordinated_domain_extents.contains_key("x"),
                        "shared cell {:?} missing local or coordinated x extent",
                        cell.plan.full_path
                    );
                    assert!(
                        cell.local_domain_extents.contains_key("y")
                            || cell.coordinated_domain_extents.contains_key("y"),
                        "shared cell {:?} missing local or coordinated y extent",
                        cell.plan.full_path
                    );
                }
            }
        }

        assert!(
            covered_cell_count > 0,
            "expected at least one non-hole shared cell with domain extent coverage"
        );

        Ok(())
    }

    #[tokio::test]
    async fn jagged_group_local_shared_row_keeps_one_owner_column_per_group()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_jagged_group_local_shared_row_plot(&ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;

        let mut a_owner_xs = Vec::new();
        for label in ["A1", "A2", "A3"] {
            let xs = collect_text_x_positions(&evaluated.scene_graph, label);
            assert_eq!(
                xs.len(),
                1,
                "expected group-A label {:?} to render once on group-local owner column",
                label
            );
            a_owner_xs.push(xs[0]);
        }

        let mut b_owner_xs = Vec::new();
        for label in ["B1", "B2"] {
            let xs = collect_text_x_positions(&evaluated.scene_graph, label);
            assert_eq!(
                xs.len(),
                1,
                "expected group-B label {:?} to render once on group-local owner column",
                label
            );
            b_owner_xs.push(xs[0]);
        }

        let a_owner_x = a_owner_xs[0];
        for x in &a_owner_xs {
            assert!(
                (*x - a_owner_x).abs() <= 1.0,
                "expected group-A row labels to share one owner column x (got {:?})",
                a_owner_xs
            );
        }

        let b_owner_x = b_owner_xs[0];
        for x in &b_owner_xs {
            assert!(
                (*x - b_owner_x).abs() <= 1.0,
                "expected group-B row labels to share one owner column x (got {:?})",
                b_owner_xs
            );
        }

        Ok(())
    }

    #[tokio::test]
    async fn nested_sparse_col_title_centers_over_coordinated_slot_span()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_nested_sparse_row_plot(&ctx).await?;
        let evaluated = compiled.evaluate(&ctx, None).await?;
        let title_positions = collect_text_x_positions(&evaluated.scene_graph, "petal_width_bin");
        assert_eq!(
            title_positions.len(),
            1,
            "expected exactly one visible petal_width_bin title"
        );
        let title_x = title_positions[0];

        let narrow_label_positions = collect_text_x_positions(&evaluated.scene_graph, "narrow");
        assert_eq!(
            narrow_label_positions.len(),
            1,
            "expected exactly one visible narrow label in top sparse row"
        );
        let narrow_x = narrow_label_positions[0];
        let medium_positions = collect_text_x_positions(&evaluated.scene_graph, "medium");
        let wide_positions = collect_text_x_positions(&evaluated.scene_graph, "wide");
        assert!(
            !medium_positions.is_empty() && !wide_positions.is_empty(),
            "expected visible medium/wide labels to infer coordinated slot pitch"
        );
        let slot_step = (wide_positions[0] - medium_positions[0]).abs();
        assert!(
            slot_step > 1.0,
            "expected positive coordinated slot pitch, got {}",
            slot_step
        );
        let expected_title_x = narrow_x + 0.5 * slot_step;

        assert!(
            (title_x - expected_title_x).abs() <= 2.0,
            "expected petal_width_bin title x={} to align with coordinated slot midpoint {}",
            title_x,
            expected_title_x
        );

        Ok(())
    }

    #[tokio::test]
    async fn nested_plot_area_measurements_use_coord_aware_overflow()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let (eval_ctx, _, measurement) =
            prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level deeply nested test should measure as FacetBandCoordMeasurement");

        let first_non_empty = outer_facet
            .cells
            .iter()
            .find(|cell| !cell.plan.is_empty)
            .expect("expected at least one non-empty outer facet cell");

        let child_measurement = &first_non_empty.measurement;
        let child_layout_spec = EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Auto,
            plot_area: EvaluatedSizeMode::Fixed {
                width: child_measurement.plot_area_width,
                height: child_measurement.plot_area_height,
            },
            margins: EvaluatedMargins {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
        };

        let child_plot = outer_facet.compiled_subplot.as_ref();
        let (_, coord_aware_layout, _) = child_plot
            .rebuild_layout_with_coord_overflow(
                &eval_ctx,
                &child_layout_spec,
                &child_measurement.scales,
                child_measurement.plot_area_width,
                child_measurement.plot_area_height,
                &child_measurement.params,
                Some(&first_non_empty.data_override),
                &eval_ctx.session_context,
                eval_ctx.facet_tree.as_ref(),
                &first_non_empty.plan.full_path,
                eval_ctx.child_frame_sharing_path(),
                Some(child_measurement.coord_measurement.as_ref()),
                GuideOverflowPhase::Final,
            )
            .await?;
        let (_, no_coord_layout, _) = child_plot
            .rebuild_layout_with_coord_overflow(
                &eval_ctx,
                &child_layout_spec,
                &child_measurement.scales,
                child_measurement.plot_area_width,
                child_measurement.plot_area_height,
                &child_measurement.params,
                Some(&first_non_empty.data_override),
                &eval_ctx.session_context,
                eval_ctx.facet_tree.as_ref(),
                &first_non_empty.plan.full_path,
                eval_ctx.child_frame_sharing_path(),
                None,
                GuideOverflowPhase::Measurement,
            )
            .await?;

        let guide_delta_coord = max_overflow_abs_delta(
            &child_measurement.layout.overflow,
            &coord_aware_layout.overflow,
        );
        let total_delta_coord = max_overflow_abs_delta(
            &child_measurement.layout.total_overflow,
            &coord_aware_layout.total_overflow,
        );
        let guide_delta_no_coord = max_overflow_abs_delta(
            &child_measurement.layout.overflow,
            &no_coord_layout.overflow,
        );
        let total_delta_no_coord = max_overflow_abs_delta(
            &child_measurement.layout.total_overflow,
            &no_coord_layout.total_overflow,
        );
        assert!(
            guide_delta_coord <= 1.0,
            "nested child measurement guide overflow should come from coord-aware pass (delta={})",
            guide_delta_coord
        );
        assert!(
            total_delta_coord <= 1.0,
            "nested child measurement total overflow should come from coord-aware pass (delta={})",
            total_delta_coord
        );
        assert!(
            guide_delta_no_coord >= guide_delta_coord + 0.5
                || total_delta_no_coord >= total_delta_coord + 0.5,
            "coord-aware overflow should be materially closer than coord-agnostic overflow (guide coord={} no_coord={}, total coord={} no_coord={})",
            guide_delta_coord,
            guide_delta_no_coord,
            total_delta_coord,
            total_delta_no_coord
        );
        Ok(())
    }

    #[tokio::test]
    async fn deeply_nested_outer_padding_inner_not_pathological() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = compile_deeply_nested_plot(&ctx).await?;
        let (_, _, measurement) = prepare_refined_top_level_measurement(&compiled, &ctx).await?;

        let outer_facet = facet_band_ref(&measurement)
            .expect("top-level deeply nested test should measure as FacetBandCoordMeasurement");
        let active_layout = outer_facet.active_layout();

        assert!(
            active_layout.padding_inner_px < 180.0,
            "outer facet padding_inner_px regressed into pathological range: {}",
            active_layout.padding_inner_px
        );
        Ok(())
    }
}
