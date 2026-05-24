use std::{collections::HashMap, sync::Arc};

use datafusion::{
    arrow::array::{ArrayRef, Float32Array, Float64Array},
    arrow::datatypes::DataType as ArrowDataType,
    common::ScalarValue,
    prelude::SessionContext,
};
use indexmap::IndexMap;

use avenger_common::{
    types::{ColorOrGradient, StrokeCap},
    value::ScalarOrArray,
};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{
    arc::SceneArcMark, mark::SceneMark, rule::SceneRuleMark, text::SceneTextMark,
};
use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline};

use crate::{
    chart_core::{evaluate_bool_expr, evaluate_string_expr},
    error::AvengerChartError,
    layout::LayoutBounds,
    serialization::LogicalExprNodeExt,
    theme::{Theme, ThemeContext},
    utils::{ScalarValueHelpers, eval_to_scalars, params_to_datafusion},
};

pub use avenger_chart_polar::{PolarAxis, PolarAxisType, PolarDirection};

// Default tick counts for polar axes
const RADIAL_DEFAULT_TICK_COUNT: f32 = 5.0;
const ANGULAR_DEFAULT_TICK_COUNT: f32 = 8.0;

pub(crate) trait PolarAxisEvaluateExt {
    /// Extract tick values from an Arrow array as Vec<f64>.
    fn extract_tick_values(ticks: &ArrayRef) -> Vec<f64>;

    /// Get theme context for label styling.
    fn get_label_theme_values(
        theme: &Theme,
        axis_ctx: &ThemeContext,
    ) -> (String, f32, f32, [f32; 4]);

