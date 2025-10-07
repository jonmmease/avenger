//! Legend-specific layout logic

use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendChannel};
use crate::theme::Theme;
use std::sync::Arc;
use taffy::Size;

/// Measure legend size with pre-built legend channels and return flexibility preference
pub async fn measure_legend_size_with_channels(
    legend_channels: &[LegendChannel],
    legend: &Legend,
    renderer: Arc<dyn crate::legend::LegendRenderer>,
    available_space: Size<f32>,
    theme: &Theme,
    params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<(Size<f32>, bool), AvengerChartError> {
    // Evaluate visibility expression - skip measurement if not visible
    use crate::plot::compiled::expr_eval::*;
    use crate::serialization::LogicalExprNodeExt;

    let visible = if let Some(node) = legend.visible.as_option().and_then(|o| o.as_ref()) {
        let expr = node.to_expr(ctx)?;
        evaluate_bool_expr(&expr, ctx, params).await?
    } else {
        true // Default to visible
    };

    if !visible {
        // Return zero size for invisible legends
        return Ok((Size { width: 0.0, height: 0.0 }, false));
    }

    // Ask the renderer to measure itself with all merged channels
    let size = renderer.measure(legend_channels, legend, available_space, theme, params, ctx).await?;
    let flexible = renderer.prefers_flexible_layout();
    Ok((size, flexible))
}
