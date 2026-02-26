//! Shared facet guide engine used by row/column guide wrappers.
//!
//! This module centralizes the duplicated measurement/rendering flow while keeping
//! axis-specific behavior in `FacetGuideAxisOps` implementations.

use std::{collections::HashMap, sync::Arc};

use avenger_scales::scales::{ConfiguredScale, band::bandwidth};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use tracing::{debug, trace};

use crate::{
    cartesian::axis::AxisPosition,
    coords::{CoordMeasurement, CoordinatedOverflow, FacetAxis, OverflowSpaceRequirement},
    error::AvengerChartError,
    facet::{
        band_positions::{BandPosition, BandPositionIterator},
        coord::{FacetBandCoordMeasurement, FacetBandProbeMeasurement, FacetCellRuntime},
        guide_utils::{
            FacetLabelMeasurementConfig, FacetLabelRenderConfig,
            facet_guide_labels_visible_for_cell, format_scalar_value, measure_facet_label_slab,
            render_facet_label_slab,
        },
        layout_plan::effective_edge_indices_for_values_at_path,
        layout_slabs::LayoutSlabs,
        sharing_level::SharingLevel,
    },
    layout::LayoutBounds,
    plot::compiled::CompiledPlot,
    serialization::LogicalPlanNodeExt,
    theme::{Theme, ThemeContext},
};

pub(crate) const HIDDEN_TOP_LOCAL_ANCHOR_EPSILON: f32 = 0.5;

/// Shared guide state carried by row/column guide wrappers.
#[derive(Clone, Default)]
pub(crate) struct FacetGuideState {
    pub(crate) facet_title: Option<String>,
    pub(crate) compiled_subplot: Option<Arc<CompiledPlot>>,
    pub(crate) facet_data_plan: Option<LogicalPlanNode>,
    pub(crate) position: Option<String>,
    pub(crate) sharing_level: SharingLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuideAnchorSource {
    CoordinatedGuide,
    LocalGuide,
    DefaultZero,
}

impl GuideAnchorSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CoordinatedGuide => "coordinated_guide",
            Self::LocalGuide => "local_guide",
            Self::DefaultZero => "default_zero",
        }
    }
}

pub(crate) trait FacetGuideAxisOps {
    fn log_name() -> &'static str;
    fn scale_key() -> &'static str;
    fn missing_scale_error() -> &'static str;
    fn place_at_end(position: Option<&str>) -> bool;
    fn axis_position(place_at_end: bool) -> AxisPosition;
    fn is_rotated() -> bool;
    fn anchor_policy_label(place_at_end: bool, title_visible: bool) -> &'static str;

    fn subplot_dimensions(plot_width: f32, plot_height: f32, band_size: f32) -> (f32, f32);

    fn compose_total_overflow(
        subplot_overflow: OverflowSpaceRequirement,
        place_at_end: bool,
        guide_anchor: f32,
        guide_slab_size: f32,
    ) -> OverflowSpaceRequirement;

    fn resolve_measure_anchor(
        place_at_end: bool,
        title_visible: bool,
        subplot_overflow: &OverflowSpaceRequirement,
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> (f32, GuideAnchorSource);

    fn resolve_evaluate_anchor(
        place_at_end: bool,
        title_visible: bool,
        coord_measurement: &dyn CoordMeasurement,
    ) -> (f32, GuideAnchorSource);

    fn merge_first_last_edge_overflow(
        first: OverflowSpaceRequirement,
        last: OverflowSpaceRequirement,
    ) -> OverflowSpaceRequirement;

    fn adjusted_plot_bounds(
        plot_bounds: &LayoutBounds,
        place_at_end: bool,
        subplot_overflow_anchor: f32,
    ) -> LayoutBounds;

    fn align_band_positions_for_render(
        band_positions: &[BandPosition],
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Vec<BandPosition> {
        band_positions.to_vec()
    }

    fn title_midpoint_override(
        _coord_measurement: &dyn CoordMeasurement,
        _band_positions: &[BandPosition],
        _plot_bounds: &LayoutBounds,
    ) -> Option<f32> {
        None
    }
}

pub(crate) struct RowGuideAxisOps;
pub(crate) struct ColGuideAxisOps;

impl FacetGuideAxisOps for RowGuideAxisOps {
    fn log_name() -> &'static str {
        "FacetRowGuide"
    }

    fn scale_key() -> &'static str {
        "row"
    }

