//! FacetColGuide implementation for column-based faceting.
//!
//! This module handles measurement and rendering of guide elements (axes, labels)
//! for column-based faceted plots.

use crate::cartesian::axis::CartesianAxis;
use crate::error::AvengerChartError;
use crate::facet::marks::facet::{CompiledFacetCol, CompiledFacetSource};
use crate::guide::{CompiledGuide, CoordinateGuide, MeasurementResult, OverflowSpaceRequirement};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
use crate::plot::compiled::CompiledPlot;
use crate::serialization::{LogicalPlanNodeExt, SerializableDataFrame};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, FromInto};
use std::collections::HashMap;
use std::sync::Arc;

/// Guide configuration for FacetCol coordinate system
#[derive(Clone, Default)]
pub struct FacetColGuideConfig {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot extracted from the facet mark
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet mark's data (used when data_override is None)
    facet_data_plan: Option<LogicalPlanNode>,
}

impl FacetColGuideConfig {
    /// Create a new FacetColGuideConfig
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the facet title
    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.facet_title = title;
        self
    }
}

impl CoordinateGuide for FacetColGuideConfig {
    type Axis = CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // Stubbed - no-op
    }

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        // Find the CompiledFacetCol mark and extract its subplot and data
        for mark in &compiled_marks {
            if let Some(facet_col) = mark.as_any().downcast_ref::<CompiledFacetCol>() {
                self.compiled_subplot = Some(facet_col.compiled_subplot().clone());
                // Extract the logical plan from the mark's data context
                self.facet_data_plan = mark.data_context().logical_plan_node().cloned();
                break;
            }
        }
    }

    fn update(&mut self, _other: Self) {
        // Stubbed - no-op
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(FacetColGuide {
            facet_title: self.facet_title,
            compiled_subplot: self.compiled_subplot,
            facet_data_plan: self.facet_data_plan,
        })
    }
}

/// Compiled guide for FacetCol coordinate system
///
/// Renders facet labels horizontally below (or above) the plot area with one
/// label per column.
#[serde_as]
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetColGuide {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot for measuring overflow
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet mark's data (used when data_override is None)
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    facet_data_plan: Option<LogicalPlanNode>,
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for FacetColGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let Some(subplot) = &self.compiled_subplot else {
            return Ok(OverflowSpaceRequirement::default());
        };

        // Get data from data_override, or fall back to stored facet_data_plan
        let data_df: Option<DataFrame> = if let Some(data) = data_override {
            Some(data.clone())
        } else if let Some(plan_node) = &self.facet_data_plan {
            // Reconstruct DataFrame from stored logical plan
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

        // Get column scale to determine subplot width
        let column_scale = scales.get("column").ok_or_else(|| {
            AvengerChartError::InternalError("No column scale found".into())
        })?;

        let subplot_width = avenger_scales::scales::band::bandwidth(&column_scale.config)
            .map_err(|e| AvengerChartError::InternalError(format!("Failed to get bandwidth: {}", e)))?;

        // Build subplot scales from full data
        let subplot_scales = subplot
            .build_scales_for_dataframe(data, subplot_width, plot_height, ctx, params)
            .await?;

        // Measure subplot guide overflow
        let configured_scales: HashMap<String, ConfiguredScale> = subplot_scales
            .iter()
            .map(|(k, v)| (k.clone(), v.configured().clone()))
            .collect();

        if let Some(guide) = &subplot.compiled_guide {
            guide.measure_overflow(
                &configured_scales,
                subplot_width,
                plot_height,
                theme,
                params,
                Some(data),
                ctx,
            ).await
        } else {
            Ok(OverflowSpaceRequirement::default())
        }
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
    ) -> Result<MeasurementResult, AvengerChartError> {
        // Return default measurement result
        Ok(MeasurementResult::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
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
