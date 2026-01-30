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

/// Prebuilt scale provider that returns already-computed scales with updated ranges.
///
/// This provider is used when scales have been pre-computed externally
/// (e.g., by row/col facet rendering logic) with fixed domains. The ranges
/// are updated based on the provided plot area dimensions for position scales.
///
/// Unlike `DynamicScaleProvider` which rebuilds scales from scratch, this provider
/// preserves the exact `ConfiguredScale` (including its `scale_impl`) and only
/// updates the range. This is important for facet shared scales where the domain
/// and scale configuration must remain exactly as computed.
pub struct PrebuiltScaleProvider {
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,
}

#[async_trait]
impl ScaleProvider for PrebuiltScaleProvider {
    async fn build_scales(
        &self,
        plot_area_width: f32,
        plot_area_height: f32,
        _ctx: &SessionContext,
        _params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        // Return scales with updated ranges for position channels
        let mut result = HashMap::new();
        for (name, scale_with_spec) in &self.scales {
            let updated_configured = match name.as_str() {
                "x" | "x2" | "xOffset" | "column" => {
                    // X position scales and column facet scale: range is [0, plot_area_width]
                    scale_with_spec
                        .configured()
                        .clone()
                        .with_range_interval((0.0, plot_area_width))
                }
                "y" | "y2" | "yOffset" | "row" => {
                    // Y position scales and row facet scale: range is [plot_area_height, 0] (inverted for canvas)
                    scale_with_spec
                        .configured()
                        .clone()
                        .with_range_interval((plot_area_height, 0.0))
                }
                _ => {
                    // Non-position scales: keep existing range
                    scale_with_spec.configured().clone()
                }
            };
            result.insert(
                name.clone(),
                ConfiguredScaleWithSpec::new(scale_with_spec.spec().clone(), updated_configured),
            );
        }
        Ok(result)
    }
}

