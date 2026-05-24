use std::sync::Arc;

use async_trait::async_trait;
use avenger_scales::scalar::Scalar;
use datafusion::{
    arrow::{
        array::{ArrayRef, ListArray},
        datatypes::DataType,
    },
    common::{
        ParamValues, Spans,
        tree_node::{TreeNode, TreeNodeRecursion},
    },
    error::DataFusionError,
    functions_aggregate::{
        expr_fn::array_agg,
        min_max::{max, min},
    },
    functions_array::expr_fn::{array_sort, make_array},
    logical_expr::{Subquery, cast as cast_expr, try_cast},
    optimizer::simplify_expressions::{ExprSimplifier, SimplifyContext},
    prelude::{DataFrame, Expr, SessionContext, col, lit},
    scalar::ScalarValue,
};
use datafusion_common::ToDFSchema;
use indexmap::IndexMap;

use crate::AvengerChartError;

pub trait DataFrameChartHelpers {
    /// Return two-element array of min and max values across all the columns in the input DataFrame.
    fn span(&self) -> Result<Expr, AvengerChartError>;

    /// Return single-column DataFrame with all columns in the input DataFrame unioned together.
    fn union_all_cols(&self, col_name: Option<&str>) -> Result<DataFrame, AvengerChartError>;

    /// Return an array expression with unique values across all the columns in the input DataFrame.
    fn unique_values(&self) -> Result<Expr, AvengerChartError>;

    /// Return an array expression with all values across all the columns in the input DataFrame.
    fn all_values(&self) -> Result<Expr, AvengerChartError>;
}

impl DataFrameChartHelpers for DataFrame {
    fn span(&self) -> Result<Expr, AvengerChartError> {
        let mut union_dfs: Vec<DataFrame> = Vec::new();
        let col_name = "span_col";

        for field in self.schema().fields() {
            if field.data_type().is_numeric() {
                union_dfs.push(self.clone().select(vec![
                    cast_expr(col(field.name()), DataType::Float32).alias(col_name),
                ])?)
            } else if matches!(
                field.data_type(),
                DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _)
            ) {
                union_dfs.push(
                    self.clone()
                        .select(vec![col(field.name()).alias(col_name)])?,
                )
            } else if let Ok(df_with_cast) = self.clone().select(vec![
                try_cast(col(field.name()), DataType::Float32).alias(col_name),
            ]) && let Ok(filtered_df) = df_with_cast.filter(col(col_name).is_not_null())
            {
                union_dfs.push(filtered_df);
            }
        }

        if union_dfs.is_empty() {
            return Err(AvengerChartError::InternalError(
                "No numeric columns found for span".to_string(),
            ));
        }

        let union_df = union_dfs
            .iter()
            .skip(1)
            .fold(union_dfs[0].clone(), |acc, df| {
                acc.union(df.clone()).unwrap()
            });

        let df = union_df
            .clone()
            .aggregate(
                vec![],
                vec![
                    min(col(col_name)).alias("min_val"),
                    max(col(col_name)).alias("max_val"),
                ],
            )?
            .select(vec![
                make_array(vec![col("min_val"), col("max_val")]).alias("span"),
            ])?;

        let subquery = Subquery {
            subquery: Arc::new(df.logical_plan().clone()),
            outer_ref_columns: vec![],
            spans: Spans(vec![]),
        };
        Ok(Expr::ScalarSubquery(subquery))
    }

    fn union_all_cols(&self, col_name: Option<&str>) -> Result<DataFrame, AvengerChartError> {
        let mut union_dfs: Vec<DataFrame> = Vec::new();
        let col_name = col_name.unwrap_or("vals");

        for field in self.schema().fields() {
            union_dfs.push(
                self.clone()
                    .select(vec![col(field.name()).alias(col_name)])?,
            );
        }

        if union_dfs.is_empty() {
            return Err(AvengerChartError::InternalError(
                "No columns found for union".to_string(),
            ));
        }

        Ok(union_dfs
            .iter()
            .skip(1)
            .fold(union_dfs[0].clone(), |acc, df| {
                acc.union(df.clone()).unwrap()
            }))
    }

    fn unique_values(&self) -> Result<Expr, AvengerChartError> {
        let col_name = "vals";
        let union_df = self.union_all_cols(Some(col_name))?;
        let uniques_df = union_df
            .clone()
            .distinct()?
            .aggregate(vec![], vec![array_agg(col(col_name)).alias("unique_vals")])?
            .select(vec![
                array_sort(col("unique_vals"), lit("ASC"), lit("NULLS FIRST")).alias("unique_vals"),
            ])?;

        let subquery = Subquery {
            subquery: Arc::new(uniques_df.logical_plan().clone()),
            outer_ref_columns: vec![],
            spans: Spans(vec![]),
        };
        Ok(Expr::ScalarSubquery(subquery))
    }

    fn all_values(&self) -> Result<Expr, AvengerChartError> {
        let col_name = "vals";
        let union_df = self.union_all_cols(Some(col_name))?;
        let all_values_df = union_df
            .clone()
            .aggregate(vec![], vec![array_agg(col(col_name)).alias("all_vals")])?
            .select(vec![col("all_vals")])?;

        let subquery = Subquery {
            subquery: Arc::new(all_values_df.logical_plan().clone()),
            outer_ref_columns: vec![],
            spans: Spans(vec![]),
        };
        Ok(Expr::ScalarSubquery(subquery))
    }
}

