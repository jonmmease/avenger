//! Legend-specific layout logic

use crate::error::AvengerChartError;
use crate::legend::{Legend, LegendChannel};
use std::sync::Arc;
use taffy::Size;

/// Measure legend size with pre-built legend channels and return flexibility preference
pub fn measure_legend_size_with_channels(
    legend_channels: &[LegendChannel],
    legend: &Legend,
    renderer: Arc<dyn crate::legend::LegendRenderer>,
    available_space: Size<f32>,
) -> Result<(Size<f32>, bool), AvengerChartError> {
    // Skip invisible legends
    if !legend.visible {
        return Ok((
            Size {
                width: 0.0,
                height: 0.0,
            },
            false,
        ));
    }

    // Ask the renderer to measure itself with all merged channels
    let size = renderer.measure(legend_channels, legend, available_space)?;
    let flexible = renderer.prefers_flexible_layout();
    Ok((size, flexible))
}
