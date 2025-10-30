use async_trait::async_trait;
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;

use crate::error::AvengerChartError;
use crate::plot::compiled::CompiledPlot;
use crate::scales::ConfiguredScaleWithSpec;

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
pub struct DefaultScaleProvider<'a> {
    pub builder: &'a crate::scales::ScaleBuilder,
    pub plot: &'a CompiledPlot,
}

#[async_trait]
impl<'a> ScaleProvider for DefaultScaleProvider<'a> {
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

/// Prebuilt scale provider that returns already-computed scales.
///
/// This provider is used when scales have been pre-computed externally
/// (e.g., by row/col facet rendering logic) and just need to be returned
/// without further computation. The dimensions are ignored since scales
/// are already built with the correct dimensions.
#[allow(dead_code)] // Used in Phase 5-6 when migrating facet rendering
pub struct PrebuiltScaleProvider {
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,
}

#[async_trait]
impl ScaleProvider for PrebuiltScaleProvider {
    async fn build_scales(
        &self,
        _plot_area_width: f32,
        _plot_area_height: f32,
        _ctx: &SessionContext,
        _params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        // Return pre-computed scales directly
        Ok(self.scales.clone())
    }
}

/// Facet scale provider that coordinates scales across subplot cells.
///
/// This provider uses a `ScaleGrouping` to build scales that respect the
/// sharing modes (Shared, Free, SharedInRow, SharedInColumn) configured for
/// each channel. The subplot position (row_idx, col_idx) determines which
/// scale group is used for each channel.
#[allow(dead_code)] // Used in Phase 6-7 when migrating grid facet rendering
pub struct FacetScaleProvider<'a> {
    pub grouping: &'a crate::facet::scale_grouping::ScaleGrouping,
    pub plot: &'a CompiledPlot,
    pub row_idx: usize,
    pub col_idx: usize,
}

#[async_trait]
impl<'a> ScaleProvider for FacetScaleProvider<'a> {
    async fn build_scales(
        &self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        self.grouping
            .build_scales_for_position(
                self.plot,
                self.row_idx,
                self.col_idx,
                plot_area_width,
                plot_area_height,
                ctx,
                params,
            )
            .await
    }
}
