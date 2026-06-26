//! Legend-specific layout logic

use std::sync::Arc;

use avenger_chart_core::DefaultLogicalExprNodeExt;
use avenger_chart_core::{
    AvengerChartError, Legend, LegendPosition, Size2D, Theme, evaluate_bool_expr,
};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{LegendChannel, LegendRenderer};

/// Measurement and layout information for a single legend.
#[derive(Debug, Clone)]
pub struct LegendMeasurement {
    /// Size of the legend (width, height).
    pub size: Size2D,
    /// Whether the legend height is flexible (e.g., for colorbars).
    pub flexible: bool,
    /// Position of the legend.
    pub position: LegendPosition,
}

/// Legend measurements keyed by legend layout id.
pub type LegendMeasurements = IndexMap<String, LegendMeasurement>;

/// Measure legend size with pre-built legend channels and return flexibility preference
pub async fn measure_legend_size_with_channels(
    legend_channels: &[LegendChannel],
    legend: &Legend,
    renderer: Arc<dyn LegendRenderer>,
    available_space: Size2D,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
    ctx: &SessionContext,
) -> Result<(Size2D, bool), AvengerChartError> {
    // Evaluate visibility expression - skip measurement if not visible
    let visible = if let Some(node) = legend.visible.as_option().and_then(|o| o.as_ref()) {
        let expr = node.to_expr(ctx)?;
        evaluate_bool_expr(&expr, ctx, params).await?
    } else {
        true // Default to visible
    };

    if !visible {
        // Return zero size for invisible legends
        return Ok((
            Size2D {
                width: 0.0,
                height: 0.0,
            },
            false,
        ));
    }

    // Ask the renderer to measure itself with all merged channels
    let size = renderer
        .measure(legend_channels, legend, available_space, theme, params, ctx)
        .await?;
    let flexible = renderer.prefers_flexible_layout();
    Ok((size, flexible))
}
