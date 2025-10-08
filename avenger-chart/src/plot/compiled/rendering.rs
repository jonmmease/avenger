//! Rendering pipeline for CompiledPlot

use std::collections::HashMap;
use std::sync::Arc;

use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use avenger_scales::scales::ConfiguredScale;

use crate::channel::value::{ChannelValue, ConditionalValue};
use crate::error::AvengerChartError;
use crate::marks::CompiledMark;
use crate::scales::ConfiguredScaleWithSpec;
use crate::serialization::{LogicalExprNodeExt, LogicalPlanNodeExt};

use super::CompiledPlot;
use super::expr_eval::evaluate_f32_expr;

/// Evaluate a SizeMode to get an EvaluatedSizeMode with concrete f32 values
async fn evaluate_size_mode(
    size_mode: &crate::layout::SizeMode,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<crate::layout::EvaluatedSizeMode, AvengerChartError> {
    use crate::layout::{EvaluatedSizeMode, SizeMode};
    use crate::serialization::LogicalExprNodeExt;
    use datafusion_proto::protobuf::LogicalExprNode;

    match size_mode {
        SizeMode::Fixed { width, height } => {
            let width_node: LogicalExprNode = width.clone().into();
            let height_node: LogicalExprNode = height.clone().into();
            let width_expr = width_node.to_expr(ctx)?;
            let height_expr = height_node.to_expr(ctx)?;
            let w = evaluate_f32_expr(&width_expr, ctx, params).await?;
            let h = evaluate_f32_expr(&height_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Fixed {
                width: w,
                height: h,
            })
        }
        SizeMode::Width(width) => {
            let width_node: LogicalExprNode = width.clone().into();
            let width_expr = width_node.to_expr(ctx)?;
            let w = evaluate_f32_expr(&width_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Width(w))
        }
        SizeMode::Height(height) => {
            let height_node: LogicalExprNode = height.clone().into();
            let height_expr = height_node.to_expr(ctx)?;
            let h = evaluate_f32_expr(&height_expr, ctx, params).await?;
            Ok(EvaluatedSizeMode::Height(h))
        }
        SizeMode::Auto => Ok(EvaluatedSizeMode::Auto),
    }
}

/// Evaluate Margins to get concrete f32 values (from expression, theme, or default)
async fn evaluate_margins(
    margins: &crate::layout::Margins,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    theme: &crate::theme::Theme,
) -> Result<crate::layout::EvaluatedMargins, AvengerChartError> {
    use crate::layout::EvaluatedMargins;
    use crate::serialization::LogicalExprNodeExt;

    // Helper to query margin from theme
    let query_margin = |property: &str| -> f32 {
        let canvas_ctx = crate::theme::ThemeContext::new("canvas").with_params(params.clone());
        theme.query(&canvas_ctx, property)
            .and_then(|v| v.as_font_size(theme.get_base_font_size(params)))
            .unwrap_or(10.0)
    };

    // Evaluate each margin field, checking expression → theme → default
    let top = match margins.top.as_ref() {
        crate::maybe::Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-top"),
    };

    let right = match margins.right.as_ref() {
        crate::maybe::Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-right"),
    };

    let bottom = match margins.bottom.as_ref() {
        crate::maybe::Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-bottom"),
    };

    let left = match margins.left.as_ref() {
        crate::maybe::Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => query_margin("margin-left"),
    };

    Ok(EvaluatedMargins {
        top,
        right,
        bottom,
        left,
    })
}

/// Evaluate a LayoutSpec to get an EvaluatedLayoutSpec with concrete f32 values
async fn evaluate_layout_spec(
    layout_spec: &crate::layout::LayoutSpec,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
    theme: &crate::theme::Theme,
) -> Result<crate::layout::EvaluatedLayoutSpec, AvengerChartError> {
    use crate::layout::EvaluatedLayoutSpec;

    let canvas = evaluate_size_mode(&layout_spec.canvas, ctx, params).await?;
    let plot_area = evaluate_size_mode(&layout_spec.plot_area, ctx, params).await?;
    let margins = evaluate_margins(&layout_spec.margins, ctx, params, theme).await?;

    Ok(EvaluatedLayoutSpec {
        canvas,
        plot_area,
        margins,
    })
}

impl CompiledPlot {
    /// Apply scaling transformation to a channel expression
    fn apply_channel_scale(
        &self,
        channel_name: &str,
        channel_value: &ChannelValue,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
    ) -> Result<datafusion::logical_expr::Expr, AvengerChartError> {
        use crate::channel::value::strip_trailing_numbers;

        match channel_value {
            ChannelValue::Value { expr } => {
                // No scaling requested, return expression as-is - convert to Expr
                expr.to_expr(ctx)
            }
            ChannelValue::Conditional {
                conditions,
                otherwise,
                ..
            } => {
                // Build a CASE WHEN expression from the conditions
                use datafusion::arrow::datatypes::DataType;
                use datafusion::logical_expr::{lit, when};
                use datafusion::scalar::ScalarValue;

                // Helper to convert color string literals to the proper ScalarValue format
                let convert_color_literal =
                    |expr: &datafusion::logical_expr::Expr| -> datafusion::logical_expr::Expr {
                        // Check if this is a string literal that might be a color
                        if let datafusion::logical_expr::Expr::Literal(scalar_value, _) = expr {
                            if let ScalarValue::Utf8(Some(s)) = scalar_value {
                                // Use our existing color parsing utility
                                if let Some(color_or_gradient) = crate::utils::parse_color_string(s)
                                {
                                    use avenger_common::types::ColorOrGradient;
                                    if let ColorOrGradient::Color(rgba) = color_or_gradient {
                                        // Convert to List ScalarValue with Float32 values
                                        let values: Vec<ScalarValue> = rgba
                                            .into_iter()
                                            .map(|v| ScalarValue::Float32(Some(v)))
                                            .collect();

                                        // Create the list array and wrap in ScalarValue
                                        let list_array = ScalarValue::new_list_nullable(
                                            &values,
                                            &DataType::Float32,
                                        );
                                        let scalar_list = ScalarValue::List(list_array);
                                        return lit(scalar_list);
                                    }
                                }
                            }
                        }
                        // Not a color literal or failed to parse, return as-is
                        expr.clone()
                    };

                // For conditional values, the scale name is derived from the channel name
                let scale_key = strip_trailing_numbers(channel_name).to_string();

                // Check if we need color conversion (for color channels)
                let needs_color_conversion = matches!(channel_name, "fill" | "stroke" | "color");

                // Helper to apply scale to a conditional value
                let apply_to_conditional = |cond_val: &ConditionalValue| -> Result<
                    datafusion::logical_expr::Expr,
                    AvengerChartError,
                > {
                    match cond_val {
                        ConditionalValue::Scaled { expr } => {
                            // Apply scale transformation
                            if let Some(scale) = scales.get(&scale_key) {
                                use crate::scales::ConfiguredScaleDataFusionExt;
                                // Convert SerializableExpr to Expr first
                                expr.to_expr(ctx)
                                    .and_then(|e| scale.to_expr(e))
                            } else {
                                // No scale found, return expression as-is - convert to Expr
                                expr.to_expr(ctx)
                            }
                        }
                        ConditionalValue::Value { expr } => {
                            // Pass through literal values unchanged - convert to Expr
                            let expr_df = expr.to_expr(ctx)?;
                            if needs_color_conversion {
                                Ok(convert_color_literal(&expr_df))
                            } else {
                                Ok(expr_df)
                            }
                        }
                    }
                };

                // Start with the first condition
                let first_cond = &conditions[0];
                let first_value = apply_to_conditional(&first_cond.1)?;
                // Convert SerializableExpr to Expr
                let first_cond_expr = first_cond.0.to_expr(ctx)?;
                let mut case_expr = when(first_cond_expr, first_value);

                // Add remaining conditions
                for (condition, value) in &conditions[1..] {
                    let scaled_value = apply_to_conditional(value)?;
                    // Convert SerializableExpr to Expr
                    let condition_expr = condition.to_expr(ctx)?;
                    case_expr = case_expr.when(condition_expr, scaled_value);
                }

                // Add the otherwise clause
                let otherwise_value = apply_to_conditional(otherwise)?;

                Ok(case_expr.otherwise(otherwise_value)?)
            }
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                ..
            } => {
                // Determine the scale to use
                let default_scale_name = strip_trailing_numbers(channel_name).to_string();
                let scale_key = scale_name.as_ref().unwrap_or(&default_scale_name);

                // Look up the configured scale
                let scale = scales.get(scale_key).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Scale '{}' not found for channel '{}'",
                        scale_key, channel_name
                    ))
                })?;

                // Apply the scale transformation
                use crate::scales::ConfiguredScaleDataFusionExt;
                // Convert SerializableExpr to Expr first
                let expr_df = expr.to_expr(ctx)?;
                if let Some(band) = band {
                    scale.to_expr_with_band(expr_df.clone(), *band)
                } else {
                    scale.to_expr(expr_df)
                }
            }
        }
    }

    /// Render a single mark with its data and transformations
    async fn render_mark(
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
                        self.apply_channel_scale(sort_channel_name, sort_channel, scales, ctx)?;

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
                    self.apply_channel_scale(channel_desc.name, channel_value, scales, ctx)?;

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
    async fn create_guide_marks(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &crate::layout::LayoutBounds,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
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
                    params,
                    ctx,
                )
                .await
        } else {
            // No guide renderer available
            Ok(vec![])
        }
    }

    /// Compute layout with overflow measurement
    async fn compute_layout(
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
                    params,
                    ctx,
                )
                .await?
        } else {
            // No guide renderer - no overflow
            crate::guide::OverflowSpaceRequirement::default()
        };

        // Get legends with theme applied
        let all_legends = self.get_legends_with_theme(scales, ctx, params);

        // Use the helper to merge legend channels
        let (_channel_groups, legends_map) =
            self.merge_legend_channels(&all_legends, scales, ctx, params).await?;

        // Prepare legend measurements
        let available_size = taffy::Size {
            width: width * INITIAL_PLOT_AREA_RATIO,
            height: height * INITIAL_PLOT_AREA_RATIO,
        };
        let legend_measurements =
            self.prepare_legend_measurements(&legends_map, scales, available_size, ctx, params).await?;

        // Create ChartLayout with overflow directly
        let layout_spec = self.get_layout_spec();

        // Evaluate the layout spec to get concrete dimensions
        let theme = self.get_theme();
        let evaluated_spec = evaluate_layout_spec(layout_spec, ctx, params, theme.as_ref()).await?;

        let mut layout = ChartLayout::new(
            &overflow,
            &evaluated_spec,
            self.get_title(),
            self.get_subtitle(),
            self.get_theme().as_ref(),
            &legend_measurements,
            ctx,
            params,
        )
        .await?;

        // Compute layout using the evaluated layout spec and return it directly
        let result = layout.compute(&evaluated_spec)?;
        Ok(result)
    }

    /// Create legends positioned according to layout
    async fn create_legends_with_layout(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        layout: &crate::layout::LayoutResult,
        _plot_width: f32,
        _plot_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get legends with theme applied (same as used for layout)
        let all_legend_configs = self.get_legends_with_theme(scales, ctx, params);

        // Use the helper to merge legend channels
        let (sorted_channel_groups, _legends_map) =
            self.merge_legend_channels(&all_legend_configs, scales, ctx, params).await?;

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

                // Check visibility before rendering
                use crate::plot::compiled::expr_eval::*;
                use crate::serialization::LogicalExprNodeExt;

                let visible = if let Some(node) = legend.visible.as_option().and_then(|o| o.as_ref()) {
                    let expr = node.to_expr(ctx)?;
                    evaluate_bool_expr(&expr, ctx, params).await?
                } else {
                    true // Default to visible
                };

                // Skip this legend group if no renderer is available or if not visible
                if visible {
                    if let Some(renderer) = renderer_opt {
                        // Render the legend with the determined renderer
                        let theme = self.get_theme();
                        let group_opt = renderer.render(
                            &channels,
                            legend,
                            bounds.x,
                            bounds.y,
                            bounds.width,
                            bounds.height,
                            theme.as_ref(),
                            params,
                            ctx,
                        ).await?;

                        // Add the legend group mark if it was rendered
                        if let Some(group) = group_opt {
                            legend_marks.push(SceneMark::Group(group));
                        }
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
            .create_guide_marks(
                scales,
                plot_area_width,
                plot_area_height,
                plot_bounds,
                params,
                ctx,
            )
            .await?;

        // Create legends
        let legend_marks = self.create_legends_with_layout(
            scales,
            &layout.taffy_layout,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
        ).await?;

        // Create title
        let title_marks = if let Some(title_bounds) = &layout.taffy_layout.title {
            self.create_title(Some(*title_bounds), ctx, params).await?
        } else {
            Vec::new()
        };

        // Create subtitle
        let subtitle_marks = if let Some(subtitle_bounds) = &layout.taffy_layout.subtitle {
            self.create_subtitle(Some(*subtitle_bounds), ctx, params)
                .await?
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

        // Merge provided params with default params first (needed for dimension evaluation)
        let merged_params = if let Some(provided) = params {
            let mut merged = self.default_params.clone();
            merged.extend(provided);
            merged
        } else {
            self.default_params.clone()
        };

        // Evaluate dimension expressions to get concrete values
        // This will be refined after we compute the actual layout
        use crate::serialization::LogicalExprNodeExt;
        use datafusion_proto::protobuf::LogicalExprNode;

        let (estimated_width, estimated_height) = match &layout_spec.canvas {
            crate::layout::SizeMode::Fixed { width, height } => {
                let width_node: LogicalExprNode = width.clone().into();
                let height_node: LogicalExprNode = height.clone().into();
                let width_expr = width_node.to_expr(ctx)?;
                let height_expr = height_node.to_expr(ctx)?;
                let w = evaluate_f32_expr(&width_expr, ctx, &merged_params).await?;
                let h = evaluate_f32_expr(&height_expr, ctx, &merged_params).await?;
                (Some(w), Some(h))
            }
            crate::layout::SizeMode::Width(width) => {
                let width_node: LogicalExprNode = width.clone().into();
                let width_expr = width_node.to_expr(ctx)?;
                let w = evaluate_f32_expr(&width_expr, ctx, &merged_params).await?;
                (Some(w), None)
            }
            crate::layout::SizeMode::Height(height) => {
                let height_node: LogicalExprNode = height.clone().into();
                let height_expr = height_node.to_expr(ctx)?;
                let h = evaluate_f32_expr(&height_expr, ctx, &merged_params).await?;
                (None, Some(h))
            }
            _ => (None, None), // No dimensions specified
        };

        // Add evaluated dimensions to merged params for media query evaluation
        // Only add params if we have actual dimension values
        let mut merged_params = merged_params;
        if let Some(w) = estimated_width {
            merged_params.insert(
                "width".to_string(),
                datafusion::common::ScalarValue::Float32(Some(w)),
            );
        }
        if let Some(h) = estimated_height {
            merged_params.insert(
                "height".to_string(),
                datafusion::common::ScalarValue::Float32(Some(h)),
            );
        }

        // Use estimated dimensions for initial scale construction
        // Default to reasonable sizes if not specified
        let estimated_plot_width = estimated_width.unwrap_or(400.0) * INITIAL_PLOT_AREA_RATIO;
        let estimated_plot_height = estimated_height.unwrap_or(300.0) * INITIAL_PLOT_AREA_RATIO;

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
                estimated_width.unwrap_or(400.0),
                estimated_height.unwrap_or(300.0),
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
        let canvas_ctx =
            crate::theme::ThemeContext::new("canvas").with_params(merged_params.clone());
        if let Some(color) = theme
            .query(&canvas_ctx, "background-color")
            .and_then(|v| v.as_color_array())
        {
            use avenger_common::types::ColorOrGradient;
            use avenger_scenegraph::marks::rect::SceneRectMark;

            // Color is already in normalized [f32; 4] format

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
