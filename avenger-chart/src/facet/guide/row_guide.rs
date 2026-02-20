//! FacetRowGuide implementation for row-based faceting.
//!
//! This module handles measurement and rendering of guide elements (axes, labels)
//! for row-based faceted plots.

use crate::cartesian::axis::{AxisPosition, CartesianAxis};
use crate::coords::CoordinatedOverflow;
use crate::error::AvengerChartError;
use crate::facet::band_positions::BandPositionIterator;
use crate::facet::coord::FacetBandCoordMeasurement;
use crate::facet::guide_utils::{
    FacetLabelMeasurementConfig, FacetLabelRenderConfig, format_scalar_value,
    measure_facet_label_slab, render_facet_label_slab,
};
use crate::facet::layout_plan::effective_edge_indices_for_values_at_path;
use crate::facet::layout_slabs::LayoutSlabs;
use crate::facet::marks::facet::CompiledFacetRow;
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

/// Guide configuration for FacetRow coordinate system
#[derive(Clone, Default)]
pub struct FacetRowGuideConfig {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot extracted from the facet mark
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet mark's data (used when data_override is None)
    facet_data_plan: Option<LogicalPlanNode>,
    /// Position of facet labels ("left" or "right")
    position: Option<String>,
}

impl FacetRowGuideConfig {
    /// Create a new FacetRowGuideConfig
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the facet title
    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.facet_title = title;
        self
    }
}

impl CoordinateGuide for FacetRowGuideConfig {
    type Axis = CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // Facet guides do not compose cartesian axes directly.
    }

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        for mark in &compiled_marks {
            if let Some(facet_row) = mark.as_any().downcast_ref::<CompiledFacetRow>() {
                self.compiled_subplot = Some(facet_row.compiled_subplot().clone());
                self.facet_data_plan = mark.data_context().logical_plan_node().cloned();
                self.facet_title = facet_row.facet_title().map(|s| s.to_string());
                self.position = facet_row.facet_position().map(|s| s.to_string());
                break;
            }
        }
    }

    fn update(&mut self, _other: Self) {
        // No-op.
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(FacetRowGuide {
            facet_title: self.facet_title,
            compiled_subplot: self.compiled_subplot,
            facet_data_plan: self.facet_data_plan,
            position: self.position,
        })
    }
}

