//! FacetColGuide implementation for column-based faceting.
//!
//! This module handles measurement and rendering of guide elements (axes, labels)
//! for column-based faceted plots.

use crate::cartesian::axis::CartesianAxis;
use crate::error::AvengerChartError;
use crate::facet::band_positions::BandPositionIterator;
use crate::facet::coord::FacetColCoordMeasurement;
use crate::facet::guide_utils::{
    FacetLabelMeasurementConfig, FacetLabelRenderConfig, measure_facet_label_slab,
    render_facet_label_slab,
};
use crate::facet::layout_plan::effective_edge_indices_for_values_at_path;
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
            coord_measurement.and_then(|cm| cm.as_any().downcast_ref::<FacetColCoordMeasurement>())
        {
            // Fast path: use pre-computed overflow from coord_measurement
            // Use total_overflow to include legend space in outer layout calculation,
            // with the same edge policy used by FacetColCoordMeasurement::local_overflow.
            fcm.local_overflow_value()
                .map(|overflow| overflow.total)
                .unwrap_or_default()
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
            coord_measurement.and_then(|cm| cm.as_any().downcast_ref::<FacetColCoordMeasurement>())
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

        // Add facet guide space for all nesting levels
        // Each level measures and renders its own labels
        let facet_guide_height = if !labels.is_empty() {
            // Get font properties from theme for measurement
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
                labels,
                is_rotated: false, // Column labels are horizontal
                font_family,
                font_size_px: label_font_size,
                title: self.facet_title.clone(),
                title_font_family,
                title_font_size_px: title_font_size,
                render_title: self.facet_title.is_some(),
            };

            measure_facet_label_slab(&measurement_config)
        } else {
            0.0
        };

        // Add facet guide height to top or bottom overflow depending on position
        let place_at_bottom = self.position.as_deref() == Some("bottom");
        let (total_top, total_bottom) = if place_at_bottom {
            (
                subplot_overflow.top,
                subplot_overflow.bottom + facet_guide_height,
            )
        } else {
            (
                subplot_overflow.top + facet_guide_height,
                subplot_overflow.bottom,
            )
        };

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetColGuide measure_overflow: position={:?}, subplot_overflow.top={:.1}, facet_guide_height={:.1}, total_top={:.1}, total_bottom={:.1}",
                self.position, subplot_overflow.top, facet_guide_height, total_top, total_bottom
            );
        }
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
        _facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        _facet_path: &[datafusion::common::ScalarValue],
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
            .downcast_ref::<FacetColCoordMeasurement>()
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

        // Determine position - "bottom" places labels below, default "top" places above
        let place_at_bottom = self.position.as_deref() == Some("bottom");

        // Use coordinated overflow value (computed globally across all facets at this nesting level)
        // This ensures all facet labels at the same depth are aligned
        // Use total overflow (guide + legend) so labels are positioned outside any legends
        let coordinated_overflow = coord_measurement.coordinated_overflow();
        let subplot_overflow = if place_at_bottom {
            coordinated_overflow
                .map(|co| co.total.bottom)
                .unwrap_or(0.0)
        } else {
            coordinated_overflow.map(|co| co.total.top).unwrap_or(0.0)
        };

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "FacetColGuide evaluate: position={:?}, plot_bounds.y={:.1}, subplot_overflow={:.1}, labels={:?}",
                self.position, plot_bounds.y, subplot_overflow, labels
            );
        }

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
            title: self.facet_title.clone(),
            title_font_family,
            title_font_size_px: title_font_size,
            render_title: self.facet_title.is_some(),
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

/// Format a ScalarValue for display as a facet label
fn format_scalar_value(value: &datafusion::common::ScalarValue) -> String {
    use datafusion::common::ScalarValue;

    match value {
        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => s.clone(),
        ScalarValue::Int8(Some(n)) => n.to_string(),
        ScalarValue::Int16(Some(n)) => n.to_string(),
        ScalarValue::Int32(Some(n)) => n.to_string(),
        ScalarValue::Int64(Some(n)) => n.to_string(),
        ScalarValue::UInt8(Some(n)) => n.to_string(),
        ScalarValue::UInt16(Some(n)) => n.to_string(),
        ScalarValue::UInt32(Some(n)) => n.to_string(),
        ScalarValue::UInt64(Some(n)) => n.to_string(),
        ScalarValue::Float32(Some(n)) => format!("{:.2}", n),
        ScalarValue::Float64(Some(n)) => format!("{:.2}", n),
        ScalarValue::Boolean(Some(b)) => b.to_string(),
        _ => format!("{:?}", value),
    }
}
