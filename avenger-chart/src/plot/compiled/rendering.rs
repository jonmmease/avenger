//! Rendering pipeline for CompiledPlot

use std::collections::HashMap;
use std::sync::Arc;

use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use avenger_scales::scales::ConfiguredScale;

use crate::channel::value::{ChannelValue, ConditionalValue};
use crate::coords::coordinate_overflow_for_guides;
use crate::error::AvengerChartError;
use crate::facet::evaluated_facet_tree::EvaluatedFacetTree;
use crate::marks::CompiledMark;
use crate::render::{EvaluationContext, RenderContext, RenderState};
use crate::scales::ConfiguredScaleWithSpec;
use crate::serialization::{LogicalExprNodeExt, LogicalPlanNodeExt};

use super::CompiledPlot;
use super::expr_eval::evaluate_f32_expr;

/// Prepared data for mark evaluation (measure or render pass)
struct PreparedMarkData {
    /// Array data batch (multiple rows), or None if all channels are scalar
    data_batch: Option<datafusion::arrow::record_batch::RecordBatch>,
    /// Scalar data batch (single row) for channels that don't vary per mark
    scalar_batch: datafusion::arrow::record_batch::RecordBatch,
    /// Render state with plot dimensions and scales
    render_state: RenderState,
}

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

    /// Evaluate a single mark with an optional provided plot-level DataFrame fallback.
    /// If `provided_plot_df` is Some, it is used when the mark has no explicit data and
    /// the channels reference columns. Otherwise, falls back to this CompiledPlot's plot-level data.
    /// Prepare data batches and context for mark evaluation.
    /// This is shared between measure and render passes.
    async fn prepare_mark_data(
        &self,
        mark: &dyn CompiledMark,
        eval_ctx: &EvaluationContext,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        provided_plot_df: Option<&datafusion::dataframe::DataFrame>,
    ) -> Result<Option<PreparedMarkData>, AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let params = &eval_ctx.params;

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
        // Priority 1: Mark's own data (e.g., reference lines) - should be used in full for all facets
        // Priority 2: Parent facet's filtered data - for nested marks without their own data
        // Priority 3: Plot-level data - fallback for top-level marks
        let df_ref = if let Some(mark_df) = mark.data_context().dataframe_with_context(ctx) {
            Some(mark_df)
        } else if let Some(df_override) = provided_plot_df.cloned() {
            Some(df_override)
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
        let data_batch = if mark.wants_full_data_batch() {
            // For container marks (facets): preserve ALL columns for nested marks
            // This works whether data comes from provided_plot_df (nested) or self.data (top-level)
            let datafusion_params = crate::utils::params_to_datafusion(params);
            let batch = if let Some(param_values) = datafusion_params {
                (*df)
                    .clone()
                    .with_param_values(param_values)?
                    .collect()
                    .await?
            } else {
                (*df).clone().collect().await?
            };

            if batch.is_empty() {
                // Return empty batch WITH SCHEMA for facets (enables key extraction)
                let arrow_schema = std::sync::Arc::new(df.schema().as_arrow().clone());
                Some(datafusion::arrow::record_batch::RecordBatch::new_empty(
                    arrow_schema,
                ))
            } else {
                use datafusion::arrow::compute::concat_batches;
                let schema = batch[0].schema();
                Some(concat_batches(&schema, &batch)?)
            }
        } else if has_array_data && !mark.wants_full_data_batch() {
            // Normal path (non-facet marks): select only needed channels
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
                return Ok(None);
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

        let render_state = RenderState::new(plot_width, plot_height, scales.clone());

        Ok(Some(PreparedMarkData {
            data_batch,
            scalar_batch,
            render_state,
        }))
    }

    /// Render a single mark to scene marks.
    pub(super) async fn render_mark_with_plot_df(
        &self,
        mark: &dyn CompiledMark,
        eval_ctx: &EvaluationContext,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        provided_plot_df: Option<&datafusion::dataframe::DataFrame>,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: &dyn crate::coords::CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let prepared = self
            .prepare_mark_data(
                mark,
                eval_ctx,
                scales,
                plot_width,
                plot_height,
                provided_plot_df,
            )
            .await?;

        let Some(prepared) = prepared else {
            return Ok(vec![]);
        };

        let render_ctx = RenderContext::new(
            eval_ctx,
            &prepared.render_state,
            facet_path,
            coord_measurement,
        );
        let coord_transform = self.coord_transform.clone_box();
        mark.render_from_data(
            prepared.data_batch.as_ref(),
            &prepared.scalar_batch,
            &render_ctx,
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
        data_override: Option<&DataFrame>,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
        coord_measurement: &dyn crate::coords::CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Use the pre-built guide renderer if available
        if let Some(compiled_guide) = &self.compiled_guide {
            let theme = self.get_theme();
            // Extract ConfiguredScale from ConfiguredScaleWithSpec for guide renderer
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();

            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                if let Some(y_scale) = configured_scales.get("y") {
                    let domain = y_scale.domain();
                    if let Some(float_arr) =
                        domain.as_any().downcast_ref::<arrow::array::Float32Array>()
                    {
                        let vals: Vec<f32> = float_arr.iter().filter_map(|v| v).collect();
                        eprintln!("create_guide_marks: y scale domain = {:?}", vals);
                    } else if let Some(float_arr) =
                        domain.as_any().downcast_ref::<arrow::array::Float64Array>()
                    {
                        let vals: Vec<f64> = float_arr.iter().filter_map(|v| v).collect();
                        eprintln!("create_guide_marks: y scale domain = {:?}", vals);
                    } else {
                        eprintln!(
                            "create_guide_marks: y scale domain type = {:?}",
                            domain.data_type()
                        );
                    }
                }
            }

            compiled_guide
                .evaluate(
                    &configured_scales,
                    plot_width,
                    plot_height,
                    plot_bounds,
                    theme.as_ref(),
                    params,
                    ctx,
                    data_override,
                    facet_tree,
                    facet_path,
                    coord_measurement,
                )
                .await
        } else {
            // No guide renderer available
            Ok(vec![])
        }
    }

    /// Compute layout with an evaluated layout specification.
    ///
    /// This unified method handles both canvas-mode and plot-area-mode layouts
    /// based on the provided EvaluatedLayoutSpec.
    pub(super) async fn compute_layout_with_spec(
        &self,
        layout_spec: &crate::layout::EvaluatedLayoutSpec,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        ctx: &SessionContext,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&DataFrame>,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
    ) -> Result<crate::render::LayoutSolution, AvengerChartError> {
        use crate::layout::{ChartLayout, EvaluatedSizeMode};

        // Check for required positional scales before measuring overflow
        self.validate_positional_scales_exist(scales)?;

        // Determine if this is plot-area mode (for overflow estimation dimensions)
        let is_plot_area_mode = matches!(
            (&layout_spec.canvas, &layout_spec.plot_area),
            (EvaluatedSizeMode::Auto, EvaluatedSizeMode::Fixed { .. })
        );

        // Get dimensions for overflow estimation
        let (estimate_width, estimate_height) = if is_plot_area_mode {
            // Plot area mode: use exact plot dimensions
            match &layout_spec.plot_area {
                EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                _ => (400.0, 300.0), // Fallback
            }
        } else {
            // Canvas mode: estimate plot area as fraction of canvas
            let (canvas_w, canvas_h) = match &layout_spec.canvas {
                EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                EvaluatedSizeMode::Width(w) => (*w, 300.0),
                EvaluatedSizeMode::Height(h) => (400.0, *h),
                EvaluatedSizeMode::Auto => (400.0, 300.0),
            };
            (
                canvas_w * Self::INITIAL_PLOT_AREA_RATIO,
                canvas_h * Self::INITIAL_PLOT_AREA_RATIO,
            )
        };

        // Measure guide overflow (axis tick labels, titles, etc.)
        let overflow = if let Some(compiled_guide) = &self.compiled_guide {
            let theme = self.get_theme();
            let configured_scales: HashMap<String, ConfiguredScale> = scales
                .iter()
                .map(|(k, v)| (k.clone(), v.configured().clone()))
                .collect();
            compiled_guide
                .measure_overflow(
                    &configured_scales,
                    estimate_width,
                    estimate_height,
                    theme.as_ref(),
                    params,
                    data_override,
                    ctx,
                    facet_tree,
                    facet_path,
                    None, // No coord_measurement yet (first pass)
                )
                .await?
        } else {
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
            width: estimate_width,
            height: estimate_height,
        };
        let legend_measurements = self
            .prepare_legend_measurements(&legends_map, scales, available_size, ctx, params)
            .await?;

        // Create and compute layout
        let mut layout = ChartLayout::new(
            &overflow,
            layout_spec,
            self.get_title(),
            self.get_subtitle(),
            self.get_theme().as_ref(),
            &legend_measurements,
            ctx,
            params,
        )
        .await?;

        let mut result = layout.compute(layout_spec)?;

        // Add legend dimensions to total_overflow so facets can position their labels correctly
        // (ChartLayout.compute() sets total_overflow to guide-only; we add legend space here)
        for (channel, measurement) in legend_measurements.iter() {
            match measurement.position {
                crate::legend::LegendPosition::Left => {
                    result.total_overflow.left += measurement.size.width;
                }
                crate::legend::LegendPosition::Right => {
                    result.total_overflow.right += measurement.size.width;
                }
                crate::legend::LegendPosition::Top => {
                    result.total_overflow.top += measurement.size.height;
                }
                crate::legend::LegendPosition::Bottom => {
                    result.total_overflow.bottom += measurement.size.height;
                }
            }
            let _ = channel; // suppress unused warning
        }

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

    /// Extract dimensions and determine layout mode from evaluated layout spec.
    ///
    /// Returns (width, height, is_plot_area_mode) where:
    /// - Plot area mode (`is_plot_area_mode=true`): canvas Auto + plot_area Fixed
    /// - Canvas mode (`is_plot_area_mode=false`): canvas specified, compute plot area later
    fn resolve_dimensions_from_spec(
        layout_spec: &crate::layout::EvaluatedLayoutSpec,
    ) -> (f32, f32, bool) {
        use crate::layout::EvaluatedSizeMode;

        const DEFAULT_WIDTH: f32 = 400.0;
        const DEFAULT_HEIGHT: f32 = 300.0;

        // Determine if this is plot area mode (canvas Auto + plot_area Fixed)
        // vs canvas mode (canvas specified, plot_area may or may not be)
        let is_plot_area_mode = matches!(
            (&layout_spec.canvas, &layout_spec.plot_area),
            (EvaluatedSizeMode::Auto, EvaluatedSizeMode::Fixed { .. })
        );

        let (width, height) = if is_plot_area_mode {
            // Plot area mode: use plot_area dimensions
            match &layout_spec.plot_area {
                EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
            }
        } else {
            // Canvas mode: use canvas dimensions (or defaults)
            match &layout_spec.canvas {
                EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                EvaluatedSizeMode::Width(w) => (*w, DEFAULT_HEIGHT),
                EvaluatedSizeMode::Height(h) => (DEFAULT_WIDTH, *h),
                EvaluatedSizeMode::Auto => {
                    // Canvas auto but not plot_area fixed - use plot_area or defaults
                    match &layout_spec.plot_area {
                        EvaluatedSizeMode::Fixed { width, height } => (*width, *height),
                        EvaluatedSizeMode::Width(w) => (*w, DEFAULT_HEIGHT),
                        EvaluatedSizeMode::Height(h) => (DEFAULT_WIDTH, *h),
                        EvaluatedSizeMode::Auto => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
                    }
                }
            }
        };

        (width, height, is_plot_area_mode)
    }

    /// Determine clip region from guide or default to plot area rect.
    fn get_clip_region(
        &self,
        scales: &std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> avenger_scenegraph::marks::group::Clip {
        use avenger_scales::scales::ConfiguredScale;
        use std::collections::HashMap;

        if let Some(ref guide) = self.compiled_guide {
            let configured_scales: HashMap<String, ConfiguredScale> = scales
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
        }
    }

    /// Compute layout and determine plot area dimensions.
    ///
    /// Handles two modes:
    /// - **Plot area mode** (`is_plot_area_mode=true`): Uses provided dimensions as plot area,
    ///   computes layout to determine canvas size
    /// - **Canvas mode** (`is_plot_area_mode=false`): Uses provided dimensions as canvas,
    ///   computes layout to determine plot area from overflow
    ///
    /// Returns (plot_area_width, plot_area_height, canvas_size, layout)
    async fn compute_layout_and_dimensions(
        &self,
        is_plot_area_mode: bool,
        width: f32,
        height: f32,
        layout_spec: &crate::layout::EvaluatedLayoutSpec,
        scale_provider: &dyn crate::plot::compiled::scale_provider::ScaleProvider,
        ctx: &datafusion::prelude::SessionContext,
        merged_params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&DataFrame>,
        facet_tree: &crate::facet::evaluated_facet_tree::EvaluatedFacetTree,
        facet_path: &[datafusion::common::ScalarValue],
    ) -> Result<(f32, f32, (f32, f32), crate::render::LayoutSolution), AvengerChartError>
    {
        if is_plot_area_mode {
            // Plot area mode: dimensions specify the plot area size
            let plot_area_width = width;
            let plot_area_height = height;

            let initial_scales = scale_provider
                .build_scales(plot_area_width, plot_area_height, ctx, merged_params)
                .await?;

            let layout = self
                .compute_layout_with_spec(
                    layout_spec,
                    &initial_scales,
                    ctx,
                    merged_params,
                    data_override,
                    facet_tree,
                    facet_path,
                )
                .await?;

            Ok((plot_area_width, plot_area_height, layout.canvas_size, layout))
        } else {
            // Canvas mode: dimensions are canvas size, compute layout to determine plot area
            let initial_plot_width = width * Self::INITIAL_PLOT_AREA_RATIO;
            let initial_plot_height = height * Self::INITIAL_PLOT_AREA_RATIO;

            let initial_scales = scale_provider
                .build_scales(initial_plot_width, initial_plot_height, ctx, merged_params)
                .await?;

            let layout = self
                .compute_layout_with_spec(
                    layout_spec,
                    &initial_scales,
                    ctx,
                    merged_params,
                    data_override,
                    facet_tree,
                    facet_path,
                )
                .await?;

            let plot_bounds = layout.plot_area_bounds();
            Ok((
                plot_bounds.width,
                plot_bounds.height,
                layout.canvas_size,
                layout,
            ))
        }
    }

    /// Measure coordinate system layout (e.g., facet cell positioning).
    ///
    /// This allows coordinate systems to compute layout data that's available
    /// to both guides and marks during rendering.
    ///
    /// Note: The caller is responsible for calling `apply_scale_adjustments()`
    /// on the returned measurement to update scales with coord-derived values.
    async fn measure_coord_system(
        &self,
        scales: &std::collections::HashMap<String, crate::scales::ConfiguredScaleWithSpec>,
        plot_area_width: f32,
        plot_area_height: f32,
        params_with_dims: &crate::render::EvaluationContext,
        data_override: Option<&DataFrame>,
        facet_path: &[datafusion::common::ScalarValue],
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<Box<dyn crate::coords::CoordMeasurement>, AvengerChartError> {
        // Get data for coord measurement: use data_override if provided (nested facets),
        // otherwise use the plot's own data (top-level).
        let plot_data = if data_override.is_some() {
            None
        } else {
            self.data.as_ref().and_then(|node| {
                node.to_logical_plan(ctx)
                    .ok()
                    .map(|plan| DataFrame::new(ctx.state().clone(), plan))
            })
        };
        let coord_data = data_override.or(plot_data.as_ref());

        self.coord_transform
            .measure(
                scales,
                plot_area_width,
                plot_area_height,
                params_with_dims,
                coord_data,
                &self.marks,
                facet_path,
            )
            .await
    }

    /// Measure plot components without rendering (for layout coordination).
    ///
    /// This method performs all setup and measurement needed for layout coordination:
    /// - Resolves plot area dimensions from layout_spec (see Layout Modes below)
    /// - Builds scales with the provided scale_provider
    /// - Calls coordinate system measure (for facet cell layout)
    /// - Computes overflow requirements for guide elements
    ///
    /// Returns `ComponentsMeasurement` for parent layout coordination and rendering.
    ///
    /// # Layout Modes
    /// The `layout_spec` determines how dimensions are resolved:
    /// - **Canvas mode** (`canvas: Fixed`, `plot_area: Auto`): Plot area is computed
    ///   by subtracting legend/title overflow from canvas dimensions
    /// - **Plot area mode** (`canvas: Auto`, `plot_area: Fixed`): Plot area dimensions
    ///   are used directly; facet subplots always use this mode
    ///
    /// # Arguments
    /// * `eval_ctx` - Evaluation context with theme, session, params, and facet tree
    /// * `layout_spec` - Evaluated layout specification determining canvas vs plot area mode
    /// * `scale_provider` - Provider for building scales (prebuilt for shared scales, or from-data)
    /// * `data_override` - Optional data override for faceted subplots (filtered data)
    /// * `facet_path` - Path of values identifying current cell in facet hierarchy (e.g., `["East", "Eng"]`).
    ///   Used for tree navigation, data filtering, and axis visibility checks.
    pub(crate) async fn measure_plot_components(
        &self,
        eval_ctx: &EvaluationContext,
        layout_spec: &crate::layout::EvaluatedLayoutSpec,
        scale_provider: &dyn crate::plot::compiled::scale_provider::ScaleProvider,
        data_override: Option<&DataFrame>,
        facet_path: &[datafusion::common::ScalarValue],
    ) -> Result<crate::plot::compiled::ComponentsMeasurement, AvengerChartError> {
        let ctx = &*eval_ctx.session_context;
        let facet_tree = &*eval_ctx.facet_tree;

        // Phase 1: Extract dimensions from layout spec
        let (width, height, is_plot_area_mode) = Self::resolve_dimensions_from_spec(layout_spec);

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "measure_plot_components: width={:.3} height={:.3} is_plot_area_mode={}",
                width, height, is_plot_area_mode
            );
        }

        // Add dimensions to params for media queries
        let params_with_dims = eval_ctx.with_dimension_params(width, height);
        let merged_params = params_with_dims.params.clone();

        // Phase 2: Compute layout and determine plot area dimensions
        let (plot_area_width, plot_area_height, canvas_size, layout) = self
            .compute_layout_and_dimensions(
                is_plot_area_mode,
                width,
                height,
                layout_spec,
                scale_provider,
                ctx,
                &merged_params,
                data_override,
                facet_tree,
                facet_path,
            )
            .await?;

        // Phase 3: Build final scales with actual plot area dimensions
        let mut final_scales = scale_provider
            .build_scales(plot_area_width, plot_area_height, ctx, &merged_params)
            .await?;

        // Phase 4: Coordinate system measurement
        let coord_measurement = self
            .measure_coord_system(
                &final_scales,
                plot_area_width,
                plot_area_height,
                &params_with_dims,
                data_override,
                facet_path,
                ctx,
            )
            .await?;

        // Apply scale adjustments from coord measurement (stays in main function
        // because it mutates final_scales which is used by later phases)
        coord_measurement.apply_scale_adjustments(&mut final_scales);

        // Phase 5: Get clip region from guide
        let clip = self.get_clip_region(&final_scales, plot_area_width, plot_area_height);

        // Note: Overflow info is available via layout.overflow (guide only) and
        // layout.total_overflow (guide + legends), computed during layout phase.

        Ok(crate::plot::compiled::ComponentsMeasurement {
            coord_measurement,
            scales: final_scales,
            plot_area_width,
            plot_area_height,
            canvas_size,
            clip,
            layout,
            params: merged_params,
        })
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
    /// Build plot components using pre-computed measurement
    ///
    /// This method renders all marks using the provided measurement results.
    /// Call `measure_plot_components()` first to get the measurement.
    ///
    /// # Arguments
    /// * `eval_ctx` - Evaluation context
    /// * `measurement` - Pre-computed measurement from `measure_plot_components()`
    /// * `data_override` - Optional data override for faceted subplots
    /// * `dimensions_are_plot_area` - If true, dimensions are plot area; if false, canvas
    /// * `facet_path` - Current cell path in facet hierarchy as values (for axis visibility).
    ///   Empty slice when not in a facet cell.
    pub async fn build_plot_components(
        &self,
        eval_ctx: &EvaluationContext,
        measurement: &crate::plot::compiled::ComponentsMeasurement,
        data_override: Option<&DataFrame>,
        dimensions_are_plot_area: bool,
        facet_path: &[datafusion::common::ScalarValue],
    ) -> Result<crate::plot::compiled::PlotComponents, AvengerChartError> {
        let ctx = &*eval_ctx.session_context;

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "build_plot_components: plot_area={}x{} dimensions_are_plot_area={}",
                measurement.plot_area_width, measurement.plot_area_height, dimensions_are_plot_area
            );
        }

        // Render components using measurement
        let layout_solution = measurement.layout.clone();

        // Extract values from measurement for convenience
        let plot_area_width = measurement.plot_area_width;
        let plot_area_height = measurement.plot_area_height;
        let canvas_size = measurement.canvas_size;
        let clip = measurement.clip.clone();
        let merged_params = measurement.params.clone();
        let merged_scales = measurement.scales.clone();

        // Create context with measurement's params (which include dimensions)
        let mark_eval_ctx = eval_ctx.with_params(merged_params.clone());

        // Render marks using pre-computed measurements from measurement
        let coord_measurement_ref: &dyn crate::coords::CoordMeasurement =
            measurement.coord_measurement.as_ref();

        let mut data_marks = Vec::new();
        for mark in &self.marks {
            let marks = self
                .render_mark_with_plot_df(
                    mark.as_ref(),
                    &mark_eval_ctx,
                    &merged_scales,
                    plot_area_width,
                    plot_area_height,
                    data_override,
                    facet_path,
                    coord_measurement_ref,
                )
                .await?;
            data_marks.extend(marks);
        }

        // Create guide marks and other components
        let (
            plot_bounds_struct,
            guide_marks,
            legend_marks,
            title_marks,
            subtitle_marks,
            debug_marks,
        ) = {
            let layout_initial = layout_solution;
            // Check if this is a top-level plot (canvas mode) or subplot (plot area mode)
            if !dimensions_are_plot_area {
                // Canvas mode: Full layout with guides, legends, titles
                // Always re-measure overflow with the final plot-area size and final scales,
                // then rebuild the outer layout so it matches subplots.
                let theme = self.get_theme();
                // Use merged_scales that include facet-driven padding updates
                let configured_scales: HashMap<String, ConfiguredScale> = merged_scales
                    .iter()
                    .map(|(k, v)| (k.clone(), v.configured().clone()))
                    .collect();
                let pb = layout_initial.plot_area_bounds();
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "Calling guide measure_overflow with pb.width={} pb.height={}",
                        pb.width, pb.height
                    );
                }
                let overflow_final = if let Some(ref compiled_guide) = self.compiled_guide {
                    compiled_guide
                        .measure_overflow(
                            &configured_scales,
                            pb.width,
                            pb.height,
                            theme.as_ref(),
                            &merged_params,
                            data_override,
                            ctx,
                            &*eval_ctx.facet_tree,
                            facet_path,
                            Some(coord_measurement_ref), // Second pass - provide for efficiency
                        )
                        .await?
                } else {
                    crate::guide::OverflowSpaceRequirement::default()
                };

                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    eprintln!(
                        "SECOND-PASS overflow (outer): left={:.3} right={:.3} top={:.3} bottom={:.3}",
                        overflow_final.left,
                        overflow_final.right,
                        overflow_final.top,
                        overflow_final.bottom
                    );
                }

                // Rebuild layout unconditionally with the second-pass overflow
                use crate::layout::ChartLayout;
                let evaluated_spec2 = evaluate_layout_spec(
                    self.get_layout_spec(),
                    ctx,
                    &merged_params,
                    theme.as_ref(),
                )
                .await?;
                let all_legends2 = self.get_legends_with_theme(&merged_scales, ctx, &merged_params);
                let (_channel_groups2, legends_map2) = self
                    .merge_legend_channels(&all_legends2, &merged_scales, ctx, &merged_params)
                    .await?;
                let legend_measurements2 = self
                    .prepare_legend_measurements(
                        &legends_map2,
                        &merged_scales,
                        taffy::Size {
                            width: plot_area_width,
                            height: plot_area_height,
                        },
                        ctx,
                        &merged_params,
                    )
                    .await?;
                let layout = {
                    let mut l = ChartLayout::new(
                        &overflow_final,
                        &evaluated_spec2,
                        self.get_title(),
                        self.get_subtitle(),
                        theme.as_ref(),
                        &legend_measurements2,
                        ctx,
                        &merged_params,
                    )
                    .await?;
                    l.compute(&evaluated_spec2)?
                };

                let plot_bounds = layout.plot_area_bounds();
                // Use original plot area dimensions to stay consistent with evaluated marks
                // Marks were evaluated with plot_area_width/height from first layout
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
                        data_override,
                        eval_ctx.facet_tree.as_ref(),
                        facet_path,
                        coord_measurement_ref,
                    )
                    .await?;

                let legend_marks = self
                    .create_legends_with_layout(
                        &merged_scales,
                        &layout.taffy_layout,
                        ctx,
                        &merged_params,
                    )
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
                    // Use INITIAL layout for debug visualization - this matches the dimensions used to position marks
                    // Marks were evaluated with plot_area_width/height from layout_initial
                    eprintln!(
                        "INITIAL layout plot width: {}",
                        layout_initial.taffy_layout.plot_area.width
                    );
                    eprintln!(
                        "FINAL layout plot width: {}",
                        layout.taffy_layout.plot_area.width
                    );
                    debug_marks.extend(crate::render::debug::create_debug_layout_rects(
                        &layout_initial.taffy_layout, // Use initial layout that matches mark positioning
                        None,                         // Use default magenta color
                        None,                         // Use default stroke width (1.0)
                        None,                         // Use default z-index (20)
                        false,                        // Don't flip label alignment
                    ));
                }

                (
                    plot_bounds_struct,
                    guide_marks,
                    legend_marks,
                    title_marks,
                    subtitle_marks,
                    debug_marks,
                )
            } else {
                // Plot area mode (subplots): plot_bounds.y = 0
                // For nested facets, the FacetColGuide computes adjusted_plot_bounds.y as
                // a NEGATIVE value, placing labels ABOVE the subplot origin (in the overflow
                // region). Data marks render at y = 0 to plot_height.
                // The subplot's overflow region is at negative y, not positive.
                let plot_bounds_struct = crate::layout::LayoutBounds {
                    x: 0.0,
                    y: 0.0,
                    width: plot_area_width,
                    height: plot_area_height,
                };

                // Create guide marks
                // Pass data_override so nested facets use filtered data
                let guide_marks = self
                    .create_guide_marks(
                        &merged_scales,
                        plot_area_width,
                        plot_area_height,
                        &plot_bounds_struct,
                        &merged_params,
                        ctx,
                        data_override,
                        eval_ctx.facet_tree.as_ref(),
                        facet_path,
                        coord_measurement_ref,
                    )
                    .await?;

                // Create legend marks from the computed layout
                // Legend positions from layout include the plot area offset, but we need them at (0,0)
                let plot_bounds = layout_initial.plot_area_bounds();
                let legend_marks_raw = self
                    .create_legends_with_layout(
                        &merged_scales,
                        &layout_initial.taffy_layout,
                        ctx,
                        &merged_params,
                    )
                    .await?;

                // Translate legend marks to be relative to (0, 0) instead of plot area offset
                let legend_marks: Vec<_> = legend_marks_raw
                    .into_iter()
                    .map(|mark| {
                        use avenger_scenegraph::marks::mark::SceneMark;
                        match mark {
                            SceneMark::Group(mut group) => {
                                // Adjust group origin by subtracting plot area offset
                                group.origin = [
                                    group.origin[0] - plot_bounds.x,
                                    group.origin[1] - plot_bounds.y,
                                ]
                                .into();
                                SceneMark::Group(group)
                            }
                            _ => mark, // Other mark types shouldn't be at this level
                        }
                    })
                    .collect();

                // Create title/subtitle marks if they exist in layout
                let title_marks = vec![];
                let subtitle_marks = vec![];
                // (Subplots typically don't have titles, but the layout might include them)

                let mut debug_marks = vec![];
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    // Use the actual computed layout which includes legends
                    // The layout has plot area at an offset due to overflow/legends
                    // We need to translate it to (0,0) for subplot coordinates
                    let plot_bounds = layout_initial.plot_area_bounds();

                    // Create a translated copy of the layout with plot area at (0,0)
                    let mut subplot_layout = layout_initial.taffy_layout.clone();

                    // Translate plot_area
                    subplot_layout.plot_area.x -= plot_bounds.x;
                    subplot_layout.plot_area.y -= plot_bounds.y;

                    // Translate guide_overflows
                    for (_, bounds) in subplot_layout.guide_overflows.iter_mut() {
                        bounds.x -= plot_bounds.x;
                        bounds.y -= plot_bounds.y;
                    }

                    // Translate legends
                    for (_, bounds) in subplot_layout.legends.iter_mut() {
                        bounds.x -= plot_bounds.x;
                        bounds.y -= plot_bounds.y;
                    }

                    // Compute unique color for each subplot using a simple hash
                    // of params to differentiate subplots without FacetContext
                    let subplot_color_string = {
                        // Use a simple hash based on params count for deterministic coloring
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        merged_params.len().hash(&mut hasher);
                        // Include some param values for more variation
                        for (key, _) in merged_params.iter().take(3) {
                            key.hash(&mut hasher);
                        }
                        let hash = hasher.finish();
                        let index = (hash % 6) as usize;

                        let colors = [
                            "hsla(15, 65%, 60%, 0.8)",  // Orange-red
                            "hsla(75, 65%, 60%, 0.8)",  // Yellow-green
                            "hsla(135, 65%, 60%, 0.8)", // Green
                            "hsla(195, 65%, 60%, 0.8)", // Cyan
                            "hsla(255, 65%, 60%, 0.8)", // Blue-purple
                            "hsla(315, 65%, 60%, 0.8)", // Magenta
                        ];

                        colors[index].to_string()
                    };

                    debug_marks.extend(crate::render::debug::create_debug_layout_rects(
                        &subplot_layout,
                        Some(subplot_color_string),
                        Some(1.0), // Same width as outer lines
                        Some(100), // Higher z-index to render on top
                        true,      // Flip label alignment to avoid overlap with outer plot labels
                    ));
                }

                (
                    plot_bounds_struct,
                    guide_marks,
                    legend_marks,
                    title_marks,
                    subtitle_marks,
                    debug_marks,
                )
            }
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
            debug_marks,
        })
    }

    /// Evaluate the plot to a scene graph
    pub async fn evaluate(
        &self,
        ctx: &SessionContext,
        params: Option<IndexMap<String, datafusion::common::ScalarValue>>,
    ) -> Result<crate::render::EvaluatedPlot, AvengerChartError> {
        use avenger_scenegraph::marks::group::SceneGroup;
        use avenger_scenegraph::scene_graph::SceneGraph;

        // 1. Merge provided params with defaults
        let merged_params = if let Some(provided) = params {
            let mut merged = self.default_params.clone();
            merged.extend(provided);
            merged
        } else {
            self.default_params.clone()
        };

        // 1.5. Build evaluated facet spec (pre-pass to discover partition structure)
        // This queries distinct values for each facet level, respecting scale sharing settings.
        // Used for efficient domain lookups in nested facet coordination.
        let facet_tree = Arc::new(EvaluatedFacetTree::from_compiled_plot(self, ctx).await?);

        // 2. Evaluate layout spec to get concrete dimensions
        let evaluated_layout_spec =
            evaluate_layout_spec(&self.layout_spec, ctx, &merged_params, self.get_theme().as_ref())
                .await?;

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

        let provider = crate::plot::compiled::scale_provider::DynamicScaleProvider {
            builder: &scale_builder,
            plot: self,
        };

        // Create EvaluationContext for the entire evaluation
        let eval_ctx = EvaluationContext::new(
            self.get_theme(),
            Arc::new(ctx.clone()),
            merged_params,
            facet_tree.clone(),
        );

        // 4. Measure plot components
        let measurement = self
            .measure_plot_components(
                &eval_ctx,
                &evaluated_layout_spec,
                &provider,
                None, // No data override for top-level plots
                &[],  // Empty facet path for top-level plots
            )
            .await?;

        // 4b. Coordinate nested facet overflow values globally and re-measure affected subplots
        // This ensures all facet labels at the same nesting depth are aligned
        // and subplot measurements have correct dimensions accounting for legend overflow
        let mut measurement = measurement;
        coordinate_overflow_for_guides(&mut measurement, &eval_ctx).await?;

        // 5. Build plot components using measurement
        let components = self
            .build_plot_components(
                &eval_ctx,
                &measurement,
                None,  // No data override for top-level plots
                false, // Canvas mode: dimensions are canvas size
                &[],   // Empty path for top-level plots (not in a facet cell)
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
        let canvas_ctx = crate::theme::ThemeContext::new("canvas", eval_ctx.params.clone());
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
            all_marks.push(avenger_scenegraph::marks::mark::SceneMark::Rect(
                background_rect,
            ));
        }

        // Add marks in proper z-order
        all_marks.push(avenger_scenegraph::marks::mark::SceneMark::Group(
            data_marks_group,
        ));
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
            marks: vec![avenger_scenegraph::marks::mark::SceneMark::Group(
                root_group,
            )],
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
