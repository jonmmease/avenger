//! Layout computation
//!
//! This module handles:
//! - Computing plot layout with guide overflow
//! - Dynamic layout with coordinate system guides
//! - Converting overflow requirements to padding
//! - Integrating with Taffy layout system

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::render::types::INITIAL_PLOT_AREA_RATIO;
use crate::render::{LayoutSolution, Padding};
use std::collections::HashMap;

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Compute layout using the coordinate system's capabilities
    pub(super) async fn compute_layout(
        &self,
        width: f32,
        height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Result<LayoutSolution, AvengerChartError> {
        // Create guide with all configurations applied
        let guide = self.create_configured_guide(scales);

        // Call the dynamic layout helper with the guide
        self.compute_layout_with_dynamic_guide(width, height, scales, guide)
            .await
    }

    /// Helper method for dynamic layout with guide
    pub(super) async fn compute_layout_with_dynamic_guide(
        &self,
        width: f32,
        height: f32,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        guide: C::Guide,
    ) -> Result<LayoutSolution, AvengerChartError> {
        // Check for required positional scales before measuring overflow
        // This ensures we provide proper error messages for literal values
        self.validate_positional_scales_exist(scales)?;

        // Measure how much space the guide needs
        let width_estimate = width * INITIAL_PLOT_AREA_RATIO;
        let height_estimate = height * INITIAL_PLOT_AREA_RATIO;
        let overflow = self
            .measure_guide_overflow(&guide, scales, width_estimate, height_estimate)
            .await?;

        tracing::trace!(
            coord_system = std::any::type_name::<C>(),
            overflow_top = overflow.top,
            overflow_bottom = overflow.bottom,
            overflow_left = overflow.left,
            overflow_right = overflow.right,
            "Measured guide overflow"
        );

        // Use Taffy layout with the overflow requirements
        let (padding, taffy_layout) = self
            .compute_layout_with_overflow(width, height, scales, overflow)
            .await?;

        Ok(LayoutSolution {
            padding,
            plot_area: Self::calculate_plot_area_from_padding(&padding, width, height),
            taffy_layout,
        })
    }

    /// Compute layout using Taffy for the coordinate system
    /// Convert overflow requirements to pseudo-axes for Taffy layout
    pub(super) async fn compute_layout_with_overflow(
        &self,
        width: f32,
        height: f32,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        overflow: crate::coords::OverflowSpaceRequirement,
    ) -> Result<(Padding, crate::layout::LayoutResult), AvengerChartError> {
        use crate::layout::ChartLayout;

        // Get legends with theme applied (ensures measurement uses correct fonts)
        let all_legends = self.get_legends_with_theme(configured_scales);

        // Use the helper to merge legend channels - exactly the same as for rendering
        let (_channel_groups, legends_map) =
            self.merge_legend_channels(&all_legends, configured_scales);

        // Create ChartLayout with overflow directly
        let mut layout = ChartLayout::new_with_overflow::<C>(
            &overflow,
            &legends_map,
            configured_scales,
            Some((width, height)),
            self.plot.get_title(),
            self.plot.get_subtitle(),
            &self.plot.marks,
            &self.plot.get_theme(),
        )?;

        // Compute layout
        let layout_result = layout.compute(width, height)?;

        // Convert layout bounds to padding
        let padding = Padding {
            left: layout_result.plot_area.x,
            right: width - (layout_result.plot_area.x + layout_result.plot_area.width),
            top: layout_result.plot_area.y,
            bottom: height - (layout_result.plot_area.y + layout_result.plot_area.height),
        };

        Ok((padding, layout_result))
    }

    /// Calculate plot area from padding and total dimensions
    pub(super) fn calculate_plot_area_from_padding(
        padding: &Padding,
        width: f32,
        height: f32,
    ) -> (f32, f32, f32, f32) {
        (
            padding.left,
            padding.top,
            width - padding.left - padding.right,
            height - padding.top - padding.bottom,
        )
    }
}