    fn missing_scale_error() -> &'static str {
        "No row scale found"
    }

    fn place_at_end(position: Option<&str>) -> bool {
        position != Some("left")
    }

    fn axis_position(place_at_end: bool) -> AxisPosition {
        if place_at_end {
            AxisPosition::Right
        } else {
            AxisPosition::Left
        }
    }

    fn is_rotated() -> bool {
        true
    }

    fn anchor_policy_label(_place_at_end: bool, _title_visible: bool) -> &'static str {
        "local_first"
    }

    fn subplot_dimensions(plot_width: f32, _plot_height: f32, band_size: f32) -> (f32, f32) {
        (plot_width, band_size)
    }

    fn compose_total_overflow(
        subplot_overflow: OverflowSpaceRequirement,
        place_at_end: bool,
        guide_anchor: f32,
        guide_slab_size: f32,
    ) -> OverflowSpaceRequirement {
        let (left, right) = if place_at_end {
            (subplot_overflow.left, guide_anchor + guide_slab_size)
        } else {
            (guide_anchor + guide_slab_size, subplot_overflow.right)
        };

        OverflowSpaceRequirement {
            top: subplot_overflow.top,
            bottom: subplot_overflow.bottom,
            left,
            right,
        }
    }

    fn resolve_measure_anchor(
        place_at_end: bool,
        _title_visible: bool,
        subplot_overflow: &OverflowSpaceRequirement,
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> (f32, GuideAnchorSource) {
        let preferred = coord_measurement.and_then(preferred_overflow_for_facet_measurement);

        if let Some((overflow, source)) = preferred {
            return (
                LayoutSlabs::from_coordinated(&overflow).guide_anchor_horizontal(place_at_end),
                source,
            );
        }

        let fallback = if place_at_end {
            subplot_overflow.right
        } else {
            subplot_overflow.left
        };
        (fallback, GuideAnchorSource::DefaultZero)
    }

    fn resolve_evaluate_anchor(
        place_at_end: bool,
        _title_visible: bool,
        coord_measurement: &dyn CoordMeasurement,
    ) -> (f32, GuideAnchorSource) {
        let coordinated = coord_measurement.coordinated_overflow();
        let local = coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .and_then(|fcm| fcm.local_overflow_value());
        resolve_row_guide_anchor_overflow_horizontal(place_at_end, coordinated, local.as_ref())
    }

    fn merge_first_last_edge_overflow(
        first: OverflowSpaceRequirement,
        last: OverflowSpaceRequirement,
    ) -> OverflowSpaceRequirement {
        OverflowSpaceRequirement {
            left: first.left.max(last.left),
            right: first.right.max(last.right),
            top: first.top,
            bottom: last.bottom,
        }
    }

    fn adjusted_plot_bounds(
        plot_bounds: &LayoutBounds,
        place_at_end: bool,
        subplot_overflow_anchor: f32,
    ) -> LayoutBounds {
        if place_at_end {
            LayoutBounds {
                x: plot_bounds.x + subplot_overflow_anchor,
                y: plot_bounds.y,
                width: plot_bounds.width,
                height: plot_bounds.height,
            }
        } else {
            LayoutBounds {
                x: plot_bounds.x - subplot_overflow_anchor,
                y: plot_bounds.y,
                width: plot_bounds.width,
                height: plot_bounds.height,
            }
        }
    }
}

