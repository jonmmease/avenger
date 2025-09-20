//! Empty guide implementation for coordinate systems without visual guides

use crate::error::AvengerChartError;
use crate::guide::{CoordinateGuide, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

/// Empty guide for coordinate systems without visual guides
#[derive(Clone, Debug, Default)]
pub struct NoGuide {
    // Store an empty map directly in the struct
    axes: HashMap<String, ()>,
}

#[async_trait::async_trait]
impl CoordinateGuide for NoGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // No-op for systems without axes
    }

    fn axes(&self) -> &HashMap<String, Self::Axis> {
        // Return reference to our empty map
        &self.axes
    }

    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &dyn Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement::default())
    }

    async fn render(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _theme: &dyn Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(Vec::new())
    }
}
