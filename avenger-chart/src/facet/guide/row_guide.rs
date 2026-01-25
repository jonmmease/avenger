//! FacetRowGuide implementation for row-based faceting (stubbed).
//!
//! This module is stubbed as part of the facet fresh start refactoring.
//! The complex guide logic has been removed to allow rebuilding the facet
//! system on the EvaluatedFacetTree abstraction.

use crate::cartesian::axis::CartesianAxis;
use crate::error::AvengerChartError;
use crate::guide::{CompiledGuide, CoordinateGuide, MeasurementResult, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Guide configuration for FacetRow coordinate system (stubbed)
#[derive(Clone, Default)]
pub struct FacetRowGuideConfig {
    /// Optional facet title rendered above the label column
    pub facet_title: Option<String>,
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
        // Stubbed - no-op
    }

    fn set_compiled_marks(
        &mut self,
        _compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        // Stubbed - no-op
    }

    fn update(&mut self, _other: Self) {
        // Stubbed - no-op
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(FacetRowGuide {
            facet_title: self.facet_title,
        })
    }
}

/// Compiled guide for FacetRow coordinate system (stubbed)
///
/// Renders facet labels vertically along the side of the plot area with one
/// label per row.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetRowGuide {
    /// Optional facet title rendered above the label column
    pub facet_title: Option<String>,
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for FacetRowGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &crate::theme::Theme,
        _params: &IndexMap<String, datafusion::common::ScalarValue>,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
        _ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        // Return default overflow
        Ok(OverflowSpaceRequirement::default())
    }

    async fn measure_with_coordination(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &crate::theme::Theme,
        _params: &IndexMap<String, datafusion::common::ScalarValue>,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
        _ctx: &SessionContext,
    ) -> Result<MeasurementResult, AvengerChartError> {
        // Return default measurement result
        Ok(MeasurementResult::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _row_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _col_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _theme: &crate::theme::Theme,
        _params: &IndexMap<String, datafusion::common::ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Return empty marks - rendering will be rebuilt
        Ok(vec![])
    }

    fn get_clip(
        &self,
        plot_width: f32,
        plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        // Return rectangular clip for the plot area
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: plot_width,
            height: plot_height,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
