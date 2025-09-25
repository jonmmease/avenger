//! Guide mark creation
//!
//! This module handles creation of guide marks (axes, grids, backgrounds)
//! using the coordinate system's guide rendering capabilities.

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::guide::CoordinateGuideBuilder;
use crate::layout::LayoutBounds;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Create guide marks (axes, grids, backgrounds, and other visual guides)
    pub(super) async fn create_guide_marks(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Create guide with all configurations applied
        let guide = self.create_configured_guide(scales);

        // Build the guide into a renderer
        let guide_renderer = guide.build();

        // Render using the built guide renderer directly
        let theme = self.plot.get_theme();
        guide_renderer
            .render(
                scales,
                plot_width,
                plot_height,
                plot_bounds,
                theme.as_ref(),
            )
            .await
    }
}