/// Extension trait for Expr to help with evaluation.
#[async_trait]
pub trait ExprHelpers {
    async fn eval_to_scalar(
        &self,
        ctx: Option<&SessionContext>,
        params: Option<&ParamValues>,
    ) -> Result<ScalarValue, DataFusionError>;
}

#[async_trait]
impl ExprHelpers for Expr {
    async fn eval_to_scalar(
        &self,
        ctx: Option<&SessionContext>,
        params: Option<&ParamValues>,
    ) -> Result<ScalarValue, DataFusionError> {
        let mut result = eval_to_scalars(vec![self.clone()], ctx, params).await?;
        result
            .pop()
            .ok_or_else(|| DataFusionError::Internal("Failed to evaluate expression".to_string()))
    }
}

pub async fn eval_to_scalars(
    mut exprs: Vec<Expr>,
    ctx: Option<&SessionContext>,
    params: Option<&ParamValues>,
) -> Result<Vec<ScalarValue>, DataFusionError> {
    let mut result_scalars = vec![];
    while let Some(Expr::Literal(scalar, _)) = exprs.first() {
        result_scalars.push(scalar.clone());
        exprs.remove(0);
    }

    if exprs.is_empty() {
        return Ok(result_scalars);
    }

    let aliased_exprs = exprs
        .into_iter()
        .enumerate()
        .map(|(ind, e)| {
            let name = format!("value_{}", ind);
            if let Expr::Alias(alias) = e {
                alias.expr.alias(name)
            } else {
                e.alias(name)
            }
        })
        .collect::<Vec<_>>();

    let ctx = ctx.cloned().unwrap_or_else(SessionContext::new);
    let df = ctx.read_empty()?;

    let res = df
        .select(aliased_exprs)?
        .with_param_values(
            params
                .cloned()
                .unwrap_or_else(|| ParamValues::Map(Default::default())),
        )?
        .collect()
        .await?;
    let batch = res.first().ok_or_else(|| {
        DataFusionError::Internal("No results returned from evaluation".to_string())
    })?;

    if res.is_empty() || res[0].num_rows() == 0 {
        return Err(DataFusionError::Internal(
            "Failed to evaluate expressions".to_string(),
        ));
    }

    for i in 0..batch.num_columns() {
        result_scalars.push(
            ScalarValue::try_from_array(batch.column(i), 0).map_err(|e| {
                DataFusionError::Internal(format!("Failed to convert column {}: {}", i, e))
            })?,
        );
    }
    Ok(result_scalars)
}

/// Synchronously simplify expressions to scalars using ExprSimplifier.
pub fn simplify_to_scalar_sync(expr: Expr) -> Result<ScalarValue, DataFusionError> {
    if let Expr::Literal(scalar, _) = expr {
        return Ok(scalar);
    }

    if !expr.column_refs().is_empty() {
        return Err(DataFusionError::Plan(
            "Expression contains column references and cannot be simplified to a scalar"
                .to_string(),
        ));
    }

    let ctx = SessionContext::new();
    let state = ctx.state();
    let props = state.execution_props();
    let empty_schema = datafusion::arrow::datatypes::Schema::empty().to_dfschema_ref()?;
    let context = SimplifyContext::new(props).with_schema(empty_schema);
    let simplifier = ExprSimplifier::new(context).with_canonicalize(true);
    let simplified = simplifier.simplify(expr.clone())?;

    tracing::trace!("Simplified {:?} to {:?}", expr, simplified);

    match simplified {
        Expr::Literal(scalar, _) => Ok(scalar),
        _ => Err(DataFusionError::Plan(format!(
            "Expression could not be simplified to a scalar: {:?}",
            simplified
        ))),
    }
}

