//! FacetColGuide implementation for column-based faceting.
//!
//! This module handles measurement and rendering of guide elements (axes, labels)
//! for column-based faceted plots.

use crate::cartesian::axis::{AxisPosition, CartesianAxis};
use crate::coords::{CoordinatedOverflow, FacetAxis};
use crate::error::AvengerChartError;
use crate::facet::band_positions::{BandPosition, BandPositionIterator};
use crate::facet::coord::FacetBandCoordMeasurement;
use crate::facet::guide_utils::{
    FacetLabelMeasurementConfig, FacetLabelRenderConfig, format_scalar_value,
    measure_facet_label_slab, render_facet_label_slab,
};
use crate::facet::layout_plan::effective_edge_indices_for_values_at_path;
use crate::facet::layout_slabs::LayoutSlabs;
use crate::facet::marks::facet::CompiledFacetCol;
use crate::guide::{CompiledGuide, CoordinateGuide, MeasurementResult, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
use crate::plot::compiled::CompiledPlot;
use crate::serialization::SerializableDataFrame;
use crate::theme::ThemeContext;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// Guide configuration for FacetCol coordinate system
#[derive(Clone, Default)]
pub struct FacetColGuideConfig {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot extracted from the facet mark
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet mark's data (used when data_override is None)
    facet_data_plan: Option<LogicalPlanNode>,
    /// Position of facet labels ("top" or "bottom")
    position: Option<String>,
}

impl FacetColGuideConfig {
    /// Create a new FacetColGuideConfig
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the facet title
    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.facet_title = title;
        self
    }
}

impl CoordinateGuide for FacetColGuideConfig {
    type Axis = CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // Stubbed - no-op
    }

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        // Find the CompiledFacetCol mark and extract its subplot, data, title, and position
        for mark in &compiled_marks {
            if let Some(facet_col) = mark.as_any().downcast_ref::<CompiledFacetCol>() {
                self.compiled_subplot = Some(facet_col.compiled_subplot().clone());
                // Extract the logical plan from the mark's data context
                self.facet_data_plan = mark.data_context().logical_plan_node().cloned();
                // Extract the facet title from the mark
                self.facet_title = facet_col.facet_title().map(|s| s.to_string());
                // Extract the facet position from the mark
                self.position = facet_col.facet_position().map(|s| s.to_string());
                break;
            }
        }
    }

    fn update(&mut self, _other: Self) {
        // Stubbed - no-op
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(FacetColGuide {
            facet_title: self.facet_title,
            compiled_subplot: self.compiled_subplot,
            facet_data_plan: self.facet_data_plan,
            position: self.position,
        })
    }
}

/// Compiled guide for FacetCol coordinate system
///
/// Renders facet labels horizontally below (or above) the plot area with one
/// label per column.
#[serde_as]
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetColGuide {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot for measuring overflow
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet mark's data (used when data_override is None)
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    facet_data_plan: Option<LogicalPlanNode>,
    /// Position of facet labels ("top" or "bottom")
    position: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GuideAnchorSource {
    CoordinatedGuide,
    LocalGuide,
    DefaultZero,
}

impl GuideAnchorSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::CoordinatedGuide => "coordinated_guide",
            Self::LocalGuide => "local_guide",
            Self::DefaultZero => "default_zero",
        }
    }
}

fn resolve_guide_anchor_overflow(
    place_at_bottom: bool,
    coordinated_overflow: Option<&CoordinatedOverflow>,
    local_overflow: Option<&CoordinatedOverflow>,
) -> (f32, GuideAnchorSource) {
    // Prefer coordinated guide anchors so sibling branches render at a
    // consistent guide depth after coordination.
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

    (0.0, GuideAnchorSource::DefaultZero)
}

fn coordinated_overflow_is_zero(overflow: &CoordinatedOverflow) -> bool {
    overflow.guide.top == 0.0
        && overflow.guide.bottom == 0.0
        && overflow.guide.left == 0.0
        && overflow.guide.right == 0.0
        && overflow.total.top == 0.0
        && overflow.total.bottom == 0.0
        && overflow.total.left == 0.0
        && overflow.total.right == 0.0
}

fn propagated_subplot_overflow(
    local_overflow: Option<CoordinatedOverflow>,
) -> OverflowSpaceRequirement {
    local_overflow
        .map(|overflow| overflow.guide)
        .unwrap_or_default()
}

