//! Legend-specific layout logic

use crate::error::AvengerChartError;
use crate::legend::{ChannelInfo, Legend, LegendChannel};
use avenger_scales::scales::ConfiguredScale;
use std::collections::HashMap;
use taffy::Size;

/// Measure legend size with mark encodings and return flexibility preference
pub fn measure_legend_size<C: crate::coords::CoordinateSystem>(
    channel: &str,
    legend: &Legend,
    scale: &ConfiguredScale,
    scales: &HashMap<String, ConfiguredScale>,
    available_space: Size<f32>,
    marks: &[Box<dyn crate::marks::Mark<C>>],
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

    // Find the mark that has this channel and its index
    let (mark_index, mark_with_channel) = marks
        .iter()
        .enumerate()
        .find(|(_, m)| m.data_context().channels().contains_key(channel))
        .ok_or_else(|| {
            AvengerChartError::InternalError(format!("Channel '{}' not found in any mark", channel))
        })?;

    // Get the renderer - either explicitly configured or from the mark
    let renderer = if let Some(ref renderer) = legend.renderer {
        // Use explicitly configured renderer
        renderer.clone()
    } else {
        // Get the mark's preferred renderer for this channel
        mark_with_channel
            .preferred_legend_renderer(channel, scale)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "No legend renderer available for channel '{}'",
                    channel
                ))
            })?
    };

    // Collect related channels from the mark (needed for correct size constants)
    let mut related_channels = HashMap::new();
    for (other_name, other_value) in mark_with_channel.data_context().channels() {
        if other_name != channel {
            // Check if this channel has a scale or is constant
            let channel_info = if let Some(other_scale) = scales.get(other_name) {
                // Channel has a scale
                ChannelInfo::Scaled {
                    expr: other_value.expr().cloned(),
                    scale: other_scale.clone(),
                }
            } else if let Some(expr) = other_value.expr() {
                // Channel has a constant expression
                ChannelInfo::Constant { expr: expr.clone() }
            } else {
                // Skip channels without expressions
                continue;
            };
            related_channels.insert(other_name.clone(), channel_info);
        }
    }

    // Create legend channels for all merged channels, not just the primary one
    let mut legend_channels = Vec::new();

    // Always add the primary channel
    legend_channels.push(LegendChannel {
        name: channel.to_string(),
        expression: mark_with_channel
            .data_context()
            .channels()
            .get(channel)
            .and_then(|v| v.expr().cloned()),
        scale: scale.clone(),
        channel_type: channel.to_string(),
        mark_type: mark_with_channel.mark_type().to_string(),
        mark_index,
        related_channels: related_channels.clone(),
    });

    // Add any merged channels
    for merged_channel_name in &legend.merged_channels {
        if merged_channel_name != channel {
            // Only add if this channel exists in the mark
            if let Some(channel_value) = mark_with_channel
                .data_context()
                .channels()
                .get(merged_channel_name)
            {
                if let Some(merged_scale) = scales.get(merged_channel_name) {
                    legend_channels.push(LegendChannel {
                        name: merged_channel_name.clone(),
                        expression: channel_value.expr().cloned(),
                        scale: merged_scale.clone(),
                        channel_type: merged_channel_name.clone(),
                        mark_type: mark_with_channel.mark_type().to_string(),
                        mark_index,
                        related_channels: related_channels.clone(),
                    });
                }
            }
        }
    }

    // Ask the renderer to measure itself with all merged channels
    let size = renderer.measure(&legend_channels, legend, available_space)?;
    let flexible = renderer.prefers_flexible_layout();
    Ok((size, flexible))
}