impl FacetGuideAxisOps for ColGuideAxisOps {
    fn log_name() -> &'static str {
        "FacetColGuide"
    }

    fn scale_key() -> &'static str {
        "column"
    }

    fn missing_scale_error() -> &'static str {
        "No column scale found"
    }

    fn place_at_end(position: Option<&str>) -> bool {
        position == Some("bottom")
    }

    fn axis_position(place_at_end: bool) -> AxisPosition {
        if place_at_end {
            AxisPosition::Bottom
        } else {
            AxisPosition::Top
        }
    }

    fn is_rotated() -> bool {
        false
    }

    fn anchor_policy_label(place_at_end: bool, title_visible: bool) -> &'static str {
        if place_at_end || title_visible {
            "coordinated_first"
        } else {
            "local_first_hidden_top_title_if_nonzero"
        }
    }

    fn subplot_dimensions(_plot_width: f32, plot_height: f32, band_size: f32) -> (f32, f32) {
        (band_size, plot_height)
    }

    fn compose_total_overflow(
        subplot_overflow: OverflowSpaceRequirement,
        place_at_end: bool,
        guide_anchor: f32,
        guide_slab_size: f32,
    ) -> OverflowSpaceRequirement {
        let (top, bottom) = if place_at_end {
            (subplot_overflow.top, guide_anchor + guide_slab_size)
        } else {
            (guide_anchor + guide_slab_size, subplot_overflow.bottom)
        };

        OverflowSpaceRequirement {
            top,
            bottom,
            left: subplot_overflow.left,
            right: subplot_overflow.right,
        }
    }

    fn resolve_measure_anchor(
        place_at_end: bool,
        title_visible: bool,
        subplot_overflow: &OverflowSpaceRequirement,
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> (f32, GuideAnchorSource) {
        let coordinated = coord_measurement.and_then(|measurement| {
            let coordinated = measurement.coordinated_overflow()?;
            if coordinated_overflow_is_zero(coordinated) {
                None
            } else {
                Some(coordinated)
            }
        });
        let local = coord_measurement.and_then(facet_local_overflow);

        let (resolved, source) = resolve_col_guide_anchor_overflow(
            place_at_end,
            title_visible,
            coordinated,
            local.as_ref(),
        );
        if matches!(source, GuideAnchorSource::DefaultZero) {
            let fallback = if place_at_end {
                subplot_overflow.bottom
            } else {
                subplot_overflow.top
            };
            (fallback, source)
        } else {
            (resolved, source)
        }
    }

    fn resolve_evaluate_anchor(
        place_at_end: bool,
        title_visible: bool,
        coord_measurement: &dyn CoordMeasurement,
    ) -> (f32, GuideAnchorSource) {
        let coordinated = coord_measurement.coordinated_overflow();
        let local = coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .and_then(|fcm| fcm.local_overflow_value());
        resolve_col_guide_anchor_overflow(place_at_end, title_visible, coordinated, local.as_ref())
    }

    fn merge_first_last_edge_overflow(
        first: OverflowSpaceRequirement,
        last: OverflowSpaceRequirement,
    ) -> OverflowSpaceRequirement {
        OverflowSpaceRequirement {
            left: first.left,
            right: last.right,
            top: first.top.max(last.top),
            bottom: first.bottom.max(last.bottom),
        }
    }

    fn adjusted_plot_bounds(
        plot_bounds: &LayoutBounds,
        place_at_end: bool,
        subplot_overflow_anchor: f32,
    ) -> LayoutBounds {
        if place_at_end {
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y + subplot_overflow_anchor,
                width: plot_bounds.width,
                height: plot_bounds.height,
            }
        } else {
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y - subplot_overflow_anchor,
                width: plot_bounds.width,
                height: plot_bounds.height,
            }
        }
    }

    fn align_band_positions_for_render(
        band_positions: &[BandPosition],
        coord_measurement: &dyn CoordMeasurement,
    ) -> Vec<BandPosition> {
        align_bands_to_nested_child_col_spans(band_positions, coord_measurement)
    }

    fn title_midpoint_override(
        coord_measurement: &dyn CoordMeasurement,
        band_positions: &[BandPosition],
        plot_bounds: &LayoutBounds,
    ) -> Option<f32> {
        coordinated_col_title_midpoint_override(coord_measurement, band_positions, plot_bounds)
    }
}

pub(crate) async fn measure_overflow_common<O: FacetGuideAxisOps>(
    state: &FacetGuideState,
    scales: &HashMap<String, ConfiguredScale>,
    plot_width: f32,
    plot_height: f32,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
    data_override: Option<&DataFrame>,
    ctx: &SessionContext,
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    coord_measurement: Option<&dyn CoordMeasurement>,
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    let subplot_overflow = if let Some(local_overflow) =
        coord_measurement.and_then(facet_local_overflow)
    {
        propagated_subplot_overflow(Some(local_overflow))
    } else {
        compute_subplot_overflow_common::<O>(
            state,
            scales,
            plot_width,
            plot_height,
            theme,
            params,
            data_override,
            ctx,
            facet_tree,
            facet_path,
        )
        .await?
    };

    let (_band_positions, labels) = band_positions_and_labels::<O>(scales, coord_measurement)?;
    let place_at_end = O::place_at_end(state.position.as_deref());
    let axis_position = O::axis_position(place_at_end);
    let guide_visible = facet_guide_labels_visible_for_cell(
        facet_tree,
        facet_path,
        axis_position,
        state.sharing_level.raw(),
    );
    if !guide_visible {
        debug!(
            guide = O::log_name(),
            position = ?state.position,
            sharing_level = state.sharing_level.raw(),
            subplot_overflow_left = subplot_overflow.left,
            subplot_overflow_right = subplot_overflow.right,
            subplot_overflow_top = subplot_overflow.top,
            subplot_overflow_bottom = subplot_overflow.bottom,
            "facet guide hidden by ownership; returning subplot overflow only"
        );
        return Ok(subplot_overflow);
    }

    let title_visible = facet_title_visible_for_cell(facet_tree, facet_path, axis_position);
    let title_for_cell = title_visible
        .then_some(state.facet_title.as_ref())
        .flatten();
    let facet_guide_slab_size =
        measure_facet_guide_slab(&labels, title_for_cell, theme, params, O::is_rotated());

    let (guide_anchor, anchor_source) = O::resolve_measure_anchor(
        place_at_end,
        title_visible,
        &subplot_overflow,
        coord_measurement,
    );

    let total = O::compose_total_overflow(
        subplot_overflow,
        place_at_end,
        guide_anchor,
        facet_guide_slab_size,
    );

    debug!(
        guide = O::log_name(),
        position = ?state.position,
        guide_anchor,
        anchor_source = anchor_source.as_str(),
        anchor_policy = O::anchor_policy_label(place_at_end, title_visible),
        guide_visible,
        title_visible,
        sharing_level = state.sharing_level.raw(),
        facet_guide_slab_size,
        total_left = total.left,
        total_right = total.right,
        total_top = total.top,
        total_bottom = total.bottom,
        "facet guide measure_overflow"
    );

    Ok(total)
}

