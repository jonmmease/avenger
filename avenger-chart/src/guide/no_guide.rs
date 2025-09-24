//! Empty guide implementation for coordinate systems without visual guides

use crate::error::AvengerChartError;
use crate::guide::{CoordinateGuideRender, CoordinateGuideBuilder, GuideUpdate, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Empty guide for coordinate systems without visual guides
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NoGuide {
    // Store an empty map directly in the struct
    axes: HashMap<String, ()>,
}

impl GuideUpdate for NoGuide {
    fn update(self, _other: Self) -> Self {
        // NoGuide has no state to update
        self
    }
}

impl CoordinateGuideBuilder for NoGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // No-op for systems without axes
    }

    fn update(&mut self, other: Self) {
        // No-op for systems without state
    }

    fn build(self) -> Box<dyn CoordinateGuideRender> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateGuideRender for NoGuide {
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
