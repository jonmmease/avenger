//! Expression evaluation utilities for converting DataFusion expressions to concrete values
//!
//! This module provides helper functions to evaluate DataFusion expressions with parameter
//! support, converting them to specific Rust types (f32, String, bool, etc.)

use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use crate::error::AvengerChartError;

/// Helper function to evaluate an expression to a concrete f32 value
pub(crate) async fn evaluate_f32_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<f32, AvengerChartError> {
    use crate::utils::{ScalarValueHelpers, eval_to_scalars, params_to_datafusion};

    // Use the existing eval_to_scalars utility which handles parameters via with_param_values()
    let scalars = eval_to_scalars(
        vec![expr.clone()],
        Some(ctx),
        params_to_datafusion(params).as_ref(),
    )
    .await
    .map_err(|e| {
        AvengerChartError::InternalError(format!("Failed to evaluate f32 expression: {}", e))
    })?;

    let scalar = scalars.first().ok_or_else(|| {
        AvengerChartError::InternalError("No value returned from f32 expression".to_string())
    })?;

    // Convert to f32 using the ScalarValueHelpers trait
    scalar.as_f32().map_err(|e| {
        AvengerChartError::InternalError(format!("Cannot convert expression result to f32: {}", e))
    })
}

/// Helper function to evaluate a string expression to a concrete String value
pub(crate) async fn evaluate_string_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<String, AvengerChartError> {
    use crate::utils::{ScalarValueHelpers, eval_to_scalars, params_to_datafusion};

    // Use the existing eval_to_scalars utility which handles parameters via with_param_values()
    let scalars = eval_to_scalars(
        vec![expr.clone()],
        Some(ctx),
        params_to_datafusion(params).as_ref(),
    )
    .await
    .map_err(|e| {
        AvengerChartError::InternalError(format!("Failed to evaluate string expression: {}", e))
    })?;

    let scalar = scalars.first().ok_or_else(|| {
        AvengerChartError::InternalError("No value returned from string expression".to_string())
    })?;

    // Convert to string using the ScalarValueHelpers trait
    scalar.as_scalar_string().map_err(|e| {
        AvengerChartError::InternalError(format!(
            "Cannot convert string expression result to string: {}",
            e
        ))
    })
}

/// Helper function to evaluate a boolean expression to a concrete bool value
pub(crate) async fn evaluate_bool_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<bool, AvengerChartError> {
    use crate::utils::{eval_to_scalars, params_to_datafusion};

    let scalars = eval_to_scalars(
        vec![expr.clone()],
        Some(ctx),
        params_to_datafusion(params).as_ref(),
    )
    .await
    .map_err(|e| {
        AvengerChartError::InternalError(format!("Failed to evaluate boolean expression: {}", e))
    })?;

    let scalar = scalars.first().ok_or_else(|| {
        AvengerChartError::InternalError("No value returned from boolean expression".to_string())
    })?;

    match scalar {
        datafusion::common::ScalarValue::Boolean(Some(b)) => Ok(*b),
        _ => Err(AvengerChartError::InternalError(format!(
            "Cannot convert expression result to bool: {}",
            scalar
        ))),
    }
}

/// Helper function to evaluate an integer expression to a concrete i32 value
pub(crate) async fn evaluate_i32_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<i32, AvengerChartError> {
    use crate::utils::{eval_to_scalars, params_to_datafusion};

    let scalars = eval_to_scalars(
        vec![expr.clone()],
        Some(ctx),
        params_to_datafusion(params).as_ref(),
    )
    .await
    .map_err(|e| {
        AvengerChartError::InternalError(format!("Failed to evaluate integer expression: {}", e))
    })?;

    let scalar = scalars.first().ok_or_else(|| {
        AvengerChartError::InternalError("No value returned from integer expression".to_string())
    })?;

    match scalar {
        datafusion::common::ScalarValue::Int32(Some(i)) => Ok(*i),
        datafusion::common::ScalarValue::Int64(Some(i)) => Ok(*i as i32),
        datafusion::common::ScalarValue::UInt32(Some(u)) => Ok(*u as i32),
        datafusion::common::ScalarValue::UInt64(Some(u)) => Ok(*u as i32),
        _ => Err(AvengerChartError::InternalError(format!(
            "Cannot convert expression result to i32: {}",
            scalar
        ))),
    }
}

