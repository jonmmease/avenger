//! Scale building and domain inference for CompiledPlot

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use crate::channel::value::strip_trailing_numbers;
use crate::error::AvengerChartError;
use crate::plot::ScaleSpec;
use crate::scales::{ConfiguredScaleWithSpec, Scale};
use crate::serialization::{LogicalExprNodeExt, LogicalPlanNodeExt};

use super::CompiledPlot;

impl CompiledPlot {
    /// Collect all channels that need scales from marks
    pub(crate) fn collect_channels_needing_scales(&self, ctx: &SessionContext) -> HashSet<String> {
        use crate::channel::resolution::resolve_all_channel_refs;

        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            // Get channels and resolve references first
            let encodings = mark.data_context().channels();
            // Try to resolve, but use original channels if resolution fails
            let resolved_encodings =
                resolve_all_channel_refs(encodings, ctx).unwrap_or_else(|_| encodings.clone());
            for (channel_name, channel_value) in resolved_encodings {
                if channel_value.get_scale_name(&channel_name).is_some() {
                    used_channels.insert(channel_name.clone());
                }
            }
        }
        used_channels
    }

    /// Build a scale by name, applying any configured transformations
    pub(crate) fn build_scale(
        &self,
        name: &str,
        plot_width: f64,
        plot_height: f64,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Scale, AvengerChartError> {
        // Strip trailing numbers to get the base scale name
        let base_name = strip_trailing_numbers(name);

        // Build the default scale for the base name
        let mut base_scale =
            self.create_default_scale_for_channel_internal(base_name, ctx, params)?;

        // Apply coordinate-specific default range if applicable
        if let Some(range) = self.get_coordinate_default_range(name, plot_width, plot_height) {
            use datafusion::prelude::lit;
            base_scale = base_scale.range_interval(lit(range.0), lit(range.1));
        }

        // Gather domain expressions from marks
        if let Ok(domain_exprs) = self.gather_scale_domain_expressions(base_name, ctx) {
            if !domain_exprs.is_empty() {
                base_scale = base_scale.domain_data_fields(domain_exprs);
            }
        }

        // Apply user-configured scale options
        match self.scale_specs.get(base_name) {
            Some(ScaleSpec::Local(scale_changes)) => Ok(base_scale.update(scale_changes.clone())),
            None => Ok(base_scale),
        }
    }

    /// Get the default range for a coordinate system channel
    fn get_coordinate_default_range(
        &self,
        name: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        // Check if this scale is mapped to a coordinate channel
        let coord_channel = self
            .scale_to_coord_channel
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(name);

        self.coord_transform
            .default_range(coord_channel, plot_area_width, plot_area_height)
    }

    /// Create a default scale for a channel based on mark data types and preferences
    fn create_default_scale_for_channel_internal(
        &self,
        channel: &str,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Scale, AvengerChartError> {
        use crate::channel::resolution::resolve_all_channel_refs;
        use crate::render::RenderContext;
        use crate::scales::{Scale, spec::Auto};
        use datafusion::logical_expr::lit;
        use std::collections::HashMap;
        use std::sync::Arc;

        // Try to infer the data type and use mark-based scale preferences
        let mut scale_spec = None;
        let mut data_type = None;
        let mut found_mark = None;

        // Look through marks to find the expression for this channel
        for mark in &self.marks {
            let channels = mark.data_context().channels();

            // Try to resolve channel references first
            let resolved_channels = match resolve_all_channel_refs(channels, ctx) {
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
                // Get the dataframe for this mark using the provided context
                let mark_df = mark.data_context().dataframe_with_context(ctx);
                let plot_df = self.data.as_ref().and_then(|node| {
                    node.to_logical_plan(ctx)
                        .ok()
                        .map(|plan| DataFrame::new(ctx.state().clone(), plan))
                });
                let df = mark_df.or(plot_df);

                // Try to get the data type of the channel
                if let Some(df) = df {
                    let schema = df.schema();
                    match channel_value.get_data_type(schema, ctx) {
                        Ok(dt) => {
                            data_type = Some(dt.clone());
                            scale_spec = mark.preferred_scale_type(channel, &dt);
                            found_mark = Some(mark);
                            break;
                        }
                        Err(_) => {
                            // Continue to next mark to try to find a valid data type
                        }
                    }
                }
            }
        }

        let scale_spec = scale_spec.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Failed to infer scale specification for channel '{}'",
                channel
            ))
        })?;

        // Create scale from spec
        // Ranges are set later after domain inference when we have full context
        // (cardinality, mark-specific requirements, theme with mark type, etc.)
        let mut scale = Scale::<Auto>::from_spec(scale_spec);

        // Apply coordinate system and mark-specific scale options
        if let (Some(dt), Some(mark)) = (&data_type, found_mark) {
            let mut default_options = HashMap::new();

            // Create ScaleImpl from ScaleSpec for methods that need it
            let scale_impl = scale.get_scale_impl().ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Failed to create scale implementation for channel '{}'",
                    channel
                ))
            })?;

            // First get coordinate system defaults (for all channels, not just position)
            // The coord_transform has a default_scale_options method that returns ScalarValue
            let coord_options = self
                .coord_transform
                .default_scale_options(channel, scale_impl.as_ref());

            // Convert ScalarValue to Expr for compatibility
            let coord_options_expr: HashMap<String, datafusion::logical_expr::Expr> = coord_options
                .into_iter()
                .map(|(k, v)| (k, lit(v)))
                .collect();
            default_options.extend(coord_options_expr);

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

    /// Gather mark data and encoding expressions that use this scale with radius information
    fn gather_scale_domain_expressions_with_radius(
        &self,
        scale_name: &str,
        configured_scales: &HashMap<String, ConfiguredScaleWithSpec>,
        context: &crate::render::RenderContext,
    ) -> Result<
        Vec<(
            Arc<DataFrame>,
            datafusion::logical_expr::Expr,
            Option<crate::marks::RadiusExpression>,
        )>,
        AvengerChartError,
    > {
        use crate::channel::resolution::resolve_all_channel_refs;
        use crate::channel::value::ChannelValue;
        use crate::scales::extensions::ConfiguredScaleDataFusionExt;

        let mut data_expressions = Vec::new();

        for mark in &self.marks {
            // Get channels and resolve references first to check if columns are referenced
            let channels = mark.data_context().channels();
            let resolved_channels = resolve_all_channel_refs(channels, &context.session_context)?;

            // Check if any expressions reference columns
            let references_columns =
                resolved_channels
                    .values()
                    .any(|channel_value| match channel_value {
                        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => expr
                            .to_expr(&context.session_context)
                            .map(|e| !e.column_refs().is_empty())
                            .unwrap_or(false),
                        ChannelValue::Conditional {
                            conditions,
                            otherwise,
                            ..
                        } => {
                            conditions.iter().any(|(condition, value)| {
                                condition
                                    .to_expr(&context.session_context)
                                    .map(|e| !e.column_refs().is_empty())
                                    .unwrap_or(false)
                                    || value
                                        .expr(&context.session_context)
                                        .ok()
                                        .map(|e| !e.column_refs().is_empty())
                                        .unwrap_or(false)
                            }) || otherwise
                                .expr(&context.session_context)
                                .ok()
                                .map(|e| !e.column_refs().is_empty())
                                .unwrap_or(false)
                        }
                    });

            // Determine DataFrame for this mark using context from RenderContext
            let mark_df = mark
                .data_context()
                .dataframe_with_context(&context.session_context);
            let plot_df = self.data.as_ref().and_then(|node| {
                node.to_logical_plan(&context.session_context)
                    .ok()
                    .map(|plan| DataFrame::new(context.session_context.state().clone(), plan))
            });

            let df = if let Some(mark_df) = mark_df {
                // Mark has explicit data
                Arc::new(mark_df)
            } else if !references_columns {
                // No column references - skip domain inference for unit marks
                continue;
            } else if let Some(plot_df) = plot_df {
                // Inherit from plot
                Arc::new(plot_df)
            } else {
                // No data available - skip this mark
                continue;
            };

            // Create channel resolver for this mark (uses configured non-positional scales and theme)
            // This resolver looks up the channel value and applies scaling if needed
            let resolve_channel = |channel_name: &str| -> datafusion::logical_expr::Expr {
                use datafusion::prelude::lit;

                // First check explicit mapping
                if let Some(channel_value) = resolved_channels.get(channel_name) {
                    // Apply scaling if needed
                    match channel_value {
                        ChannelValue::Value { expr } => {
                            // No scaling requested - convert to Expr
                            if let Ok(expr_df) = expr.to_expr(&context.session_context) {
                                return expr_df;
                            }
                            return lit(datafusion::common::ScalarValue::Null);
                        }
                        ChannelValue::Scaled {
                            expr,
                            scale_name: custom_scale_name,
                            band,
                            ..
                        } => {
                            // Determine scale name
                            let scale_key =
                                custom_scale_name.as_ref().cloned().unwrap_or_else(|| {
                                    use crate::channel::value::strip_trailing_numbers;
                                    strip_trailing_numbers(channel_name).to_string()
                                });

                            // Use configured scales if available
                            if let Some(configured) = configured_scales.get(&scale_key) {
                                // Convert SerializableExpr to Expr first
                                if let Ok(expr_df) = expr.to_expr(&context.session_context) {
                                    // Apply the scale transform
                                    if let Some(band_value) = band {
                                        return configured
                                            .to_expr_with_band(expr_df.clone(), *band_value)
                                            .unwrap_or(expr_df);
                                    } else {
                                        return configured
                                            .to_expr(expr_df.clone())
                                            .unwrap_or(expr_df);
                                    }
                                }
                                // If conversion fails, return null
                                return lit(datafusion::common::ScalarValue::Null);
                            } else {
                                // No scale configured - use raw expression, convert to Expr
                                if let Ok(expr_df) = expr.to_expr(&context.session_context) {
                                    return expr_df;
                                }
                                return lit(datafusion::common::ScalarValue::Null);
                            }
                        }
                        ChannelValue::Conditional { .. } => {
                            // Conditional values are not supported for radius calculations
                            return lit(datafusion::scalar::ScalarValue::Null);
                        }
                    }
                }

                // No explicit mapping - check for mark-provided defaults (including theme)
                if let Some(default_scalar) = mark.default_channel_value(channel_name, context) {
                    // Use mark-provided default (which includes theme defaults)
                    if std::env::var("AVENGER_DEBUG_RADIUS").is_ok() {
                        eprintln!(
                            "DEBUG: Using default for channel '{}': {:?}",
                            channel_name, default_scalar
                        );
                    }
                    return lit(default_scalar);
                }

                // No mapping and no default
                lit(datafusion::scalar::ScalarValue::Null)
            };

            // Check all encodings in the mark's data context
            for (channel, channel_value) in &resolved_channels {
                // Check if this channel uses our scale
                if let Some(channel_scale_name) = channel_value.get_scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get radius expression from the mark
                        let radius_expr = mark.radius_expression(scale_name, &resolve_channel);

                        // Get the expressions - handle conditional values properly
                        match channel_value {
                            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                                // Convert SerializableExpr to Expr
                                if let Ok(expr_df) = expr.to_expr(&context.session_context) {
                                    data_expressions.push((df.clone(), expr_df, radius_expr));
                                }
                            }
                            ChannelValue::Conditional {
                                conditions,
                                otherwise,
                                ..
                            } => {
                                // For conditional values, we use the mark's radius for all branches
                                // since radius doesn't vary by condition
                                let radius_expr_cond =
                                    mark.radius_expression(scale_name, &resolve_channel);

                                // Add expressions from all conditions and the otherwise branch
                                for (_, value) in conditions {
                                    if let Ok(expr) = value.expr(&context.session_context) {
                                        data_expressions.push((
                                            df.clone(),
                                            expr,
                                            radius_expr_cond.clone(),
                                        ));
                                    }
                                }
                                if let Ok(expr) = otherwise.expr(&context.session_context) {
                                    data_expressions.push((df.clone(), expr, radius_expr_cond));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(data_expressions)
    }

    /// Gather mark data and encoding expressions that use this scale
    fn gather_scale_domain_expressions(
        &self,
        scale_name: &str,
        ctx: &SessionContext,
    ) -> Result<Vec<(Arc<DataFrame>, datafusion::logical_expr::Expr)>, AvengerChartError> {
        use crate::channel::resolution::resolve_all_channel_refs;
        use crate::channel::value::ChannelValue;

        let mut data_expressions = Vec::new();

        for mark in &self.marks {
            // Get channels and resolve references first to check if columns are referenced
            let channels = mark.data_context().channels();
            let resolved_channels = resolve_all_channel_refs(channels, ctx)?;

            // Check if any expressions reference columns
            let references_columns =
                resolved_channels
                    .values()
                    .any(|channel_value| match channel_value {
                        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => expr
                            .to_expr(ctx)
                            .map(|e| !e.column_refs().is_empty())
                            .unwrap_or(false),
                        ChannelValue::Conditional {
                            conditions,
                            otherwise,
                            ..
                        } => {
                            conditions.iter().any(|(condition, value)| {
                                condition
                                    .to_expr(ctx)
                                    .map(|e| !e.column_refs().is_empty())
                                    .unwrap_or(false)
                                    || value
                                        .expr(ctx)
                                        .ok()
                                        .map(|e| !e.column_refs().is_empty())
                                        .unwrap_or(false)
                            }) || otherwise
                                .expr(ctx)
                                .ok()
                                .map(|e| !e.column_refs().is_empty())
                                .unwrap_or(false)
                        }
                    });

            // Determine DataFrame for this mark using context from RenderContext
            let mark_df = mark.data_context().dataframe_with_context(ctx);
            let plot_df = self.data.as_ref().and_then(|node| {
                node.to_logical_plan(ctx)
                    .ok()
                    .map(|plan| DataFrame::new(ctx.state().clone(), plan))
            });

            let df = if let Some(mark_df) = mark_df {
                // Mark has explicit data
                Arc::new(mark_df)
            } else if !references_columns {
                // No column references - skip domain inference for unit marks
                continue;
            } else if let Some(plot_df) = plot_df {
                // Inherit from plot
                Arc::new(plot_df)
            } else {
                // No data available - skip this mark
                continue;
            };

            // Check all encodings in the mark's data context
            for (channel, channel_value) in &resolved_channels {
                // Check if this channel uses our scale
                // Get the scale name this channel would use
                if let Some(channel_scale_name) = channel_value.get_scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get the expressions - handle conditional values properly
                        match channel_value {
                            ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                                // Convert SerializableExpr to Expr
                                if let Ok(expr_df) = expr.to_expr(ctx) {
                                    data_expressions.push((df.clone(), expr_df));
                                }
                            }
                            ChannelValue::Conditional {
                                conditions,
                                otherwise,
                                ..
                            } => {
                                // Add expressions from all conditions and the otherwise branch
                                for (_, value) in conditions {
                                    if let Ok(expr) = value.expr(ctx) {
                                        data_expressions.push((df.clone(), expr));
                                    }
                                }
                                if let Ok(expr) = otherwise.expr(ctx) {
                                    data_expressions.push((df.clone(), expr));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(data_expressions)
    }

    /// Build a configured scale with radius context for domain inference
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

        // Step 5: Set ranges for non-position channels using mark-specific defaults
        let is_position = self
            .coord_transform
            .required_channels()
            .contains(&name.as_ref());

        if !is_position && scale.get_range().is_none() {
            // Query marks for range defaults with full domain context
            for mark in &self.marks {
                if mark.data_context().channels().contains_key(name) {
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

        // Final fallback: query theme with generic "mark" type, then use hardcoded defaults
        if scale.get_range().is_none() {
            let range_kind = scale.get_scale_impl()
                .map(|impl_arc| impl_arc.range_kind())
                .unwrap_or(avenger_scales::scales::RangeKind::Continuous);

            let theme = self.get_theme();
            let range = theme
                .get_range_for_channel("mark", name, range_kind, None)
                .unwrap_or_else(|| crate::scales::default_range_for_channel(name, range_kind));

            scale = scale.range(range);
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
    pub(super) async fn build_initial_scales(
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
    pub(super) async fn rebuild_scales_with_final_dimensions(
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

    /// Check if a data type is numeric
    pub(super) fn is_numeric_type(dtype: &datafusion::arrow::datatypes::DataType) -> bool {
        use datafusion::arrow::datatypes::DataType;
        match dtype {
            // Standard numeric types
            DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64 => true,
            // Dictionary types are allowed if their value type is numeric
            // (This happens when categorical data goes through an ordinal scale)
            DataType::Dictionary(_, value_type) => Self::is_numeric_type(value_type),
            _ => false,
        }
    }
}
