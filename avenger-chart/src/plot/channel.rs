//! Channel resolution and gathering methods for Plot

use crate::axis::AxisUpdate;
use crate::channel::ConditionalValue;
use crate::channel::resolution::resolve_all_channel_refs;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::{ChannelValue, Mark, RadiusExpression};
use crate::plot::{AxisSpec, Plot, ScaleSpec};
use crate::render::RenderContext;
use crate::scales::{ConfiguredScaleDataFusionExt, Scale, create_default_scale_for_channel};
use avenger_scales::scales::ConfiguredScale;
use datafusion::dataframe::DataFrame;
use indexmap::IndexMap;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

impl<C: CoordinateSystem> Plot<C> {
    /// Extract scale, legend, and axis configurations from a mark's channels
    pub(crate) fn extract_channel_configs(&mut self, mark: &impl Mark<C>) {
        // Extract axis configurations from the mark
        for (channel_name, axis_config) in mark.state().axis_configs.iter() {
            match self.axis_specs.entry(channel_name.clone()) {
                Entry::Occupied(mut occupied) => {
                    // Update existing axis with new configuration
                    let AxisSpec::Local(existing) = occupied.get();
                    let updated = existing.clone().update(axis_config.clone());
                    occupied.insert(AxisSpec::Local(updated));
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(AxisSpec::Local(axis_config.clone()));
                }
            }
        }

        // Get all channel encodings from the mark
        let encodings = mark.data_context().channels();

        // Try to resolve channel references, but if it fails (e.g., due to conditional references),
        // we still want to extract configs from non-reference channels
        let resolved_encodings = match resolve_all_channel_refs(encodings) {
            Ok(resolved) => resolved,
            Err(_) => {
                // Resolution failed (probably due to conditional references)
                // Use original encodings - we'll handle the error later during rendering
                encodings.clone()
            }
        };

        for (channel_name, channel_value) in resolved_encodings {
            // Extract scale and legend configs
            let (scale_config, legend_config) = match channel_value.clone() {
                ChannelValue::Scaled {
                    scale_config,
                    legend_config,
                    ..
                }
                | ChannelValue::Conditional {
                    scale_config,
                    legend_config,
                    ..
                } => (scale_config, legend_config),
                ChannelValue::Value { .. } => {
                    // No scale or legend for identity mappings
                    continue;
                }
            };

            // Determine scale name
            let scale_key = match channel_value {
                ChannelValue::Scaled { scale_name, .. } => {
                    scale_name.as_deref().unwrap_or(&channel_name).to_string()
                }
                ChannelValue::Conditional { .. } => {
                    // Conditional always uses channel name
                    channel_name.to_string()
                }
                _ => unreachable!(),
            };

            // Extract scale config if present
            if let Some(config) = scale_config {
                match self.scale_specs.entry(scale_key.clone()) {
                    Entry::Occupied(mut occupied) => {
                        let existing_spec = occupied.get().clone();
                        match existing_spec {
                            ScaleSpec::Local(existing_scale) => {
                                // Compose the two scale configurations using update()
                                // Apply existing config first, then the new config
                                let updated_scale = existing_scale.update(config);
                                occupied.insert(ScaleSpec::Local(updated_scale));
                            }
                        }
                    }
                    Entry::Vacant(vacant) => {
                        vacant.insert(ScaleSpec::Local(config));
                    }
                }
            }

            // Extract legend config if present
            if let Some(config) = legend_config {
                // Compose legend configurations - apply all configs in order
                let existing_legend = self.legends.shift_remove(channel_name.as_str());
                let configured = if let Some(existing) = existing_legend {
                    // Apply new config on top of existing configured legend
                    existing.update(config.clone())
                } else {
                    // Use the config as-is
                    config.clone()
                };
                self.legends.insert(channel_name.clone(), configured);
            }
        }
    }

