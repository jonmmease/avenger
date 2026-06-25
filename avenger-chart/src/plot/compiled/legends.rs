//! Legend construction and configuration for CompiledPlot

use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_color::ColorOrGradient;
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    mark::SceneMark,
};
use datafusion::{
    arrow::{
        array::Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    logical_expr::{Expr, lit},
    prelude::SessionContext,
};
use indexmap::IndexMap;
use tracing::debug;

use avenger_chart_core::{
    Auto, ChannelInfo, CompiledSelectionSpec, ConfiguredScaleLegendExt, DomainValues,
    EmptyCoordMeasurement, LegendChannel, LegendContinuousOrientation, LegendContinuousSurface,
    LegendPosition, LegendRenderItem, LegendRenderer, LegendRendererSelection, MergeKey,
    ScalarValueHelpers, Scale, SharingLevel, apply_opacity_to_color, one_row_batch_from_scalars,
    params_to_datafusion,
};
use avenger_scales::scales::{ConfiguredScale, DomainKind};

use crate::{
    channel::value::ChannelValue,
    coords::extract_channel_title_from_marks,
    error::AvengerChartError,
    facet::{evaluated_facet_tree::EvaluatedFacetTree, sharing_policy},
    layout::{FrameLayout, LayoutBounds, Size2D},
    legend::{Legend, renderer_for_kind},
    marks::{CompiledMark, default_channel_value_for_eval},
    plot::compiled::{
        ChildFrameSharingPath, ContainerPathSegment, CoordinationKind, EdgeOwnershipRequest,
        MarkDataRequest, edge_ownership_scope_for_request, prepare_mark_data_runtime,
    },
    render::{
        EvaluatedEventDatumRows, EvaluatedInteractionScope, EvaluationContext, InteractionScopeId,
        InteractionScopeKind, LegendMeasurements, RenderContext, RenderState,
        types::LegendMeasurement,
    },
    scales::{ConfiguredScaleWithSpec, Linear, Time},
    serialization::LogicalExprNodeExt,
};
use avenger_chart_cartesian::Cartesian;
use avenger_chart_legend::{
    apply_legend_theme_defaults, measure_legend_size_with_channels, themed_default_legend,
};

use super::CompiledPlot;
use super::mark_data_runtime::expand_selection_predicates_with_fallback_specs;

pub(super) struct RenderedLegendMarks {
    pub marks: Vec<SceneMark>,
    pub event_datums: Vec<EvaluatedEventDatumRows>,
    pub interaction_scopes: Vec<EvaluatedInteractionScope>,
}

fn legend_item_event_datums(
    legend_index: usize,
    surface_keys: &[String],
    legend_id: Option<&str>,
    items: Vec<LegendRenderItem>,
) -> Result<Vec<EvaluatedEventDatumRows>, AvengerChartError> {
    use avenger_chart_core::event::{
        LEGEND_CHANNEL_FIELD, LEGEND_ID_FIELD, LEGEND_INDEX_FIELD, LEGEND_LABEL_FIELD,
        LEGEND_NAME_FIELD, LEGEND_SURFACE_KEY_FIELD, LEGEND_SURFACE_KIND_DISCRETE_ITEM,
        LEGEND_SURFACE_KIND_FIELD, LEGEND_VALUE_FIELD,
    };

    let schema = Arc::new(Schema::new(vec![
        Field::new(LEGEND_VALUE_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_LABEL_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_NAME_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_CHANNEL_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_INDEX_FIELD, DataType::Int64, true),
        Field::new(LEGEND_ID_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_SURFACE_KEY_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_SURFACE_KIND_FIELD, DataType::Utf8, true),
    ]));

    items
        .into_iter()
        .map(|item| {
            let mut values = HashMap::new();
            values.insert(
                LEGEND_VALUE_FIELD.to_string(),
                ScalarValue::Utf8(Some(item.value)),
            );
            values.insert(
                LEGEND_LABEL_FIELD.to_string(),
                ScalarValue::Utf8(Some(item.label)),
            );
            values.insert(
                LEGEND_NAME_FIELD.to_string(),
                ScalarValue::Utf8(Some(item.name)),
            );
            values.insert(
                LEGEND_CHANNEL_FIELD.to_string(),
                ScalarValue::Utf8(Some(item.channel)),
            );
            values.insert(
                LEGEND_INDEX_FIELD.to_string(),
                ScalarValue::Int64(Some(item.index as i64)),
            );
            values.insert(
                LEGEND_ID_FIELD.to_string(),
                ScalarValue::Utf8(legend_id.map(str::to_string)),
            );
            values.insert(
                LEGEND_SURFACE_KEY_FIELD.to_string(),
                ScalarValue::Utf8(Some(surface_keys.join("\u{1f}"))),
            );
            values.insert(
                LEGEND_SURFACE_KIND_FIELD.to_string(),
                ScalarValue::Utf8(Some(LEGEND_SURFACE_KIND_DISCRETE_ITEM.to_string())),
            );
            let rows: RecordBatch = one_row_batch_from_scalars(schema.clone(), &values)?;
            let mut mark_path = Vec::with_capacity(item.hit_rect_path.len() + 1);
            mark_path.push(legend_index);
            mark_path.extend(item.hit_rect_path);
            Ok(EvaluatedEventDatumRows {
                mark_path,
                subplot_id_path: Vec::new(),
                rows,
            })
        })
        .collect()
}

fn legend_continuous_surface_event_datums(
    legend_index: usize,
    surface_keys: &[String],
    legend_id: Option<&str>,
    surfaces: Vec<LegendContinuousSurface>,
) -> Result<Vec<EvaluatedEventDatumRows>, AvengerChartError> {
    use avenger_chart_core::event::{
        LEGEND_BAND_CHANNEL_FIELD, LEGEND_CHANNEL_FIELD, LEGEND_ID_FIELD, LEGEND_NAME_FIELD,
        LEGEND_ORIENTATION_FIELD, LEGEND_SURFACE_KEY_FIELD,
        LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR, LEGEND_SURFACE_KIND_FIELD,
        LEGEND_VALUE_CHANNEL_FIELD,
    };

    let schema = Arc::new(Schema::new(vec![
        Field::new(LEGEND_NAME_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_CHANNEL_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_ID_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_SURFACE_KEY_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_SURFACE_KIND_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_ORIENTATION_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_VALUE_CHANNEL_FIELD, DataType::Utf8, true),
        Field::new(LEGEND_BAND_CHANNEL_FIELD, DataType::Utf8, true),
    ]));

    surfaces
        .into_iter()
        .map(|surface| {
            let mut values = HashMap::new();
            values.insert(
                LEGEND_NAME_FIELD.to_string(),
                ScalarValue::Utf8(Some(surface.name)),
            );
            values.insert(
                LEGEND_CHANNEL_FIELD.to_string(),
                ScalarValue::Utf8(Some(surface.channel)),
            );
            values.insert(
                LEGEND_ID_FIELD.to_string(),
                ScalarValue::Utf8(surface.legend_id.or_else(|| legend_id.map(str::to_string))),
            );
            values.insert(
                LEGEND_SURFACE_KEY_FIELD.to_string(),
                ScalarValue::Utf8(Some(if surface_keys.is_empty() {
                    surface.surface_key
                } else {
                    surface_keys.join("\u{1f}")
                })),
            );
            values.insert(
                LEGEND_SURFACE_KIND_FIELD.to_string(),
                ScalarValue::Utf8(Some(LEGEND_SURFACE_KIND_CONTINUOUS_COLORBAR.to_string())),
            );
            values.insert(
                LEGEND_ORIENTATION_FIELD.to_string(),
                ScalarValue::Utf8(Some(
                    legend_orientation_string(surface.orientation).to_string(),
                )),
            );
            values.insert(
                LEGEND_VALUE_CHANNEL_FIELD.to_string(),
                ScalarValue::Utf8(Some(surface.value_channel)),
            );
            values.insert(
                LEGEND_BAND_CHANNEL_FIELD.to_string(),
                ScalarValue::Utf8(Some(surface.band_channel)),
            );
            let rows: RecordBatch = one_row_batch_from_scalars(schema.clone(), &values)?;
            let mut mark_path = Vec::with_capacity(surface.hit_rect_path.len() + 1);
            mark_path.push(legend_index);
            mark_path.extend(surface.hit_rect_path);
            Ok(EvaluatedEventDatumRows {
                mark_path,
                subplot_id_path: Vec::new(),
                rows,
            })
        })
        .collect()
}

fn legend_orientation_string(orientation: LegendContinuousOrientation) -> &'static str {
    match orientation {
        LegendContinuousOrientation::Top => "top",
        LegendContinuousOrientation::Bottom => "bottom",
        LegendContinuousOrientation::Left => "left",
        LegendContinuousOrientation::Right => "right",
    }
}