pub trait ScalarValueHelpers {
    fn as_i32(&self) -> Result<i32, DataFusionError>;
    fn as_f32(&self) -> Result<f32, DataFusionError>;
    fn as_f64(&self) -> Result<f64, DataFusionError>;
    fn as_f32x2(&self) -> Result<[f32; 2], DataFusionError>;
    fn as_f64x2(&self) -> Result<[f64; 2], DataFusionError>;
    fn as_scalar_string(&self) -> Result<String, DataFusionError>;
    fn as_scale_scalar(&self) -> Result<Scalar, DataFusionError>;
}

impl ScalarValueHelpers for ScalarValue {
    fn as_i32(&self) -> Result<i32, DataFusionError> {
        Ok(match self {
            ScalarValue::Float32(Some(e)) => *e as i32,
            ScalarValue::Float64(Some(e)) => *e as i32,
            ScalarValue::Int8(Some(e)) => *e as i32,
            ScalarValue::Int16(Some(e)) => *e as i32,
            ScalarValue::Int32(Some(e)) => *e,
            ScalarValue::Int64(Some(e)) => *e as i32,
            ScalarValue::UInt8(Some(e)) => *e as i32,
            ScalarValue::UInt16(Some(e)) => *e as i32,
            ScalarValue::UInt32(Some(e)) => *e as i32,
            ScalarValue::UInt64(Some(e)) => *e as i32,
            _ => {
                return Err(DataFusionError::Internal(format!(
                    "Cannot convert {self} to i32"
                )));
            }
        })
    }

    fn as_f32(&self) -> Result<f32, DataFusionError> {
        Ok(self.as_f64()? as f32)
    }

    fn as_f64(&self) -> Result<f64, DataFusionError> {
        Ok(match self {
            ScalarValue::Float32(Some(e)) => *e as f64,
            ScalarValue::Float64(Some(e)) => *e,
            ScalarValue::Int8(Some(e)) => *e as f64,
            ScalarValue::Int16(Some(e)) => *e as f64,
            ScalarValue::Int32(Some(e)) => *e as f64,
            ScalarValue::Int64(Some(e)) => *e as f64,
            ScalarValue::UInt8(Some(e)) => *e as f64,
            ScalarValue::UInt16(Some(e)) => *e as f64,
            ScalarValue::UInt32(Some(e)) => *e as f64,
            ScalarValue::UInt64(Some(e)) => *e as f64,
            ScalarValue::Date32(Some(e)) => *e as f64,
            ScalarValue::Date64(Some(e)) => *e as f64,
            ScalarValue::TimestampSecond(Some(e), _) => *e as f64,
            ScalarValue::TimestampMillisecond(Some(e), _) => *e as f64,
            ScalarValue::TimestampMicrosecond(Some(e), _) => *e as f64,
            ScalarValue::TimestampNanosecond(Some(e), _) => *e as f64,
            _ => {
                return Err(DataFusionError::Internal(format!(
                    "Cannot convert {self} to f64"
                )));
            }
        })
    }

    fn as_f32x2(&self) -> Result<[f32; 2], DataFusionError> {
        let f64x2 = self.as_f64x2()?;
        Ok([f64x2[0] as f32, f64x2[1] as f32])
    }

    fn as_f64x2(&self) -> Result<[f64; 2], DataFusionError> {
        if let ScalarValue::List(array) = self {
            let elements = array.value(0).to_scalar_vec()?;
            if let [v0, v1] = elements.as_slice() {
                return Ok([v0.as_f64()?, v1.as_f64()?]);
            }
        }
        Err(DataFusionError::Internal(format!(
            "Cannot convert {self} to [f64; 2]"
        )))
    }

    fn as_scalar_string(&self) -> Result<String, DataFusionError> {
        Ok(match self {
            ScalarValue::Utf8(Some(value)) => value.clone(),
            ScalarValue::LargeUtf8(Some(value)) => value.clone(),
            ScalarValue::Utf8View(Some(value)) => value.clone(),
            _ => {
                return Err(DataFusionError::Internal(format!(
                    "Cannot convert {self} to String"
                )));
            }
        })
    }

