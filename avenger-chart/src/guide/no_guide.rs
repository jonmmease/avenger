//! Empty guide implementation for coordinate systems without visual guides

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    coords::CoordMeasurement,
    error::AvengerChartError,
    facet::evaluated_facet_tree::EvaluatedFacetTree,
    guide::{CompiledGuide, CoordinateGuide, GuideUpdate, OverflowSpaceRequirement},
    layout::LayoutBounds,
    marks::CompiledMark,
    theme::Theme,
};

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
        _compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
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
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &SessionContext,
        _facet_tree: &EvaluatedFacetTree,
        _facet_path: &[ScalarValue],
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _facet_tree: &EvaluatedFacetTree,
        _facet_path: &[ScalarValue],
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(Vec::new())
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        // No clipping for zero-dimensional coordinate systems
        Clip::None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