/// Compiled guide for FacetRow coordinate system
#[serde_as]
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetRowGuide {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot for measuring overflow
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet mark's data (used when data_override is None)
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    facet_data_plan: Option<LogicalPlanNode>,
    /// Position of facet labels ("left" or "right")
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

fn resolve_guide_anchor_overflow_horizontal(
    place_at_right: bool,
    coordinated_overflow: Option<&CoordinatedOverflow>,
    local_overflow: Option<&CoordinatedOverflow>,
) -> (f32, GuideAnchorSource) {
    // Prefer the local guide anchor so per-branch row guides stay aligned with
    // their own subplot strip, even when coordinated siblings have wider titles.
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

fn preferred_overflow_for_facet_measurement(
    facet_measurement: &FacetBandCoordMeasurement,
) -> Option<CoordinatedOverflow> {
    if !coordinated_overflow_is_zero(&facet_measurement.coordinated_overflow) {
        Some(facet_measurement.coordinated_overflow.clone())
    } else {
        facet_measurement.local_overflow_value()
    }
}

fn propagated_subplot_overflow(
    local_overflow: Option<CoordinatedOverflow>,
) -> OverflowSpaceRequirement {
    local_overflow
        .map(|overflow| overflow.guide)
        .unwrap_or_default()
}

fn measure_facet_guide_width(
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
        is_rotated: true,
        font_family,
        font_size_px: label_font_size,
        title: facet_title.cloned(),
        title_font_family,
        title_font_size_px: title_font_size,
        render_title: facet_title.is_some(),
    };

    measure_facet_label_slab(&measurement_config)
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
impl CompiledGuide for FacetRowGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        _plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let subplot_overflow = if let Some(fcm) =
            coord_measurement.and_then(|cm| cm.as_any().downcast_ref::<FacetBandCoordMeasurement>())
        {
            // Propagate guide-only overflow through nested facet fast paths.
            // Legend slabs are applied at the legend-owning facet level only.
            propagated_subplot_overflow(fcm.local_overflow_value())
        } else {
            self.compute_subplot_overflow(
                scales,
                plot_width,
                theme,
                params,
                data_override,
                ctx,
                facet_tree,
                facet_path,
            )
            .await?
        };

        let labels: Vec<String> = if let Some(fcm) =
            coord_measurement.and_then(|cm| cm.as_any().downcast_ref::<FacetBandCoordMeasurement>())
        {
            fcm.cell_values().map(format_scalar_value).collect()
        } else {
            let row_scale = scales
                .get("row")
                .ok_or_else(|| AvengerChartError::InternalError("No row scale found".into()))?;
            let band_iter = BandPositionIterator::from_configured_scale(row_scale)?;
            band_iter.map(|bp| format_scalar_value(&bp.value)).collect()
        };

        let place_at_right = self.position.as_deref() != Some("left");
        let title_visible = facet_title_visible_for_cell(
            facet_tree,
            facet_path,
            if place_at_right {
                AxisPosition::Right
            } else {
                AxisPosition::Left
            },
        );
        let title_for_cell = if title_visible {
            self.facet_title.as_ref()
        } else {
            None
        };
        let facet_guide_width = measure_facet_guide_width(&labels, title_for_cell, theme, params);

        let local_overflow = coord_measurement
            .and_then(|measurement| {
                measurement
                    .as_any()
                    .downcast_ref::<FacetBandCoordMeasurement>()
            })
            .and_then(preferred_overflow_for_facet_measurement);

        let guide_anchor_side = local_overflow
            .as_ref()
            .map(|overflow| {
                if place_at_right {
                    overflow.guide.right
                } else {
                    overflow.guide.left
                }
            })
            .unwrap_or_else(|| {
                if place_at_right {
                    subplot_overflow.right
                } else {
                    subplot_overflow.left
                }
            });

        let (total_left, total_right) = if place_at_right {
            (subplot_overflow.left, guide_anchor_side + facet_guide_width)
        } else {
            (
                guide_anchor_side + facet_guide_width,
                subplot_overflow.right,
            )
        };

        debug!(
            position = ?self.position,
            subplot_overflow_left = subplot_overflow.left,
            subplot_overflow_right = subplot_overflow.right,
            guide_anchor_side,
            title_visible,
            facet_guide_width,
            total_left,
            total_right,
            "FacetRowGuide measure_overflow"
        );

        Ok(OverflowSpaceRequirement {
            top: subplot_overflow.top,
            bottom: subplot_overflow.bottom,
            left: total_left,
            right: total_right,
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
        let row_scale = scales
            .get("row")
            .ok_or_else(|| AvengerChartError::InternalError("No row scale found".into()))?;

        let (band_positions, labels): (Vec<_>, Vec<String>) = if let Some(fcm) = coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
        {
            let band_iter = BandPositionIterator::from_configured_scale(row_scale)?;
            let positions: Vec<_> = band_iter.collect();
            let labels: Vec<String> = fcm.cell_values().map(format_scalar_value).collect();
            (positions, labels)
        } else {
            let band_iter = BandPositionIterator::from_configured_scale(row_scale)?;
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

        let place_at_right = self.position.as_deref() != Some("left");
        let title_visible = facet_title_visible_for_cell(
            facet_tree,
            facet_path,
            if place_at_right {
                AxisPosition::Right
            } else {
                AxisPosition::Left
            },
        );
        let title_for_cell = if title_visible {
            self.facet_title.as_ref()
        } else {
            None
        };
        let facet_guide_width = measure_facet_guide_width(&labels, title_for_cell, theme, params);

        let coordinated_overflow = coord_measurement.coordinated_overflow();
        let local_overflow = coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .and_then(|fcm| fcm.local_overflow_value());
        let (subplot_overflow, anchor_source) = resolve_guide_anchor_overflow_horizontal(
            place_at_right,
            coordinated_overflow,
            local_overflow.as_ref(),
        );

        debug!(
            position = ?self.position,
            plot_bounds_x = plot_bounds.x,
            subplot_overflow,
            facet_guide_width,
            anchor_source = anchor_source.as_str(),
            coordinated_guide_left = coordinated_overflow.map(|co| co.guide.left),
            coordinated_guide_right = coordinated_overflow.map(|co| co.guide.right),
            coordinated_total_left = coordinated_overflow.map(|co| co.total.left),
            coordinated_total_right = coordinated_overflow.map(|co| co.total.right),
            local_guide_left = local_overflow.as_ref().map(|co| co.guide.left),
            local_guide_right = local_overflow.as_ref().map(|co| co.guide.right),
            local_total_left = local_overflow.as_ref().map(|co| co.total.left),
            local_total_right = local_overflow.as_ref().map(|co| co.total.right),
            title_visible,
            labels = ?labels,
            "FacetRowGuide evaluate"
        );

        let adjusted_plot_bounds = if place_at_right {
            LayoutBounds {
                x: plot_bounds.x + subplot_overflow,
                y: plot_bounds.y,
                width: plot_bounds.width,
                height: plot_bounds.height,
            }
        } else {
            LayoutBounds {
                x: plot_bounds.x - subplot_overflow,
                y: plot_bounds.y,
                width: plot_bounds.width,
                height: plot_bounds.height,
            }
        };

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

        let render_config = FacetLabelRenderConfig {
            labels,
            band_positions,
            plot_bounds: adjusted_plot_bounds,
            is_rotated: true,
            place_at_end: place_at_right,
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
            col_title_x_override: None,
        };

        Ok(render_facet_label_slab(&render_config, theme, params))
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl FacetRowGuide {
    async fn compute_subplot_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
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

        let data_df: Option<DataFrame> = if let Some(data) = data_override {
            Some(data.clone())
        } else if let Some(plan_node) = &self.facet_data_plan {
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

        let row_scale = scales
            .get("row")
            .ok_or_else(|| AvengerChartError::InternalError("No row scale found".into()))?;

        let subplot_height =
            avenger_scales::scales::band::bandwidth(&row_scale.config).map_err(|e| {
                AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e))
            })?;

        let subplot_scales = subplot
            .build_scales_for_dataframe(data, plot_width, subplot_height, ctx, params)
            .await?;

        let configured_scales: HashMap<String, ConfiguredScale> = subplot_scales
            .iter()
            .map(|(k, v)| (k.clone(), v.configured().clone()))
            .collect();

        if let Some(guide) = &subplot.compiled_guide {
            let band_positions: Vec<_> =
                BandPositionIterator::from_configured_scale(row_scale)?.collect();

            if band_positions.is_empty() {
                return guide
                    .measure_overflow(
                        &configured_scales,
                        plot_width,
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
                    plot_width,
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
                    plot_width,
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

            Ok(OverflowSpaceRequirement {
                left: first_overflow.left.max(last_overflow.left),
                right: first_overflow.right.max(last_overflow.right),
                top: first_overflow.top,
                bottom: last_overflow.bottom,
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
        guide_left: f32,
        guide_right: f32,
        total_left: f32,
        total_right: f32,
    ) -> CoordinatedOverflow {
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                left: guide_left,
                right: guide_right,
                ..Default::default()
            },
            total: OverflowSpaceRequirement {
                left: total_left,
                right: total_right,
                ..Default::default()
            },
        }
    }

    #[test]
    fn resolve_guide_anchor_right_prefers_guide_over_total() {
        let coordinated = coordinated_overflow(13.0, 41.0, 13.0, 55.0);
        let (resolved, source) =
            resolve_guide_anchor_overflow_horizontal(true, Some(&coordinated), None);
        assert_eq!(resolved, 41.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_left_prefers_guide_over_total() {
        let coordinated = coordinated_overflow(7.0, 41.0, 55.0, 41.0);
        let (resolved, source) =
            resolve_guide_anchor_overflow_horizontal(false, Some(&coordinated), None);
        assert_eq!(resolved, 7.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_falls_back_to_local_guide_overflow() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let (resolved_right, source_right) =
            resolve_guide_anchor_overflow_horizontal(true, None, Some(&local));
        let (resolved_left, source_left) =
            resolve_guide_anchor_overflow_horizontal(false, None, Some(&local));
        assert_eq!(resolved_right, 8.0);
        assert_eq!(source_right, GuideAnchorSource::LocalGuide);
        assert_eq!(resolved_left, 12.0);
        assert_eq!(source_left, GuideAnchorSource::LocalGuide);
    }

    #[test]
    fn resolve_guide_anchor_prefers_local_over_coordinated_when_both_present() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let coordinated = coordinated_overflow(5.0, 39.0, 5.0, 39.0);
        let (resolved_right, source_right) =
            resolve_guide_anchor_overflow_horizontal(true, Some(&coordinated), Some(&local));
        let (resolved_left, source_left) =
            resolve_guide_anchor_overflow_horizontal(false, Some(&coordinated), Some(&local));
        assert_eq!(resolved_right, 8.0);
        assert_eq!(source_right, GuideAnchorSource::LocalGuide);
        assert_eq!(resolved_left, 12.0);
        assert_eq!(source_left, GuideAnchorSource::LocalGuide);
    }

    #[test]
    fn resolve_guide_anchor_defaults_to_zero_without_overflow_context() {
        let (resolved, source) = resolve_guide_anchor_overflow_horizontal(true, None, None);
        assert_eq!(resolved, 0.0);
        assert_eq!(source, GuideAnchorSource::DefaultZero);
    }

    #[test]
    fn propagated_subplot_overflow_uses_guide_not_total() {
        let local = coordinated_overflow(11.0, 23.0, 49.0, 65.0);
        let propagated = propagated_subplot_overflow(Some(local));
        assert_eq!(propagated.left, 11.0);
        assert_eq!(propagated.right, 23.0);
    }
}