    /// Evaluate this axis to scene marks.
    async fn evaluate(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    async fn evaluate_radial_axis(
        &self,
        scale: &ConfiguredScale,
        center_x: f32,
        center_y: f32,
        max_radius: f32,
        theme: &Theme,
        axis_ctx: &ThemeContext,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;

    async fn evaluate_angular_axis(
        &self,
        scale: &ConfiguredScale,
        center_x: f32,
        center_y: f32,
        radius: f32,
        scales: &HashMap<String, ConfiguredScale>,
        theme: &Theme,
        axis_ctx: &ThemeContext,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

impl PolarAxisEvaluateExt for PolarAxis {
    /// Extract tick values from an Arrow array as Vec<f64>
    fn extract_tick_values(ticks: &ArrayRef) -> Vec<f64> {
        match ticks.data_type() {
            ArrowDataType::Float64 => ticks
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .iter()
                .flatten()
                .collect(),
            ArrowDataType::Float32 => ticks
                .as_any()
                .downcast_ref::<Float32Array>()
                .unwrap()
                .iter()
                .filter_map(|v| v.map(|f| f as f64))
                .collect(),
            _ => vec![],
        }
    }

    /// Get theme context for label styling
    fn get_label_theme_values(
        theme: &Theme,
        axis_ctx: &ThemeContext,
    ) -> (String, f32, f32, [f32; 4]) {
        let ctx = axis_ctx.child("label");
        let font_family = theme
            .font_family(&ctx)
            .unwrap_or_else(|| "sans-serif".to_string());
        let font_size = theme.font_size(&ctx).unwrap_or(12.0);
        let font_weight = theme.font_weight(&ctx).unwrap_or(400.0);
        let color = theme.text_color(&ctx).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        (font_family, font_size, font_weight, color)
    }

    /// Evaluate this axis to scene marks
    async fn evaluate(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Create context for theme queries with params
        let axis_ctx = theme.axis_context_with_params(Some("polar"), Some(channel), params.clone());

        // Evaluate visible expression (default to true if not set)
        let visible = if let Some(visible_node) = self.visible.as_option().and_then(|o| o.as_ref())
        {
            let visible_expr = visible_node.to_expr(ctx)?;
            evaluate_bool_expr(&visible_expr, ctx, params).await?
        } else {
            true
        };

        // Skip if invisible
        if !visible {
            return Ok(vec![]);
        }

        // Get center point
        let center_x = plot_bounds.x + plot_width / 2.0;
        let center_y = plot_bounds.y + plot_height / 2.0;
        let radius = plot_width.min(plot_height) / 2.0;

        // Evaluate axis_type expression (default to Radial)
        let axis_type_str =
            if let Some(axis_type_node) = self.axis_type.as_option().and_then(|o| o.as_ref()) {
                let axis_type_expr = axis_type_node.to_expr(ctx)?;
                evaluate_string_expr(&axis_type_expr, ctx, params).await?
            } else {
                "radial".to_string()
            };

        let axis_type = match axis_type_str.to_lowercase().as_str() {
            "angular" => PolarAxisType::Angular,
            _ => PolarAxisType::Radial,
        };

        match axis_type {
            PolarAxisType::Radial => {
                // Evaluate radial axis (circles from center)
                self.evaluate_radial_axis(
                    scale, center_x, center_y, radius, theme, &axis_ctx, params, ctx,
                )
                .await
            }
            PolarAxisType::Angular => {
                // Evaluate angular axis (lines from center)
                self.evaluate_angular_axis(
                    scale, center_x, center_y, radius, scales, theme, &axis_ctx, params, ctx,
                )
                .await
            }
        }
    }

    async fn evaluate_radial_axis(
        &self,
        scale: &ConfiguredScale,
        center_x: f32,
        center_y: f32,
        _max_radius: f32,
        theme: &Theme,
        axis_ctx: &ThemeContext,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();

        // Evaluate grid expression (default to false)
        let grid = if let Some(grid_node) = self.grid.as_option().and_then(|o| o.as_ref()) {
            let grid_expr = grid_node.to_expr(ctx)?;
            evaluate_bool_expr(&grid_expr, ctx, params).await?
        } else {
            false
        };

        // Create concentric circles for the grid using actual scale ticks
        if grid {
            // Evaluate tick_count expression
            let tick_count = if let Some(tick_count_node) =
                self.tick_count.as_option().and_then(|o| o.as_ref())
            {
                let tick_count_expr = tick_count_node.to_expr(ctx)?;
                let scalars = eval_to_scalars(
                    vec![tick_count_expr],
                    Some(ctx),
                    params_to_datafusion(params).as_ref(),
                )
                .await
                .map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to evaluate tick_count: {}",
                        e
                    ))
                })?;
                let scalar = scalars.first().ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "No value returned from tick_count expression".to_string(),
                    )
                })?;
                Some(scalar.as_i32().map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Cannot convert tick_count to i32: {}",
                        e
                    ))
                })? as f32)
            } else {
                None
            };
            let ticks = scale.ticks(tick_count.or(Some(RADIAL_DEFAULT_TICK_COUNT)))?;

            let mut radii = Vec::new();

            let tick_values = Self::extract_tick_values(&ticks);

            for value in tick_values {
                // Transform tick value through scale to get radius
                let tick_array = Arc::new(Float64Array::from(vec![value])) as ArrayRef;
                let scaled_values = scale.scale(&tick_array)?;
                if let Some(scaled_array) = scaled_values.as_any().downcast_ref::<Float64Array>() {
                    if !scaled_array.is_empty() {
                        let radius = scaled_array.value(0) as f32;
                        if radius.is_finite() && radius > 0.0 {
                            radii.push(radius);
                        }
                    }
                } else if let Some(scaled_array) =
                    scaled_values.as_any().downcast_ref::<Float32Array>()
                    && !scaled_array.is_empty()
                {
                    let radius = scaled_array.value(0);
                    if radius.is_finite() && radius > 0.0 {
                        radii.push(radius);
                    }
                }
            }

            if !radii.is_empty() {
                let grid_arc = SceneArcMark {
                    name: "polar-radial-grid".to_string(),
                    clip: false,
                    len: radii.len() as u32,
                    gradients: vec![],
                    x: ScalarOrArray::new_scalar(center_x),
                    y: ScalarOrArray::new_scalar(center_y),
                    start_angle: ScalarOrArray::new_scalar(0.0),
                    end_angle: ScalarOrArray::new_scalar(2.0 * std::f32::consts::PI),
                    outer_radius: ScalarOrArray::new_array(radii.clone()),
                    inner_radius: ScalarOrArray::new_array(
                        radii.iter().map(|r| (r - 0.5).max(0.0)).collect(),
                    ), // Thin circles
                    pad_angle: ScalarOrArray::new_scalar(0.0),
                    corner_radius: ScalarOrArray::new_scalar(0.0),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color({
                        let grid_ctx = axis_ctx.child("grid");
                        let mut color = theme
                            .stroke_color(&grid_ctx)
                            .unwrap_or([0.8, 0.8, 0.8, 1.0]);
                        if let Some(opacity) = theme.opacity(&grid_ctx) {
                            color[3] = opacity;
                        }
                        color
                    })),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
                    stroke_width: ScalarOrArray::new_scalar(0.0),
                    indices: None,
                    zindex: Some(-1),
                };

                marks.push(SceneMark::Arc(grid_arc));
            }
        }

        // Add radial tick labels (visible check already done at start of render)

        // Get tick values from scale - use same as grid
        // Evaluate tick_count expression
        let tick_count = if let Some(tick_count_node) =
            self.tick_count.as_option().and_then(|o| o.as_ref())
        {
            let tick_count_expr = tick_count_node.to_expr(ctx)?;
            let scalars = eval_to_scalars(
                vec![tick_count_expr],
                Some(ctx),
                params_to_datafusion(params).as_ref(),
            )
            .await
            .map_err(|e| {
                AvengerChartError::InternalError(format!("Failed to evaluate tick_count: {}", e))
            })?;
            let scalar = scalars.first().ok_or_else(|| {
                AvengerChartError::InternalError(
                    "No value returned from tick_count expression".to_string(),
                )
            })?;
            Some(scalar.as_i32().map_err(|e| {
                AvengerChartError::InternalError(format!("Cannot convert tick_count to i32: {}", e))
            })? as f32)
        } else {
            None
        };
        let ticks = scale.ticks(tick_count.or(Some(RADIAL_DEFAULT_TICK_COUNT)))?;

        // Format tick values as strings
        let formatted_ticks = scale.format(&ticks)?;

        let mut x_vals = Vec::new();
        let mut y_vals = Vec::new();
        let mut text_vals = Vec::new();

        let tick_values = Self::extract_tick_values(&ticks);

        // Get formatted strings and positions
        for (i, value) in tick_values.iter().enumerate() {
            // Skip zero value
            if value.abs() < 0.001 {
                continue;
            }

            // Transform tick value through scale to get radius (same as grid circles)
            let tick_array = Arc::new(Float64Array::from(vec![*value])) as ArrayRef;
            let scaled_values = scale.scale(&tick_array)?;

            let radius = if let Some(scaled_array) =
                scaled_values.as_any().downcast_ref::<Float64Array>()
            {
                scaled_array.value(0) as f32
            } else if let Some(scaled_array) = scaled_values.as_any().downcast_ref::<Float32Array>()
            {
                scaled_array.value(0)
            } else {
                continue;
            };

            if radius.is_finite() && radius > 0.0 {
                // Position labels vertically below center (at 90 degrees)
                x_vals.push(center_x);
                y_vals.push(center_y + radius);

                // Get formatted text for this tick
                match formatted_ticks.value() {
                    avenger_common::value::ScalarOrArrayValue::Array(texts) => {
                        if i < texts.len() {
                            text_vals.push(texts[i].clone());
                        }
                    }
                    avenger_common::value::ScalarOrArrayValue::Scalar(text) => {
                        text_vals.push(text.clone());
                    }
                }
            }
        }

        if !text_vals.is_empty() {
            let (font_family, font_size, font_weight, color) =
                Self::get_label_theme_values(theme, axis_ctx);

            let text_mark = SceneTextMark {
                name: "polar-r-labels".to_string(),
                clip: false,
                len: text_vals.len() as u32,
                x: ScalarOrArray::new_array(x_vals),
                y: ScalarOrArray::new_array(y_vals),
                text: ScalarOrArray::new_array(text_vals),
                font: ScalarOrArray::new_scalar(font_family),
                font_weight: ScalarOrArray::new_scalar(FontWeight::Number(font_weight)),
                font_size: ScalarOrArray::new_scalar(font_size),
                font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                color: ScalarOrArray::new_scalar(ColorOrGradient::Color(color)),
                align: ScalarOrArray::new_scalar(TextAlign::Center),
                baseline: ScalarOrArray::new_scalar(TextBaseline::Top),
                angle: ScalarOrArray::new_scalar(0.0),
                limit: ScalarOrArray::new_scalar(200.0),
                indices: None,
                zindex: Some(0),
            };
            marks.push(SceneMark::Text(Arc::new(text_mark)));
        }

        // Note: Axis title rendering removed - placement needs design work

        Ok(marks)
    }

    async fn evaluate_angular_axis(
        &self,
        scale: &ConfiguredScale,
        center_x: f32,
        center_y: f32,
        radius: f32,
        scales: &HashMap<String, ConfiguredScale>,
        theme: &Theme,
        axis_ctx: &ThemeContext,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mut marks = Vec::new();

        // Get the radial scale to determine max radius
        let max_radius = if let Some(r_scale) = scales.get("r") {
            let r_range = r_scale.numeric_interval_range()?;
            r_range.1
        } else {
            radius
        };

        // Evaluate grid expression (default to false)
        let grid = if let Some(grid_node) = self.grid.as_option().and_then(|o| o.as_ref()) {
            let grid_expr = grid_node.to_expr(ctx)?;
            evaluate_bool_expr(&grid_expr, ctx, params).await?
        } else {
            false
        };

        // Render angular grid (radial lines) based on scale ticks
        if grid {
            // Evaluate tick_count expression
            let tick_count = if let Some(tick_count_node) =
                self.tick_count.as_option().and_then(|o| o.as_ref())
            {
                let tick_count_expr = tick_count_node.to_expr(ctx)?;
                let scalars = eval_to_scalars(
                    vec![tick_count_expr],
                    Some(ctx),
                    params_to_datafusion(params).as_ref(),
                )
                .await
                .map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Failed to evaluate tick_count: {}",
                        e
                    ))
                })?;
                let scalar = scalars.first().ok_or_else(|| {
                    AvengerChartError::InternalError(
                        "No value returned from tick_count expression".to_string(),
                    )
                })?;
                Some(scalar.as_i32().map_err(|e| {
                    AvengerChartError::InternalError(format!(
                        "Cannot convert tick_count to i32: {}",
                        e
                    ))
                })? as f32)
            } else {
                None
            };
            let ticks = scale.ticks(tick_count.or(Some(ANGULAR_DEFAULT_TICK_COUNT)))?;

            let mut x_values = Vec::new();
            let mut y_values = Vec::new();
            let mut x2_values = Vec::new();
            let mut y2_values = Vec::new();

            let angle_values = Self::extract_tick_values(&ticks);

            // Get the outermost circle radius from the r scale if available
            let outer_radius = if let Some(r_scale) = scales.get("r") {
                // Get the maximum radius from the r scale's range
                if let Ok(r_range) = r_scale.numeric_interval_range() {
                    r_range.1.max(r_range.0)
                } else {
                    max_radius
                }
            } else {
                max_radius
            };

            for angle in angle_values {
                // Start from center
                x_values.push(center_x);
                y_values.push(center_y);

                // End at outer radius
                let cos_angle = (angle as f32).cos();
                let sin_angle = (angle as f32).sin();
                x2_values.push(center_x + outer_radius * cos_angle);
                y2_values.push(center_y + outer_radius * sin_angle);
            }

            let grid_rule = SceneRuleMark {
                name: "polar-angular-grid".to_string(),
                clip: false,
                len: x_values.len() as u32,
                gradients: vec![],
                x: ScalarOrArray::new_array(x_values),
                y: ScalarOrArray::new_array(y_values),
                x2: ScalarOrArray::new_array(x2_values),
                y2: ScalarOrArray::new_array(y2_values),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color({
                    let grid_ctx = axis_ctx.child("grid");
                    let mut color = theme
                        .stroke_color(&grid_ctx)
                        .unwrap_or([0.8, 0.8, 0.8, 1.0]);
                    if let Some(opacity) = theme.opacity(&grid_ctx) {
                        color[3] = opacity;
                    }
                    color
                })),
                stroke_width: ScalarOrArray::new_scalar(
                    theme.axis_grid_width(axis_ctx).unwrap_or(1.0),
                ),
                stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Butt),
                stroke_dash: None,
                indices: None,
                zindex: Some(-1),
            };

            marks.push(SceneMark::Rule(grid_rule));
        }

        // Add angular tick labels (visible check already done at start of render)

        // Get tick values from scale
        let tick_values: Vec<f32> =
            if let Ok(ticks_array) = scale.ticks(Some(ANGULAR_DEFAULT_TICK_COUNT)) {
                // Convert arrow array to vec of f32
                if let Some(arr) = ticks_array.as_any().downcast_ref::<Float32Array>() {
                    (0..arr.len()).map(|i| arr.value(i)).collect()
                } else {
                    // Fallback if not float32
                    // Evaluate tick_count expression for fallback
                    let num_ticks = if let Some(tick_count_node) =
                        self.tick_count.as_option().and_then(|o| o.as_ref())
                    {
                        let tick_count_expr = tick_count_node.to_expr(ctx)?;
                        let scalars = eval_to_scalars(
                            vec![tick_count_expr],
                            Some(ctx),
                            params_to_datafusion(params).as_ref(),
                        )
                        .await
                        .map_err(|e| {
                            AvengerChartError::InternalError(format!(
                                "Failed to evaluate tick_count: {}",
                                e
                            ))
                        })?;
                        let scalar = scalars.first().ok_or_else(|| {
                            AvengerChartError::InternalError(
                                "No value returned from tick_count expression".to_string(),
                            )
                        })?;
                        scalar.as_i32().map_err(|e| {
                            AvengerChartError::InternalError(format!(
                                "Cannot convert tick_count to i32: {}",
                                e
                            ))
                        })? as usize
                    } else {
                        8
                    };
                    (0..num_ticks)
                        .map(|i| (i as f32 / num_ticks as f32) * 2.0 * std::f32::consts::PI)
                        .collect()
                }
            } else {
                // Fallback to uniform distribution
                // Evaluate tick_count expression for fallback
                let num_ticks = if let Some(tick_count_node) =
                    self.tick_count.as_option().and_then(|o| o.as_ref())
                {
                    let tick_count_expr = tick_count_node.to_expr(ctx)?;
                    let scalars = eval_to_scalars(
                        vec![tick_count_expr],
                        Some(ctx),
                        params_to_datafusion(params).as_ref(),
                    )
                    .await
                    .map_err(|e| {
                        AvengerChartError::InternalError(format!(
                            "Failed to evaluate tick_count: {}",
                            e
                        ))
                    })?;
                    let scalar = scalars.first().ok_or_else(|| {
                        AvengerChartError::InternalError(
                            "No value returned from tick_count expression".to_string(),
                        )
                    })?;
                    scalar.as_i32().map_err(|e| {
                        AvengerChartError::InternalError(format!(
                            "Cannot convert tick_count to i32: {}",
                            e
                        ))
                    })? as usize
                } else {
                    8
                };
                (0..num_ticks)
                    .map(|i| (i as f32 / num_ticks as f32) * 2.0 * std::f32::consts::PI)
                    .collect()
            };

        let mut x_vals = Vec::new();
        let mut y_vals = Vec::new();
        let mut text_vals = Vec::new();
        let mut label_aligns = Vec::new();
        let mut label_baselines = Vec::new();

        let label_radius = max_radius + 15.0; // Place labels outside the plot

        for tick_val in tick_values {
            // Use tick value directly as angle (it's already in radians)
            let cos_angle = tick_val.cos();
            let sin_angle = tick_val.sin();

            // Position label outside the plot circle
            let x = center_x + label_radius * cos_angle;
            let y = center_y + label_radius * sin_angle;

            x_vals.push(x);
            y_vals.push(y);

            // Determine text alignment based on angle
            let align = if cos_angle.abs() < 0.1 {
                TextAlign::Center
            } else if cos_angle > 0.0 {
                TextAlign::Left
            } else {
                TextAlign::Right
            };

            let baseline = if sin_angle.abs() < 0.1 {
                TextBaseline::Middle
            } else if sin_angle > 0.0 {
                TextBaseline::Top
            } else {
                TextBaseline::Bottom
            };

            label_aligns.push(align);
            label_baselines.push(baseline);

            // Convert radians to degrees for display
            let degrees = (tick_val * 180.0 / std::f32::consts::PI).round() as i32;
            text_vals.push(format!("{}°", degrees));
        }

        if !text_vals.is_empty() {
            let (font_family, font_size, font_weight, color) =
                Self::get_label_theme_values(theme, axis_ctx);

            let text_mark = SceneTextMark {
                name: "polar-theta-labels".to_string(),
                clip: false,
                len: text_vals.len() as u32,
                x: ScalarOrArray::new_array(x_vals),
                y: ScalarOrArray::new_array(y_vals),
                text: ScalarOrArray::new_array(text_vals),
                align: ScalarOrArray::new_array(label_aligns),
                baseline: ScalarOrArray::new_array(label_baselines),
                font: ScalarOrArray::new_scalar(font_family),
                font_weight: ScalarOrArray::new_scalar(FontWeight::Number(font_weight)),
                font_size: ScalarOrArray::new_scalar(font_size),
                font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
                color: ScalarOrArray::new_scalar(ColorOrGradient::Color(color)),
                angle: ScalarOrArray::new_scalar(0.0),
                limit: ScalarOrArray::new_scalar(200.0),
                indices: None,
                zindex: Some(0),
            };
            marks.push(SceneMark::Text(Arc::new(text_mark)));
        }

        Ok(marks)
    }
}
