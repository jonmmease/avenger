//! Measurement utilities for facet guides (stubbed)
//!
//! This module is stubbed as part of the facet fresh start refactoring.
//! The complex two-pass measurement logic has been removed to allow
//! rebuilding the facet system on the EvaluatedFacetTree abstraction.

use crate::error::AvengerChartError;
use crate::guide::{MeasurementResult, OverflowSpaceRequirement};
use avenger_scales::scales::ConfiguredScale;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// Type alias for async measure overflow functions
pub type AsyncMeasureOverflowFn = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<OverflowSpaceRequirement, AvengerChartError>> + Send>>
        + Send
        + Sync,
>;

/// Trait for async measure_overflow function to enable generic callback
#[allow(async_fn_in_trait)]
pub trait MeasureOverflowCallback {
    async fn call(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        own_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        other_overflow: Option<&Vec<OverflowSpaceRequirement>>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError>;
}

/// Stubbed measurement implementation
///
/// Returns default overflow as a placeholder. The full measurement
/// logic will be rebuilt using the EvaluatedFacetTree.
#[allow(clippy::too_many_arguments)]
pub async fn measure_with_coordination_stub(
    _plot_width: f32,
    _plot_height: f32,
) -> Result<MeasurementResult, AvengerChartError> {
    // Return default measurement result
    Ok(MeasurementResult::default())
}
