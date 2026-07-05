use std::collections::HashMap;

use async_trait::async_trait;
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;

use avenger_chart_core::AvengerChartError;
use avenger_chart_scales::{ConfiguredScaleWithSpec, ScaleBuilder};

use crate::{
    plot::compiled::{
        CompiledPlot, scales::build_scale_builder_from_compiled_plot_with_view_materialized_data,
    },
    render::EvaluationContext,
};

/// Provides configured scales for a given plot area size.
///
/// This trait abstracts the source of scales, enabling the same evaluation
/// method to work for both top-level plots (which build their own scales)
/// and subplots (which may receive pre-coordinated scales from a parent facet).
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
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
    pub builder: &'a ScaleBuilder,
    pub plot: &'a CompiledPlot,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<'a> ScaleProvider for DynamicScaleProvider<'a> {
    async fn build_scales(
        &self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        Box::pin(self.plot.build_scales_from_builder(
            self.builder,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
        ))
        .await
    }
}

/// Scale provider for plots with view-local transforms.
///
/// View transforms can depend on scale domains, plot-area ranges, and pixel
/// dimensions. This provider first builds provisional scales from the ordinary
/// domain builder, then prepares view-local mark data with those scales in a
/// cache-read-only mode so non-position channels can infer domains from ready
/// materialized view output without launching work from layout probing.
pub struct ViewAwareScaleProvider<'a> {
    pub builder: &'a ScaleBuilder,
    pub plot: &'a CompiledPlot,
    pub eval_ctx: &'a EvaluationContext,
    pub data_override: Option<&'a DataFrame>,
    pub facet_path: &'a [ScalarValue],
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<'a> ScaleProvider for ViewAwareScaleProvider<'a> {
    async fn build_scales(
        &self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let base_scales = Box::pin(self.plot.build_scales_from_builder(
            self.builder,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
        ))
        .await?;
        let view_eval_ctx = self.eval_ctx.with_params(params.clone());
        let view_builder = Box::pin(
            build_scale_builder_from_compiled_plot_with_view_materialized_data(
                self.plot,
                self.data_override.cloned(),
                &view_eval_ctx,
                &base_scales,
                plot_area_width,
                plot_area_height,
                self.facet_path,
                self.plot.get_theme().as_ref(),
            ),
        )
        .await?;
        Box::pin(self.plot.build_scales_from_builder(
            &view_builder,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
        ))
        .await
    }
}