pub(crate) async fn evaluate_common<O: FacetGuideAxisOps>(
    state: &FacetGuideState,
    scales: &HashMap<String, ConfiguredScale>,
    plot_bounds: &LayoutBounds,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    coord_measurement: &dyn CoordMeasurement,
) -> Result<Vec<avenger_scenegraph::marks::mark::SceneMark>, AvengerChartError> {
    let place_at_end = O::place_at_end(state.position.as_deref());
    let axis_position = O::axis_position(place_at_end);
    let guide_visible = facet_guide_labels_visible_for_cell(
        facet_tree,
        facet_path,
        axis_position,
        state.sharing_level.raw(),
    );
    if !guide_visible {
        debug!(
            guide = O::log_name(),
            position = ?state.position,
            sharing_level = state.sharing_level.raw(),
            "facet guide evaluate hidden by ownership; skipping marks"
        );
        return Ok(vec![]);
    }

    let (band_positions, labels) = band_positions_and_labels::<O>(scales, Some(coord_measurement))?;
    if band_positions.is_empty() {
        return Ok(vec![]);
    }

    let band_positions = O::align_band_positions_for_render(&band_positions, coord_measurement);
    let title_visible = facet_title_visible_for_cell(facet_tree, facet_path, axis_position);
    let title_for_cell = title_visible
        .then_some(state.facet_title.as_ref())
        .flatten();
    let facet_guide_slab_size =
        measure_facet_guide_slab(&labels, title_for_cell, theme, params, O::is_rotated());

    let (subplot_overflow_anchor, anchor_source) =
        O::resolve_evaluate_anchor(place_at_end, title_visible, coord_measurement);
    let adjusted_plot_bounds =
        O::adjusted_plot_bounds(plot_bounds, place_at_end, subplot_overflow_anchor);

    let (font_family, label_font_size, title_font_family, title_font_size) =
        label_and_title_fonts(theme, params);
    let title_override =
        O::title_midpoint_override(coord_measurement, &band_positions, &adjusted_plot_bounds);

    debug!(
        guide = O::log_name(),
        position = ?state.position,
        subplot_overflow_anchor,
        facet_guide_slab_size,
        anchor_source = anchor_source.as_str(),
        guide_visible,
        title_visible,
        sharing_level = state.sharing_level.raw(),
        anchor_policy = O::anchor_policy_label(place_at_end, title_visible),
        labels = ?labels,
        "facet guide evaluate"
    );

    let render_config = FacetLabelRenderConfig {
        labels,
        band_positions,
        plot_bounds: adjusted_plot_bounds,
        is_rotated: O::is_rotated(),
        place_at_end,
        font_family,
        font_size_px: label_font_size,
        title: if title_visible {
            state.facet_title.clone()
        } else {
            None
        },
        title_font_family,
        title_font_size_px: title_font_size,
        render_title: title_visible && state.facet_title.is_some(),
        col_title_x_override: title_override,
    };

    Ok(render_facet_label_slab(&render_config, theme, params))
}

