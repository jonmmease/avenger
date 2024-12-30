use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use crate::{
    error::AvengerChartError,
    types::scales::{DataField, Scale, ScaleDefaultDomain, ScaleDomain, ScaleRange},
    utils::{DataFrameChartUtils, ExprHelpers},
};
use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::{concat_batches, kernels::cast},
    datatypes::{DataType, Float32Type, Schema},
};

use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};

use super::context::CompilationContext;
use arrow::array::Array;
use avenger_scales::{
    color_interpolator::{ColorInterpolator, SrgbaColorInterpolator},
    formatter::Formatters,
    scales::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext, ScaleImpl},
    utils::ScalarValueUtils,
};
use datafusion::{
    common::{utils::arrays_into_list_array, DFSchema, ParamValues},
    error::DataFusionError,
    logical_expr::{
        ColumnarValue, ExprSchemable, ScalarFunctionImplementation, ScalarUDF, ScalarUDFImpl,
        Signature, TypeSignature, Volatility,
    },
    prelude::{create_udf, lit, make_array, named_struct, when, DataFrame, Expr, SessionContext},
    scalar::ScalarValue,
};
use ordered_float::OrderedFloat;
use palette::Srgba;
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct EvaluatedScale {
    pub name: String,
    pub kind: String,
    pub scale: ConfiguredScale,
}

pub fn scale_expr<E: Into<Expr>>(scale: &Scale, values: E) -> Result<Expr, AvengerChartError> {
    let scale_impl = scale.get_scale_impl().clone();
    let domain = compile_domain(&scale.domain, scale_impl.infer_domain_from_data_method())?;
    let range = compile_range(&scale.range)?;
    let options = compile_options(&scale.options)?;

    let domain_type = scale.get_domain().data_type()?;
    let range_type = scale.get_range().data_type()?;
    let options_type = options.get_type(&DFSchema::empty())?;

    let udf = ScalarUDF::from(ScaleUDF::new(
        scale_impl.clone(),
        domain_type,
        range_type,
        options_type,
    )?);

    Ok(udf.call(vec![domain, range, options, values.into()]))
}

#[derive(Debug, Clone)]
pub struct ScaleUDF {
    signature: Signature,
    range_type: DataType,
    scale_impl: Arc<dyn ScaleImpl>,
}

impl ScaleUDF {
    pub fn new(
        scale_impl: Arc<dyn ScaleImpl>,
        domain_type: DataType,
        range_type: DataType,
        options_type: DataType,
    ) -> Result<Self, AvengerChartError> {
        let signature = Signature::new(
            TypeSignature::Coercible(vec![
                DataType::new_list(domain_type.clone(), true), // Domain array
                DataType::new_list(range_type.clone(), true),  // Range array
                options_type.clone(),                          // Options struct
                domain_type.clone(),                           // Values to scale
            ]),
            Volatility::Immutable,
        );
        Ok(Self {
            signature,
            range_type,
            scale_impl,
        })
    }
}

impl ScalarUDFImpl for ScaleUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        "scale"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> datafusion::error::Result<DataType> {
        Ok(self.range_type.clone())
    }

    fn invoke(&self, args: &[ColumnarValue]) -> datafusion::error::Result<ColumnarValue> {
        // Extract domain array from scalar
        let domain = match &args[0] {
            ColumnarValue::Scalar(ScalarValue::List(domain_arg)) => domain_arg.value(0),
            ColumnarValue::Array(array) => {
                let list_array = array.as_list_opt::<i32>().ok_or_else(|| {
                    DataFusionError::Execution(format!("Expected domain array, got {:?}", array))
                })?;
                if list_array.is_empty() {
                    return Ok(ColumnarValue::Array(
                        ScalarValue::try_from(&self.range_type)?.to_array_of_size(0)?,
                    ));
                }
                list_array.value(0)
            }
            _ => {
                return Err(DataFusionError::Execution(format!(
                    "Unexpected domain value: {:?}",
                    args[0]
                )))
            }
        };

        // Extract range array from scalar
        let ColumnarValue::Scalar(ScalarValue::List(range_arg)) = &args[1] else {
            return Err(DataFusionError::Execution(format!(
                "Expected range scalar, got {:?}",
                args[1]
            )));
        };
        let range = range_arg.value(0);

        // Extract options struct from scalar
        let ColumnarValue::Scalar(ScalarValue::Struct(options_arg)) = &args[2] else {
            return Err(DataFusionError::Execution(format!(
                "Expected options struct, got {:?}",
                args[2]
            )));
        };
        let options = options_arg
            .column_names()
            .iter()
            .map(|c| {
                (
                    c.to_string(),
                    ScalarValue::try_from_array(options_arg.column_by_name(c).unwrap(), 0).unwrap(),
                )
            })
            .collect::<HashMap<_, _>>();

        let config = ScaleConfig {
            domain,
            range,
            options,
            context: ScaleContext::default(),
        };

        let scale = ConfiguredScale {
            scale_impl: self.scale_impl.clone(),
            config,
        };

        let scaled = match &args[3] {
            ColumnarValue::Array(values) => ColumnarValue::Array(
                scale
                    .scale(&values)
                    .map_err(|e| DataFusionError::Execution(e.to_string()))?,
            ),
            ColumnarValue::Scalar(values) => ColumnarValue::Scalar(
                scale
                    .scale_scalar(values)
                    .map_err(|e| DataFusionError::Execution(e.to_string()))?,
            ),
        };
        Ok(scaled)
    }
}

