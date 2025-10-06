//! Empty guide implementation for coordinate systems without visual guides
use crate::theme::Theme;

use crate::error::AvengerChartError;
use crate::guide::{CompiledGuide, CoordinateGuide, GuideUpdate, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use avenger_scenegraph::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Empty guide for coordinate systems without visual guides
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NoGuide;

impl GuideUpdate for NoGuide {
    fn update(self, _other: Self) -> Self {
        // NoGuide has no state to update
        self
    }
}

impl CoordinateGuide for NoGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // No-op for systems without axes
    }

    fn set_compiled_marks(
        &mut self,
        _compiled_marks: Vec<std::sync::Arc<dyn crate::marks::CompiledMark>>,
        _session_context: &datafusion::prelude::SessionContext,
    ) {
        // No-op for systems without axes
    }

    fn update(&mut self, _other: Self) {
        // No-op for systems without state
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for NoGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
        _params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement::default())
    }

    async fn render(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _theme: &Theme,
        _params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(Vec::new())
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> avenger_scenegraph::marks::group::Clip {
        // No clipping for zero-dimensional coordinate systems
        avenger_scenegraph::marks::group::Clip::None
    }
}