pub(crate) async fn compute_subplot_overflow_common<O: FacetGuideAxisOps>(
    state: &FacetGuideState,
    scales: &HashMap<String, ConfiguredScale>,
    plot_width: f32,
    plot_height: f32,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
    data_override: Option<&DataFrame>,
    ctx: &SessionContext,
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    facet_path: &[ScalarValue],
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    let Some(subplot) = &state.compiled_subplot else {
        return Ok(OverflowSpaceRequirement::default());
    };

    let data_df: Option<DataFrame> = if let Some(data) = data_override {
        Some(data.clone())
    } else if let Some(plan_node) = &state.facet_data_plan {
        plan_node
            .to_logical_plan(ctx)
            .ok()
            .map(|plan| DataFrame::new(ctx.state().clone(), plan))
    } else {
        None
    };

    let Some(data) = data_df.as_ref() else {
        return Ok(OverflowSpaceRequirement::default());
    };

    let band_scale = scales
        .get(O::scale_key())
        .ok_or_else(|| AvengerChartError::InternalError(O::missing_scale_error().to_string()))?;
    let band_size = bandwidth(&band_scale.config)
        .map_err(|e| AvengerChartError::InternalError(format!("Failed to get bandwidth: {e}")))?;
    let (subplot_width, subplot_height) = O::subplot_dimensions(plot_width, plot_height, band_size);

    let subplot_scales = subplot
        .build_scales_for_dataframe(data, subplot_width, subplot_height, ctx, params)
        .await?;
    let configured_scales: HashMap<String, ConfiguredScale> = subplot_scales
        .iter()
        .map(|(k, v)| (k.clone(), v.configured().clone()))
        .collect();

    if let Some(guide) = &subplot.compiled_guide {
        let band_positions: Vec<_> =
            BandPositionIterator::from_configured_scale(band_scale)?.collect();
        if band_positions.is_empty() {
            return guide
                .measure_overflow(
                    &configured_scales,
                    subplot_width,
                    subplot_height,
                    theme,
                    params,
                    Some(data),
                    ctx,
                    facet_tree,
                    facet_path,
                    None,
                )
                .await;
        }

        let cell_values: Vec<_> = band_positions
            .iter()
            .map(|band_position| band_position.value.clone())
            .collect();
        let (first_idx, last_idx) =
            effective_edge_indices_for_values_at_path(facet_tree, facet_path, &cell_values)
                .unwrap_or((0, band_positions.len() - 1));

        let first_path = {
            let mut path = facet_path.to_vec();
            path.push(band_positions[first_idx].value.clone());
            path
        };
        let first_overflow = guide
            .measure_overflow(
                &configured_scales,
                subplot_width,
                subplot_height,
                theme,
                params,
                Some(data),
                ctx,
                facet_tree,
                &first_path,
                None,
            )
            .await?;
        if first_idx == last_idx {
            return Ok(first_overflow);
        }

        let last_path = {
            let mut path = facet_path.to_vec();
            path.push(band_positions[last_idx].value.clone());
            path
        };
        let last_overflow = guide
            .measure_overflow(
                &configured_scales,
                subplot_width,
                subplot_height,
                theme,
                params,
                Some(data),
                ctx,
                facet_tree,
                &last_path,
                None,
            )
            .await?;

        Ok(O::merge_first_last_edge_overflow(
            first_overflow,
            last_overflow,
        ))
    } else {
        Ok(OverflowSpaceRequirement::default())
    }
}

