use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use crate::{
    error::AvengerChartError,
    types::scales::{Scale, ScaleDomain, ScaleRange},
    utils::{DataFrameChartUtils, ExprHelpers},
};
use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::{concat_batches, kernels::cast},
    datatypes::{DataType, Float32Type, Schema},
};

use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};

use avenger_scales::{
    color_interpolator::{ColorInterpolator, SrgbaColorInterpolator},
    formatter::Formatters,
    scales::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleImpl},
    utils::ScalarValueUtils,
};
use datafusion::{
    common::{DFSchema, ParamValues},
    logical_expr::ExprSchemable,
    prelude::{lit, DataFrame, Expr, SessionContext},
    scalar::ScalarValue,
};
use ordered_float::OrderedFloat;
use palette::Srgba;
use std::fmt::Debug;

use super::context::CompilationContext;

#[derive(Debug, Clone)]
pub struct EvaluatedScale {
    pub name: String,
    pub kind: String,
    pub scale: ConfiguredScale,
}

pub async fn evaluate_scale(
    scale: &Scale,
    name: &str,
    ctx: &SessionContext,
    params: &ParamValues,
    scale_impls: &HashMap<String, Arc<dyn ScaleImpl>>,
    color_interpolator: Arc<dyn ColorInterpolator>,
) -> Result<EvaluatedScale, AvengerChartError> {
    let kind = scale.kind.clone().unwrap_or("linear".to_string());
    println!("kind: {}", kind);
    println!("scale_impls: {:?}", scale_impls);
    let scale_impl = scale_impls
        .get(&kind)
        .ok_or_else(|| AvengerChartError::ScaleKindLookupError(kind.to_string()))?;
    println!("found it");

    let method = scale_impl.infer_domain_from_data_method();

    // Compute domain
    let domain = evaluate_scale_domain(&scale, ctx, params, method).await?;
    let range = evaluate_scale_range(&scale.range, ctx, params).await?;
    let mut evaluated_options: HashMap<String, ScalarValue> = HashMap::new();
    for (key, value) in scale.options.iter() {
        let evaluated_value = value.eval_to_scalar(ctx, Some(params)).await?;
        evaluated_options.insert(key.to_string(), evaluated_value);
    }

    let scale_config = ScaleConfig {
        domain,
        range,
        options: evaluated_options,
    };

    let scale = ConfiguredScale {
        scale_impl: scale_impl.clone(),
        config: scale_config,
        color_interpolator,
        formatters: Formatters::default(),
    };

    Ok(EvaluatedScale {
        name: name.to_string(),
        kind,
        scale,
    })
}