    /// Create a channel resolver function for non-positional channels
    /// This is used primarily for radius calculations that need size/stroke_width expressions
    pub(crate) fn create_channel_resolver<'a>(
        mark: &'a dyn Mark<C>,
        encodings: &'a IndexMap<String, ChannelValue>,
        configured_scales: &'a HashMap<String, ConfiguredScale>,
        context: &'a RenderContext,
    ) -> impl Fn(&str) -> datafusion::logical_expr::Expr + 'a {
        use crate::channel::value::strip_trailing_numbers;
        use datafusion::prelude::lit;

        move |channel_name: &str| -> datafusion::logical_expr::Expr {
            // First check explicit mapping
            if let Some(channel_value) = encodings.get(channel_name) {
                // Apply scaling if needed
                match channel_value {
                    ChannelValue::Value { expr } => {
                        // No scaling requested
                        expr.clone()
                    }
                    ChannelValue::Scaled {
                        expr,
                        scale_name: custom_scale_name,
                        band,
                        ..
                    } => {
                        // Determine scale name
                        let scale_key = custom_scale_name
                            .as_ref()
                            .cloned()
                            .unwrap_or_else(|| strip_trailing_numbers(channel_name).to_string());

                        // Use configured scales (for non-positional channels like size, stroke_width)
                        if let Some(configured) = configured_scales.get(&scale_key) {
                            // Use ConfiguredScale.to_expr()
                            if let Some(band_value) = band {
                                configured
                                    .to_expr_with_band(expr.clone(), *band_value)
                                    .unwrap_or_else(|_| expr.clone())
                            } else {
                                configured
                                    .to_expr(expr.clone())
                                    .unwrap_or_else(|_| expr.clone())
                            }
                        } else {
                            // No scale configured for this channel - use raw expression
                            expr.clone()
                        }
                    }
                    ChannelValue::Conditional { .. } => {
                        // Conditional values are not supported for radius calculations
                        lit(datafusion::scalar::ScalarValue::Null)
                    }
                }
            } else if let Some(default_scalar) = mark.default_channel_value(channel_name, context) {
                // Use mark-provided default
                lit(default_scalar)
            } else {
                // No mapping and no default
                lit(datafusion::scalar::ScalarValue::Null)
            }
        }
    }

    /// Internal helper to create a default scale for a channel
    pub(crate) fn create_default_scale_for_channel_internal(
        &self,
        channel: &str,
    ) -> Result<Scale, AvengerChartError> {
        // Try to infer the data type and use mark-based scale preferences
        let mut scale_impl = None;
        let mut data_type = None;
        let mut found_mark = None;

        // Look through marks to find the expression for this channel
        for mark in &self.marks {
            let channels = mark.data_context().channels();

            // Try to resolve channel references first
            let resolved_channels = match resolve_all_channel_refs(channels) {
                Ok(resolved) => resolved,
                Err(e) => {
                    // If resolution failed (e.g., due to cycles), return an error
                    if channels.contains_key(channel) {
                        return Err(AvengerChartError::InternalError(format!(
                            "Cannot create scale for channel '{}': {}",
                            channel, e
                        )));
                    }
                    // Channel doesn't exist in this mark, continue to next
                    continue;
                }
            };

            if let Some(channel_value) = resolved_channels.get(channel) {
                // Get the dataframe for this mark
                // Use mark's explicit data if available, otherwise inherit from plot
                let df = mark.data_context().dataframe().or(self.data.as_ref());

                // Try to get the data type of the channel
                if let Some(df) = df {
                    let schema = df.schema();
                    if let Some(dt) = channel_value.get_data_type(schema) {
                        data_type = Some(dt.clone());
                        scale_impl = mark.preferred_scale_type(channel, &dt);
                        found_mark = Some(mark);
                        break;
                    }
                }
            }
        }

        let scale_impl = scale_impl.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Failed to infer scale implementation for channel '{}'",
                channel
            ))
        })?;

        // Create render context with theme for scale defaults
        // Use placeholder dimensions since we're not rendering yet
        let theme = self.get_theme();
        let context = RenderContext::new(theme, 0.0, 0.0);

        // Create scale with theme-based defaults
        let mut scale = create_default_scale_for_channel(channel, scale_impl.clone(), &context)?;

        // Apply coordinate system and mark-specific scale options
        // These override theme defaults
        if let (Some(dt), Some(mark)) = (&data_type, found_mark) {
            let mut default_options = HashMap::new();

            // First get coordinate system defaults (for all channels, not just position)
            // Pass the scale implementation directly
            let coord_options = self
                .coord_system()
                .default_scale_options(channel, scale_impl.as_ref());
            default_options.extend(coord_options);

            // Then get mark-specific scale option preferences
            // We already know this mark has the channel since we found it above
            let mark_options = mark.default_scale_options(channel, scale_impl.as_ref(), dt);
            // Mark preferences override coordinate system defaults
            default_options.extend(mark_options);

            // Get the supported options for this scale type
            let option_definitions = scale_impl.option_definitions();
            let supported_options: std::collections::HashSet<&str> = option_definitions
                .iter()
                .map(|def| def.name.as_str())
                .collect();

            // Only apply options that are supported by this scale
            for (key, value) in default_options {
                if supported_options.contains(key.as_str()) {
                    scale = scale.option(&key, value);
                }
            }
        }

        Ok(scale)
    }

    /// Gather mark data and encoding expressions that use this scale
    pub fn gather_scale_domain_expressions(
        &self,
        scale_name: &str,
    ) -> Result<Vec<(Arc<DataFrame>, datafusion::logical_expr::Expr)>, AvengerChartError> {
        let mut data_expressions = Vec::new();

        for mark in &self.marks {
            // Get channels and resolve references first to check if columns are referenced
            let channels = mark.data_context().channels();
            let resolved_channels = resolve_all_channel_refs(channels)?;

            // Check if any expressions reference columns
            let references_columns =
                resolved_channels
                    .values()
                    .any(|channel_value| match channel_value {
                        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                            !expr.column_refs().is_empty()
                        }
                        ChannelValue::Conditional {
                            conditions,
                            otherwise,
                            ..
                        } => {
                            conditions.iter().any(|(condition, value)| {
                                !condition.column_refs().is_empty()
                                    || !value.expr().column_refs().is_empty()
                            }) || !otherwise.expr().column_refs().is_empty()
                        }
                    });

            // Determine DataFrame for this mark
            let df = if let Some(mark_df) = mark.data_context().dataframe() {
                // Mark has explicit data
                Arc::new(mark_df.clone())
            } else if !references_columns {
                // No column references - skip domain inference for unit marks
                continue;
            } else if let Some(plot_data) = &self.data {
                // Inherit from plot
                Arc::new(plot_data.clone())
            } else {
                // No data available - skip this mark
                continue;
            };

            // Already resolved channels above

            // Check all encodings in the mark's data context
            for (channel, channel_value) in &resolved_channels {
                // Check if this channel uses our scale
                // Get the scale name this channel would use
                if let Some(channel_scale_name) = channel_value.get_scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get the expressions - handle conditional values properly
                        match channel_value {
                            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                                data_expressions.push((df.clone(), expr.clone()));
                            }
                            ChannelValue::Conditional {
                                conditions,
                                otherwise,
                                ..
                            } => {
                                // For conditional values, we need to extract field expressions
                                // (not literal values) for domain inference

                                // Add field expressions from conditions
                                for (_, value) in conditions {
                                    if let ConditionalValue::Scaled { expr } = value {
                                        data_expressions.push((df.clone(), expr.clone()));
                                    }
                                    // Skip ConditionalValue::Value as those are literals
                                }

                                // Add field expression from otherwise branch if it's a field
                                if let ConditionalValue::Scaled { expr } = otherwise {
                                    data_expressions.push((df.clone(), expr.clone()));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(data_expressions)
    }

    /// Gather mark data and encoding expressions with radius information for positional scales
    /// Uses configured scales for non-positional channels (size, stroke_width) needed for radius
    pub fn gather_scale_domain_expressions_with_radius(
        &self,
        scale_name: &str,
        configured_scales: &HashMap<String, ConfiguredScale>,
        context: &RenderContext,
    ) -> Result<ScaleDomainWithRadius, AvengerChartError> {
        let mut data_expressions = Vec::new();

        // Only gather radius for Cartesian x/y scales (including x2, y2 which map to x, y scales)
        // Polar coordinates (r, theta) don't use radius-based padding
        let needs_radius = matches!(scale_name, "x" | "y");
        if !needs_radius {
            // For non-positional scales, return without radius
            for (df, expr) in self.gather_scale_domain_expressions(scale_name)? {
                data_expressions.push((df, expr, None));
            }
            return Ok(data_expressions);
        }

        for mark in &self.marks {
            // Get channels and resolve references first
            let encodings = mark.data_context().channels();
            let resolved_encodings =
                crate::channel::resolution::resolve_all_channel_refs(encodings)?;

            // Check if any expressions reference columns
            let references_columns =
                resolved_encodings
                    .values()
                    .any(|channel_value| match channel_value {
                        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                            !expr.column_refs().is_empty()
                        }
                        ChannelValue::Conditional {
                            conditions,
                            otherwise,
                            ..
                        } => {
                            conditions.iter().any(|(condition, value)| {
                                !condition.column_refs().is_empty()
                                    || !value.expr().column_refs().is_empty()
                            }) || !otherwise.expr().column_refs().is_empty()
                        }
                    });

            // Determine DataFrame for this mark
            let df = if let Some(mark_df) = mark.data_context().dataframe() {
                // Mark has explicit data
                Arc::new(mark_df.clone())
            } else if !references_columns {
                // No column references - skip domain inference for unit marks
                continue;
            } else if let Some(plot_data) = &self.data {
                // Inherit from plot
                Arc::new(plot_data.clone())
            } else {
                // No data available - skip this mark
                continue;
            };

            // Create channel resolver for this mark (uses configured non-positional scales)
            let resolve_channel = Self::create_channel_resolver(
                mark.as_ref(),
                &resolved_encodings,
                configured_scales,
                context,
            );

            for (channel, position_channel_value) in &resolved_encodings {
                // Check if this channel uses our scale
                if let Some(channel_scale_name) = position_channel_value.get_scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get the position expressions - handle conditional values properly
                        match position_channel_value {
                            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                                // Get radius expression from the mark
                                let radius_expr =
                                    mark.radius_expression(scale_name, &resolve_channel);
                                data_expressions.push((df.clone(), expr.clone(), radius_expr));
                            }
                            ChannelValue::Conditional {
                                conditions,
                                otherwise,
                                ..
                            } => {
                                // For conditional values, we need to extract field expressions

                                // For radius expressions, we use the mark's radius for all branches
                                // since radius doesn't vary by condition
                                let radius_expr =
                                    mark.radius_expression(scale_name, &resolve_channel);

                                // Add field expressions from conditions
                                for (_, value) in conditions {
                                    if let ConditionalValue::Scaled { expr } = value {
                                        data_expressions.push((
                                            df.clone(),
                                            expr.clone(),
                                            radius_expr.clone(),
                                        ));
                                    }
                                }

                                // Add field expression from otherwise branch if it's a field
                                if let ConditionalValue::Scaled { expr } = otherwise {
                                    data_expressions.push((df.clone(), expr.clone(), radius_expr));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(data_expressions)
    }

    /// Collect all channels that need scales
    pub fn collect_channels_needing_scales(&self) -> HashSet<String> {
        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            // Get channels and resolve references first
            let encodings = mark.data_context().channels();
            // Try to resolve, but use original channels if resolution fails
            let resolved_encodings =
                resolve_all_channel_refs(encodings).unwrap_or_else(|_| encodings.clone());
            for (channel_name, channel_value) in resolved_encodings {
                if channel_value.get_scale_name(&channel_name).is_some() {
                    used_channels.insert(channel_name.clone());
                }
            }
        }
        used_channels
    }
}

/// Type alias for scale domain expressions with optional radius information
pub(crate) type ScaleDomainWithRadius = Vec<(
    Arc<DataFrame>,
    datafusion::logical_expr::Expr,
    Option<RadiusExpression>,
)>;