fn label_and_title_fonts(
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> (String, f32, String, f32) {
    let label_ctx = ThemeContext::new("guide", params.clone())
        .child("facet")
        .child("label");
    let title_ctx = ThemeContext::new("guide", params.clone())
        .child("facet")
        .child("title");

    let label_font_size = theme.font_size(&label_ctx).unwrap_or(10.0);
    let title_font_size = theme.font_size(&title_ctx).unwrap_or(12.0);
    let font_family = theme
        .font_family(&label_ctx)
        .unwrap_or_else(|| "sans-serif".to_string());
    let title_font_family = theme
        .font_family(&title_ctx)
        .unwrap_or_else(|| "sans-serif".to_string());

    (
        font_family,
        label_font_size,
        title_font_family,
        title_font_size,
    )
}

fn measure_facet_guide_slab(
    labels: &[String],
    facet_title: Option<&String>,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
    is_rotated: bool,
) -> f32 {
    if labels.is_empty() {
        return 0.0;
    }

    let (font_family, label_font_size, title_font_family, title_font_size) =
        label_and_title_fonts(theme, params);
    let measurement_config = FacetLabelMeasurementConfig {
        labels: labels.to_vec(),
        is_rotated,
        font_family,
        font_size_px: label_font_size,
        title: facet_title.cloned(),
        title_font_family,
        title_font_size_px: title_font_size,
        render_title: facet_title.is_some(),
    };

    measure_facet_label_slab(&measurement_config)
}

fn band_positions_and_labels<O: FacetGuideAxisOps>(
    scales: &HashMap<String, ConfiguredScale>,
    coord_measurement: Option<&dyn CoordMeasurement>,
) -> Result<(Vec<BandPosition>, Vec<String>), AvengerChartError> {
    let band_scale = scales
        .get(O::scale_key())
        .ok_or_else(|| AvengerChartError::InternalError(O::missing_scale_error().to_string()))?;
    let band_positions: Vec<_> = BandPositionIterator::from_configured_scale(band_scale)?.collect();

    let coord_values = coord_measurement.and_then(facet_measurement_values);
    let labels = labels_from_values_or_band_positions(coord_values, &band_positions);

    Ok((band_positions, labels))
}

fn facet_measurement_values(measurement: &dyn CoordMeasurement) -> Option<Vec<ScalarValue>> {
    if let Some(facet_measurement) = measurement.as_any().downcast_ref::<FacetBandCoordMeasurement>()
    {
        return Some(facet_measurement.cell_values().cloned().collect::<Vec<_>>());
    }
    measurement
        .as_any()
        .downcast_ref::<FacetBandProbeMeasurement>()
        .map(|facet_measurement| facet_measurement.cell_values().cloned().collect::<Vec<_>>())
}

fn facet_local_overflow(measurement: &dyn CoordMeasurement) -> Option<CoordinatedOverflow> {
    if let Some(facet_measurement) = measurement.as_any().downcast_ref::<FacetBandCoordMeasurement>()
    {
        return facet_measurement.local_overflow_value();
    }
    measurement
        .as_any()
        .downcast_ref::<FacetBandProbeMeasurement>()
        .map(|facet_measurement| facet_measurement.local_overflow_value())
}

fn labels_from_values_or_band_positions(
    coord_values: Option<Vec<ScalarValue>>,
    band_positions: &[BandPosition],
) -> Vec<String> {
    if let Some(values) = coord_values {
        values.iter().map(format_scalar_value).collect()
    } else {
        band_positions
            .iter()
            .map(|bp| format_scalar_value(&bp.value))
            .collect()
    }
}

fn facet_title_visible_for_cell(
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    facet_path: &[ScalarValue],
    axis_position: AxisPosition,
) -> bool {
    facet_tree
        .axis_visibility_for_path(facet_path, axis_position, SharingLevel::GLOBAL.raw())
        .show_title
}

pub(crate) fn coordinated_overflow_is_zero(overflow: &CoordinatedOverflow) -> bool {
    overflow.guide.top == 0.0
        && overflow.guide.bottom == 0.0
        && overflow.guide.left == 0.0
        && overflow.guide.right == 0.0
        && overflow.total.top == 0.0
        && overflow.total.bottom == 0.0
        && overflow.total.left == 0.0
        && overflow.total.right == 0.0
}

pub(crate) fn preferred_overflow_for_facet_measurement(
    measurement: &dyn CoordMeasurement,
) -> Option<(CoordinatedOverflow, GuideAnchorSource)> {
    if let Some(coordinated) = measurement
        .coordinated_overflow()
        .filter(|overflow| !coordinated_overflow_is_zero(overflow))
    {
        return Some((coordinated.clone(), GuideAnchorSource::CoordinatedGuide));
    }

    facet_local_overflow(measurement).map(|overflow| (overflow, GuideAnchorSource::LocalGuide))
}

pub(crate) fn propagated_subplot_overflow(
    local_overflow: Option<CoordinatedOverflow>,
) -> OverflowSpaceRequirement {
    local_overflow
        .map(|overflow| overflow.guide)
        .unwrap_or_default()
}

pub(crate) fn resolve_row_guide_anchor_overflow_horizontal(
    place_at_right: bool,
    coordinated_overflow: Option<&CoordinatedOverflow>,
    local_overflow: Option<&CoordinatedOverflow>,
) -> (f32, GuideAnchorSource) {
    if let Some(local) = local_overflow {
        return (
            LayoutSlabs::from_coordinated(local).guide_anchor_horizontal(place_at_right),
            GuideAnchorSource::LocalGuide,
        );
    }

    if let Some(coordinated) = coordinated_overflow {
        return (
            LayoutSlabs::from_coordinated(coordinated).guide_anchor_horizontal(place_at_right),
            GuideAnchorSource::CoordinatedGuide,
        );
    }

    (0.0, GuideAnchorSource::DefaultZero)
}

pub(crate) fn resolve_col_guide_anchor_overflow(
    place_at_bottom: bool,
    title_visible: bool,
    coordinated_overflow: Option<&CoordinatedOverflow>,
    local_overflow: Option<&CoordinatedOverflow>,
) -> (f32, GuideAnchorSource) {
    let coordinated_first = place_at_bottom || title_visible;

    if coordinated_first {
        if let Some(coordinated) = coordinated_overflow {
            return (
                LayoutSlabs::from_coordinated(coordinated).guide_anchor(place_at_bottom),
                GuideAnchorSource::CoordinatedGuide,
            );
        }

        if let Some(local) = local_overflow {
            return (
                LayoutSlabs::from_coordinated(local).guide_anchor(place_at_bottom),
                GuideAnchorSource::LocalGuide,
            );
        }
    } else {
        let local_anchor =
            local_overflow.map(|local| LayoutSlabs::from_coordinated(local).guide_anchor(false));
        if let Some(local_anchor) =
            local_anchor.filter(|anchor| anchor.abs() > HIDDEN_TOP_LOCAL_ANCHOR_EPSILON)
        {
            return (local_anchor, GuideAnchorSource::LocalGuide);
        }

        if let Some(coordinated) = coordinated_overflow {
            return (
                LayoutSlabs::from_coordinated(coordinated).guide_anchor(false),
                GuideAnchorSource::CoordinatedGuide,
            );
        }

        if let Some(local_anchor) = local_anchor {
            return (local_anchor, GuideAnchorSource::LocalGuide);
        }
    }

    (0.0, GuideAnchorSource::DefaultZero)
}

fn child_col_span_midpoint(cell: &FacetCellRuntime, recursion_depth: usize) -> Option<f32> {
    let child_facet = cell
        .measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()?;
    if child_facet.axis != FacetAxis::Column {
        return None;
    }

    let child_col_scale = cell.measurement.scales.get("column")?;
    let child_band_positions: Vec<_> = BandPositionIterator::from_scale(child_col_scale)
        .ok()?
        .collect();
    if child_band_positions.is_empty() {
        return None;
    }

    let aligned_child_positions = align_bands_to_nested_child_col_spans_for_measurement(
        &child_band_positions,
        child_facet,
        recursion_depth + 1,
    );
    let first = aligned_child_positions.first()?;
    let last = aligned_child_positions.last()?;
    Some(0.5 * (first.center() + last.center()))
}

fn align_bands_to_nested_child_col_spans_for_measurement(
    band_positions: &[BandPosition],
    facet_measurement: &FacetBandCoordMeasurement,
    recursion_depth: usize,
) -> Vec<BandPosition> {
    let child_midpoints: Vec<Option<f32>> = band_positions
        .iter()
        .enumerate()
        .map(|(idx, _)| {
            facet_measurement
                .cells
                .get(idx)
                .and_then(|cell| child_col_span_midpoint(cell, recursion_depth))
                .filter(|center| center.is_finite())
        })
        .collect();

    align_bands_to_child_span_midpoints(band_positions, &child_midpoints, recursion_depth)
}

fn align_bands_to_child_span_midpoints(
    band_positions: &[BandPosition],
    child_midpoints: &[Option<f32>],
    recursion_depth: usize,
) -> Vec<BandPosition> {
    band_positions
        .iter()
        .enumerate()
        .map(|(idx, band)| {
            let child_span_midpoint = child_midpoints.get(idx).and_then(|center| *center);
            let aligned_center = child_span_midpoint
                .map(|child_center| band.start() + child_center)
                .unwrap_or_else(|| band.center());

            trace!(
                recursion_depth,
                cell_index = idx,
                band_start = band.start(),
                band_center = band.center(),
                child_span_midpoint = child_span_midpoint,
                aligned_center,
                used_child_midpoint = child_span_midpoint.is_some(),
                "FacetColGuide nested span alignment"
            );

            BandPosition::new(
                band.value.clone(),
                aligned_center - 0.5 * band.bandwidth,
                band.bandwidth,
            )
        })
        .collect()
}

fn align_bands_to_nested_child_col_spans(
    band_positions: &[BandPosition],
    coord_measurement: &dyn CoordMeasurement,
) -> Vec<BandPosition> {
    let Some(facet_measurement) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
    else {
        return band_positions.to_vec();
    };

    align_bands_to_nested_child_col_spans_for_measurement(band_positions, facet_measurement, 0)
}

fn coordinated_col_title_midpoint_override(
    coord_measurement: &dyn CoordMeasurement,
    band_positions: &[BandPosition],
    plot_bounds: &LayoutBounds,
) -> Option<f32> {
    let facet_measurement = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()?;
    if facet_measurement.axis != FacetAxis::Column {
        return None;
    }

    let visible_band_count = band_positions.len();
    if visible_band_count == 0 {
        return None;
    }

    let active_layout = facet_measurement
        .coordinated_layout
        .as_ref()
        .unwrap_or(&facet_measurement.local_layout);
    coordinated_col_title_midpoint_override_for_layout(
        active_layout.n,
        active_layout.padding_inner_px,
        band_positions,
        plot_bounds,
    )
}

fn coordinated_col_title_midpoint_override_for_layout(
    effective_slot_count: usize,
    padding_inner_px: f32,
    band_positions: &[BandPosition],
    plot_bounds: &LayoutBounds,
) -> Option<f32> {
    let visible_band_count = band_positions.len();
    if effective_slot_count <= visible_band_count {
        return None;
    }

    let first = band_positions.first()?;
    let slot_step = first.bandwidth + padding_inner_px;
    if !slot_step.is_finite() || slot_step <= 0.0 {
        return None;
    }

    let last_center = first.center() + slot_step * (effective_slot_count.saturating_sub(1) as f32);
    if !last_center.is_finite() {
        return None;
    }

    let override_x = plot_bounds.x + 0.5 * (first.center() + last_center);
    override_x.is_finite().then_some(override_x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn overflow(
        guide_left: f32,
        guide_right: f32,
        guide_top: f32,
        guide_bottom: f32,
        total_left: f32,
        total_right: f32,
        total_top: f32,
        total_bottom: f32,
    ) -> CoordinatedOverflow {
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                left: guide_left,
                right: guide_right,
                top: guide_top,
                bottom: guide_bottom,
            },
            total: OverflowSpaceRequirement {
                left: total_left,
                right: total_right,
                top: total_top,
                bottom: total_bottom,
            },
        }
    }

    #[test]
    fn labels_from_values_or_band_positions_prefers_coord_values() {
        let band_positions = vec![
            BandPosition::new(s("scale-A"), 0.0, 10.0),
            BandPosition::new(s("scale-B"), 10.0, 10.0),
        ];
        let labels = labels_from_values_or_band_positions(
            Some(vec![s("coord-A"), s("coord-B")]),
            &band_positions,
        );
        assert_eq!(labels, vec!["coord-A".to_string(), "coord-B".to_string()]);
    }

    #[test]
    fn merge_first_last_edge_overflow_matches_axis_policies() {
        let first = OverflowSpaceRequirement {
            left: 7.0,
            right: 11.0,
            top: 13.0,
            bottom: 17.0,
        };
        let last = OverflowSpaceRequirement {
            left: 19.0,
            right: 23.0,
            top: 29.0,
            bottom: 31.0,
        };

        let col = ColGuideAxisOps::merge_first_last_edge_overflow(first.clone(), last.clone());
        assert_eq!(col.left, 7.0);
        assert_eq!(col.right, 23.0);
        assert_eq!(col.top, 29.0);
        assert_eq!(col.bottom, 31.0);

        let row = RowGuideAxisOps::merge_first_last_edge_overflow(first, last);
        assert_eq!(row.left, 19.0);
        assert_eq!(row.right, 23.0);
        assert_eq!(row.top, 13.0);
        assert_eq!(row.bottom, 31.0);
    }

    #[test]
    fn column_anchor_policy_matches_hidden_title_behavior() {
        let local = overflow(0.0, 0.0, 12.0, 8.0, 0.0, 0.0, 60.0, 40.0);
        let coordinated = overflow(0.0, 0.0, 7.0, 39.0, 0.0, 0.0, 7.0, 39.0);
        let (resolved, source) =
            resolve_col_guide_anchor_overflow(false, false, Some(&coordinated), Some(&local));
        assert_eq!(resolved, 12.0);
        assert_eq!(source, GuideAnchorSource::LocalGuide);
    }

    #[test]
    fn row_anchor_policy_prefers_local_when_coordinated_present() {
        let local = overflow(12.0, 8.0, 0.0, 0.0, 60.0, 40.0, 0.0, 0.0);
        let coordinated = overflow(5.0, 39.0, 0.0, 0.0, 5.0, 39.0, 0.0, 0.0);
        let (resolved, source) =
            resolve_row_guide_anchor_overflow_horizontal(true, Some(&coordinated), Some(&local));
        assert_eq!(resolved, 8.0);
        assert_eq!(source, GuideAnchorSource::LocalGuide);
    }

    #[test]
    fn column_child_span_alignment_can_shift_label_centers() {
        let bands = vec![
            BandPosition::new(s("A"), 0.0, 10.0),
            BandPosition::new(s("B"), 10.0, 10.0),
        ];
        let aligned = align_bands_to_child_span_midpoints(&bands, &[Some(2.0), None], 0);
        assert_eq!(aligned.len(), 2);
        assert!((aligned[0].center() - 2.0).abs() < 1e-6);
        assert!((aligned[1].center() - bands[1].center()).abs() < 1e-6);
    }

    #[test]
    fn coordinated_col_title_midpoint_override_uses_effective_slot_count() {
        let bands = vec![
            BandPosition::new(s("A"), 10.0, 20.0),
            BandPosition::new(s("B"), 35.0, 20.0),
        ];
        let bounds = LayoutBounds {
            x: 100.0,
            y: 0.0,
            width: 300.0,
            height: 120.0,
        };
        let override_x =
            coordinated_col_title_midpoint_override_for_layout(4, 5.0, &bands, &bounds);
        assert_eq!(override_x, Some(157.5));
    }
}
