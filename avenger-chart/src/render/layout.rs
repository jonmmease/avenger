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
use crate::layout::ChartLayout;
use crate::render::LayoutSolution;
use crate::render::types::INITIAL_PLOT_AREA_RATIO;
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

        // Call the layout helper with the guide
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

        // Get legends with theme applied (ensures measurement uses correct fonts)
        let all_legends = self.get_legends_with_theme(scales);

        // Use the helper to merge legend channels - exactly the same as for rendering
        let (_channel_groups, legends_map) = self.merge_legend_channels(&all_legends, scales);

        // Prepare legend measurements
        let available_size = taffy::Size {
            width: width * INITIAL_PLOT_AREA_RATIO,
            height: height * INITIAL_PLOT_AREA_RATIO,
        };
        let legend_measurements =
            self.prepare_legend_measurements(&legends_map, scales, available_size)?;

        // Create ChartLayout with overflow directly
        let layout_spec = self.plot.get_layout_spec();
        let mut layout = ChartLayout::new_with_overflow(
            &overflow,
            &legends_map,
            layout_spec,
            self.plot.get_title(),
            self.plot.get_subtitle(),
            &self.plot.get_theme(),
            &legend_measurements,
        )?;

        // Compute layout using the layout spec and return it directly
        layout.compute(layout_spec)
    }
}
