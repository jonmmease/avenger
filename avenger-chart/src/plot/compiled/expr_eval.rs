//! Expression evaluation utilities for converting DataFusion expressions to concrete values
//!
//! This module provides helper functions to evaluate DataFusion expressions with parameter
//! support, converting them to specific Rust types (f32, String, bool, etc.)

use datafusion::prelude::SessionContext;
use indexmap::IndexMap;

use crate::error::AvengerChartError;

/// Helper function to evaluate a dimension expression to a concrete f32 value
pub(crate) async fn evaluate_dimension_expr(
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
        AvengerChartError::InternalError(format!("Failed to evaluate dimension expression: {}", e))
    })?;

    let scalar = scalars.first().ok_or_else(|| {
        AvengerChartError::InternalError("No value returned from dimension expression".to_string())
    })?;

    // Convert to f32 using the ScalarValueHelpers trait
    scalar.as_f32().map_err(|e| {
        AvengerChartError::InternalError(format!(
            "Cannot convert dimension expression result to f32: {}",
            e
        ))
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