fn legend_colorbar_scope_id(surface_key: &str) -> String {
    format!("legend-colorbar:{surface_key}")
}

fn legend_continuous_surface_interaction_scopes(
    plot_area: LayoutBounds,
    legend_origin: [f32; 2],
    surfaces: &[LegendContinuousSurface],
) -> Vec<EvaluatedInteractionScope> {
    surfaces
        .iter()
        .filter_map(|surface| {
            if surface.kind != avenger_chart_core::LegendSurfaceKind::ContinuousColorbar {
                return None;
            }
            let local_bounds = LayoutBounds {
                x: legend_origin[0] + surface.bounds.x - plot_area.x,
                y: legend_origin[1] + surface.bounds.y - plot_area.y,
                width: surface.bounds.width,
                height: surface.bounds.height,
            };
            let mut scales = HashMap::new();
            scales.insert(surface.value_channel.clone(), surface.value_scale.clone());
            scales.insert(surface.band_channel.clone(), surface.band_scale.clone());
            Some(EvaluatedInteractionScope {
                id: InteractionScopeId(0),
                kind: InteractionScopeKind::LegendColorbar,
                scope_id: legend_colorbar_scope_id(&surface.surface_key),
                bounds: local_bounds,
                plot_area_width: surface.bounds.width,
                plot_area_height: surface.bounds.height,
                facet_path: Vec::new(),
                logical_facet_values: Vec::new(),
                coord_node_path: Vec::new(),
                subplot_id_path: Vec::new(),
                child_frame_path: Vec::new(),
                coord_transform: Box::new(Cartesian::new()),
                channels: vec![surface.value_channel.clone(), surface.band_channel.clone()],
                scales,
                sharing_owner_paths: HashMap::new(),
            })
        })
        .collect()
}

fn configured_scale_with_spec(
    spec: Scale<Auto>,
    configured: ConfiguredScale,
) -> ConfiguredScaleWithSpec {
    ConfiguredScaleWithSpec::new(spec, configured)
}

fn colorbar_value_scale_spec(configured: &ConfiguredScale) -> Scale<Auto> {
    match configured.scale_impl.domain_kind() {
        DomainKind::Temporal => Scale::<Time>::new().into_auto(),
        _ => Scale::<Linear>::new().into_auto(),
    }
}

fn colorbar_overlay_scales(
    surface: &LegendContinuousSurface,
) -> HashMap<String, ConfiguredScaleWithSpec> {
    let value_spec = colorbar_value_scale_spec(&surface.value_scale);
    let cross_spec = Scale::<Linear>::new().into_auto();
    let mut scales = HashMap::new();
    if surface.value_channel == "x" {
        scales.insert(
            "x".to_string(),
            configured_scale_with_spec(value_spec.clone(), surface.value_scale.clone()),
        );
        scales.insert(
            "y".to_string(),
            configured_scale_with_spec(cross_spec, surface.band_scale.clone()),
        );
    } else {
        scales.insert(
            "x".to_string(),
            configured_scale_with_spec(cross_spec, surface.band_scale.clone()),
        );
        scales.insert(
            "y".to_string(),
            configured_scale_with_spec(value_spec, surface.value_scale.clone()),
        );
    }
    scales
}

fn set_interactive_recursive(mark: &mut SceneMark, interactive: bool) {
    mark.set_interactive(interactive);
    if let SceneMark::Group(group) = mark {
        for child in &mut group.marks {
            set_interactive_recursive(child, interactive);
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

fn scene_group_at_path_mut<'a>(
    group: &'a mut SceneGroup,
    path: &[usize],
) -> Option<&'a mut SceneGroup> {
    if path.is_empty() {
        return Some(group);
    }
    let (first, rest) = path.split_first()?;
    let mark = group.marks.get_mut(*first)?;
    match mark {
        SceneMark::Group(child_group) => scene_group_at_path_mut(child_group, rest),
        _ => None,
    }
}

fn insert_colorbar_overlay_group(
    legend_group: &mut SceneGroup,
    surface: &LegendContinuousSurface,
    overlay_marks: Vec<SceneMark>,
) {
    if overlay_marks.is_empty() {
        return;
    }
    let Some(surface_group) = scene_group_at_path_mut(legend_group, &surface.surface_group_path)
    else {
        return;
    };
    let local_gradient_index = surface
        .gradient_rect_path
        .last()
        .copied()
        .unwrap_or(surface_group.marks.len().saturating_sub(1));
    let insert_index = local_gradient_index
        .saturating_add(1)
        .min(surface_group.marks.len());
    let overlay_group = SceneGroup {
        name: format!("{}-colorbar-overlays", surface.surface_key),
        interactive: false,
        clip: Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: surface.bounds.width,
            height: surface.bounds.height,
        },
        marks: overlay_marks,
        ..Default::default()
    };
    surface_group
        .marks
        .insert(insert_index, SceneMark::Group(overlay_group));
}

async fn apply_related_legend_item_opacity(
    eval_ctx: &EvaluationContext,
    selection_specs: &IndexMap<String, CompiledSelectionSpec>,
    channels: &[LegendChannel],
    legend_group: &mut avenger_scenegraph::marks::group::SceneGroup,
    items: &[LegendRenderItem],
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<(), AvengerChartError> {
    if channels
        .iter()
        .any(|channel| channel.channel_type == "opacity")
    {
        return Ok(());
    }
    let Some(primary_channel) = channels.first() else {
        return Ok(());
    };
    let Some(opacity_expr) = primary_channel
        .related_channels
        .get("opacity")
        .and_then(related_channel_expression)
    else {
        return Ok(());
    };
    let Some(primary_expr) = &primary_channel.expression else {
        return Ok(());
    };
    let primary_refs = primary_expr.column_refs();
    if primary_refs.len() != 1 {
        return Ok(());
    }
    let primary_column = primary_refs
        .iter()
        .next()
        .map(|column| column.name.clone())
        .unwrap_or_default();
    if primary_column.is_empty() {
        return Ok(());
    }
    let domain_values = match primary_channel.scale.domain_values()? {
        DomainValues::Discrete(values) => values,
        DomainValues::Interval(_, _) => return Ok(()),
    };
    if domain_values.is_empty() || items.is_empty() {
        return Ok(());
    }

    let mut available_columns = HashSet::new();
    available_columns.insert(primary_column.clone());
    let opacity_expr = expand_selection_predicates_with_fallback_specs(
        opacity_expr,
        eval_ctx,
        Some(&available_columns),
        Some(selection_specs),
    )?;
    if opacity_expr
        .column_refs()
        .iter()
        .any(|column| !available_columns.contains(&column.name))
    {
        return Ok(());
    }
    let domain_array = ScalarValue::iter_to_array(domain_values.into_iter())?;
    let schema = Arc::new(Schema::new(vec![Field::new(
        primary_column,
        domain_array.data_type().clone(),
        true,
    )]));
    let batch = RecordBatch::try_new(schema, vec![domain_array])?;
    let mut df = ctx.read_batch(batch)?;
    df = df.select(vec![opacity_expr.alias("__legend_item_opacity")])?;
    if let Some(datafusion_params) = params_to_datafusion(params) {
        df = df.with_param_values(datafusion_params)?;
    }
    let batches = df.collect().await?;
    let Some(batch) = batches.first() else {
        return Ok(());
    };
    let Some(opacity_array) = batch.column_by_name("__legend_item_opacity") else {
        return Ok(());
    };
    for (index, item) in items.iter().enumerate() {
        if index >= opacity_array.len() {
            break;
        }
        let opacity = ScalarValue::try_from_array(opacity_array.as_ref(), index)?
            .as_f32()
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        apply_opacity_to_legend_item(legend_group, &item.group_path, opacity);
    }
    Ok(())
}

fn related_channel_expression(info: &ChannelInfo) -> Option<Expr> {
    match info {
        ChannelInfo::Constant { expr }
        | ChannelInfo::Scaled {
            expr: Some(expr), ..
        } => Some(expr.clone()),
        ChannelInfo::Scaled { expr: None, .. } => None,
    }
}

fn apply_opacity_to_legend_item(
    legend_group: &mut avenger_scenegraph::marks::group::SceneGroup,
    group_path: &[usize],
    opacity: f32,
) {
    let Some(SceneMark::Group(item_group)) = scene_group_mark_at_path_mut(legend_group, group_path)
    else {
        return;
    };
    for mark in item_group
        .marks
        .iter_mut()
        .filter(|mark| !mark.interactive())
    {
        apply_opacity_to_scene_mark(mark, opacity);
    }
}

fn scene_group_mark_at_path_mut<'a>(
    group: &'a mut avenger_scenegraph::marks::group::SceneGroup,
    path: &[usize],
) -> Option<&'a mut SceneMark> {
    let (first, rest) = path.split_first()?;
    let mark = group.marks.get_mut(*first)?;
    if rest.is_empty() {
        return Some(mark);
    }
    match mark {
        SceneMark::Group(child_group) => scene_group_mark_at_path_mut(child_group, rest),
        _ => None,
    }
}

