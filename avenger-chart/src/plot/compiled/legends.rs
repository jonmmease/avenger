//! Legend construction and configuration for CompiledPlot

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{common::ScalarValue, logical_expr::lit, prelude::SessionContext};
use indexmap::IndexMap;
use tracing::debug;

use crate::{
    channel::value::ChannelValue,
    coords::{EmptyCoordMeasurement, extract_channel_title_from_marks},
    error::AvengerChartError,
    facet::{evaluated_facet_tree::EvaluatedFacetTree, sharing_policy},
    layout::legend::measure_legend_size_with_channels,
    layout::{FrameLayout, Size2D},
    legend::{
        ChannelInfo, Legend, LegendChannel, LegendPosition, MergeKey, renderer::LegendRenderer,
    },
    marks::CompiledMark,
    maybe::Maybe,
    plot::compiled::{
        ChildFrameSharingPath, ContainerPathSegment, CoordinationKind, EdgeOwnershipRequest,
        SharingLevel, edge_ownership_scope_for_request,
    },
    render::{
        EvaluationContext, LegendMeasurements, RenderContext, RenderState, types::LegendMeasurement,
    },
    scales::ConfiguredScaleWithSpec,
    serialization::LogicalExprNodeExt,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegendPlanScope {
    TopLevel,
    FacetCell,
    ChildFrame { sharing_path: ChildFrameSharingPath },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum HoistedLegendAnchor {
    FacetPath(Vec<ScalarValue>),
    ChildFrameContainer(Vec<ContainerPathSegment>),
}

#[derive(Clone)]
pub(crate) struct PreparedLegendGroup {
    pub layout_key: String,
    pub primary_channel: String,
    pub channels: Vec<LegendChannel>,
    pub legend: Legend,
    pub renderer: Arc<dyn LegendRenderer>,
}

#[derive(Clone)]
pub(crate) struct HoistedLegendRequest {
    pub anchor: HoistedLegendAnchor,
    pub owner: HoistedLegendAnchor,
    pub position: LegendPosition,
    pub sharing_level: SharingLevel,
    pub group: PreparedLegendGroup,
}

#[derive(Clone, Default)]
pub(crate) struct PreparedLegendPlan {
    pub groups: Vec<PreparedLegendGroup>,
    pub measurements: LegendMeasurements,
    pub hoisted_requests: Vec<HoistedLegendRequest>,
}

impl PreparedLegendPlan {
    pub(crate) fn retarget_scales(
        &mut self,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) {
        for group in &mut self.groups {
            retarget_legend_group_scales(group, configured_scales);
        }

        for request in &mut self.hoisted_requests {
            retarget_legend_group_scales(&mut request.group, configured_scales);
        }
    }
}

fn retarget_legend_group_scales(
    group: &mut PreparedLegendGroup,
    configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
) {
    for channel in &mut group.channels {
        if let Some(scale) = configured_scales.get(&channel.name) {
            channel.scale = scale.configured().clone();
        }

        for (related_name, related_channel) in &mut channel.related_channels {
            if let (
                Some(scale),
                ChannelInfo::Scaled {
                    scale: related_scale,
                    ..
                },
            ) = (configured_scales.get(related_name), related_channel)
            {
                *related_scale = scale.configured().clone();
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LegendDisposition {
    RenderHere,
    Hoist {
        anchor: HoistedLegendAnchor,
        sharing_level: SharingLevel,
    },
    Suppress,
}

impl CompiledPlot {
    fn effective_group_sharing_level(
        facet_tree: &EvaluatedFacetTree,
        channels: &[LegendChannel],
        default_sharing_level: SharingLevel,
    ) -> SharingLevel {
        channels
            .iter()
            .map(|channel| {
                channel.sharing_level.map_or_else(
                    || {
                        if default_sharing_level.is_free() {
                            SharingLevel::FREE
                        } else {
                            facet_tree.channel_domain_sharing_level_typed(channel.name.as_str())
                        }
                    },
                    SharingLevel::from_raw,
                )
            })
            .min()
            .unwrap_or(default_sharing_level)
    }

    #[cfg(test)]
    fn legend_visible_for_facet_cell(
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        sharing_level: SharingLevel,
        legend_position: LegendPosition,
        primary_channel: &str,
    ) -> bool {
        if sharing_level.is_free() {
            return true;
        }

        if facet_path.is_empty() {
            return true;
        }

        let Some(resolved) = facet_tree.resolve_path_info(facet_path) else {
            return true;
        };

        if resolved.indices.is_empty() {
            return true;
        }

        sharing_policy::legend_ownership_scope(
            primary_channel,
            facet_path,
            &resolved.indices,
            &resolved.local_level_counts,
            resolved.indices.len() as u8,
            sharing_level,
            legend_position,
        )
        .current_position_owns()
    }

    fn legend_disposition_for_facet_path(
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        sharing_level: SharingLevel,
        legend_position: LegendPosition,
        primary_channel: &str,
    ) -> LegendDisposition {
        if sharing_level.is_free() || facet_path.is_empty() {
            return LegendDisposition::RenderHere;
        }

        let Some(resolved) = facet_tree.resolve_path_info(facet_path) else {
            return LegendDisposition::RenderHere;
        };

        if resolved.indices.is_empty() {
            return LegendDisposition::RenderHere;
        }

        let ownership_scope = sharing_policy::legend_ownership_scope(
            primary_channel,
            facet_path,
            &resolved.indices,
            &resolved.local_level_counts,
            resolved.indices.len() as u8,
            sharing_level,
            legend_position,
        );

        if !ownership_scope.current_position_owns() {
            return LegendDisposition::Suppress;
        }

        let anchor_path = ownership_scope.anchor_path;
        if anchor_path == facet_path {
            LegendDisposition::RenderHere
        } else {
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(anchor_path),
                sharing_level,
            }
        }
    }

    fn child_frame_anchor_for_sharing(
        sharing_path: &ChildFrameSharingPath,
        sharing_level: SharingLevel,
    ) -> Vec<ContainerPathSegment> {
        let full_path = sharing_path.container_path();
        let keep_count = sharing_level.ancestor_keep_count(full_path.len(), full_path.len() as u8);
        full_path.into_iter().take(keep_count).collect()
    }

    fn legend_disposition_for_child_frame_path(
        sharing_path: &ChildFrameSharingPath,
        sharing_level: SharingLevel,
        legend_position: LegendPosition,
        primary_channel: &str,
    ) -> LegendDisposition {
        if sharing_level.is_free() || sharing_path.levels().is_empty() {
            return LegendDisposition::RenderHere;
        }

        let position_indices = sharing_path.position_indices();
        let level_counts = sharing_path.level_counts();
        let ownership = edge_ownership_scope_for_request(EdgeOwnershipRequest::new(
            CoordinationKind::LegendOwnership,
            format!("{primary_channel}:{legend_position:?}"),
            sharing_policy::legend_edge_for_position(legend_position),
            &position_indices,
            &level_counts,
            position_indices.len() as u8,
            sharing_level,
        ));

        if !ownership.current_position_owns() {
            return LegendDisposition::Suppress;
        }

        let full_path = sharing_path.container_path();
        let anchor_path = Self::child_frame_anchor_for_sharing(sharing_path, sharing_level);
        if anchor_path == full_path {
            LegendDisposition::RenderHere
        } else {
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::ChildFrameContainer(anchor_path),
                sharing_level,
            }
        }
    }

    fn legend_layout_key(primary_channel: &str) -> String {
        primary_channel.to_string()
    }

    fn hoisted_legend_layout_key(
        primary_channel: &str,
        anchor: &HoistedLegendAnchor,
        owner: &HoistedLegendAnchor,
    ) -> String {
        let mut hasher = DefaultHasher::new();
        anchor.hash(&mut hasher);
        owner.hash(&mut hasher);
        format!("{primary_channel}@{:016x}", hasher.finish())
    }

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
        F: FnOnce(&Legend) -> &Maybe<Option<U>>,
        G: FnOnce() -> Option<T>,
    {
        if matches!(field_check(legend), Maybe::Unset)
            && let Some(value) = theme_query()
        {
            *legend = setter(legend.clone(), value);
        }
    }

    /// Create default legends for channels with scales
    fn create_default_legends(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        session_context: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
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
            let title = self.infer_legend_title(channel, session_context);
            let position = self.default_legend_position(channel);
            let mut legend = Legend::new().title(title).position(position);

            // Create legend context for querying theme values
            let legend_ctx = theme.legend_context_with_params(legend_type, params.clone());
            let bg_ctx = legend_ctx.child("background");
            let base_font_size = theme.get_base_font_size(&legend_ctx.params);

            // Apply background padding if set
            if let Some(value) = theme.query(&bg_ctx, "padding")
                && let Some(padding) = value.as_font_size(&legend_ctx.params, base_font_size)
            {
                legend = legend.background_padding(padding);
            }

            // Apply background corner radius if set
            if let Some(value) = theme.query(&bg_ctx, "corner-radius")
                && let Some(radius) = value.as_font_size(&legend_ctx.params, base_font_size)
            {
                legend = legend.background_corner_radius(radius);
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
            if let Some(value) = theme.query(&bg_ctx, "stroke-width")
                && let Some(stroke_width) = value.as_font_size(&legend_ctx.params, base_font_size)
            {
                legend = legend.background_stroke_width(stroke_width);
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
    ) -> Option<Arc<dyn LegendRenderer>> {
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
        session_context: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
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
            if matches!(legend.position, Maybe::Unset) {
                // Check if theme has any media queries that affect legend position
                let has_media_queries =
                    theme.has_media_queries_for_property(&legend_ctx, "position");

                if !has_media_queries {
                    // No media queries, apply static position from theme
                    if let Some(theme_value) = theme.query(&legend_ctx, "position")
                        && let Some(position_str) = theme_value.as_string()
                    {
                        let position = match position_str.to_lowercase().as_str() {
                            "top" => Some(LegendPosition::Top),
                            "bottom" => Some(LegendPosition::Bottom),
                            "left" => Some(LegendPosition::Left),
                            "right" => Some(LegendPosition::Right),
                            _ => None,
                        };
                        if let Some(pos) = position {
                            *legend = legend.clone().position(pos);
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
        channel_value: &ChannelValue,
        scale: &ConfiguredScaleWithSpec,
        mark: &dyn CompiledMark,
        mark_index: usize,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> LegendChannel {
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
        let eval_ctx = EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            params.clone(),
            Arc::new(EvaluatedFacetTree::empty()),
        );
        let render_state = RenderState::new(
            100.0, // Dummy values for getting defaults
            100.0,
            std::collections::HashMap::new(),
        );
        let context = RenderContext::new(&eval_ctx, &render_state, &[], &EmptyCoordMeasurement);

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
            sharing_level: channel_value.get_share_mode().map(|mode| mode.to_level()),
            mark_type,
            mark_index,
            related_channels,
        }
    }

    fn legend_disposition(
        channel: &str,
        scope: &LegendPlanScope,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        resolved_position: LegendPosition,
        child_frame_sharing_level: SharingLevel,
        facet_sharing_level: SharingLevel,
        _configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        _params: &IndexMap<String, ScalarValue>,
    ) -> LegendDisposition {
        match scope {
            LegendPlanScope::TopLevel => LegendDisposition::RenderHere,
            LegendPlanScope::FacetCell => Self::legend_disposition_for_facet_path(
                facet_tree,
                facet_path,
                facet_sharing_level,
                resolved_position,
                channel,
            ),
            LegendPlanScope::ChildFrame { sharing_path } => {
                if !child_frame_sharing_level.is_free() {
                    let child_frame_disposition = Self::legend_disposition_for_child_frame_path(
                        sharing_path,
                        child_frame_sharing_level,
                        resolved_position,
                        channel,
                    );
                    if child_frame_disposition != LegendDisposition::RenderHere {
                        return child_frame_disposition;
                    }
                }

                if !facet_sharing_level.is_free() && !facet_path.is_empty() {
                    return Self::legend_disposition_for_facet_path(
                        facet_tree,
                        facet_path,
                        facet_sharing_level,
                        resolved_position,
                        channel,
                    );
                }

                LegendDisposition::RenderHere
            }
        }
    }

    async fn resolve_legend_position(
        &self,
        legend: &Legend,
        primary_channel: &LegendChannel,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<LegendPosition, AvengerChartError> {
        use super::expr_eval::evaluate_legend_position_expr;

        if let Some(node) = legend.position.as_option().and_then(|o| o.as_ref()) {
            let expr = node.to_expr(ctx)?;
            return evaluate_legend_position_expr(&expr, ctx, params).await;
        }

        // Position not set - check if theme has a position with runtime params.
        if let Some(theme) = &self.theme {
            // Determine legend type for theme context.
            let legend_type = if let Some(scale) = scales.get(primary_channel.name.as_str()) {
                if let Some(mark) = self.marks.iter().find(|m| {
                    m.data_context()
                        .channels()
                        .contains_key(primary_channel.name.as_str())
                }) {
                    mark.preferred_legend_renderer(&primary_channel.name, scale.configured())
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

            let legend_ctx = theme.legend_context_with_params(legend_type, params.clone());
            if let Some(theme_value) = theme.query(&legend_ctx, "position")
                && let Some(position_str) = theme_value.as_string()
            {
                return Ok(match position_str.to_lowercase().as_str() {
                    "top" => LegendPosition::Top,
                    "bottom" => LegendPosition::Bottom,
                    "left" => LegendPosition::Left,
                    "right" => LegendPosition::Right,
                    _ => LegendPosition::Right,
                });
            }
        }

        Ok(LegendPosition::Right)
    }

    /// Merge legend channels based on merge keys
    pub(super) async fn merge_legend_channels(
        &self,
        all_legends: &IndexMap<String, Legend>,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<(Vec<Vec<LegendChannel>>, IndexMap<String, Legend>), AvengerChartError> {
        use super::expr_eval::{evaluate_bool_expr, evaluate_i32_expr};

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
        let mut channel_groups: Vec<Vec<LegendChannel>> = Vec::new();

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
        let mut groups_with_order: Vec<(Vec<LegendChannel>, i32)> = Vec::new();

        // Evaluate order expressions
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

    pub(super) async fn prepare_legend_plan(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        available_space: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        facet_tree: &EvaluatedFacetTree,
        facet_path: &[ScalarValue],
        _child_frame_sharing_path: &ChildFrameSharingPath,
        scope: LegendPlanScope,
    ) -> Result<PreparedLegendPlan, AvengerChartError> {
        let mut legend_measurements = LegendMeasurements::new();
        let mut groups = Vec::new();
        let mut hoisted_requests = Vec::new();

        // Get all legends including channel-level configs
        let all_legends = self.get_legends_with_theme(scales, ctx, params);

        // Merge channels to get the same groups that will be used for rendering
        let (sorted_channel_groups, legends_map) = self
            .merge_legend_channels(&all_legends, scales, ctx, params)
            .await?;

        for channels in sorted_channel_groups {
            if channels.is_empty() {
                continue;
            }

            // Get the primary channel (first in group)
            let primary_channel = &channels[0];

            let Some(legend) = legends_map.get(&primary_channel.name) else {
                // Visibility expression evaluated to false in merge_legend_channels.
                continue;
            };

            let resolved_position = self
                .resolve_legend_position(legend, primary_channel, scales, ctx, params)
                .await?;

            let child_frame_sharing_level = Self::effective_group_sharing_level(
                facet_tree,
                channels.as_slice(),
                SharingLevel::FREE,
            );
            let facet_sharing_level = Self::effective_group_sharing_level(
                facet_tree,
                channels.as_slice(),
                SharingLevel::GLOBAL,
            );

            let disposition = Self::legend_disposition(
                &primary_channel.name,
                &scope,
                facet_tree,
                facet_path,
                resolved_position,
                child_frame_sharing_level,
                facet_sharing_level,
                scales,
                params,
            );
            if disposition == LegendDisposition::Suppress {
                continue;
            }

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
                let owner = match &scope {
                    LegendPlanScope::ChildFrame { sharing_path } => {
                        HoistedLegendAnchor::ChildFrameContainer(sharing_path.container_path())
                    }
                    LegendPlanScope::TopLevel | LegendPlanScope::FacetCell => {
                        HoistedLegendAnchor::FacetPath(facet_path.to_vec())
                    }
                };
                let layout_key = match &disposition {
                    LegendDisposition::Hoist { anchor, .. } => {
                        Self::hoisted_legend_layout_key(&primary_channel.name, anchor, &owner)
                    }
                    LegendDisposition::RenderHere | LegendDisposition::Suppress => {
                        Self::legend_layout_key(&primary_channel.name)
                    }
                };
                let group = PreparedLegendGroup {
                    layout_key: layout_key.clone(),
                    primary_channel: primary_channel.name.clone(),
                    channels: channels.clone(),
                    legend: legend.clone(),
                    renderer,
                };

                if let LegendDisposition::Hoist {
                    anchor,
                    sharing_level,
                } = disposition
                {
                    hoisted_requests.push(HoistedLegendRequest {
                        anchor,
                        owner,
                        position: resolved_position,
                        sharing_level,
                        group,
                    });
                    continue;
                }

                // Measure the legend with the same channels that will be used for rendering
                let theme = self.get_theme();
                let (size, flexible) = measure_legend_size_with_channels(
                    &channels,
                    legend,
                    group.renderer.clone(),
                    available_space,
                    theme.as_ref(),
                    params,
                    ctx,
                )
                .await?;

                debug!(
                    channel = primary_channel.name.as_str(),
                    width = size.width,
                    height = size.height,
                    flexible,
                    position = ?resolved_position,
                    child_frame_sharing = child_frame_sharing_level.raw(),
                    facet_sharing = facet_sharing_level.raw(),
                    "Legend measure"
                );
                legend_measurements.insert(
                    layout_key,
                    LegendMeasurement {
                        size,
                        flexible,
                        position: resolved_position,
                    },
                );

                groups.push(group);
            }
        }

        Ok(PreparedLegendPlan {
            groups,
            measurements: legend_measurements,
            hoisted_requests,
        })
    }

    pub(super) async fn render_legends_from_plan(
        &self,
        legend_plan: &PreparedLegendPlan,
        layout: &FrameLayout,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let theme = self.get_theme();
        let mut legend_marks = Vec::new();

        for group in &legend_plan.groups {
            let Some(bounds) = layout.legends.get(&group.layout_key) else {
                continue;
            };
            let group_opt = group
                .renderer
                .evaluate(
                    &group.channels,
                    &group.legend,
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                    theme.as_ref(),
                    params,
                    ctx,
                )
                .await?;
            if let Some(rendered_group) = group_opt {
                legend_marks.push(SceneMark::Group(rendered_group));
            }
        }

        Ok(legend_marks)
    }

    pub(super) async fn add_measured_hoisted_legends_to_plan(
        &self,
        legend_plan: &mut PreparedLegendPlan,
        requests: Vec<HoistedLegendRequest>,
        available_space: Size2D,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<(), AvengerChartError> {
        let theme = self.get_theme();
        for request in requests {
            let layout_key = request.group.layout_key.clone();
            if legend_plan.measurements.contains_key(&layout_key) {
                continue;
            }

            let (size, flexible) = measure_legend_size_with_channels(
                &request.group.channels,
                &request.group.legend,
                request.group.renderer.clone(),
                available_space,
                theme.as_ref(),
                params,
                ctx,
            )
            .await?;

            debug!(
                channel = request.group.primary_channel.as_str(),
                layout_key = layout_key.as_str(),
                owner = ?request.owner,
                anchor = ?request.anchor,
                width = size.width,
                height = size.height,
                flexible,
                position = ?request.position,
                sharing = request.sharing_level.raw(),
                "Hoisted legend measure"
            );

            legend_plan.measurements.insert(
                layout_key,
                LegendMeasurement {
                    size,
                    flexible,
                    position: request.position,
                },
            );
            legend_plan.groups.push(request.group);
        }

        Ok(())
    }

    /// Infer a title for the legend based on channel
    fn infer_legend_title(&self, channel: &str, session_context: &SessionContext) -> String {
        // First try to extract from marks (like we do for axes)
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
    fn default_legend_position(&self, _channel: &str) -> LegendPosition {
        // All legends default to the right
        LegendPosition::Right
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_scales::scales::band::BandScale;
    use datafusion::logical_expr::Expr;
    use indexmap::IndexMap;
    use std::collections::HashMap;

    use crate::{
        container::{ChildFrameSharingLevel, ChildFrameSharingPath, ContainerPathSegment},
        facet::evaluated_facet_tree::PartitionNode,
        guide::FacetDirection,
    };

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn make_simple_scale() -> ConfiguredScale {
        let domain = ScalarValue::iter_to_array(vec![s("A"), s("B")]).unwrap();
        BandScale::configured(domain, (0.0, 100.0))
    }

    fn make_legend_channel(name: &str) -> LegendChannel {
        LegendChannel {
            name: name.to_string(),
            expression: None::<Expr>,
            scale: make_simple_scale(),
            channel_type: name.to_string(),
            sharing_level: None,
            mark_type: "symbol".to_string(),
            mark_index: 0,
            related_channels: HashMap::new(),
        }
    }

    fn make_two_level_column_tree_with_sharing(levels: HashMap<String, u8>) -> EvaluatedFacetTree {
        let mut outer_children: IndexMap<ScalarValue, Box<PartitionNode>> = IndexMap::new();
        for outer in ["DivA", "DivB"] {
            let leaf = PartitionNode::leaf(
                FacetDirection::Column,
                0,
                "department".to_string(),
                None,
                vec![s("Dept1"), s("Dept2")],
            );
            outer_children.insert(s(outer), Box::new(leaf));
        }

        let root = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "division".to_string(),
            None,
            outer_children,
        );
        EvaluatedFacetTree::new_with_channel_domain_sharing_levels(Some(root), levels)
    }

    fn make_three_level_column_tree_with_sharing(
        levels: HashMap<String, u8>,
    ) -> EvaluatedFacetTree {
        let mut division_children: IndexMap<ScalarValue, Box<PartitionNode>> = IndexMap::new();
        for division in ["DivA", "DivB"] {
            let mut department_children: IndexMap<ScalarValue, Box<PartitionNode>> =
                IndexMap::new();
            for department in ["Dept1", "Dept2"] {
                let team_leaf = PartitionNode::leaf(
                    FacetDirection::Column,
                    0,
                    "team".to_string(),
                    None,
                    vec![s("Team1"), s("Team2")],
                );
                department_children.insert(s(department), Box::new(team_leaf));
            }

            let department_node = PartitionNode::branch(
                FacetDirection::Column,
                0,
                "department".to_string(),
                None,
                department_children,
            );
            division_children.insert(s(division), Box::new(department_node));
        }

        let root = PartitionNode::branch(
            FacetDirection::Column,
            0,
            "division".to_string(),
            None,
            division_children,
        );
        EvaluatedFacetTree::new_with_channel_domain_sharing_levels(Some(root), levels)
    }

    #[test]
    fn legend_visibility_free_always_true() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        let visible = CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            SharingLevel::FREE,
            LegendPosition::Right,
            "fill",
        );
        assert!(visible);
    }

    #[test]
    fn legend_visibility_level1_right_last_in_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            SharingLevel::from_raw(1),
            LegendPosition::Right,
            "fill",
        ));
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Right,
            "fill",
        ));
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Right,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level1_left_first_in_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            SharingLevel::from_raw(1),
            LegendPosition::Left,
            "fill",
        ));
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Left,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level1_top_first_in_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept1")],
            SharingLevel::from_raw(1),
            LegendPosition::Top,
            "fill",
        ));
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Top,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level1_bottom_last_in_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept1")],
            SharingLevel::from_raw(1),
            LegendPosition::Bottom,
            "fill",
        ));
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            SharingLevel::from_raw(1),
            LegendPosition::Bottom,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level2_coarser_grouping() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept2")],
            SharingLevel::from_raw(2),
            LegendPosition::Right,
            "fill",
        ));
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept2")],
            SharingLevel::from_raw(2),
            LegendPosition::Right,
            "fill",
        ));
    }

    #[test]
    fn legend_visibility_level_ge_depth_global_group() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert!(CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivA"), s("Dept1")],
            SharingLevel::from_raw(3),
            LegendPosition::Left,
            "fill",
        ));
        assert!(!CompiledPlot::legend_visible_for_facet_cell(
            &tree,
            &[s("DivB"), s("Dept1")],
            SharingLevel::from_raw(3),
            LegendPosition::Left,
            "fill",
        ));
    }

    #[test]
    fn effective_group_sharing_uses_min_level() {
        let mut levels = HashMap::new();
        levels.insert("fill".to_string(), 1);
        levels.insert("stroke".to_string(), 255);
        let tree = make_two_level_column_tree_with_sharing(levels);
        let channels = vec![make_legend_channel("fill"), make_legend_channel("stroke")];
        assert_eq!(
            CompiledPlot::effective_group_sharing_level(&tree, &channels, SharingLevel::GLOBAL),
            SharingLevel::from_raw(1)
        );
    }

    #[test]
    fn legend_disposition_free_renders_in_facet_cell() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        assert_eq!(
            CompiledPlot::legend_disposition_for_facet_path(
                &tree,
                &[s("DivA"), s("Dept1")],
                SharingLevel::FREE,
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::RenderHere
        );
    }

    #[test]
    fn legend_disposition_non_owner_suppresses() {
        let tree = make_three_level_column_tree_with_sharing(HashMap::new());
        assert_eq!(
            CompiledPlot::legend_disposition_for_facet_path(
                &tree,
                &[s("DivA"), s("Dept1"), s("Team1")],
                SharingLevel::from_raw(2),
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Suppress
        );
    }

    #[test]
    fn legend_disposition_level2_owner_hoists_to_facet_group() {
        let tree = make_three_level_column_tree_with_sharing(HashMap::new());
        assert_eq!(
            CompiledPlot::legend_disposition_for_facet_path(
                &tree,
                &[s("DivA"), s("Dept2"), s("Team2")],
                SharingLevel::from_raw(2),
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(vec![s("DivA")]),
                sharing_level: SharingLevel::from_raw(2),
            }
        );
    }

    #[test]
    fn legend_disposition_global_owner_hoists_to_root_group() {
        let tree = make_three_level_column_tree_with_sharing(HashMap::new());
        assert_eq!(
            CompiledPlot::legend_disposition_for_facet_path(
                &tree,
                &[s("DivB"), s("Dept2"), s("Team2")],
                SharingLevel::GLOBAL,
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(vec![]),
                sharing_level: SharingLevel::GLOBAL,
            }
        );
    }

    #[test]
    fn legend_disposition_child_frame_non_owner_suppresses() {
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(0, 2, Some("left")));

        assert_eq!(
            CompiledPlot::legend_disposition_for_child_frame_path(
                &sharing_path,
                SharingLevel::GLOBAL,
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Suppress
        );
    }

    #[test]
    fn legend_disposition_child_frame_owner_hoists_to_container() {
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("right")));

        assert_eq!(
            CompiledPlot::legend_disposition_for_child_frame_path(
                &sharing_path,
                SharingLevel::GLOBAL,
                LegendPosition::Right,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::ChildFrameContainer(vec![]),
                sharing_level: SharingLevel::GLOBAL,
            }
        );
    }

    #[test]
    fn legend_disposition_child_frame_level1_hoists_to_immediate_parent() {
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(0, 2, Some("outer")))
            .appended(ChildFrameSharingLevel::vconcat_child(1, 2, Some("inner")));

        assert_eq!(
            CompiledPlot::legend_disposition_for_child_frame_path(
                &sharing_path,
                SharingLevel::from_raw(1),
                LegendPosition::Bottom,
                "fill",
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::ChildFrameContainer(vec![
                    ContainerPathSegment::concat_child(0, Some("outer"))
                ]),
                sharing_level: SharingLevel::from_raw(1),
            }
        );
    }

    #[test]
    fn free_child_frame_legend_inside_facet_can_use_facet_ownership() {
        let tree = make_two_level_column_tree_with_sharing(HashMap::new());
        let sharing_path = ChildFrameSharingPath::root()
            .appended(ChildFrameSharingLevel::hconcat_child(1, 2, Some("petal")));

        assert_eq!(
            CompiledPlot::legend_disposition(
                "fill",
                &LegendPlanScope::ChildFrame { sharing_path },
                &tree,
                &[s("DivB"), s("Dept2")],
                LegendPosition::Right,
                SharingLevel::FREE,
                SharingLevel::GLOBAL,
                &HashMap::new(),
                &IndexMap::new(),
            ),
            LegendDisposition::Hoist {
                anchor: HoistedLegendAnchor::FacetPath(vec![]),
                sharing_level: SharingLevel::GLOBAL,
            }
        );
    }

    #[test]
    fn hoisted_legend_layout_key_includes_owner_identity() {
        let anchor = HoistedLegendAnchor::FacetPath(vec![]);
        let sepal =
            HoistedLegendAnchor::ChildFrameContainer(vec![ContainerPathSegment::concat_child(
                0,
                Some("sepal"),
            )]);
        let petal =
            HoistedLegendAnchor::ChildFrameContainer(vec![ContainerPathSegment::concat_child(
                1,
                Some("petal"),
            )]);

        assert_ne!(
            CompiledPlot::hoisted_legend_layout_key("fill", &anchor, &sepal),
            CompiledPlot::hoisted_legend_layout_key("fill", &anchor, &petal)
        );
    }
}
