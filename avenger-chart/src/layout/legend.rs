//! Legend-specific layout logic

use std::sync::Arc;

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;
use taffy::Size;

use crate::{
    error::AvengerChartError,
    legend::{Legend, LegendChannel, LegendRenderer},
    plot::compiled::expr_eval::evaluate_bool_expr,
    serialization::LogicalExprNodeExt,
    theme::Theme,
};

/// Measure legend size with pre-built legend channels and return flexibility preference
pub async fn measure_legend_size_with_channels(
    legend_channels: &[LegendChannel],
    legend: &Legend,
    renderer: Arc<dyn LegendRenderer>,
    available_space: Size<f32>,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
    ctx: &SessionContext,
) -> Result<(Size<f32>, bool), AvengerChartError> {
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
            Size {
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