fn apply_opacity_to_scene_mark(mark: &mut SceneMark, opacity: f32) {
    match mark {
        SceneMark::Symbol(symbol) => {
            apply_opacity_to_color_values(&mut symbol.fill, opacity);
            apply_opacity_to_color_values(&mut symbol.stroke, opacity);
        }
        SceneMark::Rect(rect) => {
            apply_opacity_to_color_values(&mut rect.fill, opacity);
            apply_opacity_to_color_values(&mut rect.stroke, opacity);
        }
        SceneMark::Line(line) => {
            line.stroke = apply_opacity_to_color(&line.stroke, opacity);
        }
        SceneMark::Rule(rule) => {
            apply_opacity_to_color_values(&mut rule.stroke, opacity);
        }
        SceneMark::Area(area) => {
            area.fill = apply_opacity_to_color(&area.fill, opacity);
        }
        SceneMark::Group(group) => {
            for child in &mut group.marks {
                apply_opacity_to_scene_mark(child, opacity);
            }
        }
        SceneMark::Arc(_)
        | SceneMark::Path(_)
        | SceneMark::Trail(_)
        | SceneMark::Text(_)
        | SceneMark::Image(_) => {}
    }
}

fn apply_opacity_to_color_values(values: &mut ScalarOrArray<ColorOrGradient>, opacity: f32) {
    *values = match values.value() {
        ScalarOrArrayValue::Scalar(color) => {
            ScalarOrArray::new_scalar(apply_opacity_to_color(color, opacity))
        }
        ScalarOrArrayValue::Array(colors) => ScalarOrArray::new_array(
            colors
                .iter()
                .map(|color| apply_opacity_to_color(color, opacity))
                .collect(),
        ),
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegendPlanScope {
    TopLevel,
    FacetCell,
    ChildFrame { sharing_path: ChildFrameSharingPath },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum HoistedLegendAnchor {
    FacetPath(Vec<ScalarValue>),
    ChildFrameContainer(Vec<ContainerPathSegment>),
}

#[derive(Clone)]
pub(crate) struct PreparedLegendGroup {
    pub layout_key: String,
    pub primary_channel: String,
    pub channels: Vec<LegendChannel>,
    pub legend: Arc<Legend>,
    pub renderer: Arc<dyn LegendRenderer>,
}

#[derive(Clone)]
pub(crate) struct HoistedLegendRequest {
    pub anchor: HoistedLegendAnchor,
    pub owner: HoistedLegendAnchor,
    pub position: LegendPosition,
    pub sharing_level: SharingLevel,
    pub group: PreparedLegendGroup,
}

#[derive(Clone, Default)]
pub(crate) struct PreparedLegendPlan {
    pub groups: Vec<PreparedLegendGroup>,
    pub measurements: LegendMeasurements,
    pub hoisted_requests: Vec<HoistedLegendRequest>,
}

impl PreparedLegendPlan {
    pub(crate) fn retarget_scales(
        &mut self,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) {
        for group in &mut self.groups {
            retarget_legend_group_scales(group, configured_scales);
        }

        for request in &mut self.hoisted_requests {
            retarget_legend_group_scales(&mut request.group, configured_scales);
        }
    }
}

fn retarget_legend_group_scales(
    group: &mut PreparedLegendGroup,
    configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
) {
    for channel in &mut group.channels {
        if let Some(scale) = configured_scales.get(&channel.name) {
            channel.scale = scale.configured().clone();
        }

        for (related_name, related_channel) in &mut channel.related_channels {
            if let (
                Some(scale),
                ChannelInfo::Scaled {
                    scale: related_scale,
                    ..
                },
            ) = (configured_scales.get(related_name), related_channel)
            {
                *related_scale = scale.configured().clone();
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LegendDisposition {
    RenderHere,
    Hoist {
        anchor: HoistedLegendAnchor,
        sharing_level: SharingLevel,
    },
    Suppress,
}

impl CompiledPlot {
    fn effective_group_sharing_level(
        facet_tree: &EvaluatedFacetTree,
        channels: &[LegendChannel],
        default_sharing_level: SharingLevel,
    ) -> SharingLevel {
        channels
            .iter()
            .map(|channel| {
                channel.sharing_level.map_or_else(
                    || {
                        if default_sharing_level.is_free() {
                            SharingLevel::FREE
                        } else {
                            facet_tree.channel_domain_sharing_level_typed(channel.name.as_str())
                        }
                    },
                    SharingLevel::from_raw,
                )
            })
            .min()
            .unwrap_or(default_sharing_level)
    }

    #[cfg(test)]
    fn legend_visible_for_facet_cell(
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        sharing_level: SharingLevel,
        legend_position: LegendPosition,
        primary_channel: &str,
    ) -> bool {
        if sharing_level.is_free() {
            return true;
        }

        if facet_path.is_empty() {
            return true;
        }

        let Some(resolved) = facet_tree.resolve_path_info(facet_path) else {
            return true;
        };

        if resolved.indices.is_empty() {
            return true;
        }

        sharing_policy::legend_ownership_scope(
            primary_channel,
            facet_path,
            &resolved.indices,
            &resolved.local_level_counts,
            resolved.indices.len() as u8,
            sharing_level,
            legend_position,
        )
        .current_position_owns()
    }

    fn legend_disposition_for_facet_path(
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        sharing_level: SharingLevel,
        legend_position: LegendPosition,
        primary_channel: &str,
    ) -> LegendDisposition {
        if sharing_level.is_free() || facet_path.is_empty() {
            return LegendDisposition::RenderHere;
        }

        let Some(resolved) = facet_tree.resolve_path_info(facet_path) else {
            return LegendDisposition::RenderHere;
        };

        if resolved.indices.is_empty() {
            return LegendDisposition::RenderHere;
        }

        let anchor_path = facet_tree.sharing_owner_path(facet_path, sharing_level.raw());
        let physical_sharing_level =
            SharingLevel::from_raw(facet_path.len().saturating_sub(anchor_path.len()) as u8);
        let ownership_scope = sharing_policy::legend_ownership_scope(
            primary_channel,
            facet_path,
            &resolved.indices,
            &resolved.local_level_counts,
            resolved.indices.len() as u8,
            physical_sharing_level,
            legend_position,
        );

        if !ownership_scope.current_position_owns() {
            return LegendDisposition::Suppress;
        }

        if anchor_path == facet_path {
            LegendDisposition::RenderHere
        } else {
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(anchor_path),
                sharing_level,
            }
        }
    }

    fn child_frame_anchor_for_sharing(
        sharing_path: &ChildFrameSharingPath,
        sharing_level: SharingLevel,
    ) -> Vec<ContainerPathSegment> {
        let full_path = sharing_path.container_path();
        let keep_count = sharing_level.ancestor_keep_count(full_path.len(), full_path.len() as u8);
        full_path.into_iter().take(keep_count).collect()
    }

    fn legend_disposition_for_child_frame_path(
        sharing_path: &ChildFrameSharingPath,
        sharing_level: SharingLevel,
        legend_position: LegendPosition,
        primary_channel: &str,
    ) -> LegendDisposition {
        if sharing_level.is_free() || sharing_path.levels().is_empty() {
            return LegendDisposition::RenderHere;
        }

        let position_indices = sharing_path.position_indices();
        let level_counts = sharing_path.level_counts();
        let ownership = edge_ownership_scope_for_request(EdgeOwnershipRequest::new(
            CoordinationKind::LegendOwnership,
            format!("{primary_channel}:{legend_position:?}"),
            sharing_policy::legend_edge_for_position(legend_position),
            &position_indices,
            &level_counts,
            position_indices.len() as u8,
            sharing_level,
        ));

        if !ownership.current_position_owns() {
            return LegendDisposition::Suppress;
        }

        let full_path = sharing_path.container_path();
        let anchor_path = Self::child_frame_anchor_for_sharing(sharing_path, sharing_level);
        if anchor_path == full_path {
            LegendDisposition::RenderHere
        } else {
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::ChildFrameContainer(anchor_path),
                sharing_level,
            }
        }
    }

    fn legend_layout_key(primary_channel: &str) -> String {
        primary_channel.to_string()
    }

    fn hoisted_legend_layout_key(
        primary_channel: &str,
        anchor: &HoistedLegendAnchor,
        owner: &HoistedLegendAnchor,
    ) -> String {
        let mut hasher = DefaultHasher::new();
        anchor.hash(&mut hasher);
        owner.hash(&mut hasher);
        format!("{primary_channel}@{:016x}", hasher.finish())
    }

    /// Create default legends for channels with scales
    fn create_default_legends(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        session_context: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> IndexMap<String, Legend> {
        let mut default_legends = IndexMap::new();

        // Build set of channels to skip for legends
        let mut skip_channels = std::collections::HashSet::new();

        // Add positional channels from the coordinate system
        for &channel in self.coord_transform.required_channels() {
            skip_channels.insert(channel.to_string());
            // Also skip interval variants (e.g., "x2" for "x")
            skip_channels.insert(format!("{}2", channel));
        }

        // Add channels that marks indicate shouldn't have legends
        for (channel, scale) in scales {
            // Find the mark that has this channel
            if let Some(mark) = self
                .marks
                .iter()
                .find(|m| m.data_context().channels().contains_key(channel))
            {
                // If the mark that has the channel says no legend, skip it
                if mark
                    .preferred_legend_renderer(channel, scale.configured())
                    .is_none()
                {
                    skip_channels.insert(channel.clone());
                }
            }
        }

        // Sort channels for deterministic ordering
        let mut sorted_channels: Vec<_> = scales.keys().collect();
        sorted_channels.sort();

        for channel in sorted_channels {
            // Skip channels that don't need legends
            if skip_channels.contains(channel) {
                continue;
            }

            // Skip if legend already configured
            if self.legends.contains_key(channel) {
                continue;
            }

            // Determine legend renderer type for CSS selector support
            // (e.g., legend[type="symbol"], legend[type="line"], legend[type="colorbar"])
            let legend_type = self.legend_renderer_theme_selector_for_channel(channel, scales);

            // Create legend with theme defaults
            let theme = self.get_theme();
            let title = self.infer_legend_title(channel, session_context);
            let position = self.default_legend_position(channel);
            let legend =
                themed_default_legend(title, position, legend_type, theme.as_ref(), params);

            default_legends.insert(channel.clone(), legend);
        }

        default_legends
    }

    /// Get the appropriate legend renderer for a channel
    pub(super) fn get_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScaleWithSpec,
    ) -> Option<Arc<dyn LegendRenderer>> {
        match self.get_legend_renderer_selection(channel, scale)? {
            LegendRendererSelection::BuiltIn(kind) => Some(renderer_for_kind(kind)),
            LegendRendererSelection::Custom(renderer) => Some(renderer),
        }
    }

    fn get_legend_renderer_selection(
        &self,
        channel: &str,
        scale: &ConfiguredScaleWithSpec,
    ) -> Option<LegendRendererSelection> {
        // Find the first mark that has this channel and get its preference
        for mark in &self.marks {
            if mark.data_context().channels().contains_key(channel) {
                return mark.preferred_legend_renderer(channel, scale.configured());
            }
        }
        None
    }

    fn legend_renderer_theme_selector_for_channel(
        &self,
        channel: &str,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) -> Option<&'static str> {
        let scale = scales.get(channel)?;
        self.get_legend_renderer_selection(channel, scale)?
            .theme_selector()
    }

    /// Get legends with theme applied (matching PlotRenderer behavior)
    pub(super) fn get_legends_with_theme(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        session_context: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> IndexMap<String, Legend> {
        // 1. Start with plot-level legends
        let mut all_legends = self.legends.clone();

        // 2. Apply channel-level legend configs (from mark encodings)
        for mark in &self.marks {
            for (channel_name, channel_value) in mark.data_context().channels() {
                if let Some(channel_legend) = channel_value.get_legend_config() {
                    all_legends
                        .entry(channel_name.clone())
                        .and_modify(|legend| {
                            *legend = legend.clone().update(channel_legend.clone())
                        })
                        .or_insert_with(|| channel_legend.clone());
                }
            }
        }

        // 3. Apply defaults for channels with scales but no legend config
        let default_legends = self.create_default_legends(scales, session_context, params);
        for (channel, default_legend) in default_legends {
            all_legends
                .entry(channel)
                .and_modify(|legend| *legend = default_legend.clone().update(legend.clone()))
                .or_insert(default_legend);
        }

        // 4. Apply theme (only for Unset properties)
        let theme = self.get_theme();
        for (channel, legend) in all_legends.iter_mut() {
            // Determine legend type for this channel (for CSS selector support)
            let legend_type = self.legend_renderer_theme_selector_for_channel(channel, scales);

            apply_legend_theme_defaults(legend, legend_type, theme.as_ref(), params);
        }

        all_legends
    }

    /// Build a legend channel for a specific channel in a mark
    fn build_legend_channel(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        scale: &ConfiguredScaleWithSpec,
        mark: &dyn CompiledMark,
        mark_index: usize,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> LegendChannel {
        // Collect related channels from the mark
        let mut related_channels = HashMap::new();

        // First add explicitly set channels
        for (other_name, other_value) in mark.data_context().channels() {
            if other_name != channel_name {
                // Check if this channel has a scale or is constant
                let channel_info = if let Some(other_scale) = configured_scales.get(other_name) {
                    // Channel has a scale
                    ChannelInfo::Scaled {
                        expr: other_value.expr(ctx),
                        scale: other_scale.configured().clone(),
                    }
                } else if let Some(expr) = other_value
                    .expr(ctx)
                    .or_else(|| other_value.expr_for_domain(ctx))
                {
                    // Channel has a constant expression
                    ChannelInfo::Constant { expr: expr.clone() }
                } else {
                    // Skip channels without expressions
                    continue;
                };
                related_channels.insert(other_name.clone(), channel_info);
            }
        }

        // For channels not explicitly set, check if they have theme defaults
        // This ensures legend symbols match the chart's actual appearance
        let eval_ctx = avenger_chart_core::EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            params.clone(),
        );

        // Iterate through all supported channels of this mark
        for channel_desc in mark.supported_channels() {
            let other_name = channel_desc.name;
            if other_name != channel_name && !related_channels.contains_key(other_name) {
                // Channel not explicitly set - check for theme default
                if let Some(default_value) =
                    default_channel_value_for_eval(mark, other_name, &eval_ctx)
                {
                    // Add as a constant channel
                    let expr = lit(default_value);
                    related_channels.insert(other_name.to_string(), ChannelInfo::Constant { expr });
                }
            }
        }

        // Get the mark type name
        let mark_type = mark.mark_type().to_string();
        LegendChannel {
            name: channel_name.to_string(),
            expression: channel_value.expr(ctx),
            scale: scale.configured().clone(),
            channel_type: channel_name.to_string(), // Use channel name as type
            sharing_level: channel_value.get_domain_scope().map(|mode| mode.to_level()),
            mark_type,
            mark_index,
            related_channels,
        }
    }

    fn legend_disposition(
        channel: &str,
        scope: &LegendPlanScope,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        resolved_position: LegendPosition,
        child_frame_sharing_level: SharingLevel,
        facet_sharing_level: SharingLevel,
        _configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        _params: &IndexMap<String, ScalarValue>,
    ) -> LegendDisposition {
        match scope {
            LegendPlanScope::TopLevel => LegendDisposition::RenderHere,
            LegendPlanScope::FacetCell => Self::legend_disposition_for_facet_path(
                facet_tree,
                facet_path,
                facet_sharing_level,
                resolved_position,
                channel,
            ),
            LegendPlanScope::ChildFrame { sharing_path } => {
                if !child_frame_sharing_level.is_free() && !sharing_path.levels().is_empty() {
                    let child_frame_depth = sharing_path.levels().len() as u8;
                    let crosses_child_frame_boundary = child_frame_sharing_level.is_global()
                        || child_frame_sharing_level.raw() > child_frame_depth;
                    let child_frame_scope_level = if crosses_child_frame_boundary {
                        SharingLevel::from_raw(child_frame_depth)
                    } else {
                        child_frame_sharing_level
                    };

                    let child_frame_disposition = Self::legend_disposition_for_child_frame_path(
                        sharing_path,
                        child_frame_scope_level,
                        resolved_position,
                        channel,
                    );

                    if !crosses_child_frame_boundary || facet_path.is_empty() {
                        if child_frame_disposition != LegendDisposition::RenderHere {
                            return child_frame_disposition;
                        }
                    } else {
                        if child_frame_disposition == LegendDisposition::Suppress {
                            return LegendDisposition::Suppress;
                        }

                        let remaining_facet_level = if child_frame_sharing_level.is_global() {
                            SharingLevel::GLOBAL
                        } else {
                            SharingLevel::from_raw(
                                child_frame_sharing_level.raw() - child_frame_depth,
                            )
                        };

                        return Self::legend_disposition_for_facet_path(
                            facet_tree,
                            facet_path,
                            remaining_facet_level,
                            resolved_position,
                            channel,
                        );
                    }
                }

                if !facet_sharing_level.is_free() && !facet_path.is_empty() {
                    return Self::legend_disposition_for_facet_path(
                        facet_tree,
                        facet_path,
                        facet_sharing_level,
                        resolved_position,
                        channel,
                    );
                }

                LegendDisposition::RenderHere
            }
        }
    }

    async fn resolve_legend_position(
        &self,
        legend: &Legend,
        primary_channel: &LegendChannel,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<LegendPosition, AvengerChartError> {
        use avenger_chart_core::evaluate_legend_position_expr;

        if let Some(node) = legend.position.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            return evaluate_legend_position_expr(&expr, ctx, params).await;
        }

        // Position not set - check if theme has a position with runtime params.
        if let Some(theme) = &self.theme {
            // Determine legend type for theme context.
            let legend_type = self
                .legend_renderer_theme_selector_for_channel(primary_channel.name.as_str(), scales);

            let legend_ctx = theme.legend_context_with_params(legend_type, params.clone());
            if let Some(theme_value) = theme.query(&legend_ctx, "position")
                && let Some(position_str) = theme_value.as_string()
            {
                return Ok(match position_str.to_lowercase().as_str() {
                    "top" => LegendPosition::Top,
                    "bottom" => LegendPosition::Bottom,
                    "left" => LegendPosition::Left,
                    "right" => LegendPosition::Right,
                    _ => LegendPosition::Right,
                });
            }
        }

        Ok(LegendPosition::Right)
    }

    /// Merge legend channels based on merge keys
    pub(super) async fn merge_legend_channels(
        &self,
        all_legends: &IndexMap<String, Legend>,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<(Vec<Vec<LegendChannel>>, IndexMap<String, Legend>), AvengerChartError> {
        use avenger_chart_core::{evaluate_bool_expr, evaluate_i32_expr};

        // Collect all channels that need legends from all marks
        let mut all_channels = Vec::new();

        for (mark_index, mark) in self.marks.iter().enumerate() {
            for (channel_name, channel_value) in mark.data_context().channels() {
                // Skip if no scale or no legend config
                if !configured_scales.contains_key(channel_name)
                    || !all_legends.contains_key(channel_name)
                {
                    continue;
                }

                // Note: Visibility is evaluated and filtered in merge_legend_channels
                let _legend_config = &all_legends[channel_name];

                let scale = &configured_scales[channel_name];

                let legend_channel = self.build_legend_channel(
                    channel_name,
                    channel_value,
                    scale,
                    mark.as_ref(),
                    mark_index,
                    configured_scales,
                    ctx,
                    params,
                );

                all_channels.push(legend_channel);
            }
        }

        // Group channels by MergeKey
        let mut channel_groups: Vec<Vec<LegendChannel>> = Vec::new();

        for channel in all_channels {
            let merge_key = MergeKey::from_channel(&channel);

            if merge_key.is_none() {
                // Continuous scales or channels without expressions should not be merged
                channel_groups.push(vec![channel]);
            } else {
                // Find if this key already exists in any group
                let mut found = false;
                for group in channel_groups.iter_mut() {
                    if !group.is_empty() {
                        // Check if this group has the same merge key
                        let group_key = MergeKey::from_channel(&group[0]);
                        if group_key == merge_key {
                            group.push(channel.clone());
                            found = true;
                            break;
                        }
                    }
                }

                if !found {
                    // Create a new group for this merge key
                    channel_groups.push(vec![channel]);
                }
            }
        }

        // Sort channel groups by their legend order
        let mut groups_with_order: Vec<(Vec<LegendChannel>, i32)> = Vec::new();

        // Evaluate order expressions
        for channels in channel_groups {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    let order = if let Some(node) =
                        legend_config.order.as_option().and_then(|o| o.as_ref())
                    {
                        let expr = node.to_expr(ctx)?;
                        evaluate_i32_expr(&expr, ctx, params).await?
                    } else {
                        i32::MAX
                    };
                    groups_with_order.push((channels, order));
                }
            }
        }

        // Sort by order value
        groups_with_order.sort_by_key(|(_, order)| *order);

        // Extract sorted channel groups
        let sorted_channel_groups: Vec<Vec<LegendChannel>> = groups_with_order
            .iter()
            .map(|(channels, _)| channels.clone())
            .collect();

        // Create the legends map with merged channel info for layout
        let mut legends_map: IndexMap<String, Legend> = IndexMap::new();
        for (channels, _) in groups_with_order {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    // Evaluate visibility expression
                    let visible = if let Some(node) =
                        legend_config.visible.as_option().and_then(|o| o.as_ref())
                    {
                        let expr = node.to_expr(ctx)?;
                        evaluate_bool_expr(&expr, ctx, params).await?
                    } else {
                        true // Default to visible
                    };
                    if visible {
                        let mut legend_with_merged = legend_config.clone();
                        for channel in channels.iter().skip(1) {
                            if let Some(other) = all_legends.get(&channel.name) {
                                legend_with_merged = legend_with_merged.update(other.clone());
                            }
                        }
                        legend_with_merged.validate_event_surface()?;
                        // Populate merged_channels with all channel types in this group
                        legend_with_merged.merged_channels =
                            channels.iter().map(|ch| ch.channel_type.clone()).collect();
                        legends_map.insert(primary_channel.name.clone(), legend_with_merged);
                    }
                }
            }
        }

        Ok((sorted_channel_groups, legends_map))
    }

    pub(super) async fn prepare_legend_plan(
        &self,
        eval_ctx: &EvaluationContext,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        available_space: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        _child_frame_sharing_path: &ChildFrameSharingPath,
        scope: LegendPlanScope,
    ) -> Result<PreparedLegendPlan, AvengerChartError> {
        let mut legend_measurements = LegendMeasurements::new();
        let mut groups: Vec<PreparedLegendGroup> = Vec::new();
        let mut hoisted_requests = Vec::new();

        // Get all legends including channel-level configs
        let all_legends = self.get_legends_with_theme(scales, ctx, params);

        // Merge channels to get the same groups that will be used for rendering
        let (sorted_channel_groups, legends_map) = self
            .merge_legend_channels(&all_legends, scales, ctx, params)
            .await?;

        for channels in sorted_channel_groups {
            if channels.is_empty() {
                continue;
            }

            // Get the primary channel (first in group)
            let primary_channel = &channels[0];

            let Some(legend) = legends_map.get(&primary_channel.name) else {
                // Visibility expression evaluated to false in merge_legend_channels.
                continue;
            };

            let resolved_position = self
                .resolve_legend_position(legend, primary_channel, scales, ctx, params)
                .await?;

            let child_frame_sharing_level = Self::effective_group_sharing_level(
                facet_tree,
                channels.as_slice(),
                SharingLevel::FREE,
            );
            let facet_sharing_level = Self::effective_group_sharing_level(
                facet_tree,
                channels.as_slice(),
                SharingLevel::GLOBAL,
            );

            let disposition = Self::legend_disposition(
                &primary_channel.name,
                &scope,
                facet_tree,
                facet_path,
                resolved_position,
                child_frame_sharing_level,
                facet_sharing_level,
                scales,
                params,
            );
            if disposition == LegendDisposition::Suppress {
                continue;
            }

            // Get scale for primary channel
            let scale = scales.get(&primary_channel.name).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Scale not found for channel '{}'",
                    primary_channel.name
                ))
            })?;

            // Determine the appropriate renderer for this group of channels
            let renderer = if channels.len() > 1 {
                // Multiple channels - use the primary renderer when it supports merging them.
                self.get_legend_renderer(&primary_channel.channel_type, scale)
                    .filter(|renderer| renderer.supports_merge(&channels))
            } else {
                // Single channel - use the unified renderer selection
                self.get_legend_renderer(&primary_channel.channel_type, scale)
            };

            if let Some(renderer) = renderer {
                let owner = match &scope {
                    LegendPlanScope::ChildFrame { sharing_path } => {
                        HoistedLegendAnchor::ChildFrameContainer(sharing_path.container_path())
                    }
                    LegendPlanScope::TopLevel | LegendPlanScope::FacetCell => {
                        HoistedLegendAnchor::FacetPath(facet_path.to_vec())
                    }
                };
                let layout_key = match &disposition {
                    LegendDisposition::Hoist { anchor, .. } => {
                        Self::hoisted_legend_layout_key(&primary_channel.name, anchor, &owner)
                    }
                    LegendDisposition::RenderHere | LegendDisposition::Suppress => {
                        Self::legend_layout_key(&primary_channel.name)
                    }
                };
                let group = PreparedLegendGroup {
                    layout_key: layout_key.clone(),
                    primary_channel: primary_channel.name.clone(),
                    channels: channels.clone(),
                    legend: Arc::new(legend.clone()),
                    renderer,
                };

                if let LegendDisposition::Hoist {
                    anchor,
                    sharing_level,
                } = disposition
                {
                    hoisted_requests.push(HoistedLegendRequest {
                        anchor,
                        owner,
                        position: resolved_position,
                        sharing_level,
                        group,
                    });
                    continue;
                }

                if groups.iter().any(|group| group.layout_key == layout_key) {
                    continue;
                }

                // Measure the legend with the same channels that will be used for rendering.
                let measurement = self
                    .measure_legend_group(
                        eval_ctx,
                        &group,
                        resolved_position,
                        available_space,
                        ctx,
                        params,
                    )
                    .await?;

                debug!(
                    channel = primary_channel.name.as_str(),
                    width = measurement.size.width,
                    height = measurement.size.height,
                    flexible = measurement.flexible,
                    position = ?resolved_position,
                    child_frame_sharing = child_frame_sharing_level.raw(),
                    facet_sharing = facet_sharing_level.raw(),
                    "Legend measure"
                );
                legend_measurements.insert(layout_key, measurement);

                groups.push(group);
            }
        }

        Ok(PreparedLegendPlan {
            groups,
            measurements: legend_measurements,
            hoisted_requests,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn measure_legend_group(
        &self,
        eval_ctx: &EvaluationContext,
        group: &PreparedLegendGroup,
        position: LegendPosition,
        available_space: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<LegendMeasurement, AvengerChartError> {
        let Some(cache) = eval_ctx.legend_measurement_cache() else {
            let measurement = self
                .measure_legend_group_uncached(
                    eval_ctx,
                    group,
                    position,
                    available_space,
                    ctx,
                    params,
                )
                .await?;
            eval_ctx.record_legend_measurements(1);
            return Ok(measurement);
        };

        let key = self.legend_measurement_cache_key(
            group,
            available_space,
            position,
            params,
            eval_ctx.text_measurement_cache_tag(),
        );
        let cached = {
            cache
                .lock()
                .expect("legend measurement cache lock poisoned")
                .get(&key)
        };
        if let Some(measurement) = cached {
            eval_ctx.record_legend_measurement_cache_hit();
            return Ok(measurement);
        }

        eval_ctx.record_legend_measurement_cache_miss();
        let measurement = self
            .measure_legend_group_uncached(eval_ctx, group, position, available_space, ctx, params)
            .await?;
        eval_ctx.record_legend_measurements(1);
        cache
            .lock()
            .expect("legend measurement cache lock poisoned")
            .insert(key, measurement.clone());
        Ok(measurement)
    }

    async fn measure_legend_group_uncached(
        &self,
        eval_ctx: &EvaluationContext,
        group: &PreparedLegendGroup,
        position: LegendPosition,
        available_space: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<LegendMeasurement, AvengerChartError> {
        let theme = self.get_theme();
        let (size, flexible) = measure_legend_size_with_channels(
            &group.channels,
            group.legend.as_ref(),
            group.renderer.clone(),
            available_space,
            theme.as_ref(),
            params,
            ctx,
            eval_ctx.text_measurer(),
        )
        .await?;
        Ok(LegendMeasurement {
            size,
            flexible,
            position,
        })
    }

    pub(super) async fn render_legends_from_plan(
        &self,
        eval_ctx: &EvaluationContext,
        legend_plan: &PreparedLegendPlan,
        layout: &FrameLayout,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<RenderedLegendMarks, AvengerChartError> {
        let theme = self.get_theme();
        let mut legend_marks = Vec::new();
        let mut event_datums = Vec::new();
        let mut interaction_scopes = Vec::new();

        for group in &legend_plan.groups {
            let Some(bounds) = layout.legends.get(&group.layout_key) else {
                continue;
            };
            let group_opt = group
                .renderer
                .evaluate(
                    &group.channels,
                    group.legend.as_ref(),
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                    theme.as_ref(),
                    params,
                    ctx,
                    eval_ctx.text_measurer(),
                )
                .await?;
            if let Some(mut rendered) = group_opt {
                apply_related_legend_item_opacity(
                    eval_ctx,
                    &self.selection_specs,
                    &group.channels,
                    &mut rendered.group,
                    &rendered.items,
                    ctx,
                    params,
                )
                .await?;
                if !group.legend.event_bindings.is_empty()
                    && rendered.items.is_empty()
                    && rendered.continuous_surfaces.is_empty()
                {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Legend '{}' has event bindings but does not expose interactive legend surfaces",
                        group.primary_channel
                    )));
                }
                let overlay_channels = group
                    .channels
                    .iter()
                    .filter(|channel| {
                        self.legend_colorbar_overlays
                            .iter()
                            .any(|overlay| overlay.channel_name == channel.name)
                    })
                    .map(|channel| channel.name.as_str())
                    .collect::<Vec<_>>();
                if !overlay_channels.is_empty() && rendered.continuous_surfaces.is_empty() {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Legend '{}' has colorbar overlay marks but did not render a colorbar surface",
                        overlay_channels.join(", ")
                    )));
                }
                let legend_index = legend_marks.len();
                let legend_origin = rendered.group.origin;
                let surface_keys = group
                    .channels
                    .iter()
                    .map(|channel| channel.name.clone())
                    .collect::<Vec<_>>();
                event_datums.extend(legend_item_event_datums(
                    legend_index,
                    &surface_keys,
                    group.legend.id.as_deref(),
                    rendered.items,
                )?);
                event_datums.extend(legend_continuous_surface_event_datums(
                    legend_index,
                    &surface_keys,
                    group.legend.id.as_deref(),
                    rendered.continuous_surfaces.clone(),
                )?);
                interaction_scopes.extend(legend_continuous_surface_interaction_scopes(
                    layout.plot_area,
                    legend_origin,
                    &rendered.continuous_surfaces,
                ));
                for surface in &rendered.continuous_surfaces {
                    let overlay_marks = self
                        .render_colorbar_overlay_marks(eval_ctx, surface, &group.channels)
                        .await?;
                    insert_colorbar_overlay_group(&mut rendered.group, surface, overlay_marks);
                }
                legend_marks.push(SceneMark::Group(rendered.group));
            }
        }

        Ok(RenderedLegendMarks {
            marks: legend_marks,
            event_datums,
            interaction_scopes,
        })
    }

    async fn render_colorbar_overlay_marks(
        &self,
        eval_ctx: &EvaluationContext,
        surface: &LegendContinuousSurface,
        channels: &[LegendChannel],
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut overlay_marks = Vec::new();
        let mut seen_channels = HashSet::new();
        for channel in channels {
            if !seen_channels.insert(channel.name.as_str()) {
                continue;
            }
            let Some(overlay) = self
                .legend_colorbar_overlays
                .iter()
                .find(|overlay| overlay.channel_name == channel.name)
            else {
                continue;
            };
            let marks = &overlay.marks;
            let scales = colorbar_overlay_scales(surface);
            let render_state =
                RenderState::new(surface.bounds.width, surface.bounds.height, scales.clone());
            let coord_measurement = EmptyCoordMeasurement;
            let render_ctx = RenderContext::new(eval_ctx, &render_state, &[], &coord_measurement);
            let coord_transform = Cartesian::new();

            for mark in marks {
                let Some(prepared) = prepare_mark_data_runtime(MarkDataRequest {
                    mark: mark.as_ref(),
                    coord_transform: Some(&coord_transform),
                    plot_data: None,
                    provided_plot_df: None,
                    facet_data_scope: None,
                    prepared_logical: None,
                    prepared_base: None,
                    eval_ctx,
                    evaluation_metrics: eval_ctx.evaluation_metrics.clone(),
                    scales: &scales,
                    plot_width: surface.bounds.width,
                    plot_height: surface.bounds.height,
                })
                .await?
                else {
                    continue;
                };
                let mut marks = mark
                    .render_from_data(
                        prepared.data_batch.as_ref(),
                        &prepared.scalar_batch,
                        &render_ctx,
                        &coord_transform,
                    )
                    .await?;
                if let Some(id) = mark.state().id.as_deref() {
                    for scene_mark in &mut marks {
                        set_scene_mark_name(scene_mark, id);
                    }
                }
                for scene_mark in &mut marks {
                    set_interactive_recursive(scene_mark, false);
                }
                overlay_marks.extend(marks);
            }
        }
        Ok(overlay_marks)
    }

    pub(super) async fn add_measured_hoisted_legends_to_plan(
        &self,
        eval_ctx: &EvaluationContext,
        legend_plan: &mut PreparedLegendPlan,
        requests: Vec<HoistedLegendRequest>,
        available_space: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<(), AvengerChartError> {
        for request in requests {
            let layout_key = request.group.layout_key.clone();
            if legend_plan.measurements.contains_key(&layout_key) {
                continue;
            }

            let measurement = self
                .measure_legend_group(
                    eval_ctx,
                    &request.group,
                    request.position,
                    available_space,
                    ctx,
                    params,
                )
                .await?;

            debug!(
                channel = request.group.primary_channel.as_str(),
                layout_key = layout_key.as_str(),
                owner = ?request.owner,
                anchor = ?request.anchor,
                width = measurement.size.width,
                height = measurement.size.height,
                flexible = measurement.flexible,
                position = ?request.position,
                sharing = request.sharing_level.raw(),
                "Hoisted legend measure"
            );

            legend_plan.measurements.insert(layout_key, measurement);
            legend_plan.groups.push(request.group);
        }

        Ok(())
    }

    /// Infer a title for the legend based on channel
    fn infer_legend_title(&self, channel: &str, session_context: &SessionContext) -> String {
        // First try to extract from marks (like we do for axes)
        if let Some(title) = extract_channel_title_from_marks(&self.marks, channel, session_context)
        {
            return title;
        }

        // Fallback: convert underscores to spaces and apply title case
        channel
            .split('_')
            .map(|word| {
                // Capitalize first letter of each word
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Get default legend position for a channel
    fn default_legend_position(&self, _channel: &str) -> LegendPosition {
        // All legends default to the right
        LegendPosition::Right
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_scales::scales::{ConfiguredScale, band::BandScale};
    use datafusion::logical_expr::Expr;
    use indexmap::IndexMap;
    use std::collections::HashMap;

    use crate::{
        container::{ChildFrameSharingLevel, ChildFrameSharingPath, ContainerPathSegment},
        facet::FacetDirection,
        facet::evaluated_facet_tree::PartitionNode,
    };

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn make_simple_scale() -> ConfiguredScale {
        let domain = ScalarValue::iter_to_array(vec![s("A"), s("B")]).unwrap();
        BandScale::configured(domain, (0.0, 100.0))
    }

    fn make_legend_channel(name: &str) -> LegendChannel {
        LegendChannel {
            name: name.to_string(),
            expression: None::<Expr>,
            scale: make_simple_scale(),
            channel_type: name.to_string(),
            sharing_level: None,
            mark_type: "symbol".to_string(),
            mark_index: 0,
            related_channels: HashMap::new(),
        }
    }

    fn make_two_level_column_tree_with_sharing(levels: HashMap<String, u8>) -> EvaluatedFacetTree {
        let mut outer_children: IndexMap<ScalarValue, Box<PartitionNode>> = IndexMap::new();
        for outer in ["DivA", "DivB"] {
            let leaf = PartitionNode::leaf(
                FacetDirection::Column,
                0,
                "department".to_string(),
                None,
                vec![s("Dept1"), s("Dept2")],
            );
            outer_children.insert(s(outer), Box::new(leaf));
        }

        let root = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "division".to_string(),
            None,
            outer_children,
        );
        EvaluatedFacetTree::new_with_channel_domain_sharing_levels(Some(root), levels)
    }

    fn make_three_level_column_tree_with_sharing(
        levels: HashMap<String, u8>,
    ) -> EvaluatedFacetTree {
        let mut division_children: IndexMap<ScalarValue, Box<PartitionNode>> = IndexMap::new();
        for division in ["DivA", "DivB"] {
            let mut department_children: IndexMap<ScalarValue, Box<PartitionNode>> =
                IndexMap::new();
            for department in ["Dept1", "Dept2"] {
                let team_leaf = PartitionNode::leaf(
                    FacetDirection::Column,
                    0,
                    "team".to_string(),
                    None,
                    vec![s("Team1"), s("Team2")],
                );
                department_children.insert(s(department), Box::new(team_leaf));
            }

            let department_node = PartitionNode::branch(
                FacetDirection::Column,
                0,
                "department".to_string(),
                None,
                department_children,
            );
            division_children.insert(s(division), Box::new(department_node));
        }

        let root = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "division".to_string(),
            None,
            division_children,
        );
        EvaluatedFacetTree::new_with_channel_domain_sharing_levels(Some(root), levels)
    }

    #[test]
    fn legend_visibility_free_always_true() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        let visible = CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            SharingLevel::FREE,
            LegendPosition::Right,
            "fill",
        );
        assert!(visible);
    }

    #[test]
    fn legend_visibility_level1_right_last_in_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            SharingLevel::from_raw(1),
            LegendPosition::Right,
            "fill",
        ));
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Right,
            "fill",
        ));
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Right,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level1_left_first_in_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            SharingLevel::from_raw(1),
            LegendPosition::Left,
            "fill",
        ));
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Left,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level1_top_first_in_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept1")],
            SharingLevel::from_raw(1),
            LegendPosition::Top,
            "fill",
        ));
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Top,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level1_bottom_last_in_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept1")],
            SharingLevel::from_raw(1),
            LegendPosition::Bottom,
            "fill",
        ));
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Bottom,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level2_coarser_grouping() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept2")],
            SharingLevel::from_raw(2),
            LegendPosition::Right,
            "fill",
        ));
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            SharingLevel::from_raw(2),
            LegendPosition::Right,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level_ge_depth_global_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            SharingLevel::from_raw(3),
            LegendPosition::Left,
            "fill",
        ));
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept1")],
            SharingLevel::from_raw(3),
            LegendPosition::Left,
            "fill",
        ));
    }

    #[test]
    fn effective_group_sharing_uses_min_level() {
        let mut levels = HashMap::new();
        levels.insert("fill".to_string(), 1);
        levels.insert("stroke".to_string(), 255);
        let tree = make_two_level_column_tree_with_sharing(levels);
        let channels = vec![make_legend_channel("fill"), make_legend_channel("stroke")];
        assert_eq!(
            CompiledPlot::effective_group_sharing_level(&tree, &channels, SharingLevel::GLOBAL),
            SharingLevel::from_raw(1)
        );
    }

    #[test]
    fn legend_disposition_free_renders_in_facet_cell() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert_eq!(
            CompiledPlot::legend_disposition_for_facet_path(
                &tree,
                &[s("DivA"), s("Dept1")],
                SharingLevel::FREE,
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::RenderHere
        );
    }

    #[test]
    fn legend_disposition_non_owner_suppresses() {
        let tree = make_three_level_column_tree_with_sharing(HashMap::new());
        assert_eq!(
            CompiledPlot::legend_disposition_for_facet_path(
                &tree,
                &[s("DivA"), s("Dept1"), s("Team1")],
                SharingLevel::from_raw(2),
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Suppress
        );
    }

    #[test]
    fn legend_disposition_level2_owner_hoists_to_facet_group() {
        let tree = make_three_level_column_tree_with_sharing(HashMap::new());
        assert_eq!(
            CompiledPlot::legend_disposition_for_facet_path(
                &tree,
                &[s("DivA"), s("Dept2"), s("Team2")],
                SharingLevel::from_raw(2),
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(vec![s("DivA")]),
                sharing_level: SharingLevel::from_raw(2),
            }
        );
    }

    #[test]
    fn legend_disposition_global_owner_hoists_to_root_group() {
        let tree = make_three_level_column_tree_with_sharing(HashMap::new());
        assert_eq!(
            CompiledPlot::legend_disposition_for_facet_path(
                &tree,
                &[s("DivB"), s("Dept2"), s("Team2")],
                SharingLevel::GLOBAL,
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(vec![]),
                sharing_level: SharingLevel::GLOBAL,
            }
        );
    }

    #[test]
    fn legend_disposition_child_frame_non_owner_suppresses() {
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(0, 2, Some("left")));

        assert_eq!(
            CompiledPlot::legend_disposition_for_child_frame_path(
                &sharing_path,
                SharingLevel::GLOBAL,
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Suppress
        );
    }

    #[test]
    fn legend_disposition_child_frame_owner_hoists_to_container() {
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("right")));

        assert_eq!(
            CompiledPlot::legend_disposition_for_child_frame_path(
                &sharing_path,
                SharingLevel::GLOBAL,
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::ChildFrameContainer(vec![]),
                sharing_level: SharingLevel::GLOBAL,
            }
        );
    }

    #[test]
    fn legend_disposition_free_child_frame_renders_inside_child() {
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("right")));

        assert_eq!(
            CompiledPlot::legend_disposition(
                "fill",
                &LegendPlanScope::ChildFrame { sharing_path },
                &make_two_level_column_tree_with_sharing(HashMap::new()),
                &[],
                LegendPosition::Right,
                SharingLevel::FREE,
                SharingLevel::FREE,
                &HashMap::new(),
                &IndexMap::new(),
            ),
            LegendDisposition::RenderHere
        );
    }

    #[test]
    fn legend_disposition_child_frame_level1_hoists_to_immediate_parent() {
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(0, 2, Some("outer")))
            .appended(ChildFrameSharingLevel::vconcat_child(1, 2, Some("inner")));

        assert_eq!(
            CompiledPlot::legend_disposition_for_child_frame_path(
                &sharing_path,
                SharingLevel::from_raw(1),
                LegendPosition::Bottom,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::ChildFrameContainer(vec![
                    ContainerPathSegment::concat_child(0, Some("outer"))
                ]),
                sharing_level: SharingLevel::from_raw(1),
            }
        );
    }

    #[test]
    fn legend_disposition_child_frame_shared_hoists_to_root_container() {
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("outer")))
            .appended(ChildFrameSharingLevel::vconcat_child(1, 2, Some("inner")));

        assert_eq!(
            CompiledPlot::legend_disposition_for_child_frame_path(
                &sharing_path,
                SharingLevel::GLOBAL,
                LegendPosition::Bottom,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::ChildFrameContainer(vec![]),
                sharing_level: SharingLevel::GLOBAL,
            }
        );
    }

    #[test]
    fn free_child_frame_legend_inside_facet_can_use_facet_ownership() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("petal")));

        assert_eq!(
            CompiledPlot::legend_disposition(
                "fill",
                &LegendPlanScope::ChildFrame { sharing_path },
                &tree,
                &[s("DivB"), s("Dept2")],
                LegendPosition::Right,
                SharingLevel::FREE,
                SharingLevel::GLOBAL,
                &HashMap::new(),
                &IndexMap::new(),
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(vec![]),
                sharing_level: SharingLevel::GLOBAL,
            }
        );
    }

    #[test]
    fn legend_disposition_child_frame_level_beyond_frame_depth_continues_to_facet_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("petal")));

        assert_eq!(
            CompiledPlot::legend_disposition(
                "fill",
                &LegendPlanScope::ChildFrame { sharing_path },
                &tree,
                &[s("DivA"), s("Dept2")],
                LegendPosition::Right,
                SharingLevel::from_raw(2),
                SharingLevel::from_raw(2),
                &HashMap::new(),
                &IndexMap::new(),
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(vec![s("DivA")]),
                sharing_level: SharingLevel::from_raw(1),
            }
        );
    }

    #[test]
    fn legend_disposition_child_frame_global_beyond_frame_depth_continues_to_root_facet_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("petal")));

        assert_eq!(
            CompiledPlot::legend_disposition(
                "fill",
                &LegendPlanScope::ChildFrame { sharing_path },
                &tree,
                &[s("DivB"), s("Dept2")],
                LegendPosition::Right,
                SharingLevel::GLOBAL,
                SharingLevel::GLOBAL,
                &HashMap::new(),
                &IndexMap::new(),
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(vec![]),
                sharing_level: SharingLevel::GLOBAL,
            }
        );
    }

    #[test]
    fn legend_disposition_child_frame_level_beyond_frame_depth_still_suppresses_child_non_owner() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(0, 2, Some("sepal")));

        assert_eq!(
            CompiledPlot::legend_disposition(
                "fill",
                &LegendPlanScope::ChildFrame { sharing_path },
                &tree,
                &[s("DivB"), s("Dept2")],
                LegendPosition::Right,
                SharingLevel::GLOBAL,
                SharingLevel::GLOBAL,
                &HashMap::new(),
                &IndexMap::new(),
            ),
            LegendDisposition::Suppress
        );
    }

    #[test]
    fn hoisted_legend_layout_key_includes_owner_identity() {
        let anchor = HoistedLegendAnchor::FacetPath(vec![]);
        let sepal =
            HoistedLegendAnchor::ChildFrameContainer(vec![ContainerPathSegment::concat_child(
                0,
                Some("sepal"),
            )]);
        let petal =
            HoistedLegendAnchor::ChildFrameContainer(vec![ContainerPathSegment::concat_child(
                1,
                Some("petal"),
            )]);

        assert_ne!(
            CompiledPlot::hoisted_legend_layout_key("fill", &anchor, &sepal),
            CompiledPlot::hoisted_legend_layout_key("fill", &anchor, &petal)
        );
    }
}