fn measure_facet_guide_height(
    labels: &[String],
    facet_title: Option<&String>,
    theme: &crate::theme::Theme,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> f32 {
    if labels.is_empty() {
        return 0.0;
    }

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

    let measurement_config = FacetLabelMeasurementConfig {
        labels: labels.to_vec(),
        is_rotated: false, // Column labels are horizontal
        font_family,
        font_size_px: label_font_size,
        title: facet_title.cloned(),
        title_font_family,
        title_font_size_px: title_font_size,
        render_title: facet_title.is_some(),
    };

    measure_facet_label_slab(&measurement_config)
}

fn child_col_span_midpoint(cell: &crate::facet::coord::FacetCellRuntime) -> Option<f32> {
    let child_facet = cell
        .measurement
        .coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()?;
    if child_facet.axis != FacetAxis::Column {
        return None;
    }

    let child_col_scale = cell.measurement.scales.get("column")?;
    let mut band_iter = BandPositionIterator::from_scale(child_col_scale).ok()?;
    let first = band_iter.next()?;
    let mut last = first.clone();
    for position in band_iter {
        last = position;
    }

    Some(0.5 * (first.center() + last.center()))
}

fn align_bands_to_nested_child_col_spans(
    band_positions: &[BandPosition],
    coord_measurement: &dyn crate::coords::CoordMeasurement,
) -> Vec<BandPosition> {
    let Some(facet_measurement) = coord_measurement
        .as_any()
        .downcast_ref::<FacetBandCoordMeasurement>()
    else {
        return band_positions.to_vec();
    };

    band_positions
        .iter()
        .enumerate()
        .map(|(idx, band)| {
            let aligned_center = facet_measurement
                .cells
                .get(idx)
                .and_then(child_col_span_midpoint)
                .filter(|center| center.is_finite())
                .map(|child_center| band.start() + child_center)
                .unwrap_or_else(|| band.center());

            BandPosition::new(
                band.value.clone(),
                aligned_center - 0.5 * band.bandwidth,
                band.bandwidth,
            )
        })
        .collect()
}

fn facet_title_visible_for_cell(
    facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
    facet_path: &[datafusion::common::ScalarValue],
    axis_position: AxisPosition,
) -> bool {
    facet_tree
        .axis_visibility_for_path(facet_path, axis_position, 255)
        .show_title
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for FacetColGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // Try to extract pre-computed subplot overflow from coord_measurement.
        // This optimization avoids expensive re-measurement when coord_measurement
        // is already populated (second pass).
        let subplot_overflow = if let Some(fcm) =
            coord_measurement.and_then(|cm| cm.as_any().downcast_ref::<FacetBandCoordMeasurement>())
        {
            // Fast path: propagate guide-only overflow to parent facet levels.
            // Legend slabs are reserved where legends render and must not be
            // recursively re-applied by ancestor facets.
            propagated_subplot_overflow(fcm.local_overflow_value())
        } else {
            // Slow path: compute subplot overflow (first pass, before coord_measurement exists)
            self.compute_subplot_overflow(
                scales,
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

        // Get labels for measurement.
        // Priority: Use cell_values from coord_measurement (has Level(N)-aware enumeration)
        // Fallback: Use column scale domain (first pass before coord_measurement exists)
        let labels: Vec<String> = if let Some(fcm) =
            coord_measurement.and_then(|cm| cm.as_any().downcast_ref::<FacetBandCoordMeasurement>())
        {
            // Second pass: use Level(N)-aware cell_values from coord measurement
            fcm.cell_values().map(format_scalar_value).collect()
        } else {
            // First pass: fall back to scale domain
            let column_scale = scales
                .get("column")
                .ok_or_else(|| AvengerChartError::InternalError("No column scale found".into()))?;
            let band_iter = BandPositionIterator::from_configured_scale(column_scale)?;
            band_iter.map(|bp| format_scalar_value(&bp.value)).collect()
        };

        let place_at_bottom = self.position.as_deref() == Some("bottom");
        let title_visible = facet_title_visible_for_cell(
            facet_tree,
            facet_path,
            if place_at_bottom {
                AxisPosition::Bottom
            } else {
                AxisPosition::Top
            },
        );
        let title_for_cell = if title_visible {
            self.facet_title.as_ref()
        } else {
            None
        };

        // Add facet guide space for all nesting levels
        // Each level measures and renders its own labels
        let facet_guide_height = measure_facet_guide_height(&labels, title_for_cell, theme, params);

        let coordinated_overflow = coord_measurement.and_then(|measurement| {
            let coordinated = measurement.coordinated_overflow()?;
            if coordinated_overflow_is_zero(coordinated) {
                None
            } else {
                Some(coordinated)
            }
        });
        let local_overflow = coord_measurement
            .and_then(|measurement| {
                measurement
                    .as_any()
                    .downcast_ref::<FacetBandCoordMeasurement>()
            })
            .and_then(|fcm| fcm.local_overflow_value());
        let (resolved_anchor, anchor_source) = resolve_guide_anchor_overflow(
            place_at_bottom,
            coordinated_overflow,
            local_overflow.as_ref(),
        );
        let default_subplot_anchor = if place_at_bottom {
            subplot_overflow.bottom
        } else {
            subplot_overflow.top
        };
        let guide_anchor = if matches!(anchor_source, GuideAnchorSource::DefaultZero) {
            default_subplot_anchor
        } else {
            resolved_anchor
        };

        // Add facet guide height to top or bottom overflow depending on position.
        let (total_top, total_bottom) = if place_at_bottom {
            (subplot_overflow.top, guide_anchor + facet_guide_height)
        } else {
            (guide_anchor + facet_guide_height, subplot_overflow.bottom)
        };

        debug!(
            position = ?self.position,
            subplot_overflow_top = subplot_overflow.top,
            subplot_overflow_bottom = subplot_overflow.bottom,
            guide_anchor,
            anchor_source = anchor_source.as_str(),
            title_visible,
            facet_guide_height,
            total_top,
            total_bottom,
            "FacetColGuide measure_overflow"
        );
        Ok(OverflowSpaceRequirement {
            top: total_top,
            bottom: total_bottom,
            left: subplot_overflow.left,
            right: subplot_overflow.right,
        })
    }

    async fn measure_with_coordination(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &crate::theme::Theme,
        _params: &IndexMap<String, datafusion::common::ScalarValue>,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
        _ctx: &SessionContext,
        _facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        _facet_path: &[datafusion::common::ScalarValue],
        _coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
    ) -> Result<MeasurementResult, AvengerChartError> {
        // Return default measurement result
        Ok(MeasurementResult::default())
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: &dyn crate::coords::CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Render facet labels for all nesting levels
        // Each level renders its own labels in its local coordinate system

        // Get column scale for band positions
        let column_scale = scales
            .get("column")
            .ok_or_else(|| AvengerChartError::InternalError("No column scale found".into()))?;

        // Get band positions and labels from coord_measurement's cell_values (Level(N)-aware)
        // or fall back to scale domain
        let (band_positions, labels): (Vec<_>, Vec<String>) = if let Some(fcm) = coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        {
            // Use cell_values from coord measurement (Level(N)-aware enumeration)
            let band_iter = BandPositionIterator::from_configured_scale(column_scale)?;
            let positions: Vec<_> = band_iter.collect();
            let labels: Vec<String> = fcm.cell_values().map(format_scalar_value).collect();
            (positions, labels)
        } else {
            // Fall back to scale domain
            let band_iter = BandPositionIterator::from_configured_scale(column_scale)?;
            let positions: Vec<_> = band_iter.collect();
            let labels: Vec<String> = positions
                .iter()
                .map(|bp| format_scalar_value(&bp.value))
                .collect();
            (positions, labels)
        };

        if band_positions.is_empty() {
            return Ok(vec![]);
        }
        let band_positions =
            align_bands_to_nested_child_col_spans(&band_positions, coord_measurement);

        // Determine position - "bottom" places labels below, default "top" places above
        let place_at_bottom = self.position.as_deref() == Some("bottom");
        let title_visible = facet_title_visible_for_cell(
            facet_tree,
            facet_path,
            if place_at_bottom {
                AxisPosition::Bottom
            } else {
                AxisPosition::Top
            },
        );
        let title_for_cell = if title_visible {
            self.facet_title.as_ref()
        } else {
            None
        };
        let facet_guide_height = measure_facet_guide_height(&labels, title_for_cell, theme, params);

        // Anchor facet guide rendering to GUIDE overflow, not total overflow.
        // This preserves outer->inner facet-guide hierarchy under top legends.
        //
        // Legend space is still reserved via measure_overflow/layout, but legends should not
        // reorder nested facet guide slabs during rendering.
        let coordinated_overflow = coord_measurement.coordinated_overflow();
        let local_overflow = coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .and_then(|fcm| fcm.local_overflow_value());
        let (subplot_overflow, anchor_source) = resolve_guide_anchor_overflow(
            place_at_bottom,
            coordinated_overflow,
            local_overflow.as_ref(),
        );

        debug!(
            position = ?self.position,
            plot_bounds_y = plot_bounds.y,
            subplot_overflow,
            facet_guide_height,
            anchor_source = anchor_source.as_str(),
            coordinated_guide_top = coordinated_overflow.map(|co| co.guide.top),
            coordinated_guide_bottom = coordinated_overflow.map(|co| co.guide.bottom),
            coordinated_total_top = coordinated_overflow.map(|co| co.total.top),
            coordinated_total_bottom = coordinated_overflow.map(|co| co.total.bottom),
            local_guide_top = local_overflow.as_ref().map(|co| co.guide.top),
            local_guide_bottom = local_overflow.as_ref().map(|co| co.guide.bottom),
            local_total_top = local_overflow.as_ref().map(|co| co.total.top),
            local_total_bottom = local_overflow.as_ref().map(|co| co.total.bottom),
            title_visible,
            labels = ?labels,
            "FacetColGuide evaluate"
        );

        // Adjust plot_bounds to account for subplot overflow
        // For position=top: shift up by top overflow (subtract, since y increases downward)
        // For position=bottom: shift down by bottom overflow
        // Note: calculate_col_label_positions adds plot_bounds.height when place_at_end=true,
        // so we only need to add the subplot_overflow here
        let adjusted_plot_bounds = if place_at_bottom {
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y + subplot_overflow,
                width: plot_bounds.width,
                height: plot_bounds.height,
            }
        } else {
            LayoutBounds {
                x: plot_bounds.x,
                y: plot_bounds.y - subplot_overflow,
                width: plot_bounds.width,
                height: plot_bounds.height,
            }
        };

        // Get font properties from theme
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

        // Configure rendering with adjusted bounds
        let render_config = FacetLabelRenderConfig {
            labels,
            band_positions,
            plot_bounds: adjusted_plot_bounds,
            is_rotated: false, // Column labels are horizontal
            place_at_end: place_at_bottom,
            font_family,
            font_size_px: label_font_size,
            title: if title_visible {
                self.facet_title.clone()
            } else {
                None
            },
            title_font_family,
            title_font_size_px: title_font_size,
            render_title: title_visible && self.facet_title.is_some(),
        };

        Ok(render_facet_label_slab(&render_config, theme, params))
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        // Facet guides don't clip - nested coordinate systems handle their own clipping
        Clip::None
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl FacetColGuide {
    /// Compute subplot overflow by measuring the subplot's guide.
    /// This is the slow path used when coord_measurement doesn't have pre-computed data.
    async fn compute_subplot_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        use datafusion::dataframe::DataFrame;

        use crate::serialization::LogicalPlanNodeExt;

        let Some(subplot) = &self.compiled_subplot else {
            return Ok(OverflowSpaceRequirement::default());
        };

        // Get data from data_override, or fall back to stored facet_data_plan
        let data_df: Option<DataFrame> = if let Some(data) = data_override {
            Some(data.clone())
        } else if let Some(plan_node) = &self.facet_data_plan {
            // Reconstruct DataFrame from stored logical plan
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

        // Get column scale to determine subplot width
        let column_scale = scales
            .get("column")
            .ok_or_else(|| AvengerChartError::InternalError("No column scale found".into()))?;

        let subplot_width =
            avenger_scales::scales::band::bandwidth(&column_scale.config).map_err(|e| {
                AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e))
            })?;

        // Build subplot scales from full data
        let subplot_scales = subplot
            .build_scales_for_dataframe(data, subplot_width, plot_height, ctx, params)
            .await?;

        // Measure subplot guide overflow
        let configured_scales: HashMap<String, ConfiguredScale> = subplot_scales
            .iter()
            .map(|(k, v)| (k.clone(), v.configured().clone()))
            .collect();

        if let Some(guide) = &subplot.compiled_guide {
            // For correct visibility-aware measurement in nested facets, we need to check
            // both first and last cells' paths. The first cell's left overflow becomes the
            // parent's left overflow, and the last cell's right overflow becomes the
            // parent's right overflow.
            let band_positions: Vec<_> =
                BandPositionIterator::from_configured_scale(column_scale)?.collect();

            if band_positions.is_empty() {
                return guide
                    .measure_overflow(
                        &configured_scales,
                        subplot_width,
                        plot_height,
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

            // Measure first cell for correct left overflow (Y-axis on left)
            let first_path = {
                let mut path = facet_path.to_vec();
                path.push(band_positions[first_idx].value.clone());
                path
            };
            let first_overflow = guide
                .measure_overflow(
                    &configured_scales,
                    subplot_width,
                    plot_height,
                    theme,
                    params,
                    Some(data),
                    ctx,
                    facet_tree,
                    &first_path,
                    None,
                )
                .await?;

            // If only one cell, first and last are the same
            if first_idx == last_idx {
                return Ok(first_overflow);
            }

            // Measure last cell for correct right overflow (Y-axis on right, if any)
            let last_path = {
                let mut path = facet_path.to_vec();
                path.push(band_positions[last_idx].value.clone());
                path
            };
            let last_overflow = guide
                .measure_overflow(
                    &configured_scales,
                    subplot_width,
                    plot_height,
                    theme,
                    params,
                    Some(data),
                    ctx,
                    facet_tree,
                    &last_path,
                    None,
                )
                .await?;

            // Combine: first's left, last's right, max of top/bottom
            Ok(OverflowSpaceRequirement {
                left: first_overflow.left,
                right: last_overflow.right,
                top: first_overflow.top.max(last_overflow.top),
                bottom: first_overflow.bottom.max(last_overflow.bottom),
            })
        } else {
            Ok(OverflowSpaceRequirement::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coordinated_overflow(
        guide_top: f32,
        guide_bottom: f32,
        total_top: f32,
        total_bottom: f32,
    ) -> CoordinatedOverflow {
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                top: guide_top,
                bottom: guide_bottom,
                ..Default::default()
            },
            total: OverflowSpaceRequirement {
                top: total_top,
                bottom: total_bottom,
                ..Default::default()
            },
        }
    }

    #[test]
    fn resolve_guide_anchor_top_prefers_guide_over_total() {
        let coordinated = coordinated_overflow(41.0, 34.0, 55.0, 34.0);
        let (resolved, source) = resolve_guide_anchor_overflow(false, Some(&coordinated), None);
        assert_eq!(resolved, 41.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_bottom_prefers_guide_over_total() {
        let coordinated = coordinated_overflow(41.0, 7.0, 41.0, 55.0);
        let (resolved, source) = resolve_guide_anchor_overflow(true, Some(&coordinated), None);
        assert_eq!(resolved, 7.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_falls_back_to_local_guide_overflow() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let (resolved_top, source_top) = resolve_guide_anchor_overflow(false, None, Some(&local));
        let (resolved_bottom, source_bottom) =
            resolve_guide_anchor_overflow(true, None, Some(&local));
        assert_eq!(resolved_top, 12.0);
        assert_eq!(source_top, GuideAnchorSource::LocalGuide);
        assert_eq!(resolved_bottom, 8.0);
        assert_eq!(source_bottom, GuideAnchorSource::LocalGuide);
    }

    #[test]
    fn resolve_guide_anchor_prefers_coordinated_over_local_when_both_present() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let coordinated = coordinated_overflow(5.0, 39.0, 5.0, 39.0);
        let (resolved_top, source_top) =
            resolve_guide_anchor_overflow(false, Some(&coordinated), Some(&local));
        let (resolved_bottom, source_bottom) =
            resolve_guide_anchor_overflow(true, Some(&coordinated), Some(&local));
        assert_eq!(resolved_top, 5.0);
        assert_eq!(source_top, GuideAnchorSource::CoordinatedGuide);
        assert_eq!(resolved_bottom, 39.0);
        assert_eq!(source_bottom, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_defaults_to_zero_without_overflow_context() {
        let (resolved, source) = resolve_guide_anchor_overflow(false, None, None);
        assert_eq!(resolved, 0.0);
        assert_eq!(source, GuideAnchorSource::DefaultZero);
    }

    #[test]
    fn propagated_subplot_overflow_uses_guide_not_total() {
        let local = coordinated_overflow(17.0, 9.0, 53.0, 41.0);
        let propagated = propagated_subplot_overflow(Some(local));
        assert_eq!(propagated.top, 17.0);
        assert_eq!(propagated.bottom, 9.0);
    }
}
