//! Legend creation and management
//!
//! This module handles:
//! - Creating and positioning legends
//! - Merging legend channels based on merge keys
//! - Applying theme settings to legends
//! - Inferring legend titles and default positions

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::layout::legend::measure_legend_size_with_channels;
use crate::legend::{ChannelInfo, Legend, LegendChannel, MergeKey};
use avenger_scenegraph::marks::mark::SceneMark;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

/// Type alias for legend measurements: channel -> (size, is_flexible)
pub type LegendMeasurements = IndexMap<String, (taffy::Size<f32>, bool)>;

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Create legend marks based on configured legends
    /// Create legends using Taffy layout positions
    pub(super) async fn create_legends_with_layout(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        layout: &crate::layout::LayoutResult,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get legends with theme applied (same as used for layout)
        let all_legend_configs = self.get_legends_with_theme(scales);

        // Use the helper to merge legend channels - exactly the same as for layout
        let (sorted_channel_groups, _legends_map) =
            self.merge_legend_channels(&all_legend_configs, scales);

        // Create legend marks positioned according to layout
        let mut legend_marks = Vec::new();

        for channels in sorted_channel_groups {
            if channels.is_empty() {
                continue;
            }

            // Get the primary channel (first in group)
            let primary_channel = &channels[0];

            // Get legend config for primary channel
            let legend = &all_legend_configs[&primary_channel.name];

            // Get layout bounds for this legend
            if let Some(bounds) = layout.legends.get(&primary_channel.name) {
                // Determine the appropriate renderer for this group of channels
                let renderer_opt = if channels.len() > 1 {
                    // Multiple channels - try to get a merged renderer
                    // Find the mark that these channels belong to
                    let mark_opt = self.plot.mark_renderers.get(primary_channel.mark_index);

                    mark_opt
                        .and_then(|mark| mark.preferred_merged_legend_renderer(&channels, &scales))
                } else {
                    // Single channel - use the unified renderer selection
                    scales.get(&primary_channel.name).and_then(|scale| {
                        self.get_legend_renderer(
                            &primary_channel.channel_type,
                            legend,
                            scale,
                            Some(primary_channel.mark_index),
                        )
                    })
                };

                // Skip this legend group if no renderer is available
                let Some(renderer) = renderer_opt else {
                    continue;
                };

                // Render the legend with all channels in the group
                if let Some(group) = renderer.render(
                    &channels,
                    legend,
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                )? {
                    legend_marks.push(SceneMark::Group(group));
                }
            }
        }

        Ok(legend_marks)
    }

    /// Create default legends for channels with ConfiguredScale
    pub(super) fn create_default_legends(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> IndexMap<String, Legend> {
        let mut default_legends = IndexMap::new();

        // Build set of channels to skip for legends
        let mut skip_channels = std::collections::HashSet::new();

        // Add positional channels from the coordinate system
        for &channel in self.plot.coord_system().required_channels() {
            skip_channels.insert(channel.to_string());
            // Also skip interval variants (e.g., "x2" for "x")
            skip_channels.insert(format!("{}2", channel));
        }

        // Add channels that marks indicate shouldn't have legends
        // (by returning None from preferred_legend_renderer)
        // Only check marks that actually have the channel
        for (channel, scale) in scales {
            // Find the mark that has this channel
            if let Some(mark) = self
                .plot
                .mark_renderers
                .iter()
                .find(|m| m.data_context().channels().contains_key(channel))
            {
                // If the mark that has the channel says no legend, skip it
                if mark.preferred_legend_renderer(channel, scale).is_none() {
                    skip_channels.insert(channel.clone());
                }
            }
        }

        // Sort channels for deterministic ordering
        let mut sorted_channels: Vec<_> = scales.keys().collect();
        sorted_channels.sort();

        for channel in sorted_channels {
            // Skip channels that don't need legends
            if skip_channels.contains(channel) {
                continue;
            }

            // Skip if legend already configured
            if self.plot.legends.contains_key(channel) {
                continue;
            }

            // ConfiguredScale always has resolved domain, so always create a legend
            // for channels that have scales
            let theme = self.plot.get_theme();
            let mut legend = Legend::new()
                .title(self.infer_legend_title(channel))
                .position(self.default_legend_position(channel))
                .background_padding(theme.legend_background_padding())
                .background_corner_radius(theme.legend_background_corner_radius());

            // Apply optional theme defaults
            if let Some(fill) = theme.legend_background_fill() {
                legend = legend.background_fill(fill);
            }
            if let Some(stroke) = theme.legend_background_stroke() {
                legend = legend.background_stroke(stroke);
            }

            // Set text colors and typography from theme
            legend.title_color = crate::maybe::Maybe::Set(theme.legend_title_color());
            legend.label_color = crate::maybe::Maybe::Set(theme.legend_label_color());
            legend.title_font_family = crate::maybe::Maybe::Set(theme.legend_title_font_family());
            legend.title_font_size = crate::maybe::Maybe::Set(theme.legend_title_font_size());
            legend.title_font_weight = crate::maybe::Maybe::Set(theme.legend_title_font_weight());
            legend.label_font_family = crate::maybe::Maybe::Set(theme.legend_label_font_family());
            legend.label_font_size = crate::maybe::Maybe::Set(theme.legend_label_font_size());
            legend.label_font_weight = crate::maybe::Maybe::Set(theme.legend_label_font_weight());
            legend.tick_font_family = crate::maybe::Maybe::Set(theme.legend_tick_font_family());
            legend.tick_font_size = crate::maybe::Maybe::Set(theme.legend_tick_font_size());
            legend.tick_font_weight = crate::maybe::Maybe::Set(theme.legend_tick_font_weight());
            legend.tick_color = crate::maybe::Maybe::Set(theme.legend_tick_color());

            default_legends.insert(channel.clone(), legend);
        }

        default_legends
    }

    /// Infer a title for the legend based on channel
    fn infer_legend_title(&self, channel: &str) -> String {
        // First try to extract from marks (like we do for axes)
        use crate::coords::extract_channel_title_from_marks;
        if let Some(title) = extract_channel_title_from_marks(&self.plot.mark_renderers, channel) {
            return title;
        }

        // Fallback: convert underscores to spaces and apply title case
        channel
            .split('_')
            .map(|word| {
                // Capitalize first letter of each word
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Get default legend position for a channel
    fn default_legend_position(&self, _channel: &str) -> crate::legend::LegendPosition {
        // All legends default to the right
        crate::legend::LegendPosition::Right
    }

    /// Get the appropriate legend renderer for a channel
    pub(super) fn get_legend_renderer(
        &self,
        channel: &str,
        legend: &Legend,
        scale: &avenger_scales::scales::ConfiguredScale,
        mark_index: Option<usize>,
    ) -> Option<Arc<dyn crate::legend::LegendRenderer>> {
        if let Some(ref renderer) = legend.renderer {
            // Use explicitly configured renderer
            Some(renderer.clone())
        } else if let Some(idx) = mark_index {
            // Find the mark and get its preference
            self.plot
                .mark_renderers
                .get(idx)
                .and_then(|mark| mark.preferred_legend_renderer(channel, scale))
        } else {
            None
        }
    }

    /// Build a LegendChannel from mark and channel information
    pub(super) fn build_legend_channel(
        &self,
        channel_name: &str,
        channel_value: &crate::marks::ChannelValue,
        scale: &avenger_scales::scales::ConfiguredScale,
        mark: &dyn crate::marks::MarkRenderer,
        mark_index: usize,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> LegendChannel {
        // Collect related channels
        let mut related_channels = HashMap::new();
        for (other_name, other_value) in mark.data_context().channels() {
            if other_name != channel_name {
                // Check if this channel has a scale or is constant
                let channel_info = if let Some(other_scale) = configured_scales.get(other_name) {
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

        LegendChannel {
            name: channel_name.to_string(),
            expression: channel_value.expr().cloned(),
            scale: scale.clone(),
            channel_type: channel_name.to_string(),
            mark_type: mark.mark_type().to_string(),
            mark_index,
            related_channels,
        }
    }

    /// Prepare legend measurements for layout computation
    pub fn prepare_legend_measurements(
        &self,
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        available_space: taffy::Size<f32>,
    ) -> Result<LegendMeasurements, AvengerChartError> {
        let mut legend_measurements = LegendMeasurements::new();

        for (channel, legend) in legends.iter() {
            if let Some(scale) = scales.get(channel) {
                // Build legend channels for this channel
                let legend_channels =
                    self.build_legend_channels_for_channel(channel, legend, scales)?;

                // Get the renderer
                let renderer = if !legend_channels.is_empty() {
                    self.get_legend_renderer(
                        channel,
                        legend,
                        scale,
                        Some(legend_channels[0].mark_index),
                    )
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "No legend renderer available for channel '{}'",
                            channel
                        ))
                    })?
                } else {
                    continue;
                };

                // Measure the legend
                let (size, flexible) = measure_legend_size_with_channels(
                    &legend_channels,
                    legend,
                    renderer,
                    available_space,
                )?;

                legend_measurements.insert(channel.clone(), (size, flexible));
            }
        }

        Ok(legend_measurements)
    }

    /// Build legend channels for a specific channel
    /// This is used by both layout measurement and rendering
    pub fn build_legend_channels_for_channel(
        &self,
        channel: &str,
        legend: &Legend,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Result<Vec<LegendChannel>, AvengerChartError> {
        // Find the mark that has this channel
        let (mark_index, mark) = self
            .plot
            .mark_renderers
            .iter()
            .enumerate()
            .find(|(_, m)| m.data_context().channels().contains_key(channel))
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Channel '{}' not found in any mark",
                    channel
                ))
            })?;

        let channel_value = mark.data_context().channels().get(channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!("Channel '{}' not found in mark", channel))
        })?;

        let scale = scales.get(channel).ok_or_else(|| {
            AvengerChartError::InternalError(format!("Scale for channel '{}' not found", channel))
        })?;

        let mut legend_channels = Vec::new();

        // Always add the primary channel
        let primary_channel = self.build_legend_channel(
            channel,
            channel_value,
            scale,
            mark.as_ref(),
            mark_index,
            scales,
        );
        legend_channels.push(primary_channel);

        // Add any merged channels
        for merged_channel_name in &legend.merged_channels {
            if merged_channel_name != channel {
                // Only add if this channel exists in the mark
                if let Some(merged_channel_value) =
                    mark.data_context().channels().get(merged_channel_name)
                {
                    if let Some(merged_scale) = scales.get(merged_channel_name) {
                        let merged_channel = self.build_legend_channel(
                            merged_channel_name,
                            merged_channel_value,
                            merged_scale,
                            mark.as_ref(),
                            mark_index,
                            scales,
                        );
                        legend_channels.push(merged_channel);
                    }
                }
            }
        }

        Ok(legend_channels)
    }

    /// Helper function to merge legend channels based on MergeKey
    /// Returns a sorted list of channel groups and a legends map for layout
    pub(super) fn merge_legend_channels(
        &self,
        all_legends: &IndexMap<String, Legend>,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> (Vec<Vec<LegendChannel>>, IndexMap<String, Legend>) {
        // Collect all channels that need legends from all marks
        let mut all_channels = Vec::new();

        for (mark_index, mark) in self.plot.mark_renderers.iter().enumerate() {
            for (channel_name, channel_value) in mark.data_context().channels() {
                // Skip if no scale or no legend config
                if !configured_scales.contains_key(channel_name)
                    || !all_legends.contains_key(channel_name)
                {
                    continue;
                }

                let legend_config = &all_legends[channel_name];
                if matches!(legend_config.visible, crate::maybe::Maybe::Set(false)) {
                    continue;
                }

                let scale = &configured_scales[channel_name];

                let legend_channel = self.build_legend_channel(
                    channel_name,
                    channel_value,
                    scale,
                    mark.as_ref(),
                    mark_index,
                    configured_scales,
                );

                all_channels.push(legend_channel);
            }
        }

        // Group channels by MergeKey - use a vector to preserve order
        // Channels without merge keys (continuous scales) are kept separate
        let mut channel_groups: Vec<Vec<LegendChannel>> = Vec::new();

        for channel in all_channels {
            let merge_key = MergeKey::from_channel(&channel);

            if merge_key.is_none() {
                // Continuous scales or channels without expressions should not be merged
                // Each gets its own group
                channel_groups.push(vec![channel]);
            } else {
                // Find if this key already exists in any group
                let mut found = false;
                for group in channel_groups.iter_mut() {
                    if !group.is_empty() {
                        // Check if this group has the same merge key
                        let group_key = MergeKey::from_channel(&group[0]);
                        if group_key == merge_key {
                            group.push(channel.clone());
                            found = true;
                            break;
                        }
                    }
                }

                if !found {
                    // Create a new group for this merge key
                    channel_groups.push(vec![channel]);
                }
            }
        }

        // Sort channel groups by their legend order
        let mut groups_with_order: Vec<(Vec<LegendChannel>, i32)> = Vec::new();

        for channels in channel_groups {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    let order = legend_config.order.clone().unwrap_or(i32::MAX);
                    groups_with_order.push((channels, order));
                }
            }
        }

        // Sort by order value
        groups_with_order.sort_by_key(|(_, order)| *order);

        // Extract sorted channel groups
        let sorted_channel_groups: Vec<Vec<LegendChannel>> = groups_with_order
            .iter()
            .map(|(channels, _)| channels.clone())
            .collect();

        // Create the legends map with merged channel info for layout
        let mut legends_map: IndexMap<String, Legend> = IndexMap::new();
        for (channels, _) in groups_with_order {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    if !matches!(legend_config.visible, crate::maybe::Maybe::Set(false)) {
                        // Clone the legend config and add merged channel information
                        let mut legend_with_merged = legend_config.clone();
                        // Populate merged_channels with all channel types in this group
                        legend_with_merged.merged_channels =
                            channels.iter().map(|ch| ch.channel_type.clone()).collect();
                        legends_map.insert(primary_channel.name.clone(), legend_with_merged);
                    }
                }
            }
        }

        (sorted_channel_groups, legends_map)
    }

    /// Get legends with theme applied - used for both layout measurement and rendering
    pub(super) fn get_legends_with_theme(
        &self,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> IndexMap<String, Legend> {
        // 1. Start with plot-level legends
        let mut all_legends = self.plot.legends.clone();

        // 2. Apply channel-level legend configs
        for mark in &self.plot.mark_renderers {
            for (channel_name, channel_value) in mark.data_context().channels() {
                if let Some(channel_legend) = channel_value.get_legend_config() {
                    all_legends
                        .entry(channel_name.clone())
                        .and_modify(|legend| {
                            *legend = legend.clone().update(channel_legend.clone())
                        })
                        .or_insert_with(|| channel_legend.clone());
                }
            }
        }

        // 3. Apply defaults for channels with scales but no legend config
        let default_legends = self.create_default_legends(configured_scales);
        for (channel, default_legend) in default_legends {
            all_legends
                .entry(channel)
                .and_modify(|legend| *legend = default_legend.clone().update(legend.clone()))
                .or_insert(default_legend);
        }

        // 4. Apply theme (only for Unset properties)
        let theme = self.plot.get_theme();
        for legend in all_legends.values_mut() {
            // Theme only fills in Unset values using the update pattern
            let theme_legend = Legend {
                visible: crate::maybe::Maybe::Unset,
                title: crate::maybe::Maybe::Unset,
                position: crate::maybe::Maybe::Unset,
                orientation: crate::maybe::Maybe::Unset,
                symbol_size: crate::maybe::Maybe::Unset,
                gradient_length: crate::maybe::Maybe::Unset,
                gradient_thickness: crate::maybe::Maybe::Unset,
                columns: crate::maybe::Maybe::Unset,
                label_limit: crate::maybe::Maybe::Unset,
                format_number: crate::maybe::Maybe::Unset,
                background_fill: crate::maybe::Maybe::Unset,
                background_stroke: crate::maybe::Maybe::Unset,
                background_corner_radius: crate::maybe::Maybe::Unset,
                background_padding: crate::maybe::Maybe::Unset,
                order: crate::maybe::Maybe::Unset,
                contributing_marks: Vec::new(),
                renderer: None,
                merged_channels: Vec::new(),
                title_color: crate::maybe::Maybe::Set(theme.legend_title_color()),
                label_color: crate::maybe::Maybe::Set(theme.legend_label_color()),
                theme_mark_defaults: Some(theme.mark_defaults_map()),
                title_font_family: crate::maybe::Maybe::Set(theme.legend_title_font_family()),
                title_font_size: crate::maybe::Maybe::Set(theme.legend_title_font_size()),
                title_font_weight: crate::maybe::Maybe::Set(theme.legend_title_font_weight()),
                label_font_family: crate::maybe::Maybe::Set(theme.legend_label_font_family()),
                label_font_size: crate::maybe::Maybe::Set(theme.legend_label_font_size()),
                label_font_weight: crate::maybe::Maybe::Set(theme.legend_label_font_weight()),
                tick_font_family: crate::maybe::Maybe::Set(theme.legend_tick_font_family()),
                tick_font_size: crate::maybe::Maybe::Set(theme.legend_tick_font_size()),
                tick_font_weight: crate::maybe::Maybe::Set(theme.legend_tick_font_weight()),
                tick_color: crate::maybe::Maybe::Set(theme.legend_tick_color()),
            };
            *legend = theme_legend.update(legend.clone());
        }

        all_legends
    }
}