/// Helper to compute a numeric domain span from a scale
async fn evaluate_scale_domain(
    scale: &Scale,
    ctx: &SessionContext,
    params: &ParamValues,
    method: InferDomainFromDataMethod,
) -> Result<ArrayRef, AvengerChartError> {
    let domain = &scale.domain;

    match domain
        .clone()
        .unwrap_or_else(|| ScaleDomain::Interval(lit(0.0), lit(1.0)))
    {
        ScaleDomain::Interval(start_expr, end_expr) => {
            if method != InferDomainFromDataMethod::Interval {
                return Err(AvengerChartError::InternalError(format!(
                    "Scale named {} does not support interval domain",
                    scale.name
                )));
            }
            let start = start_expr.eval_to_f32(ctx, Some(params)).await?;
            let end = end_expr.eval_to_f32(ctx, Some(params)).await?;
            Ok(Arc::new(Float32Array::from(vec![start, end])))
        }
        ScaleDomain::DataField(data_field) => {
            let df = ctx.table(&data_field.dataset).await?;
            let df_with_field = df.select_columns(&[&data_field.field])?;

            match method {
                InferDomainFromDataMethod::Interval => {
                    let span = df_with_field
                        .span()?
                        .eval_to_scalar(ctx, Some(params))
                        .await?;
                    let interval = span.as_f32_2()?;
                    Ok(Arc::new(Float32Array::from(vec![interval[0], interval[1]])))
                }
                InferDomainFromDataMethod::Unique => {
                    let unique_df = df_with_field.uniques(Some(true), None)?;
                    let unique_schema = unique_df.schema().clone();
                    let unique_batches = unique_df.collect().await?;
                    let unique_batch = concat_batches(unique_schema.inner(), &unique_batches)?;
                    Ok(unique_batch.column(0).clone())
                }
                InferDomainFromDataMethod::All => {
                    let all_df = df_with_field.union_all_cols(None)?;
                    let all_schema = all_df.schema().clone();
                    let all_batches = all_df.collect().await?;
                    let all_batch = concat_batches(all_schema.inner(), &all_batches)?;
                    Ok(all_batch.column(0).clone())
                }
            }
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

            match method {
                InferDomainFromDataMethod::Interval => {
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
                    Ok(Arc::new(Float32Array::from(vec![min, max])))
                }
                _ => {
                    // Union all columns from each dataset
                    let mut single_col_dfs: Vec<DataFrame> = Vec::new();
                    for (dataset, fields) in fields_by_dataset {
                        if fields.is_empty() {
                            continue;
                        }
                        let df = ctx.table(&dataset).await?;
                        let field_strs = fields.iter().map(|f| f.as_str()).collect::<Vec<_>>();
                        let df_with_fields = df.select_columns(&field_strs)?;
                        single_col_dfs.push(df_with_fields.union_all_cols(None)?);
                    }

                    if single_col_dfs.is_empty() {
                        return Err(AvengerChartError::InternalError(
                            "No fields to infer domain from".to_string(),
                        ));
                    }

                    // Union all of the single column dataframes
                    let union_df = single_col_dfs
                        .iter()
                        .fold(single_col_dfs[0].clone(), |acc, df| {
                            acc.union(df.clone()).unwrap()
                        });

                    if method == InferDomainFromDataMethod::Unique {
                        // Keep unique values
                        let unique_df = union_df.uniques(Some(true), None)?;
                        let unique_schema = unique_df.schema().clone();
                        let unique_batches = unique_df.collect().await?;
                        let unique_batch = concat_batches(unique_schema.inner(), &unique_batches)?;
                        Ok(unique_batch.column(0).clone())
                    } else {
                        // Keep all columns
                        let all_schema = union_df.schema().clone();
                        let all_batches = union_df.collect().await?;
                        let all_batch = concat_batches(all_schema.inner(), &all_batches)?;
                        Ok(all_batch.column(0).clone())
                    }
                }
            }
        }
        ScaleDomain::Discrete(values) => {
            // Evaluate all of the value and concat into an array
            let mut scalars = Vec::new();
            for expr in values {
                let scalar = expr.eval_to_scalar(ctx, Some(params)).await?;
                scalars.push(scalar);
            }
            Ok(ScalarValue::iter_to_array(scalars)?)
        }
    }
}

async fn evaluate_scale_range(
    range: &Option<ScaleRange>,
    ctx: &SessionContext,
    params: &ParamValues,
) -> Result<ArrayRef, AvengerChartError> {
    match range
        .clone()
        .unwrap_or_else(|| ScaleRange::Numeric(lit(0.0), lit(1.0)))
    {
        ScaleRange::Numeric(start_expr, end_expr) => {
            let start = start_expr.eval_to_f32(ctx, Some(params)).await?;
            let end = end_expr.eval_to_f32(ctx, Some(params)).await?;
            Ok(Arc::new(Float32Array::from(vec![start, end])))
        }
        ScaleRange::Color(colors) => {
            let colors = colors
                .iter()
                .map(|c| ScalarValue::make_rgba(c.red, c.green, c.blue, c.alpha))
                .collect::<Vec<_>>();

            // Convert scalars to array
            let array = ScalarValue::iter_to_array(colors)?;
            Ok(array)
        }
        _ => {
            todo!("evaluate range")
        }
    }
}

// /// Helper to compute a numeric range span from a scale
// async fn compute_numeric_range_span(
//     range: &Option<ScaleRange>,
//     ctx: &SessionContext,
//     params: &ParamValues,
// ) -> Result<[f32; 2], AvengerChartError> {
//     let span = match range
//         .clone()
//         .unwrap_or_else(|| ScaleRange::Numeric(lit(0.0), lit(1.0)))
//     {
//         ScaleRange::Numeric(start_expr, end_expr) => {
//             let start = start_expr.eval_to_f32(ctx, Some(params)).await?;
//             let end = end_expr.eval_to_f32(ctx, Some(params)).await?;
//             [start, end]
//         }
//         _ => {
//             return Err(AvengerChartError::InternalError(
//                 "Numeric range not supported for linear scale".to_string(),
//             ))
//         }
//     };
//     Ok(span)
// }
