//! Legend construction and configuration for CompiledPlot

use std::collections::HashMap;
use std::sync::Arc;

use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::render::RenderContext;
use crate::scales::ConfiguredScaleWithSpec;

use super::CompiledPlot;

/// Convert normalized color array [0.0-1.0] to hex string
fn color_array_to_hex(color: [f32; 4]) -> String {
    let r = (color[0] * 255.0) as u8;
    let g = (color[1] * 255.0) as u8;
    let b = (color[2] * 255.0) as u8;
    let a = (color[3] * 255.0) as u8;

    if a < 255 {
        format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a)
    } else {
        format!("#{:02x}{:02x}{:02x}", r, g, b)
    }
}

impl CompiledPlot {
    /// Apply a theme value to a legend field if the field is Unset
    ///
    /// This helper reduces repetition when applying theme defaults to legend properties.
    /// It checks if a field is Unset and applies the theme value using a setter function.
    fn apply_theme_to_legend<T, U, F, G>(
        legend: &mut Legend,
        field_check: F,
        theme_query: G,
        setter: impl FnOnce(Legend, T) -> Legend,
    ) where
        F: FnOnce(&Legend) -> &crate::maybe::Maybe<Option<U>>,
        G: FnOnce() -> Option<T>,
    {
        if matches!(field_check(legend), crate::maybe::Maybe::Unset) {
            if let Some(value) = theme_query() {
                *legend = setter(legend.clone(), value);
            }
        }
    }

