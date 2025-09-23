//! Scale building and configuration
//!
//! This module handles all scale-related functionality including:
//! - Building initial scales with estimated dimensions
//! - Rebuilding scales with final dimensions
//! - Scale validation and error handling
//! - Configured scale creation with radius context

use super::PlotRenderer;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::marks::ChannelValue;
use crate::render::RenderContext;
use crate::scales::Scale;
use datafusion::prelude::DataFrame;
use std::collections::HashMap;
use std::sync::Arc;

impl<C: CoordinateSystem> PlotRenderer<'_, C> {
    /// Build initial scales with estimated dimensions
    /// Returns (raw_scales, configured_non_positional, configured_positional)
    pub(super) async fn build_initial_scales(
        &self,
        context: &RenderContext,
    ) -> Result<
        (
            HashMap<String, Scale>,
            HashMap<String, avenger_scales::scales::ConfiguredScale>,
            HashMap<String, avenger_scales::scales::ConfiguredScale>,
        ),
        AvengerChartError,
    > {
        // Collect all channels that need scales
        let mut channels_with_scales = self.plot.collect_channels_needing_scales();

        // Also include any channels with explicit scale specs
        // (even if they only have literal values, we need the scale for layout)
        for channel in self.plot.scale_specs.keys() {
            channels_with_scales.insert(channel.clone());
        }

        // Build initial scale definitions
        let mut initial_scales = HashMap::new();
        for channel in &channels_with_scales {
            let scale = self.plot.get_scale(channel)?;
            initial_scales.insert(channel.clone(), scale);
        }

        // Separate scales into positional and non-positional
        let mut positional_scales = HashMap::new();
        let mut non_positional_scales = HashMap::new();

        // Get the positional channels from the coordinate system
        let required_channels: Vec<String> = self
            .plot
            .coord_system()
            .required_channels()
            .iter()
            .map(|&s| s.to_string())
            .collect();

        for (name, scale) in &initial_scales {
            if required_channels.contains(name) {
                positional_scales.insert(name.clone(), scale.clone());
            } else {
                non_positional_scales.insert(name.clone(), scale.clone());
            }
        }

        // Build non-positional scales first (no radius context needed)
        let mut configured_non_positional = HashMap::new();
        for (name, scale) in &non_positional_scales {
            let configured = self
                .build_configured_scale_with_radius_context(scale.clone(), name, context, None)
                .await?;
            configured_non_positional.insert(name.clone(), configured);
        }

        // Build positional scales with radius context
        let mut configured_positional = HashMap::new();
        for (name, scale) in &positional_scales {
            let configured = self
                .build_configured_scale_with_radius_context(
                    scale.clone(),
                    name,
                    context,
                    Some(&configured_non_positional),
                )
                .await?;
            configured_positional.insert(name.clone(), configured);
        }

        Ok((
            initial_scales,
            configured_non_positional,
            configured_positional,
        ))
    }

    /// Validate that required positional scales exist and provide proper error messages
    pub(super) fn validate_positional_scales_exist(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Result<(), AvengerChartError> {
        // Get the required positional channels from the coordinate system
        let required_channels = self.plot.coord_system().required_channels();

        // Build a set of positional channel names to check
        // Include both base channels and their interval variants (e.g., "x" and "x2")
        let mut positional_channels = std::collections::HashSet::new();
        for &channel in required_channels {
            positional_channels.insert(channel.to_string());
            // Also check for interval variant (e.g., "x2" for "x")
            positional_channels.insert(format!("{}2", channel));
        }

        // Check if positional channels are being used with literal values
        for mark in &self.plot.marks {
            for (channel_name, channel_value) in mark.data_context().channels() {
                // Check if this is a positional channel
                if positional_channels.contains(channel_name) {
                    // Check if this is a literal value (no scale needed)
                    if channel_value.get_scale_name(channel_name).is_none() {
                        // Get the base scale name (e.g., "x" from "x2")
                        let base_scale_name = channel_name.trim_end_matches('2');

                        // Check if we have an explicit scale spec for this channel or its base
                        if !self.plot.scale_specs.contains_key(channel_name)
                            && !self.plot.scale_specs.contains_key(base_scale_name)
                        {
                            // This is a literal value with no explicit scale - error
                            return self.create_positional_literal_error(
                                channel_name,
                                channel_value,
                                std::any::type_name::<C>(),
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Create error for literal values in positional scales
    fn create_positional_literal_error(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        coord_system_name: &str,
    ) -> Result<(), AvengerChartError> {
        use datafusion::logical_expr::Expr as DfExpr;

        // Extract the literal value description
        let literal_value = match channel_value {
            ChannelValue::Value { expr } => {
                // Check if the expression is a literal
                match expr {
                    DfExpr::Literal(scalar_value, _) => match scalar_value {
                        datafusion_common::ScalarValue::Utf8(Some(_))
                        | datafusion_common::ScalarValue::LargeUtf8(Some(_)) => {
                            "string literal".to_string()
                        }
                        datafusion_common::ScalarValue::Float32(Some(_))
                        | datafusion_common::ScalarValue::Float64(Some(_))
                        | datafusion_common::ScalarValue::Int32(Some(_))
                        | datafusion_common::ScalarValue::Int64(Some(_))
                        | datafusion_common::ScalarValue::Int8(Some(_))
                        | datafusion_common::ScalarValue::Int16(Some(_))
                        | datafusion_common::ScalarValue::UInt8(Some(_))
                        | datafusion_common::ScalarValue::UInt16(Some(_))
                        | datafusion_common::ScalarValue::UInt32(Some(_))
                        | datafusion_common::ScalarValue::UInt64(Some(_)) => {
                            "numeric literal".to_string()
                        }
                        _ => "literal value".to_string(),
                    },
                    _ => "expression".to_string(),
                }
            }
            _ => "literal value".to_string(),
        };

        // Create helpful suggestion based on the literal type
        let suggestion = if literal_value.contains("string") {
            "Did you mean to reference a column? Use col(\"column_name\") to reference a column."
                .to_string()
        } else {
            "To use a literal value, provide an explicit domain using .scale_x() or .scale_y().\n\
             Or use col(\"column_name\") to reference a data column."
                .to_string()
        };

        // Extract coordinate system name (remove module path)
        let coord_system = coord_system_name
            .split("::")
            .last()
            .unwrap_or(coord_system_name);

        Err(AvengerChartError::PositionalScaleLiteralError {
            scale_name: channel_name.to_string(),
            coord_system: coord_system.to_string(),
            literal_value,
            suggestion,
        })
    }

    /// Rebuild scales with final dimensions from layout
    pub(super) async fn rebuild_scales_with_final_dimensions(
        &self,
        initial_scales: &HashMap<String, Scale>,
        configured_non_positional: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        context: &RenderContext,
    ) -> Result<HashMap<String, avenger_scales::scales::ConfiguredScale>, AvengerChartError> {
        let mut final_configured_scales = HashMap::new();

        // Pass through non-positional scales unchanged
        for (name, configured_scale) in configured_non_positional {
            final_configured_scales.insert(name.clone(), configured_scale.clone());
        }

        // Rebuild positional scales with final dimensions
        for (name, scale) in initial_scales {
            // Check if this is a positional scale for the current coordinate system
            let is_positional = self
                .plot
                .coord_system()
                .required_channels()
                .iter()
                .any(|&ch| ch == name);

            if is_positional {
                let rebuilt_scale = self
                    .build_configured_scale_with_radius_context(
                        scale.clone(),
                        name,
                        context,
                        Some(configured_non_positional),
                    )
                    .await?;
                final_configured_scales.insert(name.clone(), rebuilt_scale);
            }
        }

        Ok(final_configured_scales)
    }

    /// Build a ConfiguredScale directly, handling domain processing with radius context
    /// This combines the functionality of process_scale_domain_with_radius and build_scale_with_context
    pub(super) async fn build_configured_scale_with_radius_context(
        &self,
        scale: Scale,
        name: &str,
        context: &RenderContext,
        configured_non_positional: Option<
            &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        >,
    ) -> Result<avenger_scales::scales::ConfiguredScale, AvengerChartError> {
        let mut scale = scale;

        // Step 1: Process domain with radius if applicable
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
                    .plot
                    .coord_system()
                    .required_channels()
                    .iter()
                    .any(|&ch| name == ch || name == format!("{}2", ch));

                if scale
                    .get_scale_impl()
                    .map(|impl_| impl_.supports_radius_expansion())
                    .unwrap_or(false)
                    && is_positional
                {
                    // Use the method that gathers radius information
                    let data_expressions_with_radius =
                        self.plot.gather_scale_domain_expressions_with_radius(
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
                    let data_expressions = self.plot.gather_scale_domain_expressions(name)?;
                    if !data_expressions.is_empty() {
                        scale = scale.domain_data_fields(data_expressions);
                    }
                }
            } else {
                // No radius context - use standard domain gathering
                let data_expressions = self.plot.gather_scale_domain_expressions(name)?;
                if !data_expressions.is_empty() {
                    scale = scale.domain_data_fields(data_expressions);
                }
            }
        }

        // Step 2: Apply default range if it's a coordinate channel
        if let Some((start, end)) = self.plot.get_coordinate_default_range(
            name,
            context.plot_width as f64,
            context.plot_height as f64,
        ) {
            scale = scale.range_interval(
                datafusion::logical_expr::lit(start),
                datafusion::logical_expr::lit(end),
            );
        }

        // Step 3: Infer domain from data if needed
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
            // Infer domain from data
            scale = scale
                .infer_domain_from_data(context.plot_width, context.plot_height)
                .await?;
        }

        // Step 4: Normalize domain (apply zero, nice, padding)
        scale = scale
            .normalize_domain(context.plot_width, context.plot_height)
            .await?;

        // Step 5: Apply mark-specific range if not a position channel AND no range is set
        // Position channels already have their ranges set in Step 2
        let is_position = self
            .plot
            .coord_system()
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
                let is_default = match (start, end.as_ref()) {
                    (
                        Expr::Literal(ScalarValue::Float64(Some(v1)), _),
                        Expr::Literal(ScalarValue::Float64(Some(v2)), _),
                    ) => (*v1 - 0.0).abs() < 0.001 && (*v2 - 1.0).abs() < 0.001,
                    (
                        Expr::Literal(ScalarValue::Float32(Some(v1)), _),
                        Expr::Literal(ScalarValue::Float32(Some(v2)), _),
                    ) => (*v1 as f64 - 0.0).abs() < 0.001 && (*v2 as f64 - 1.0).abs() < 0.001,
                    _ => false,
                };
                !is_default
            }
            None => false, // No range set, use default
        };

        if !is_position && !has_user_range {
            // Find the first mark that uses this channel
            for mark in &self.plot.marks {
                if mark.data_context().channels().contains_key(name) {
                    // Get data type from the channel expression
                    // First resolve channel references
                    let channels = mark.data_context().channels();
                    let resolved_channels =
                        crate::channel::resolution::resolve_all_channel_refs(channels)
                            .ok()
                            .unwrap_or_else(|| channels.clone());

                    let data_type = resolved_channels
                        .get(name)
                        .and_then(|channel_value| channel_value.expr())
                        .and_then(|expr| {
                            // Try to get data type from mark's dataframe
                            let df = mark
                                .data_context()
                                .dataframe()
                                .or(self.plot.data.as_ref())?;
                            use datafusion::logical_expr::ExprSchemable;
                            expr.get_type(df.schema()).ok()
                        });

                    if let Some(dt) = data_type {
                        let theme = self.plot.get_theme();
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

        // Step 6: Create ConfiguredScale
        scale
            .create_configured_scale(context.plot_width, context.plot_height)
            .await
    }
}
