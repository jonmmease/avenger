//! Shared scale building helpers for faceting
//!
//! This module provides common scale building logic used by all facet types
//! (FacetRow, FacetCol, GridFacet) to ensure consistent behavior.

use crate::error::AvengerChartError;
use crate::plot::compiled::CompiledPlot;
use crate::scales::ConfiguredScaleWithSpec;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use indexmap::IndexMap;
use std::collections::HashMap;

/// Build scales for a subplot, using shared scales if provided
///
/// This helper centralizes the scale building logic for faceted layouts:
/// - If `shared_scales_opt` is Some, returns those scales (shared across subplots)
/// - If `shared_scales_opt` is None, builds fresh scales from the filtered data (independent scales)
///
/// This function is used by all facet types to ensure consistent scale handling.
///
/// # Arguments
/// * `compiled_subplot` - The compiled subplot specification
/// * `shared_scales_opt` - Optional shared scales (if None, builds from filtered data)
/// * `filter_df` - The filtered dataframe for this specific subplot
/// * `plot_width` - Width of the subplot plot area
/// * `plot_height` - Height of the subplot plot area
/// * `ctx` - DataFusion session context
/// * `params` - Parameters including FacetContext
pub async fn build_scales_helper(
    compiled_subplot: &CompiledPlot,
    shared_scales_opt: &Option<HashMap<String, ConfiguredScaleWithSpec>>,
    filter_df: &DataFrame,
    plot_width: f32,
    plot_height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
    if let Some(shared) = shared_scales_opt {
        Ok(shared.clone())
    } else {
        compiled_subplot
            .build_scales_for_dataframe(filter_df, plot_width, plot_height, ctx, params)
            .await
    }
}