    /// Create default legends for channels with scales
    fn create_default_legends(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        session_context: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> IndexMap<String, Legend> {
        let mut default_legends = IndexMap::new();

        // Build set of channels to skip for legends
        let mut skip_channels = std::collections::HashSet::new();

        // Add positional channels from the coordinate system
        for &channel in self.coord_transform.required_channels() {
            skip_channels.insert(channel.to_string());
            // Also skip interval variants (e.g., "x2" for "x")
            skip_channels.insert(format!("{}2", channel));
        }

        // Add channels that marks indicate shouldn't have legends
        for (channel, scale) in scales {
            // Find the mark that has this channel
            if let Some(mark) = self
                .marks
                .iter()
                .find(|m| m.data_context().channels().contains_key(channel))
            {
                // If the mark that has the channel says no legend, skip it
                if mark
                    .preferred_legend_renderer(channel, scale.configured())
                    .is_none()
                {
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
            if self.legends.contains_key(channel) {
                continue;
            }

            // Determine legend renderer type for CSS selector support
            // (e.g., legend[type="symbol"], legend[type="line"], legend[type="colorbar"])
            let legend_type = if let Some(scale) = scales.get(channel) {
                if let Some(mark) = self
                    .marks
                    .iter()
                    .find(|m| m.data_context().channels().contains_key(channel))
                {
                    if let Some(renderer) =
                        mark.preferred_legend_renderer(channel, scale.configured())
                    {
                        Some(match renderer.name() {
                            "CompiledSymbolLegend" => "symbol",
                            "CompiledLineLegend" => "line",
                            "CompiledColorbar" => "colorbar",
                            "CompiledRectLegend" => "rect",
                            _ => "symbol", // default fallback
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Create legend with theme defaults
            let theme = self.get_theme();
            let mut legend = Legend::new()
                .title(self.infer_legend_title(channel, session_context))
                .position(self.default_legend_position(channel));

            // Create legend context for querying theme values
            let legend_ctx = theme.legend_context_with_params(legend_type, params.clone());
            let bg_ctx = legend_ctx.child("background");
            let base_font_size = theme.get_base_font_size(&legend_ctx.params);

            // Apply background padding if set
            if let Some(value) = theme.query(&bg_ctx, "padding") {
                if let Some(padding) = value.as_font_size(&legend_ctx.params, base_font_size) {
                    legend = legend.background_padding(padding);
                }
            }

            // Apply background corner radius if set
            if let Some(value) = theme.query(&bg_ctx, "corner-radius") {
                if let Some(radius) = value.as_font_size(&legend_ctx.params, base_font_size) {
                    legend = legend.background_corner_radius(radius);
                }
            }

            // Apply optional theme defaults
            if let Some(fill) = theme.fill_color(&bg_ctx) {
                // Convert [f32; 4] to hex string
                legend = legend.background_fill(color_array_to_hex(fill));
            }
            if let Some(stroke) = theme.stroke_color(&bg_ctx) {
                // Convert [f32; 4] to hex string
                legend = legend.background_stroke(color_array_to_hex(stroke));
            }
            // Apply stroke-width from theme
            if let Some(value) = theme.query(&bg_ctx, "stroke-width") {
                if let Some(stroke_width) = value.as_font_size(&legend_ctx.params, base_font_size) {
                    legend = legend.background_stroke_width(stroke_width);
                }
            }

            // Set text colors and typography from theme (using defaults if theme doesn't specify)
            if let Some(color) = theme.text_color(&legend_ctx.child("title")) {
                legend = legend.title_color(color_array_to_hex(color));
            }
            if let Some(color) = theme.text_color(&legend_ctx.child("label")) {
                legend = legend.label_color(color_array_to_hex(color));
            }
            if let Some(font_family) = theme.font_family(&legend_ctx.child("title")) {
                legend = legend.title_font_family(font_family);
            }
            if let Some(size) = theme.font_size(&legend_ctx.child("title")) {
                legend = legend.title_font_size(size);
            }
            if let Some(weight) = theme.font_weight(&legend_ctx.child("title")) {
                legend = legend.title_font_weight(weight);
            }
            if let Some(font_family) = theme.font_family(&legend_ctx.child("label")) {
                legend = legend.label_font_family(font_family);
            }
            if let Some(size) = theme.font_size(&legend_ctx.child("label")) {
                legend = legend.label_font_size(size);
            }
            if let Some(weight) = theme.font_weight(&legend_ctx.child("label")) {
                legend = legend.label_font_weight(weight);
            }
            if let Some(font_family) = theme.font_family(&legend_ctx.child("tick")) {
                legend = legend.tick_font_family(font_family);
            }
            if let Some(size) = theme.font_size(&legend_ctx.child("tick")) {
                legend = legend.tick_font_size(size);
            }
            if let Some(weight) = theme.font_weight(&legend_ctx.child("tick")) {
                legend = legend.tick_font_weight(weight);
            }
            if let Some(color) = theme.text_color(&legend_ctx.child("tick")) {
                legend = legend.tick_color(color_array_to_hex(color));
            }

            default_legends.insert(channel.clone(), legend);
        }

        default_legends
    }

    /// Get the appropriate legend renderer for a channel
    pub(super) fn get_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScaleWithSpec,
    ) -> Option<Arc<dyn crate::legend::renderer::LegendRenderer>> {
        // Find the first mark that has this channel and get its preference
        for mark in &self.marks {
            if mark.data_context().channels().contains_key(channel) {
                return mark.preferred_legend_renderer(channel, scale.configured());
            }
        }
        None
    }

    /// Get legends with theme applied (matching PlotRenderer behavior)
    pub(super) fn get_legends_with_theme(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        session_context: &datafusion::prelude::SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> IndexMap<String, Legend> {
        // 1. Start with plot-level legends
        let mut all_legends = self.legends.clone();

        // 2. Apply channel-level legend configs (from mark encodings)
        for mark in &self.marks {
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
        let default_legends = self.create_default_legends(scales, session_context, params);
        for (channel, default_legend) in default_legends {
            all_legends
                .entry(channel)
                .and_modify(|legend| *legend = default_legend.clone().update(legend.clone()))
                .or_insert(default_legend);
        }

        // 4. Apply theme (only for Unset properties)
        let theme = self.get_theme();
        for (channel, legend) in all_legends.iter_mut() {
            // Determine legend type for this channel (for CSS selector support)
            let legend_type = if let Some(scale) = scales.get(channel) {
                if let Some(mark) = self
                    .marks
                    .iter()
                    .find(|m| m.data_context().channels().contains_key(channel))
                {
                    mark.preferred_legend_renderer(channel, scale.configured())
                        .map(|renderer| match renderer.name() {
                            "CompiledSymbolLegend" => "symbol",
                            "CompiledLineLegend" => "line",
                            "CompiledColorbar" => "colorbar",
                            "CompiledRectLegend" => "rect",
                            _ => "symbol",
                        })
                } else {
                    None
                }
            } else {
                None
            };

            // Create legend context for querying theme values
            let legend_ctx = theme.legend_context_with_params(legend_type, params.clone());

            // Theme only fills in Unset values - apply theme properties if not explicitly set

            // Title styling
            Self::apply_theme_to_legend(
                legend,
                |l| &l.title_color,
                || {
                    theme
                        .text_color(&legend_ctx.child("title"))
                        .map(color_array_to_hex)
                },
                |l, v| l.title_color(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.title_font_family,
                || theme.font_family(&legend_ctx.child("title")),
                |l, v| l.title_font_family(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.title_font_size,
                || theme.font_size(&legend_ctx.child("title")),
                |l, v| l.title_font_size(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.title_font_weight,
                || theme.font_weight(&legend_ctx.child("title")),
                |l, v| l.title_font_weight(v),
            );

            // Label styling
            Self::apply_theme_to_legend(
                legend,
                |l| &l.label_color,
                || {
                    theme
                        .text_color(&legend_ctx.child("label"))
                        .map(color_array_to_hex)
                },
                |l, v| l.label_color(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.label_font_family,
                || theme.font_family(&legend_ctx.child("label")),
                |l, v| l.label_font_family(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.label_font_size,
                || theme.font_size(&legend_ctx.child("label")),
                |l, v| l.label_font_size(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.label_font_weight,
                || theme.font_weight(&legend_ctx.child("label")),
                |l, v| l.label_font_weight(v),
            );

            // Tick styling (for colorbar legends)
            Self::apply_theme_to_legend(
                legend,
                |l| &l.tick_color,
                || {
                    theme
                        .text_color(&legend_ctx.child("tick"))
                        .map(color_array_to_hex)
                },
                |l, v| l.tick_color(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.tick_font_family,
                || theme.font_family(&legend_ctx.child("tick")),
                |l, v| l.tick_font_family(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.tick_font_size,
                || theme.font_size(&legend_ctx.child("tick")),
                |l, v| l.tick_font_size(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.tick_font_weight,
                || theme.font_weight(&legend_ctx.child("tick")),
                |l, v| l.tick_font_weight(v),
            );

            // Background styling
            Self::apply_theme_to_legend(
                legend,
                |l| &l.background_fill,
                || {
                    theme
                        .fill_color(&legend_ctx.child("background"))
                        .map(color_array_to_hex)
                },
                |l, v| l.background_fill(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.background_stroke,
                || {
                    theme
                        .stroke_color(&legend_ctx.child("background"))
                        .map(color_array_to_hex)
                },
                |l, v| l.background_stroke(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.background_stroke_width,
                || {
                    let bg_ctx = legend_ctx.child("background");
                    let base_font_size = theme.get_base_font_size(&legend_ctx.params);
                    theme
                        .query(&bg_ctx, "stroke-width")
                        .and_then(|value| value.as_font_size(&legend_ctx.params, base_font_size))
                },
                |l, v| l.background_stroke_width(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.background_padding,
                || {
                    let bg_ctx = legend_ctx.child("background");
                    let base_font_size = theme.get_base_font_size(&legend_ctx.params);
                    theme
                        .query(&bg_ctx, "padding")
                        .and_then(|value| value.as_font_size(&legend_ctx.params, base_font_size))
                },
                |l, v| l.background_padding(v),
            );
            Self::apply_theme_to_legend(
                legend,
                |l| &l.background_corner_radius,
                || {
                    let bg_ctx = legend_ctx.child("background");
                    let base_font_size = theme.get_base_font_size(&legend_ctx.params);
                    theme
                        .query(&bg_ctx, "corner-radius")
                        .and_then(|value| value.as_font_size(&legend_ctx.params, base_font_size))
                },
                |l, v| l.background_corner_radius(v),
            );

            // Apply legend position from theme if not explicitly set
            // Don't set position at compile time if there are media queries -
            // it will be determined at evaluate time with actual parameters
            if matches!(legend.position, crate::maybe::Maybe::Unset) {
                // Check if theme has any media queries that affect legend position
                let has_media_queries =
                    theme.has_media_queries_for_property(&legend_ctx, "position");

                if !has_media_queries {
                    // No media queries, apply static position from theme
                    if let Some(theme_value) = theme.query(&legend_ctx, "position") {
                        if let Some(position_str) = theme_value.as_string() {
                            let position = match position_str.to_lowercase().as_str() {
                                "top" => Some(crate::legend::LegendPosition::Top),
                                "bottom" => Some(crate::legend::LegendPosition::Bottom),
                                "left" => Some(crate::legend::LegendPosition::Left),
                                "right" => Some(crate::legend::LegendPosition::Right),
                                _ => None,
                            };
                            if let Some(pos) = position {
                                *legend = legend.clone().position(pos);
                            }
                        }
                    }
                }
                // If there are media queries, leave position unset - it will be
                // determined at evaluate time
            }

            // Note: Don't apply theme background settings - they're only for default legends
            // This matches PlotRenderer behavior
        }

        all_legends
    }

    /// Build a legend channel for a specific channel in a mark
    fn build_legend_channel(
        &self,
        channel_name: &str,
        channel_value: &crate::channel::value::ChannelValue,
        scale: &ConfiguredScaleWithSpec,
        mark: &dyn crate::marks::CompiledMark,
        mark_index: usize,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> crate::legend::LegendChannel {
        use crate::legend::{ChannelInfo, LegendChannel};
        use datafusion::logical_expr::lit;

        // Collect related channels from the mark
        let mut related_channels = HashMap::new();

        // First add explicitly set channels
        for (other_name, other_value) in mark.data_context().channels() {
            if other_name != channel_name {
                // Check if this channel has a scale or is constant
                let channel_info = if let Some(other_scale) = configured_scales.get(other_name) {
                    // Channel has a scale
                    ChannelInfo::Scaled {
                        expr: other_value.expr(ctx),
                        scale: other_scale.configured().clone(),
                    }
                } else if let Some(expr) = other_value.expr(ctx) {
                    // Channel has a constant expression
                    ChannelInfo::Constant { expr: expr.clone() }
                } else {
                    // Skip channels without expressions
                    continue;
                };
                related_channels.insert(other_name.clone(), channel_info);
            }
        }

        // For channels not explicitly set, check if they have theme defaults
        // This ensures legend symbols match the chart's actual appearance
        let context = RenderContext::new(
            self.get_theme(),
            100.0, // Dummy values for getting defaults
            100.0,
            Arc::new(ctx.clone()),
            params.clone(),
            std::collections::HashMap::new(),
            Arc::new(crate::facet::computed_facet_spec::EvaluatedFacetTree::empty()),
            None, // No coordination context for legend defaults
        );

        // Iterate through all supported channels of this mark
        for channel_desc in mark.supported_channels() {
            let other_name = channel_desc.name;
            if other_name != channel_name && !related_channels.contains_key(other_name) {
                // Channel not explicitly set - check for theme default
                if let Some(default_value) = mark.default_channel_value(other_name, &context) {
                    // Add as a constant channel
                    let expr = lit(default_value);
                    related_channels.insert(other_name.to_string(), ChannelInfo::Constant { expr });
                }
            }
        }

        // Get the mark type name
        let mark_type = mark.mark_type().to_string();

        LegendChannel {
            name: channel_name.to_string(),
            expression: channel_value.expr(ctx),
            scale: scale.configured().clone(),
            channel_type: channel_name.to_string(), // Use channel name as type
            mark_type,
            mark_index,
            related_channels,
        }
    }

    /// Merge legend channels based on merge keys
    pub(super) async fn merge_legend_channels(
        &self,
        all_legends: &IndexMap<String, Legend>,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<
        (
            Vec<Vec<crate::legend::LegendChannel>>,
            IndexMap<String, Legend>,
        ),
        AvengerChartError,
    > {
        use crate::legend::MergeKey;

        // Collect all channels that need legends from all marks
        let mut all_channels = Vec::new();

        for (mark_index, mark) in self.marks.iter().enumerate() {
            for (channel_name, channel_value) in mark.data_context().channels() {
                // Skip if no scale or no legend config
                if !configured_scales.contains_key(channel_name)
                    || !all_legends.contains_key(channel_name)
                {
                    continue;
                }

                // Note: Visibility is evaluated and filtered in merge_legend_channels
                let _legend_config = &all_legends[channel_name];

                let scale = &configured_scales[channel_name];

                let legend_channel = self.build_legend_channel(
                    channel_name,
                    channel_value,
                    scale,
                    mark.as_ref(),
                    mark_index,
                    configured_scales,
                    ctx,
                    params,
                );

                all_channels.push(legend_channel);
            }
        }

        // Group channels by MergeKey
        let mut channel_groups: Vec<Vec<crate::legend::LegendChannel>> = Vec::new();

        for channel in all_channels {
            let merge_key = MergeKey::from_channel(&channel);

            if merge_key.is_none() {
                // Continuous scales or channels without expressions should not be merged
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
        let mut groups_with_order: Vec<(Vec<crate::legend::LegendChannel>, i32)> = Vec::new();

        // Evaluate order expressions
        use crate::plot::compiled::expr_eval::*;
        use crate::serialization::LogicalExprNodeExt;

        for channels in channel_groups {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    let order = if let Some(node) =
                        legend_config.order.as_option().and_then(|o| o.as_ref())
                    {
                        let expr = node.to_expr(ctx)?;
                        evaluate_i32_expr(&expr, ctx, params).await?
                    } else {
                        i32::MAX
                    };
                    groups_with_order.push((channels, order));
                }
            }
        }

        // Sort by order value
        groups_with_order.sort_by_key(|(_, order)| *order);

        // Extract sorted channel groups
        let sorted_channel_groups: Vec<Vec<crate::legend::LegendChannel>> = groups_with_order
            .iter()
            .map(|(channels, _)| channels.clone())
            .collect();

        // Create the legends map with merged channel info for layout
        let mut legends_map: IndexMap<String, Legend> = IndexMap::new();
        for (channels, _) in groups_with_order {
            if !channels.is_empty() {
                let primary_channel = &channels[0];
                if let Some(legend_config) = all_legends.get(&primary_channel.name) {
                    // Evaluate visibility expression
                    let visible = if let Some(node) =
                        legend_config.visible.as_option().and_then(|o| o.as_ref())
                    {
                        let expr = node.to_expr(ctx)?;
                        evaluate_bool_expr(&expr, ctx, params).await?
                    } else {
                        true // Default to visible
                    };
                    if visible {
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

        Ok((sorted_channel_groups, legends_map))
    }

    /// Prepare legend measurements for layout computation
    /// Note: This should be called with the legends_map from merge_legend_channels
    /// to ensure measurements match rendering
    pub(super) async fn prepare_legend_measurements(
        &self,
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        available_space: taffy::Size<f32>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<crate::render::LegendMeasurements, AvengerChartError> {
        use crate::layout::legend::measure_legend_size_with_channels;
        use crate::render::types::LegendMeasurement;
        use avenger_scales::scales::ConfiguredScale;

        let mut legend_measurements = crate::render::LegendMeasurements::new();

        // Get all legends including channel-level configs
        let all_legends = self.get_legends_with_theme(scales, ctx, params);

        // Merge channels to get the same groups that will be used for rendering
        // Use the passed-in legends parameter which is already sorted
        let (sorted_channel_groups, _) = self
            .merge_legend_channels(&all_legends, scales, ctx, params)
            .await?;

        for channels in sorted_channel_groups {
            if channels.is_empty() {
                continue;
            }

            // Get the primary channel (first in group)
            let primary_channel = &channels[0];

            // Get legend config - first try the passed-in legends (from merge),
            // then fall back to all_legends
            let legend = legends
                .get(&primary_channel.name)
                .or_else(|| all_legends.get(&primary_channel.name))
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Legend configuration not found for channel '{}'",
                        primary_channel.name
                    ))
                })?;

            // Get scale for primary channel
            let scale = scales.get(&primary_channel.name).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Scale not found for channel '{}'",
                    primary_channel.name
                ))
            })?;

            // Determine the appropriate renderer for this group of channels
            let renderer = if channels.len() > 1 {
                // Multiple channels - try to get a merged renderer
                let mark_opt = self.marks.get(primary_channel.mark_index);
                // Extract ConfiguredScale from ConfiguredScaleWithSpec for mark's renderer
                let configured_scales: HashMap<String, ConfiguredScale> = scales
                    .iter()
                    .map(|(k, v)| (k.clone(), v.configured().clone()))
                    .collect();
                mark_opt
                    .and_then(|mark| {
                        mark.preferred_merged_legend_renderer(&channels, &configured_scales)
                    })
                    .or_else(|| self.get_legend_renderer(&primary_channel.channel_type, scale))
            } else {
                // Single channel - use the unified renderer selection
                self.get_legend_renderer(&primary_channel.channel_type, scale)
            };

            if let Some(renderer) = renderer {
                // Measure the legend with the same channels that will be used for rendering
                let theme = self.get_theme();
                let (size, flexible) = measure_legend_size_with_channels(
                    &channels,
                    legend,
                    renderer,
                    available_space,
                    theme.as_ref(),
                    params,
                    ctx,
                )
                .await?;
                // Evaluate position expression
                use crate::plot::compiled::expr_eval::*;
                use crate::serialization::LogicalExprNodeExt;
                let position =
                    if let Some(node) = legend.position.as_option().and_then(|o| o.as_ref()) {
                        let expr = node.to_expr(ctx)?;
                        evaluate_legend_position_expr(&expr, ctx, params).await?
                    } else {
                        // Position not set - check if theme has a position with runtime params
                        // This handles media queries that depend on runtime parameters
                        if let Some(theme) = &self.theme {
                            // Determine legend type for theme context
                            let legend_type =
                                if let Some(scale) = scales.get(primary_channel.name.as_str()) {
                                    if let Some(mark) = self.marks.iter().find(|m| {
                                        m.data_context()
                                            .channels()
                                            .contains_key(primary_channel.name.as_str())
                                    }) {
                                        mark.preferred_legend_renderer(
                                            &primary_channel.name,
                                            scale.configured(),
                                        )
                                        .map(|renderer| {
                                            match renderer.name() {
                                                "CompiledSymbolLegend" => "symbol",
                                                "CompiledLineLegend" => "line",
                                                "CompiledColorbar" => "colorbar",
                                                "CompiledRectLegend" => "rect",
                                                _ => "symbol",
                                            }
                                        })
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };

                            let legend_ctx =
                                theme.legend_context_with_params(legend_type, params.clone());
                            if let Some(theme_value) = theme.query(&legend_ctx, "position") {
                                if let Some(position_str) = theme_value.as_string() {
                                    match position_str.to_lowercase().as_str() {
                                        "top" => crate::legend::LegendPosition::Top,
                                        "bottom" => crate::legend::LegendPosition::Bottom,
                                        "left" => crate::legend::LegendPosition::Left,
                                        "right" => crate::legend::LegendPosition::Right,
                                        _ => crate::legend::LegendPosition::Right,
                                    }
                                } else {
                                    crate::legend::LegendPosition::Right
                                }
                            } else {
                                crate::legend::LegendPosition::Right
                            }
                        } else {
                            crate::legend::LegendPosition::Right
                        }
                    };
                if std::env::var("AVENGER_DEBUG_LEGEND").is_ok() {
                    eprintln!(
                        "LEGEND MEASURE: channel='{}' size=({:.1},{:.1}) flexible={} position={:?}",
                        primary_channel.name, size.width, size.height, flexible, position
                    );
                }
                legend_measurements.insert(
                    primary_channel.name.clone(),
                    LegendMeasurement {
                        size,
                        flexible,
                        position,
                    },
                );
            }
        }

        Ok(legend_measurements)
    }

    /// Infer a title for the legend based on channel
    fn infer_legend_title(
        &self,
        channel: &str,
        session_context: &datafusion::prelude::SessionContext,
    ) -> String {
        // First try to extract from marks (like we do for axes)
        use crate::coords::extract_channel_title_from_marks;
        if let Some(title) = extract_channel_title_from_marks(&self.marks, channel, session_context)
        {
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
}
