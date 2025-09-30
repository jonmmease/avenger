//! Rendering pipeline implementation for CompiledPlot

use super::compiled_plot::CompiledPlot;
use super::specs::{AxisSpec, ScaleSpec};
use super::title::{PlotSubtitle, PlotTitle};

use crate::channel::value::{ChannelValue, ConditionalValue, strip_trailing_numbers};
use crate::coords::CoordinateSystemTransform;
use crate::error::AvengerChartError;
use crate::guide::CompiledGuide;
use crate::layout::{LayoutSpec, Margins};
use crate::legend::Legend;
use crate::marks::CompiledMark;
use crate::render::RenderContext;
use crate::scales::ConfiguredScaleWithSpec;
use crate::scales::Scale;
use crate::serialization::{LogicalExprNodeExt, LogicalPlanNodeExt};
use crate::theme::Theme;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

impl CompiledPlot {
    /// Build a configured scale with radius context for polar coordinates
    async fn build_configured_scale_with_radius_context(
        &self,
        mut scale: Scale,
        name: &str,
        context: &crate::render::RenderContext,
        configured_non_positional: Option<&HashMap<String, ConfiguredScaleWithSpec>>,
    ) -> Result<ConfiguredScaleWithSpec, AvengerChartError> {
        // Process domain with radius if applicable
        if scale
            .domain
            .as_ref()
            .map(|d| {
                matches!(
                    &d.default_domain,
                    crate::scales::ScaleDefaultDomain::DomainExprs(_)
                )
            })
            .unwrap_or(false)
        {
            // Only use radius-aware gathering for positional scales that support it
            if let Some(configured_non_positional) = configured_non_positional {
                // Check if this is a positional channel (including interval variants)
                let is_positional = self
                    .coord_transform
                    .required_channels()
                    .iter()
                    .any(|&ch| ch == name || name == &format!("{}2", ch));

                if scale
                    .get_scale_impl()
                    .map(|impl_| impl_.supports_radius_expansion())
                    .unwrap_or(false)
                    && is_positional
                {
                    // Use the method that gathers radius information
                    let data_expressions_with_radius = self
                        .gather_scale_domain_expressions_with_radius(
                            name,
                            configured_non_positional,
                            context,
                        )?;

                    // Check if any expressions actually have radius
                    let has_radius = data_expressions_with_radius
                        .iter()
                        .any(|(_, _, radius)| radius.is_some());

                    if !data_expressions_with_radius.is_empty() && has_radius {
                        // Use the method that accepts radius
                        scale = scale.domain_data_fields_with_radius(data_expressions_with_radius);
                    } else if !data_expressions_with_radius.is_empty() {
                        // Convert to standard expressions (without radius)
                        let data_expressions: Vec<(
                            Arc<DataFrame>,
                            datafusion::logical_expr::Expr,
                        )> = data_expressions_with_radius
                            .into_iter()
                            .map(|(df, expr, _)| (df, expr))
                            .collect();
                        scale = scale.domain_data_fields(data_expressions);
                    }
                } else {
                    // Use standard domain gathering for non-linear scales
                    let data_expressions =
                        self.gather_scale_domain_expressions(name, &context.session_context)?;
                    if !data_expressions.is_empty() {
                        scale = scale.domain_data_fields(data_expressions);
                    }
                }
            } else {
                // No radius context - use standard domain gathering
                let data_expressions =
                    self.gather_scale_domain_expressions(name, &context.session_context)?;
                if !data_expressions.is_empty() {
                    scale = scale.domain_data_fields(data_expressions);
                }
            }
        }

        // Apply coordinate system defaults if this is a positional channel
        let base_name = strip_trailing_numbers(name);
        if let Some((min, max)) = self.coord_transform.default_range(
            base_name,
            context.plot_width as f64,
            context.plot_height as f64,
        ) {
            use datafusion::prelude::lit;
            scale = scale.range_interval(lit(min), lit(max));
        }

        // Step 3: Infer domain from data if needed (resolve DomainExprs)
        if scale
            .domain
            .as_ref()
            .map(|d| {
                matches!(
                    &d.default_domain,
                    crate::scales::ScaleDefaultDomain::DomainExprs(_)
                )
            })
            .unwrap_or(false)
        {
            scale = scale
                .infer_domain_from_data(
                    context.plot_width,
                    context.plot_height,
                    &context.session_context,
                    &context.params,
                )
                .await?;
        }

        // Step 4: Normalize domain (apply zero, nice, padding)
        scale = scale
            .normalize_domain(
                context.plot_width,
                context.plot_height,
                &context.session_context,
            )
            .await?;

        // Step 5: Apply mark-specific range if not a position channel AND no range is set
        // Position channels already have their ranges set
        let is_position = self
            .coord_transform
            .required_channels()
            .contains(&name.as_ref());

        // Check if scale already has a user-specified range
        // The default range is [0, 1], so check if it's been customized from that
        let has_user_range = match scale.get_range() {
            Some(crate::scales::ScaleRange::Color(_)) => true, // Custom color range
            Some(crate::scales::ScaleRange::Discrete(_)) => true, // Custom discrete values
            Some(crate::scales::ScaleRange::Numeric(start, end)) => {
                // Check if it's not the default [0, 1] range
                use datafusion::logical_expr::Expr;
                use datafusion_common::ScalarValue;
                let is_default = if let (Ok(start_expr), Ok(end_expr)) = (
                    start.to_expr(&context.session_context),
                    end.to_expr(&context.session_context),
                ) {
                    match (&start_expr, &end_expr) {
                        (Expr::Literal(v1, _), Expr::Literal(v2, _)) => match (v1, v2) {
                            (ScalarValue::Float64(Some(v1)), ScalarValue::Float64(Some(v2))) => {
                                (v1 - 0.0).abs() < 0.001 && (v2 - 1.0).abs() < 0.001
                            }
                            (ScalarValue::Float32(Some(v1)), ScalarValue::Float32(Some(v2))) => {
                                (*v1 as f64 - 0.0).abs() < 0.001 && (*v2 as f64 - 1.0).abs() < 0.001
                            }
                            _ => false,
                        },
                        _ => false,
                    }
                } else {
                    false
                };
                !is_default
            }
            None => false, // No range set, use default
        };

        if !is_position && !has_user_range {
            // Find the first mark that uses this channel
            for mark in &self.marks {
                if mark.data_context().channels().contains_key(name) {
                    // Get data type from the channel expression
                    // First resolve channel references
                    let channels = mark.data_context().channels();
                    let resolved_channels = crate::channel::resolution::resolve_all_channel_refs(
                        channels,
                        &context.session_context,
                    )
                    .ok()
                    .unwrap_or_else(|| channels.clone());

                    let data_type = resolved_channels
                        .get(name)
                        .and_then(|channel_value| channel_value.expr(&context.session_context))
                        .and_then(|expr| {
                            // Try to get data type from mark's dataframe using context from RenderContext
                            let mark_df = mark
                                .data_context()
                                .dataframe_with_context(&context.session_context);
                            let plot_df = self.data.as_ref().and_then(|node| {
                                node.to_logical_plan(&context.session_context)
                                    .ok()
                                    .map(|plan| {
                                        DataFrame::new(
                                            context.session_context.state().clone(),
                                            plan,
                                        )
                                    })
                            });
                            let df = mark_df.or(plot_df)?;
                            use datafusion::logical_expr::ExprSchemable;
                            expr.get_type(df.schema()).ok()
                        });

                    if let Some(dt) = data_type {
                        let theme = self.get_theme();
                        // Convert domain to ResolvedDomain if available
                        if let (Some(scale_impl), Some(domain)) =
                            (scale.get_scale_impl(), scale.get_domain())
                        {
                            let resolved_domain = domain.to_resolved()?;
                            if let Some(mark_range) = mark.default_channel_range(
                                name,
                                scale_impl.as_ref(),
                                &resolved_domain,
                                &dt,
                                theme.as_ref(),
                            ) {
                                scale = scale.range(mark_range);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Create the configured scale and wrap with the original spec
        let configured = scale
            .clone()
            .create_configured_scale(
                context.plot_width,
                context.plot_height,
                &context.session_context,
                &context.params,
            )
            .await?;

        Ok(ConfiguredScaleWithSpec::new(scale, configured))
    }

    /// Build initial scales with estimated dimensions
    pub async fn build_initial_scales(
        &self,
        context: &crate::render::RenderContext,
    ) -> Result<
        (
            HashMap<String, Scale>,
            HashMap<String, ConfiguredScaleWithSpec>,
            HashMap<String, ConfiguredScaleWithSpec>,
        ),
        AvengerChartError,
    > {
        // Collect all channels that need scales
        let mut channels_with_scales =
            self.collect_channels_needing_scales(&context.session_context);

        // Also include any channels with explicit scale specs
        for channel in self.scale_specs.keys() {
            channels_with_scales.insert(channel.clone());
        }

        // Build initial scale definitions
        let mut initial_scales = HashMap::new();
        for channel in &channels_with_scales {
            let scale = self.build_scale(
                channel,
                context.plot_width as f64,
                context.plot_height as f64,
                &context.session_context,
                &context.params,
            )?;
            initial_scales.insert(channel.clone(), scale);
        }

        // Separate scales into positional and non-positional
        let mut positional_scales = HashMap::new();
        let mut non_positional_scales = HashMap::new();

        // Get the positional channels from the coordinate system
        let required_channels: Vec<String> = self
            .coord_transform
            .required_channels()
            .iter()
            .map(|&s| s.to_string())
            .collect();

        for (name, scale) in &initial_scales {
            // Check if this is a positional channel or its interval variant
            let is_positional = required_channels
                .iter()
                .any(|ch| name == ch || name == &format!("{}2", ch));

            if is_positional {
                positional_scales.insert(name.clone(), scale.clone());
            } else {
                non_positional_scales.insert(name.clone(), scale.clone());
            }
        }

        // Build configured non-positional scales first (they don't depend on plot dimensions)
        let mut configured_non_positional = HashMap::new();
        for (name, scale) in non_positional_scales {
            let configured = self
                .build_configured_scale_with_radius_context(
                    scale, &name, context, None, // No radius context for non-positional scales
                )
                .await?;
            configured_non_positional.insert(name, configured);
        }

        // Build configured positional scales with estimated dimensions
        let mut configured_positional = HashMap::new();
        for (name, scale) in positional_scales {
            let configured = self
                .build_configured_scale_with_radius_context(
                    scale,
                    &name,
                    context,
                    Some(&configured_non_positional),
                )
                .await?;
            configured_positional.insert(name, configured);
        }

        Ok((
            initial_scales,
            configured_non_positional,
            configured_positional,
        ))
    }

    /// Rebuild positional scales with final dimensions after layout
    pub async fn rebuild_scales_with_final_dimensions(
        &self,
        initial_scales: &HashMap<String, Scale>,
        configured_non_positional: &HashMap<String, ConfiguredScaleWithSpec>,
        context: &crate::render::RenderContext,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let mut final_configured_scales = configured_non_positional.clone();

        // Get the positional channels from the coordinate system
        let required_channels: Vec<String> = self
            .coord_transform
            .required_channels()
            .iter()
            .map(|&s| s.to_string())
            .collect();

        // Rebuild positional scales with final dimensions
        for (name, scale) in initial_scales {
            // Check if this is a positional channel or its interval variant
            let is_positional = required_channels
                .iter()
                .any(|ch| name == ch || name == &format!("{}2", ch));

            if is_positional {
                let configured = self
                    .build_configured_scale_with_radius_context(
                        scale.clone(),
                        name,
                        context,
                        Some(configured_non_positional),
                    )
                    .await?;
                final_configured_scales.insert(name.clone(), configured);
            }
        }

        Ok(final_configured_scales)
    }

    // ===== Phase 4: Legend Channel Methods =====

    /// Build legend channels for a specific channel
    pub fn build_legend_channels_for_channel(
        &self,
        channel: &str,
        _legend: &Legend,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<crate::legend::LegendChannel>, AvengerChartError> {
        // Find the mark that has this channel
        let (mark_index, mark) = self
            .marks
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
            ctx,
            params,
        );
        legend_channels.push(primary_channel);

        // Note: We're not handling merged_channels from the legend config here
        // as that would require accessing legend.merged_channels which might not exist yet
        // This can be enhanced later if needed

        Ok(legend_channels)
    }

    /// Merge legend channels based on merge keys
    pub fn merge_legend_channels(
        &self,
        all_legends: &IndexMap<String, Legend>,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> (
        Vec<Vec<crate::legend::LegendChannel>>,
        IndexMap<String, Legend>,
    ) {
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

    /// Prepare legend measurements for layout computation
    /// Note: This should be called with the legends_map from merge_legend_channels
    /// to ensure measurements match rendering
    pub fn prepare_legend_measurements(
        &self,
        legends: &IndexMap<String, Legend>,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        available_space: taffy::Size<f32>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<crate::render::LegendMeasurements, AvengerChartError> {
        use crate::layout::legend::measure_legend_size_with_channels;

        let mut legend_measurements = crate::render::LegendMeasurements::new();

        // Get all legends including channel-level configs
        let all_legends = self.get_legends_with_theme(scales, ctx);

        // Merge channels to get the same groups that will be used for rendering
        let (sorted_channel_groups, _) =
            self.merge_legend_channels(&all_legends, scales, ctx, params);

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
                let (size, flexible) = measure_legend_size_with_channels(
                    &channels,
                    legend,
                    renderer,
                    available_space,
                )?;
                legend_measurements.insert(primary_channel.name.clone(), (size, flexible));
            }
        }

        Ok(legend_measurements)
    }

    // ===== Phase 5: Core Rendering Methods =====

    /// Render a single mark with its data and transformations
    pub async fn render_mark(
        &self,
        mark: &dyn CompiledMark,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get channel mappings from DataContext
        let channels = mark.data_context().channels();

        // Resolve channel references (e.g., ":x" -> actual x expression)
        let channels = crate::channel::resolution::resolve_all_channel_refs(channels, ctx)?;

        // Check if any channel expressions reference columns
        let references_columns = channels.values().any(|channel_value| match channel_value {
            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                // Convert to Expr to check column refs
                expr.to_expr(ctx)
                    .map(|e| !e.column_refs().is_empty())
                    .unwrap_or(false)
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                conditions.iter().any(|(condition, value)| {
                    let cond_has_refs = condition
                        .to_expr(ctx)
                        .map(|e| !e.column_refs().is_empty())
                        .unwrap_or(false);
                    let value_has_refs = value
                        .expr(ctx)
                        .map(|e| !e.column_refs().is_empty())
                        .unwrap_or(false);
                    cond_has_refs || value_has_refs
                }) || otherwise
                    .expr(ctx)
                    .map(|e| !e.column_refs().is_empty())
                    .unwrap_or(false)
            }
        });

        // Determine data source - convert from LogicalPlans to DataFrames using the context
        let df_ref = if let Some(mark_df) = mark.data_context().dataframe_with_context(ctx) {
            // Mark has explicit data
            Some(mark_df)
        } else if !references_columns {
            // No column references - use unit data
            None
        } else if let Some(plot_df) = self.data.as_ref().and_then(|node| {
            node.to_logical_plan(ctx)
                .ok()
                .map(|plan| DataFrame::new(ctx.state().clone(), plan))
        }) {
            // Inherit from plot
            Some(plot_df)
        } else {
            // No data available but columns are referenced
            return Err(AvengerChartError::InternalError(
                "Mark expressions reference columns but no data is available".to_string(),
            ));
        };

        // Check if mark has a sorting channel and apply sorting if needed
        let df = if let Some(df_ref) = df_ref {
            if let Some(sort_channel_name) = mark.sorting_channel() {
                if let Some(sort_channel) = channels.get(sort_channel_name) {
                    // Apply sorting transformation
                    let sort_expr =
                        self.apply_channel_scale(sort_channel_name, sort_channel, scales)?;

                    // Sort the DataFrame by the sorting expression
                    let sorted_df = df_ref.sort(vec![sort_expr.sort(true, false)])?;
                    Arc::new(sorted_df)
                } else {
                    Arc::new(df_ref)
                }
            } else {
                Arc::new(df_ref)
            }
        } else {
            // Unit data source - create minimal DataFrame with single row
            let empty_df = ctx
                .sql("SELECT 1 as _dummy")
                .await
                .map_err(|e| AvengerChartError::DataFusionError(e))?;
            Arc::new(empty_df)
        };

        // Get supported channels from the mark
        let supported_channels = mark.supported_channels();

        // Separate channels into those that need array data vs scalar data
        let mut array_channels = Vec::new();
        let mut scalar_channels = Vec::new();
        let mut has_array_data = false;

        for channel_desc in &supported_channels {
            if let Some(channel_value) = channels.get(channel_desc.name) {
                // Apply scaling to get the final expression
                let scaled_expr =
                    self.apply_channel_scale(channel_desc.name, channel_value, scales)?;

                // Check if this channel references columns (needs array data)
                if channel_desc.allow_column_ref && scaled_expr.any_column_refs() {
                    array_channels.push((channel_desc.name, scaled_expr));
                    has_array_data = true;
                } else {
                    scalar_channels.push((channel_desc.name, scaled_expr));
                }
            }
        }

        // Build array data batch if needed
        let data_batch = if has_array_data {
            let mut select_exprs = vec![];
            for (name, expr) in &array_channels {
                select_exprs.push(expr.clone().alias(*name));
            }

            let datafusion_params = crate::utils::params_to_datafusion(params);
            let batch = if let Some(param_values) = datafusion_params {
                (*df)
                    .clone()
                    .select(select_exprs)?
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                (*df).clone().select(select_exprs)?.collect().await?
            };

            if batch.is_empty() {
                None
            } else {
                Some(batch[0].clone())
            }
        } else {
            None
        };

        // Build scalar data batch
        let mut scalar_select_exprs = vec![];
        for (name, expr) in &scalar_channels {
            scalar_select_exprs.push(expr.clone().alias(*name));
        }

        let scalar_batch = if !scalar_select_exprs.is_empty() {
            let datafusion_params = crate::utils::params_to_datafusion(params);
            let batch = if let Some(param_values) = datafusion_params {
                (*df)
                    .clone()
                    .select(scalar_select_exprs)?
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                (*df).clone().select(scalar_select_exprs)?.collect().await?
            };
            if batch.is_empty() {
                // Create empty batch with correct schema
                return Ok(vec![]);
            } else {
                batch[0].clone()
            }
        } else {
            // Create an empty record batch
            use datafusion::arrow::array::Int32Array;
            use datafusion::arrow::datatypes::{DataType, Field, Schema};
            datafusion::arrow::record_batch::RecordBatch::try_new(
                Arc::new(Schema::new(vec![Field::new(
                    "_dummy",
                    DataType::Int32,
                    false,
                )])),
                vec![Arc::new(Int32Array::from(vec![0]))],
            )?
        };

        // Validate positional channel types
        self.validate_positional_channel_types(&data_batch, &scalar_batch)?;

        // Create render context with theme and dimensions
        let theme = self.get_theme();
        let context = crate::render::RenderContext::new(
            theme,
            plot_width,
            plot_height,
            Arc::new(ctx.clone()),
            params.clone(),
        );

        // Clone the coordinate transform
        let coord_transform = self.coord_transform.clone_box();

        // Render the mark with data
        mark.render_from_data(
            data_batch.as_ref(),
            &scalar_batch,
            &context,
            coord_transform,
        )
    }

    /// Create guide marks (axes, grids) for the coordinate system
    pub async fn create_guide_marks(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &crate::layout::LayoutBounds,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Use the pre-built guide renderer if available
        if let Some(compiled_guide) = &self.compiled_guide {
            let theme = self.get_theme();
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            compiled_guide
                .render(
                    &configured_scales,
                    plot_width,
                    plot_height,
                    plot_bounds,
                    theme.as_ref(),
                )
                .await
        } else {
            // No guide renderer available
            Ok(vec![])
        }
    }

    // ===== Phase 6: Layout and Legend Rendering =====

    /// Compute layout with overflow measurement
    pub async fn compute_layout(
        &self,
        width: f32,
        height: f32,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<crate::render::LayoutSolution, AvengerChartError> {
        use crate::layout::ChartLayout;
        const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

        // Check for required positional scales before measuring overflow
        self.validate_positional_scales_exist(scales)?;

        // Measure how much space the guide needs if we have one
        let overflow = if let Some(compiled_guide) = &self.compiled_guide {
            let width_estimate = width * INITIAL_PLOT_AREA_RATIO;
            let height_estimate = height * INITIAL_PLOT_AREA_RATIO;

            let theme = self.get_theme();
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            compiled_guide
                .measure_overflow(
                    &configured_scales,
                    width_estimate,
                    height_estimate,
                    theme.as_ref(),
                )
                .await?
        } else {
            // No guide renderer - no overflow
            crate::guide::OverflowSpaceRequirement::default()
        };

        // Get legends with theme applied
        let all_legends = self.get_legends_with_theme(scales, ctx);

        // Use the helper to merge legend channels
        let (_channel_groups, legends_map) =
            self.merge_legend_channels(&all_legends, scales, ctx, params);

        // Prepare legend measurements
        let available_size = taffy::Size {
            width: width * INITIAL_PLOT_AREA_RATIO,
            height: height * INITIAL_PLOT_AREA_RATIO,
        };
        let legend_measurements =
            self.prepare_legend_measurements(&legends_map, scales, available_size, ctx, params)?;

        // Create ChartLayout with overflow directly
        let layout_spec = self.get_layout_spec();
        let mut layout = ChartLayout::new_with_overflow(
            &overflow,
            &legends_map,
            layout_spec,
            self.get_title(),
            self.get_subtitle(),
            self.get_theme().as_ref(),
            &legend_measurements,
        )?;

        // Compute layout using the layout spec and return it directly
        layout.compute(layout_spec)
    }

    /// Create legends positioned according to layout
    pub fn create_legends_with_layout(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        layout: &crate::layout::LayoutResult,
        _plot_width: f32,
        _plot_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get legends with theme applied (same as used for layout)
        let all_legend_configs = self.get_legends_with_theme(scales, ctx);

        // Use the helper to merge legend channels
        let (sorted_channel_groups, _legends_map) =
            self.merge_legend_channels(&all_legend_configs, scales, ctx, params);

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
                    let mark_opt = self.marks.get(primary_channel.mark_index);
                    // Extract ConfiguredScale from ConfiguredScaleWithSpec for mark's renderer
                    let configured_scales: HashMap<String, ConfiguredScale> = scales
                        .iter()
                        .map(|(k, v)| (k.clone(), v.configured().clone()))
                        .collect();

                    mark_opt.and_then(|mark| {
                        mark.preferred_merged_legend_renderer(&channels, &configured_scales)
                    })
                } else {
                    // Single channel - use the unified renderer selection
                    scales.get(&primary_channel.name).and_then(|scale| {
                        self.get_legend_renderer(&primary_channel.channel_type, scale)
                    })
                };

                // Skip this legend group if no renderer is available
                if let Some(renderer) = renderer_opt {
                    // Render the legend with the determined renderer
                    let group_opt = renderer.render(
                        &channels,
                        legend,
                        bounds.x,
                        bounds.y,
                        bounds.width,
                        bounds.height,
                    )?;

                    // Add the legend group mark if it was rendered
                    if let Some(group) = group_opt {
                        legend_marks.push(SceneMark::Group(group));
                    }
                }
            }
        }

        Ok(legend_marks)
    }

    /// Render all components (marks, axes, legends, titles)
    async fn render_all_components(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        layout: &crate::render::LayoutSolution,
        _width: f32,
        _height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<
        (
            Vec<SceneMark>, // mark_groups
            Vec<SceneMark>, // axis_marks
            Vec<SceneMark>, // legend_marks
            Vec<SceneMark>, // title_marks
            Vec<SceneMark>, // subtitle_marks
        ),
        AvengerChartError,
    > {
        let plot_bounds = layout.plot_area_bounds();
        let plot_area_width = plot_bounds.width;
        let plot_area_height = plot_bounds.height;

        // Render marks
        let mut mark_groups = Vec::new();
        for mark in &self.marks {
            let scene_marks = self
                .render_mark(
                    mark.as_ref(),
                    scales,
                    plot_area_width,
                    plot_area_height,
                    ctx,
                    params,
                )
                .await?;
            mark_groups.extend(scene_marks);
        }

        // Create guide marks (axes, grids, backgrounds)
        let guide_marks = self
            .create_guide_marks(scales, plot_area_width, plot_area_height, plot_bounds)
            .await?;

        // Create legends
        let legend_marks = self.create_legends_with_layout(
            scales,
            &layout.taffy_layout,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
        )?;

        // Create title
        let title_marks = if let Some(title_bounds) = &layout.taffy_layout.title {
            self.create_title(Some(*title_bounds))?
        } else {
            Vec::new()
        };

        // Create subtitle
        let subtitle_marks = if let Some(subtitle_bounds) = &layout.taffy_layout.subtitle {
            self.create_subtitle(Some(*subtitle_bounds))?
        } else {
            Vec::new()
        };

        Ok((
            mark_groups,
            guide_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
        ))
    }

    /// Render the plot to a scene graph
    pub async fn render(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, datafusion::common::ScalarValue>>,
    ) -> Result<crate::render::RenderResult, AvengerChartError> {
        use crate::render::RenderContext;
        use avenger_scenegraph::marks::group::SceneGroup;
        use avenger_scenegraph::scene_graph::SceneGraph;
        const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

        // Get layout spec and estimate initial dimensions
        let layout_spec = &self.layout_spec;

        // For now, we'll use a simple estimation for canvas size
        // This will be refined after we compute the actual layout
        let (estimated_width, estimated_height) = match &layout_spec.canvas {
            crate::layout::SizeMode::Fixed { width, height } => (*width, *height),
            _ => (400.0, 300.0), // Default for Auto or other modes
        };

        // Use estimated dimensions for initial scale construction
        let estimated_plot_width = estimated_width * INITIAL_PLOT_AREA_RATIO;
        let estimated_plot_height = estimated_height * INITIAL_PLOT_AREA_RATIO;

        // Merge provided params with default params
        let merged_params = if let Some(provided) = params {
            let mut merged = self.default_params.clone();
            merged.extend(provided);
            merged
        } else {
            self.default_params.clone()
        };

        // Create initial RenderContext with estimated dimensions and SessionContext
        let theme = self.get_theme();
        let initial_context = RenderContext::new(
            theme.clone(),
            estimated_plot_width,
            estimated_plot_height,
            Arc::new(ctx.clone()),
            merged_params.clone(),
        );

        let (initial_scales, configured_non_positional, configured_positional) =
            self.build_initial_scales(&initial_context).await?;

        // Merge configured scales for layout computation
        let mut initial_configured_scales = configured_non_positional.clone();
        initial_configured_scales.extend(configured_positional.clone());

        // STAGE 2: COMPUTE LAYOUT USING INITIAL SCALES
        let layout = self
            .compute_layout(
                estimated_width,
                estimated_height,
                &initial_configured_scales,
                ctx,
                &merged_params,
            )
            .await?;
        let plot_bounds = layout.plot_area_bounds();
        let (final_width, final_height) = layout.canvas_size;
        let plot_area_x = plot_bounds.x;
        let plot_area_y = plot_bounds.y;
        let plot_area_width = plot_bounds.width;
        let plot_area_height = plot_bounds.height;

        // STAGE 3: REBUILD POSITIONAL SCALES WITH FINAL DIMENSIONS
        // Create final RenderContext with actual plot dimensions
        let final_context = RenderContext::new(
            theme.clone(),
            plot_area_width,
            plot_area_height,
            Arc::new(ctx.clone()),
            merged_params.clone(),
        );

        let final_configured_scales = self
            .rebuild_scales_with_final_dimensions(
                &initial_scales,
                &configured_non_positional,
                &final_context,
            )
            .await?;

        // STAGE 4: RENDER ALL COMPONENTS WITH FINAL SCALES
        let all_component_marks = self
            .render_all_components(
                &final_configured_scales,
                &layout,
                final_width,
                final_height,
                ctx,
                &merged_params,
            )
            .await?;

        let (mark_groups, guide_marks, legend_marks, title_marks, subtitle_marks) =
            all_component_marks;

        // Compose all elements into a scene graph
        // A single Plot should produce a single top-level group
        let mut all_marks = Vec::new();

        // Get the appropriate clipping region from the coordinate system
        // Get the appropriate clipping region from the guide renderer if available
        let clip = if let Some(ref guide) = self.compiled_guide {
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = final_configured_scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            guide.get_clip(plot_area_width, plot_area_height, &configured_scales)
        } else {
            // Default to rectangular clip for plot area
            avenger_scenegraph::marks::group::Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: plot_area_width,
                height: plot_area_height,
            }
        };

        let data_marks_group = SceneGroup {
            origin: [plot_area_x, plot_area_y],
            marks: mark_groups,
            clip,
            zindex: Some(0), // Data marks have lowest z-index
            ..Default::default()
        };

        // Add background rect if theme specifies one
        if let Some(bg_color) = theme.canvas_background() {
            use avenger_common::types::ColorOrGradient;
            use avenger_scenegraph::marks::rect::SceneRectMark;

            // Parse the color string to RGBA - fail if color is invalid
            let color = crate::utils::parse_color_to_array_strict(&bg_color)?;

            let background_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(final_width.into()),
                height: Some(final_height.into()),
                fill: ColorOrGradient::Color(color).into(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(), // No stroke
                stroke_width: 0.0.into(),
                zindex: Some(-100), // Ensure it's behind everything
                ..Default::default()
            };
            all_marks.push(SceneMark::Rect(background_rect));
        }

        // Add marks in proper z-order:
        // 1. Clipped data marks (background)
        all_marks.push(SceneMark::Group(data_marks_group));

        // 2. Guide marks (axes, grids, backgrounds - can overflow the plot area)
        all_marks.extend(guide_marks);

        // 3. Legends (positioned outside plot area)
        all_marks.extend(legend_marks);

        // 4. Title (can overflow, rendered on top)
        all_marks.extend(title_marks);

        // 5. Subtitle (can overflow, rendered on top)
        all_marks.extend(subtitle_marks);

        // 6. Debug: Add layout bounds visualization if AVENGER_CHART_DEBUG_LAYOUT is set
        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            all_marks.extend(crate::render::debug::create_debug_layout_rects(
                &layout.taffy_layout,
            ));
        }

        // Wrap everything in a single root group
        let root_group = SceneGroup {
            marks: all_marks,
            ..Default::default()
        };

        // Use the computed canvas size from layout
        let scene_graph = SceneGraph {
            marks: vec![SceneMark::Group(root_group)],
            width: final_width,
            height: final_height,
            origin: [0.0, 0.0],
        };

        // Build spatial index for hit testing
        let rtree = avenger_geometry::rtree::SceneGraphRTree::from_scene_graph(&scene_graph);

        Ok(crate::render::RenderResult {
            scene_graph,
            rtree: Some(rtree),
        })
    }
}
