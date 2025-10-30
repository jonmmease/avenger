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

impl CompiledPlot {
    /// Initial estimate for plot area as ratio of total canvas size
    const INITIAL_PLOT_AREA_RATIO: f32 = 0.8;

    /// Default margin in pixels when not specified in theme or expression
    const DEFAULT_MARGIN: f32 = 10.0;
}

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
        let canvas_ctx = crate::theme::ThemeContext::new("canvas", params.clone());
        theme
            .query(&canvas_ctx, property)
            .and_then(|v| v.as_font_size(params, theme.get_base_font_size(params)))
            .unwrap_or(CompiledPlot::DEFAULT_MARGIN)
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
                                expr.to_expr(ctx).and_then(|e| scale.to_expr(e))
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

                // Optional debugging for size scale behavior
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

    /// Evaluate a single mark with its data and transformations
    #[allow(dead_code)] // Will be removed in Phase 8 after facet migration
    async fn evaluate_mark(
        &self,
        mark: &dyn CompiledMark,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<(Vec<SceneMark>, Box<dyn crate::layout::LayoutInfo>), AvengerChartError> {
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
                // Concat all batches into a single RecordBatch
                use datafusion::arrow::compute::concat_batches;
                let schema = batch[0].schema();
                let combined = concat_batches(&schema, &batch)?;
                Some(combined)
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
                return Ok((vec![], Box::new(())));
            } else {
                // Concat all batches into a single RecordBatch
                use datafusion::arrow::compute::concat_batches;
                let schema = batch[0].schema();
                concat_batches(&schema, &batch)?
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
            scales.clone(),
        );

        // Clone the coordinate transform
        let coord_transform = self.coord_transform.clone_box();

        // Evaluate the mark with data
        mark.evaluate_from_data(
            data_batch.as_ref(),
            &scalar_batch,
            &context,
            coord_transform,
        )
        .await
    }

    /// Evaluate a single mark with an optional provided plot-level DataFrame fallback.
    /// If `provided_plot_df` is Some, it is used when the mark has no explicit data and
    /// the channels reference columns. Otherwise, falls back to this CompiledPlot's plot-level data.
    pub(super) async fn evaluate_mark_with_plot_df(
        &self,
        mark: &dyn CompiledMark,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        provided_plot_df: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<(Vec<SceneMark>, Box<dyn crate::layout::LayoutInfo>), AvengerChartError> {
        // Get channel mappings from DataContext
        let channels = mark.data_context().channels();

        // Resolve channel references
        let channels = crate::channel::resolution::resolve_all_channel_refs(channels, ctx)?;

        // Check if any channel expressions reference columns
        let references_columns = channels.values().any(|channel_value| match channel_value {
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

        // Determine data source
        let df_ref = if let Some(df_override) = provided_plot_df.cloned() {
            Some(df_override)
        } else if let Some(mark_df) = mark.data_context().dataframe_with_context(ctx) {
            Some(mark_df)
        } else if !references_columns {
            None
        } else if let Some(df) = self.data.as_ref().and_then(|node| {
            node.to_logical_plan(ctx)
                .ok()
                .map(|plan| DataFrame::new(ctx.state().clone(), plan))
        }) {
            Some(df)
        } else {
            return Err(AvengerChartError::InternalError(
                "Mark expressions reference columns but no data is available".to_string(),
            ));
        };

        // Sorting
        let df = if let Some(df_ref) = df_ref {
            if let Some(sort_channel_name) = mark.sorting_channel() {
                if let Some(sort_channel) = channels.get(sort_channel_name) {
                    let sort_expr =
                        self.apply_channel_scale(sort_channel_name, sort_channel, scales, ctx)?;
                    let sorted_df = df_ref.sort(vec![sort_expr.sort(true, false)])?;
                    Arc::new(sorted_df)
                } else {
                    Arc::new(df_ref)
                }
            } else {
                Arc::new(df_ref)
            }
        } else {
            let empty_df = ctx
                .sql("SELECT 1 as _dummy")
                .await
                .map_err(|e| AvengerChartError::DataFusionError(e))?;
            Arc::new(empty_df)
        };

        // Channels split
        let supported_channels = mark.supported_channels();
        let mut array_channels = Vec::new();
        let mut scalar_channels = Vec::new();
        let mut has_array_data = false;
        for channel_desc in &supported_channels {
            if let Some(channel_value) = channels.get(channel_desc.name) {
                let scaled_expr =
                    self.apply_channel_scale(channel_desc.name, channel_value, scales, ctx)?;
                if channel_desc.allow_column_ref && scaled_expr.any_column_refs() {
                    array_channels.push((channel_desc.name, scaled_expr));
                    has_array_data = true;
                } else {
                    scalar_channels.push((channel_desc.name, scaled_expr));
                }
            }
        }

        // Build array data batch
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
                use datafusion::arrow::compute::concat_batches;
                let schema = batch[0].schema();
                let combined = concat_batches(&schema, &batch)?;
                Some(combined)
            }
        } else {
            None
        };

        // Scalar data batch
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
                return Ok((vec![], Box::new(())));
            } else {
                use datafusion::arrow::compute::concat_batches;
                let schema = batch[0].schema();
                concat_batches(&schema, &batch)?
            }
        } else {
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

        self.validate_positional_channel_types(&data_batch, &scalar_batch)?;

        let theme = self.get_theme();
        let context = crate::render::RenderContext::new(
            theme,
            plot_width,
            plot_height,
            Arc::new(ctx.clone()),
            params.clone(),
            scales.clone(),
        );
        let coord_transform = self.coord_transform.clone_box();
        mark.evaluate_from_data(
            data_batch.as_ref(),
            &scalar_batch,
            &context,
            coord_transform,
        )
        .await
    }

    /// Create guide marks (axes, grids) for the coordinate system
    pub(super) async fn create_guide_marks(
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
                .evaluate(
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
    pub(super) async fn compute_layout(
        &self,
        width: f32,
        height: f32,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<crate::render::LayoutSolution, AvengerChartError> {
        use crate::layout::ChartLayout;

        // Check for required positional scales before measuring overflow
        self.validate_positional_scales_exist(scales)?;

        // Measure how much space the guide needs if we have one
        let overflow = if let Some(compiled_guide) = &self.compiled_guide {
            let width_estimate = width * Self::INITIAL_PLOT_AREA_RATIO;
            let height_estimate = height * Self::INITIAL_PLOT_AREA_RATIO;

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
        let (_channel_groups, legends_map) = self
            .merge_legend_channels(&all_legends, scales, ctx, params)
            .await?;

        // Prepare legend measurements
        let available_size = taffy::Size {
            width: width * Self::INITIAL_PLOT_AREA_RATIO,
            height: height * Self::INITIAL_PLOT_AREA_RATIO,
        };
        let legend_measurements = self
            .prepare_legend_measurements(&legends_map, scales, available_size, ctx, params)
            .await?;

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
    pub(super) async fn create_legends_with_layout(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        layout: &crate::layout::LayoutResult,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Get legends with theme applied (same as used for layout)
        let all_legend_configs = self.get_legends_with_theme(scales, ctx, params);

        // Use the helper to merge legend channels
        let (sorted_channel_groups, _legends_map) = self
            .merge_legend_channels(&all_legend_configs, scales, ctx, params)
            .await?;

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

                let visible =
                    if let Some(node) = legend.visible.as_option().and_then(|o| o.as_ref()) {
                        let expr = node.to_expr(ctx)?;
                        evaluate_bool_expr(&expr, ctx, params).await?
                    } else {
                        true // Default to visible
                    };

                // Skip this legend group if no renderer is available or if not visible
                if visible {
                    if let Some(renderer) = renderer_opt {
                        // Evaluate the legend with the determined renderer
                        let theme = self.get_theme();
                        let group_opt = renderer
                            .evaluate(
                                &channels,
                                legend,
                                bounds.x,
                                bounds.y,
                                bounds.width,
                                bounds.height,
                                theme.as_ref(),
                                params,
                                ctx,
                            )
                            .await?;

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

    /// Evaluate all components (marks, axes, legends, titles)
    #[allow(dead_code)] // Will be removed in Phase 8 after facet migration
    async fn evaluate_all_components(
        &self,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        layout: &crate::render::LayoutSolution,
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
        let mut layout_infos: Vec<Box<dyn crate::layout::LayoutInfo>> = Vec::new();
        for mark in &self.marks {
            let (scene_marks, layout_info) = self
                .evaluate_mark(
                    mark.as_ref(),
                    scales,
                    plot_area_width,
                    plot_area_height,
                    ctx,
                    params,
                )
                .await?;
            mark_groups.extend(scene_marks);
            layout_infos.push(layout_info);
        }

        // Merge scale updates from layout info
        let merged_scales = crate::layout::merge_scale_updates(scales, &layout_infos);

        // Create guide marks (axes, grids, backgrounds) using merged scales
        let guide_marks = self
            .create_guide_marks(
                &merged_scales,
                plot_area_width,
                plot_area_height,
                plot_bounds,
                params,
                ctx,
            )
            .await?;

        // Create legends
        let legend_marks = self
            .create_legends_with_layout(scales, &layout.taffy_layout, ctx, params)
            .await?;

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

    /// Build plot components with explicit dimensions and scale provider (recursive entry point)
    ///
    /// This method supports both top-level plots and subplots by accepting:
    /// - Explicit dimensions (canvas size or plot area size, controlled by `dimensions_are_plot_area`)
    /// - A scale provider (build new scales or use shared scales from parent)
    /// - Evaluation mode (measure overflow or full render)
    /// - Optional data override (for faceted subplots)
    ///
    /// When `dimensions_are_plot_area` is false (canvas mode), the dimensions represent the full
    /// canvas and layout is computed to determine the plot area. When true (plot area mode), the
    /// dimensions represent the already-determined plot area size.
    ///
    /// This enables true recursive rendering where the same logic works at all nesting levels.
    pub async fn build_plot_components(
        &self,
        width: f32,
        height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        scale_provider: &dyn crate::plot::compiled::scale_provider::ScaleProvider,
        mode: crate::plot::compiled::EvaluationMode,
        data_override: Option<&DataFrame>,
        dimensions_are_plot_area: bool,
    ) -> Result<crate::plot::compiled::PlotComponents, AvengerChartError> {
        use crate::plot::compiled::EvaluationMode;
        use avenger_scales::scales::ConfiguredScale;
        use std::collections::HashMap;

        // 1. Merge default params with provided
        let mut merged_params = self.default_params.clone();
        merged_params.extend(params.clone());

        // 2. Inject canvas dimensions into params for media queries
        merged_params.insert(
            "width".to_string(),
            datafusion::common::ScalarValue::Float32(Some(width)),
        );
        merged_params.insert(
            "height".to_string(),
            datafusion::common::ScalarValue::Float32(Some(height)),
        );

        // 3. Determine plot area dimensions and build scales
        let (plot_area_width, plot_area_height, canvas_size, layout_opt) = if dimensions_are_plot_area {
            // Plot area mode: dimensions are already the plot area size
            // No need to build scales here - they'll be built below

            // For plot area mode, canvas size equals plot area size (no additional space for guides/legends)
            (width, height, (width, height), None)
        } else {
            // Canvas mode: dimensions are canvas size, compute layout to determine plot area
            // Build initial scales with estimated 80% of canvas for plot area
            let initial_plot_width = width * Self::INITIAL_PLOT_AREA_RATIO;
            let initial_plot_height = height * Self::INITIAL_PLOT_AREA_RATIO;
            let initial_scales = scale_provider
                .build_scales(initial_plot_width, initial_plot_height, ctx, &merged_params)
                .await?;

            // Compute layout with initial scales
            let layout = self
                .compute_layout(
                    width,
                    height,
                    &initial_scales,
                    ctx,
                    &merged_params,
                )
                .await?;

            let plot_bounds = layout.plot_area_bounds();
            (plot_bounds.width, plot_bounds.height, layout.canvas_size, Some(layout))
        };

        // 4. Build final scales with actual plot area dimensions
        let final_scales = scale_provider
            .build_scales(plot_area_width, plot_area_height, ctx, &merged_params)
            .await?;

        // Get clip region
        let clip = if let Some(ref guide) = self.compiled_guide {
            let configured_scales: HashMap<String, ConfiguredScale> = final_scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            guide.get_clip(plot_area_width, plot_area_height, &configured_scales)
        } else {
            avenger_scenegraph::marks::group::Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: plot_area_width,
                height: plot_area_height,
            }
        };

        match mode {
            EvaluationMode::Measure => {
                // 6a. Measure mode: compute overflow, return minimal output
                let overflow = self
                    .measure_guide_overflow_with_scales(
                        &final_scales,
                        plot_area_width,
                        plot_area_height,
                        ctx,
                        &merged_params,
                    )
                    .await?;

                Ok(crate::plot::compiled::PlotComponents {
                    data_marks: vec![],
                    guide_marks: vec![],
                    legend_marks: vec![],
                    title_marks: vec![],
                    subtitle_marks: vec![],
                    plot_bounds: crate::layout::LayoutBounds {
                        x: 0.0,
                        y: 0.0,
                        width: plot_area_width,
                        height: plot_area_height,
                    },
                    clip,
                    size: canvas_size,
                    size_is_canvas: !dimensions_are_plot_area,
                    overflow: Some(overflow),
                    debug_marks: Vec::new(), // No debug marks in Measure mode
                })
            }

            EvaluationMode::Render => {
                // 6b. Render mode: full component evaluation
                // If no data_override, marks will use their own internal data
                // (facets pass filtered data here, top-level plots pass None)
                let df_opt = data_override;

                // Evaluate marks (with optional data override for facets)
                let mut data_marks = Vec::new();
                let mut layout_infos = Vec::new();
                for mark in &self.marks {
                    let (marks, layout_info) = self
                        .evaluate_mark_with_plot_df(
                            mark.as_ref(),
                            &final_scales,
                            plot_area_width,
                            plot_area_height,
                            ctx,
                            &merged_params,
                            df_opt,
                        )
                        .await?;
                    data_marks.extend(marks);
                    layout_infos.push(layout_info);
                }

                // Merge scale updates from layout info
                let merged_scales = crate::layout::merge_scale_updates(&final_scales, &layout_infos);

                // Create guide marks and other components based on mode
                let (plot_bounds_struct, guide_marks, legend_marks, title_marks, subtitle_marks, debug_marks) =
                    if let Some(layout) = layout_opt {
                        // Canvas mode: Full layout with guides, legends, titles
                        let plot_bounds = layout.plot_area_bounds();
                        let plot_bounds_struct = crate::layout::LayoutBounds {
                            x: plot_bounds.x,
                            y: plot_bounds.y,
                            width: plot_area_width,
                            height: plot_area_height,
                        };

                        let guide_marks = self
                            .create_guide_marks(
                                &merged_scales,
                                plot_area_width,
                                plot_area_height,
                                &plot_bounds_struct,
                                &merged_params,
                                ctx,
                            )
                            .await?;

                        let legend_marks = self
                            .create_legends_with_layout(&final_scales, &layout.taffy_layout, ctx, &merged_params)
                            .await?;

                        let title_marks = if let Some(title_bounds) = &layout.taffy_layout.title {
                            self.create_title(Some(*title_bounds), ctx, &merged_params)
                                .await?
                        } else {
                            Vec::new()
                        };

                        let subtitle_marks = if let Some(subtitle_bounds) = &layout.taffy_layout.subtitle {
                            self.create_subtitle(Some(*subtitle_bounds), ctx, &merged_params)
                                .await?
                        } else {
                            Vec::new()
                        };

                        let mut debug_marks = vec![];
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            debug_marks.extend(crate::render::debug::create_debug_layout_rects(
                                &layout.taffy_layout,
                                None, // Use default magenta color
                                None, // Use default stroke width (1.0)
                                None, // Use default z-index (20)
                                false, // Don't flip label alignment
                            ));
                        }

                        (plot_bounds_struct, guide_marks, legend_marks, title_marks, subtitle_marks, debug_marks)
                    } else {
                        // Plot area mode: No legends or titles (subplots don't need these)
                        // But we DO want debug marks for subplots (with cyan color to distinguish from outer plot)
                        let plot_bounds_struct = crate::layout::LayoutBounds {
                            x: 0.0,
                            y: 0.0,
                            width: plot_area_width,
                            height: plot_area_height,
                        };

                        let guide_marks = self
                            .create_guide_marks(
                                &merged_scales,
                                plot_area_width,
                                plot_area_height,
                                &plot_bounds_struct,
                                &merged_params,
                                ctx,
                            )
                            .await?;

                        let mut debug_marks = vec![];
                        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                            // Create a minimal layout result for the plot area
                            let minimal_layout = crate::layout::LayoutResult {
                                plot_area: crate::layout::LayoutBounds {
                                    x: 0.0,
                                    y: 0.0,
                                    width: plot_area_width,
                                    height: plot_area_height,
                                },
                                guide_overflows: std::collections::HashMap::new(),
                                legends: indexmap::IndexMap::new(),
                                title: None,
                                subtitle: None,
                            };
                            debug_marks.extend(crate::render::debug::create_debug_layout_rects(
                                &minimal_layout,
                                Some([0.0, 1.0, 1.0, 0.7]), // Cyan color for subplot debug marks
                                Some(0.5), // Thinner lines (0.5 instead of 1.0)
                                Some(100), // Higher z-index to render on top
                                true, // Flip label alignment to avoid overlap with outer plot labels
                            ));
                        }

                        (plot_bounds_struct, guide_marks, Vec::new(), Vec::new(), Vec::new(), debug_marks)
                    };

                // Debug marks are kept separate - they're in absolute canvas coordinates
                // and should not be translated with the data marks group

                Ok(crate::plot::compiled::PlotComponents {
                    data_marks,
                    guide_marks,
                    legend_marks,
                    title_marks,
                    subtitle_marks,
                    plot_bounds: plot_bounds_struct,
                    clip,
                    size: canvas_size,
                    size_is_canvas: !dimensions_are_plot_area,
                    overflow: None,
                    debug_marks, // Separate field for absolute-positioned debug marks
                })
            }
        }
    }

    /// Evaluate the plot to a scene graph
    pub async fn evaluate(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, datafusion::common::ScalarValue>>,
    ) -> Result<crate::render::EvaluatedPlot, AvengerChartError> {
        use avenger_scenegraph::marks::group::SceneGroup;
        use avenger_scenegraph::scene_graph::SceneGraph;
        use crate::serialization::LogicalExprNodeExt;
        use datafusion_proto::protobuf::LogicalExprNode;

        // 1. Merge provided params with defaults
        let merged_params = if let Some(provided) = params {
            let mut merged = self.default_params.clone();
            merged.extend(provided);
            merged
        } else {
            self.default_params.clone()
        };

        // 2. Evaluate canvas dimensions from layout spec
        let (estimated_width, estimated_height) = match &self.layout_spec.canvas {
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

        let canvas_width = estimated_width.unwrap_or(400.0);
        let canvas_height = estimated_height.unwrap_or(300.0);

        // 3. Build scale provider
        use crate::plot::compiled::scales::build_scale_builder_from_marks;
        let scale_builder = build_scale_builder_from_marks(
            &self.marks,
            &self.scale_specs,
            &self.coord_transform,
            &self.data,
            None,
            ctx,
            &merged_params,
            self.get_theme().as_ref(),
        )
        .await?;

        let provider = crate::plot::compiled::scale_provider::DefaultScaleProvider {
            builder: &scale_builder,
            plot: self,
        };

        // 4. Call recursive evaluation method
        let components = self
            .build_plot_components(
                canvas_width,
                canvas_height,
                ctx,
                &merged_params,
                &provider,
                crate::plot::compiled::EvaluationMode::Render,
                None, // No data override for top-level plots
                false, // Canvas mode: dimensions are canvas size
            )
            .await?;

        // 5. Compose scene graph from components
        let plot_bounds = components.plot_bounds;
        let (final_width, final_height) = components.size;

        // Create clipped data marks group
        let data_marks_group = SceneGroup {
            origin: [plot_bounds.x, plot_bounds.y],
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };

        let mut all_marks = Vec::new();

        // Add background rect if theme specifies one
        let theme = self.get_theme();
        let canvas_ctx = crate::theme::ThemeContext::new("canvas", merged_params.clone());
        if let Some(color) = theme
            .query(&canvas_ctx, "background-color")
            .and_then(|v| v.as_color_array())
        {
            use avenger_common::types::ColorOrGradient;
            use avenger_scenegraph::marks::rect::SceneRectMark;

            let background_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(final_width.into()),
                height: Some(final_height.into()),
                fill: ColorOrGradient::Color(color).into(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0]).into(),
                stroke_width: 0.0.into(),
                zindex: Some(-100),
                ..Default::default()
            };
            all_marks.push(avenger_scenegraph::marks::mark::SceneMark::Rect(background_rect));
        }

        // Add marks in proper z-order
        all_marks.push(avenger_scenegraph::marks::mark::SceneMark::Group(data_marks_group));
        all_marks.extend(components.guide_marks);
        all_marks.extend(components.legend_marks);
        all_marks.extend(components.title_marks);
        all_marks.extend(components.subtitle_marks);

        // Debug marks are added at root level (not inside data_marks_group)
        // because they use absolute canvas coordinates and should not be translated
        all_marks.extend(components.debug_marks);

        // Wrap in root group
        let root_group = SceneGroup {
            marks: all_marks,
            ..Default::default()
        };

        // Create scene graph
        let scene_graph = SceneGraph {
            marks: vec![avenger_scenegraph::marks::mark::SceneMark::Group(root_group)],
            width: final_width,
            height: final_height,
            origin: [0.0, 0.0],
        };

        // Build spatial index
        let rtree = avenger_geometry::rtree::SceneGraphRTree::from_scene_graph(&scene_graph);

        Ok(crate::render::EvaluatedPlot {
            scene_graph,
            rtree: Some(rtree),
        })
    }
}
