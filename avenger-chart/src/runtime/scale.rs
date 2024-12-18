use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use arrow::datatypes::{DataType, Schema};
use async_trait::async_trait;
use avenger_scales::{
    color::continuous_color::LinearSrgbaScale,
    numeric::linear::{LinearNumericScale, LinearNumericScaleConfig},
};
use datafusion::{
    common::{DFSchema, ParamValues},
    logical_expr::ExprSchemable,
    prelude::{lit, DataFrame, Expr, SessionContext},
    scalar::ScalarValue,
};
use ordered_float::OrderedFloat;

use crate::{
    error::AvengerChartError,
    scales::{numeric::NumericScale, ScaleImpl},
    types::scales::{Scale, ScaleDomain, ScaleRange},
    utils::{DataFrameChartUtils, ExprHelpers, ScalarValueUtils},
};

// Define the type alias for the async scale compiler function
#[async_trait]
pub trait ScaleCompiler: Send + Sync {
    async fn compile(
        &self,
        scale: &Scale,
        ctx: &SessionContext,
        params: &ParamValues,
    ) -> Result<ScaleImpl, AvengerChartError>;
}

pub struct LinearScaleCompiler;

#[async_trait]
impl ScaleCompiler for LinearScaleCompiler {
    async fn compile(
        &self,
        scale: &Scale,
        ctx: &SessionContext,
        params: &ParamValues,
    ) -> Result<ScaleImpl, AvengerChartError> {
        // Compute domain
        let domain: [f32; 2] = compute_domain_span(&scale.domain, ctx, params).await?;

        // Compute range
        if let Some(ScaleRange::Rgb(colors)) = &scale.range {
            // Linear scale config with domain
            let config = LinearNumericScaleConfig {
                domain: (domain[0], domain[1]),
                ..Default::default()
            };
            let numeric_scale = LinearNumericScale::new(&config);

            Ok(ScaleImpl::LinearSrgba(LinearSrgbaScale::from_scale(
                Arc::new(move || numeric_scale.clone()),
                colors.clone(),
            )?))
        } else {
            let range: [f32; 2] = compute_numeric_range_span(&scale.range, ctx, params).await?;

            let config = LinearNumericScaleConfig {
                domain: (domain[0], domain[1]),
                range: (range[0], range[1]),
                ..Default::default()
            };

            Ok(ScaleImpl::Numeric(NumericScale::new_linear(
                LinearNumericScale::new(&config),
            )))
        }
    }
}

/// Helper to convert an expression to a f32
async fn expr_to_f32(
    expr: &Expr,
    ctx: &SessionContext,
    params: Option<&ParamValues>,
) -> Result<f32, AvengerChartError> {
    // Must apply params before looking up the schema, or DataFusion errors
    let expr = if let Some(params) = params {
        expr.clone().apply_params(params)?
    } else {
        expr.clone()
    };

    let schema = DFSchema::empty();

    let ScalarValue::Float32(Some(f32_value)) = expr
        .clone()
        .cast_to(&DataType::Float32, &schema)?
        .eval_to_scalar(ctx, None)
        .await?
    else {
        return Err(AvengerChartError::InternalError(
            "Expected start of interval to have been casted to a float".to_string(),
        ));
    };
    Ok(f32_value)
}

/// Helper to compute a numeric domain span from a scale
async fn compute_domain_span(
    domain: &Option<ScaleDomain>,
    ctx: &SessionContext,
    params: &ParamValues,
) -> Result<[f32; 2], AvengerChartError> {
    let span = match domain
        .clone()
        .unwrap_or_else(|| ScaleDomain::Interval(lit(0.0), lit(1.0)))
    {
        ScaleDomain::Interval(start_expr, end_expr) => {
            let start = expr_to_f32(&start_expr, ctx, Some(params)).await?;
            let end = expr_to_f32(&end_expr, ctx, Some(params)).await?;
            [start, end]
        }
        ScaleDomain::DataField(data_field) => {
            let df = ctx.table(&data_field.dataset).await?;
            let df_with_field = df.select_columns(&[&data_field.field])?;
            let span = df_with_field
                .span()?
                .eval_to_scalar(ctx, Some(params))
                .await?;
            span.as_f32_2()?
        }

        ScaleDomain::DataFields(vec) => {
            // Group fields by dataset
            let mut fields_by_dataset: HashMap<String, Vec<String>> = HashMap::new();
            for data_field in vec {
                fields_by_dataset
                    .entry(data_field.dataset.clone())
                    .or_insert_with(|| Vec::new())
                    .push(data_field.field.clone());
            }

            // Compute span for all of the columns from each dataset
            let mut spans: Vec<[f32; 2]> = Vec::new();
            for (dataset, fields) in fields_by_dataset {
                let df = ctx.table(&dataset).await?;
                let field_strs = fields.iter().map(|f| f.as_str()).collect::<Vec<_>>();
                let df_with_fields = df.select_columns(&field_strs)?;
                let span = df_with_fields
                    .span()?
                    .eval_to_scalar(ctx, Some(params))
                    .await?;
                spans.push(span.as_f32_2()?);
            }

            // Compute min and max of all spans
            let min = spans
                .iter()
                .map(|s| OrderedFloat(s[0]))
                .min()
                .unwrap_or(OrderedFloat(0.0))
                .0;
            let max = spans
                .iter()
                .map(|s| OrderedFloat(s[1]))
                .max()
                .unwrap_or(OrderedFloat(1.0))
                .0;
            [min, max]
        }
        ScaleDomain::Discrete(_) => {
            return Err(AvengerChartError::InternalError(
                "Discrete domain not supported for linear scale".to_string(),
            ));
        }
    };
    Ok(span)
}

/// Helper to compute a numeric range span from a scale
async fn compute_numeric_range_span(
    range: &Option<ScaleRange>,
    ctx: &SessionContext,
    params: &ParamValues,
) -> Result<[f32; 2], AvengerChartError> {
    let span = match range
        .clone()
        .unwrap_or_else(|| ScaleRange::Numeric(lit(0.0), lit(1.0)))
    {
        ScaleRange::Numeric(start_expr, end_expr) => {
            println!("end_expr: {:?}", end_expr);
            println!("params: {:?}", params);

            let start = expr_to_f32(&start_expr, ctx, Some(params)).await?;
            let end = expr_to_f32(&end_expr, ctx, Some(params)).await?;
            println!("done");
            [start, end]
        }
        _ => {
            return Err(AvengerChartError::InternalError(
                "Numeric range not supported for linear scale".to_string(),
            ))
        }
    };
    Ok(span)
}