/// Helper function to evaluate a usize expression to a concrete usize value
#[allow(dead_code)]
pub(crate) async fn evaluate_usize_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<usize, AvengerChartError> {
    use crate::utils::{eval_to_scalars, params_to_datafusion};

    let scalars = eval_to_scalars(
        vec![expr.clone()],
        Some(ctx),
        params_to_datafusion(params).as_ref(),
    )
    .await
    .map_err(|e| {
        AvengerChartError::InternalError(format!("Failed to evaluate usize expression: {}", e))
    })?;

    let scalar = scalars.first().ok_or_else(|| {
        AvengerChartError::InternalError("No value returned from usize expression".to_string())
    })?;

    match scalar {
        datafusion::common::ScalarValue::UInt64(Some(u)) => Ok(*u as usize),
        datafusion::common::ScalarValue::UInt32(Some(u)) => Ok(*u as usize),
        datafusion::common::ScalarValue::Int64(Some(i)) if *i >= 0 => Ok(*i as usize),
        datafusion::common::ScalarValue::Int32(Some(i)) if *i >= 0 => Ok(*i as usize),
        _ => Err(AvengerChartError::InternalError(format!(
            "Cannot convert expression result to usize: {}",
            scalar
        ))),
    }
}

/// Helper function to evaluate a f64 expression to a concrete f64 value
pub(crate) async fn evaluate_f64_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<f64, AvengerChartError> {
    use crate::utils::{ScalarValueHelpers, eval_to_scalars, params_to_datafusion};

    let scalars = eval_to_scalars(
        vec![expr.clone()],
        Some(ctx),
        params_to_datafusion(params).as_ref(),
    )
    .await
    .map_err(|e| {
        AvengerChartError::InternalError(format!("Failed to evaluate f64 expression: {}", e))
    })?;

    let scalar = scalars.first().ok_or_else(|| {
        AvengerChartError::InternalError("No value returned from f64 expression".to_string())
    })?;

    // Try f64 first, then f32
    scalar
        .as_f64()
        .or_else(|_| scalar.as_f32().map(|f| f as f64))
        .map_err(|e| {
            AvengerChartError::InternalError(format!(
                "Cannot convert expression result to f64: {}",
                e
            ))
        })
}

/// Helper function to evaluate an AxisPosition expression (string-only)
pub(crate) async fn evaluate_axis_position_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<crate::cartesian::axis::AxisPosition, AvengerChartError> {
    use crate::cartesian::axis::AxisPosition;

    let s = evaluate_string_expr(expr, ctx, params).await?;

    match s.to_lowercase().as_str() {
        "top" => Ok(AxisPosition::Top),
        "bottom" => Ok(AxisPosition::Bottom),
        "left" => Ok(AxisPosition::Left),
        "right" => Ok(AxisPosition::Right),
        _ => Err(AvengerChartError::InternalError(format!(
            "Invalid axis position string '{}'. Must be one of: 'top', 'bottom', 'left', 'right'",
            s
        ))),
    }
}

/// Helper function to evaluate a LegendPosition expression (string-only)
pub(crate) async fn evaluate_legend_position_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<crate::legend::LegendPosition, AvengerChartError> {
    use crate::legend::LegendPosition;

    let s = evaluate_string_expr(expr, ctx, params).await?;

    match s.to_lowercase().as_str() {
        "top" => Ok(LegendPosition::Top),
        "bottom" => Ok(LegendPosition::Bottom),
        "left" => Ok(LegendPosition::Left),
        "right" => Ok(LegendPosition::Right),
        _ => Err(AvengerChartError::InternalError(format!(
            "Invalid legend position string '{}'. Must be one of: 'top', 'bottom', 'left', 'right'",
            s
        ))),
    }
}

/// Helper function to evaluate a LegendOrientation expression (string-only)
#[allow(dead_code)]
pub(crate) async fn evaluate_legend_orientation_expr(
    expr: &datafusion::prelude::Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, datafusion::common::ScalarValue>,
) -> Result<crate::legend::LegendOrientation, AvengerChartError> {
    use crate::legend::LegendOrientation;

    let s = evaluate_string_expr(expr, ctx, params).await?;

    match s.to_lowercase().as_str() {
        "horizontal" => Ok(LegendOrientation::Horizontal),
        "vertical" => Ok(LegendOrientation::Vertical),
        _ => Err(AvengerChartError::InternalError(format!(
            "Invalid legend orientation string '{}'. Must be one of: 'horizontal', 'vertical'",
            s
        ))),
    }
}
