use std::collections::HashMap;

use async_trait::async_trait;
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{
    error::AvengerChartError, plot::compiled::CompiledPlot, scales::ConfiguredScaleWithSpec,
};

/// Provides configured scales for a given plot area size.
///
/// This trait abstracts the source of scales, enabling the same evaluation
/// method to work for both top-level plots (which build their own scales)
/// and subplots (which may receive pre-coordinated scales from a parent facet).
#[async_trait]
pub trait ScaleProvider: Send + Sync {
    /// Build scales for the given plot area dimensions.
    ///
    /// # Arguments
    /// * `plot_area_width` - Width of the plot area (not including margins/legends)
    /// * `plot_area_height` - Height of the plot area (not including margins/legends)
    /// * `ctx` - DataFusion session context for data queries
    /// * `params` - Plot parameters (may include width/height for media queries)
    async fn build_scales(
        &self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError>;
}

/// Default scale provider that builds scales from a ScaleBuilder.
///
/// Used for top-level plots and any subplot that builds its own scales
/// independently (e.g., facets with completely free scales).
pub struct DynamicScaleProvider<'a> {
    pub builder: &'a crate::scales::ScaleBuilder,
    pub plot: &'a CompiledPlot,
}

#[async_trait]
impl<'a> ScaleProvider for DynamicScaleProvider<'a> {
    async fn build_scales(
        &self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        self.plot
            .build_scales_from_builder(self.builder, plot_area_width, plot_area_height, ctx, params)
            .await
    }
}
