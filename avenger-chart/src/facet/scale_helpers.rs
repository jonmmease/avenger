//! Shared scale building helpers for faceting
//!
//! This module provides common scale building logic used by all facet types
//! (FacetRow, FacetCol, GridFacet) to ensure consistent behavior.

use crate::error::AvengerChartError;
use crate::plot::compiled::CompiledPlot;
use crate::scales::ConfiguredScaleWithSpec;
use crate::scales::builder::ScaleBuilder;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::collections::HashMap;

/// Build scales for a subplot, using shared scales or a ScaleBuilder if provided
///
/// This helper centralizes the scale building logic for faceted layouts:
/// - If `shared_scales_opt` is Some, returns those scales (shared across subplots)
/// - If `shared_scales_opt` is None and `free_scale_builder` is Some, uses the builder for free scales
/// - Otherwise, builds fresh scales from the filtered data (fallback for backward compatibility)
///
/// Using ScaleBuilder enables radius-aware domain inference for both shared and free scales.
///
/// This function is used by all facet types to ensure consistent scale handling.
///
/// # Arguments
/// * `compiled_subplot` - The compiled subplot specification
/// * `shared_scales_opt` - Optional shared scales (if Some, returns these)
/// * `free_scale_builder` - Optional ScaleBuilder for building free scales with radius support
/// * `filter_df` - The filtered dataframe for this specific subplot
/// * `plot_width` - Width of the subplot plot area
/// * `plot_height` - Height of the subplot plot area
/// * `ctx` - DataFusion session context
/// * `params` - Parameters including FacetContext
pub async fn build_scales_helper(
    compiled_subplot: &CompiledPlot,
    shared_scales_opt: &Option<HashMap<String, ConfiguredScaleWithSpec>>,
    free_scale_builder: &Option<ScaleBuilder>,
    filter_df: &DataFrame,
    plot_width: f32,
    plot_height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
    if let Some(shared) = shared_scales_opt {
        Ok(shared.clone())
    } else if let Some(builder) = free_scale_builder {
        // Use ScaleBuilder for radius-aware free scales
        let built = compiled_subplot
            .build_scales_from_builder(builder, plot_width, plot_height, ctx, params)
            .await?;

        // Defensive backfill removed.
        // The fallback to build_scales_for_dataframe is not radius-aware and
        // was causing symbol clipping in faceted plots with free scales.
        // Removing this ensures that only the radius-aware ScaleBuilder path is used.
        // Any failures in the builder should now surface as proper errors rather than
        // being silently handled by an incorrect implementation.

        Ok(built)
    } else {
        // Fallback to simple build (no radius support)
        compiled_subplot
            .build_scales_for_dataframe(filter_df, plot_width, plot_height, ctx, params)
            .await
    }
}