    fn as_scale_scalar(&self) -> Result<Scalar, DataFusionError> {
        let scalar = match self {
            Self::Float64(Some(v)) => Scalar::from_f32(*v as f32),
            ScalarValue::Float32(Some(v)) => Scalar::from_f32(*v),
            ScalarValue::Int64(Some(v)) => Scalar::from_f32(*v as f32),
            ScalarValue::Int32(Some(v)) => Scalar::from_f32(*v as f32),
            ScalarValue::Boolean(Some(v)) => Scalar::from_bool(*v),
            _ => {
                return Err(DataFusionError::Internal(format!(
                    "Cannot convert {self} to avenger_scales::scalar::Scalar"
                )));
            }
        };
        Ok(scalar)
    }
}

pub trait ArrayRefHelpers {
    fn to_scalar_vec(&self) -> Result<Vec<ScalarValue>, DataFusionError>;
    fn list_el_to_scalar_vec(&self) -> Result<Vec<ScalarValue>, DataFusionError>;
    fn list_el_len(&self) -> Result<usize, DataFusionError>;
    fn list_el_dtype(&self) -> Result<DataType, DataFusionError>;
}

impl ArrayRefHelpers for ArrayRef {
    fn to_scalar_vec(&self) -> Result<Vec<ScalarValue>, DataFusionError> {
        (0..self.len())
            .map(|i| ScalarValue::try_from_array(self, i))
            .collect::<Result<Vec<_>, DataFusionError>>()
    }

    fn list_el_to_scalar_vec(&self) -> Result<Vec<ScalarValue>, DataFusionError> {
        let a = self
            .as_any()
            .downcast_ref::<ListArray>()
            .ok_or(DataFusionError::Internal(
                "list_el_to_scalar_vec called on non-List type".to_string(),
            ))?;
        a.value(0).to_scalar_vec()
    }

    fn list_el_len(&self) -> Result<usize, DataFusionError> {
        let a = self
            .as_any()
            .downcast_ref::<ListArray>()
            .ok_or(DataFusionError::Internal(
                "list_el_len called on non-List type".to_string(),
            ))?;
        Ok(a.value(0).len())
    }

    fn list_el_dtype(&self) -> Result<DataType, DataFusionError> {
        let a = self
            .as_any()
            .downcast_ref::<ListArray>()
            .ok_or(DataFusionError::Internal(
                "list_el_len called on non-List type".to_string(),
            ))?;
        Ok(a.value(0).data_type().clone())
    }
}

/// Convert avenger_scales::scalar::Scalar to datafusion::scalar::ScalarValue.
pub fn scalar_to_scalar_value(scalar: &Scalar) -> ScalarValue {
    if let Ok(b) = scalar.as_boolean() {
        ScalarValue::Boolean(Some(b))
    } else if let Ok(f) = scalar.as_f32() {
        ScalarValue::Float32(Some(f))
    } else if let Ok(i) = scalar.as_i32() {
        ScalarValue::Int32(Some(i))
    } else if let Ok(s) = scalar.as_string() {
        ScalarValue::Utf8(Some(s))
    } else {
        ScalarValue::Null
    }
}

/// Convert IndexMap of parameter values to DataFusion's ParamValues::Map variant.
pub fn params_to_datafusion(params: &IndexMap<String, ScalarValue>) -> Option<ParamValues> {
    if params.is_empty() {
        None
    } else {
        Some(ParamValues::Map(params.clone().into_iter().collect()))
    }
}

/// Check if an expression contains any aggregate functions.
pub fn contains_aggregate(expr: &Expr) -> bool {
    let mut has_aggregate = false;
    let _ = expr.apply(|e| {
        if matches!(e, Expr::AggregateFunction(_)) {
            has_aggregate = true;
        }
        Ok(TreeNodeRecursion::Continue)
    });
    has_aggregate
}

/// Partition a list of expressions into grouping and aggregate expressions.
pub fn partition_expressions(exprs: Vec<Expr>) -> (Vec<Expr>, Vec<Expr>) {
    let mut group_by = Vec::new();
    let mut aggregates = Vec::new();

    for expr in exprs {
        if contains_aggregate(&expr) {
            aggregates.push(expr);
        } else {
            group_by.push(expr);
        }
    }

    (group_by, aggregates)
}
