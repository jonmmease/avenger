/// Generic facet evaluation logic shared between FacetRow and FacetCol implementations
///
/// This module is STUBBED as part of the facet fresh start refactoring.
/// The complex evaluation logic has been removed to allow rebuilding the facet
/// system on the EvaluatedFacetTree abstraction.
use crate::error::AvengerChartError;
use crate::facet::dimension_config::FacetDimensionConfig;
use crate::marks::CompiledMarkState;
use crate::plot::CompiledPlot;
use crate::render::RenderContext;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

/// Convert RecordBatch to DataFrame using DataFusion's read_batch method
///
/// This creates a DataFrame from a RecordBatch using DataFusion's built-in `read_batch()`
/// method, which internally creates an unnamed MemTable (with table name "?table?").
/// This avoids manual table registration and naming collisions.
///
/// # Arguments
/// * `batch` - The RecordBatch to convert
/// * `ctx` - The SessionContext for DataFrame operations
///
/// # Returns
/// A DataFrame backed by an in-memory table containing the batch data
pub fn batch_to_dataframe(
    batch: &RecordBatch,
    ctx: &SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    // Use DataFusion's built-in read_batch which creates an unnamed table
    ctx.read_batch(batch.clone()).map_err(|e| {
        AvengerChartError::InternalError(format!(
            "Failed to create DataFrame from RecordBatch: {}",
            e
        ))
    })
}

/// Evaluate a faceted visualization (STUBBED)
///
/// This function is stubbed. Full facet evaluation including measurement,
/// coordination, and rendering will be rebuilt on the EvaluatedFacetTree
/// abstraction.
#[allow(clippy::too_many_arguments)]
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig>(
    _facet_coord: &dyn crate::coords::CoordinateSystemTransform,
    _compiled_subplot: &Arc<CompiledPlot>,
    _state: &CompiledMarkState,
    _data_override: Option<&datafusion::dataframe::DataFrame>,
    _facet_title: Option<String>,
    _facet_spacing: Option<f32>,
    _context: &RenderContext,
    // Orientation-specific closures:
    // Returns (width, height) given band size and context
    _subplot_dims: impl Fn(f32, &RenderContext) -> (f32, f32) + Clone,
    // Returns [x, y] translation given band position
    _group_origin: impl Fn(f32) -> [f32; 2] + Clone,
) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
    // STUBBED: Return empty result
    // TODO: Implement facet evaluation using EvaluatedFacetTree
    Ok((Vec::new(), crate::layout::LayoutUpdates::default()))
}