/// Helper to build an Expr that evaluates to a ScalarValue::List with the domain of the scale
pub fn compile_domain(
    domain: &ScaleDomain,
    method: InferDomainFromDataMethod,
) -> Result<Expr, AvengerChartError> {
    // Compile raw domain
    let raw_expr = if let Some(domain_raw) = &domain.raw_domain {
        domain_raw.clone()
    } else {
        lit(ScalarValue::Null)
    };

    let default_domain_expr = match &domain.default_domain {
        ScaleDefaultDomain::Interval(start_expr, end_expr) => {
            if method != InferDomainFromDataMethod::Interval {
                return Err(AvengerChartError::InternalError(format!(
                    "Scale does not support interval domain",
                )));
            }
            make_array(vec![start_expr.clone(), end_expr.clone()])
        }
        ScaleDefaultDomain::DataField(DataField { dataframe, field }) => {
            let df_with_field = dataframe.as_ref().clone().select_columns(&[field])?;

            match method {
                InferDomainFromDataMethod::Interval => df_with_field.span()?,
                InferDomainFromDataMethod::Unique => df_with_field.unique_values()?,
                InferDomainFromDataMethod::All => df_with_field.all_values()?,
            }
        }

        ScaleDefaultDomain::DataFields(data_fields) => {
            let mut single_col_dfs: Vec<DataFrame> = Vec::new();

            for DataField { dataframe, field } in data_fields {
                let df = dataframe.clone();
                let df_with_field = df.as_ref().clone().select_columns(&[field])?;
                single_col_dfs.push(df_with_field);
            }

            // Union all of the single column dataframes
            let union_df = single_col_dfs
                .iter()
                .fold(single_col_dfs[0].clone(), |acc, df| {
                    acc.union(df.clone()).unwrap()
                });

            match method {
                InferDomainFromDataMethod::Interval => union_df.span()?,
                InferDomainFromDataMethod::Unique => union_df.unique_values()?,
                InferDomainFromDataMethod::All => union_df.all_values()?,
            }
        }
        ScaleDefaultDomain::Discrete(values) => make_array(values.clone()),
    };

    // Use raw domain if not null, otherwise use default domain
    let domain_expr =
        when(raw_expr.clone().is_not_null(), raw_expr).otherwise(default_domain_expr)?;
    Ok(domain_expr)
}

fn compile_range(range: &ScaleRange) -> Result<Expr, AvengerChartError> {
    match range {
        ScaleRange::Numeric(start_expr, end_expr) => {
            Ok(make_array(vec![start_expr.clone(), end_expr.clone()]))
        }
        ScaleRange::Color(colors) => {
            let colors = colors
                .iter()
                .map(|c| lit(ScalarValue::make_rgba(c.red, c.green, c.blue, c.alpha)))
                .collect::<Vec<_>>();

            Ok(make_array(colors))
        }
        _ => {
            todo!("evaluate range")
        }
    }
}

fn compile_options(options: &HashMap<String, Expr>) -> Result<Expr, AvengerChartError> {
    let mut struct_args = options
        .iter()
        .flat_map(|(key, value)| vec![lit(key), value.clone()])
        .collect::<Vec<_>>();

    if struct_args.is_empty() {
        struct_args.extend(vec![lit("_dummy"), lit(0.0f32)]);
    }

    Ok(named_struct(struct_args))
}

pub async fn eval_scale(
    scale: &Scale,
    ctx: &SessionContext,
    params: Option<&ParamValues>,
) -> Result<ConfiguredScale, AvengerChartError> {
    let scale_impl = scale.get_scale_impl().clone();

    // Extract domain as Array
    let domain_scalar = compile_domain(
        scale.get_domain(),
        scale_impl.infer_domain_from_data_method(),
    )?
    .eval_to_scalar(ctx, params)
    .await?;

    let ScalarValue::List(domain_list_array) = domain_scalar else {
        return Err(AvengerChartError::InternalError(format!(
            "Unexpected domain value: {:?}",
            scale.get_domain()
        )));
    };
    let domain_array = domain_list_array.value(0);

    // Extract range as Array
    let range_scalar = compile_range(scale.get_range())?
        .eval_to_scalar(ctx, params)
        .await?;
    let ScalarValue::List(range_list_array) = range_scalar else {
        return Err(AvengerChartError::InternalError(
            "Unexpected range value: {".to_string(),
        ));
    };
    let range_array = range_list_array.value(0);

    // Eval options
    let mut options = HashMap::new();
    for (k, v) in scale.get_options().iter() {
        options.insert(k.to_string(), v.eval_to_scalar(ctx, params).await?);
    }

    Ok(ConfiguredScale {
        scale_impl: scale_impl.clone(),
        config: ScaleConfig {
            domain: domain_array,
            range: range_array,
            options,
            context: ScaleContext::default(),
        },
    })
}
