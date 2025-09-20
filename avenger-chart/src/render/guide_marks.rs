//! Guide mark creation
//!
//! This module handles creation of guide marks (axes, grids, backgrounds)
//! using the coordinate system's guide rendering capabilities.

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
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

        // Render the guide using the coordinate system
        let theme = self.plot.get_theme();
        self.plot
            .coord_system()
            .render_guide(&guide, scales, plot_width, plot_height, plot_bounds, &theme)
            .await
    }
}
