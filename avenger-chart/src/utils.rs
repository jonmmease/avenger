use crate::error::AvengerChartError;
use arrow::array::{ArrayRef, ListArray};
use arrow::datatypes::DataType;
use async_trait::async_trait;
use avenger_scales::scalar::Scalar;
use datafusion::common::{ParamValues, Spans};
use datafusion::error::DataFusionError;
use datafusion::functions_aggregate::expr_fn::array_agg;
use datafusion::functions_aggregate::min_max::{max, min};
use datafusion::functions_array::expr_fn::{array_sort, make_array};
use datafusion::logical_expr::Subquery;
use datafusion::prelude::{DataFrame, Expr, SessionContext, col, lit};
use datafusion::scalar::ScalarValue;
use std::sync::Arc;

pub trait DataFrameChartHelpers {
    /// Return two-element array of min and max values across all of the columns in the input DataFrame
    fn span(&self) -> Result<Expr, AvengerChartError>;

    /// Return single-column DataFrame with all columns in the input DataFrame unioned (concatenated) together
    fn union_all_cols(&self, col_name: Option<&str>) -> Result<DataFrame, AvengerChartError>;

    /// Return an array expression with unique values across all of the columns in the input DataFrame
    fn unique_values(&self) -> Result<Expr, AvengerChartError>;

    /// Return an array expression with all values across all of the columns in the input DataFrame
    fn all_values(&self) -> Result<Expr, AvengerChartError>;
}

impl DataFrameChartHelpers for DataFrame {
    fn span(&self) -> Result<Expr, AvengerChartError> {
        // Collect single column DataFrames for all numeric columns
        let mut union_dfs: Vec<DataFrame> = Vec::new();
        let col_name = "span_col";

        for field in self.schema().fields() {
            if field.data_type().is_numeric() {
                use datafusion::arrow::datatypes::DataType;
                use datafusion::logical_expr::cast;

                union_dfs.push(self.clone().select(vec![
                    cast(col(field.name()), DataType::Float32).alias(col_name),
                ])?);
            }
        }

        if union_dfs.is_empty() {
            return Err(AvengerChartError::InternalError(
                "No numeric columns found for span".to_string(),
            ));
        }

        // Union all the DataFrames
        let union_df = union_dfs
            .iter()
            .skip(1)
            .fold(union_dfs[0].clone(), |acc, df| {
                acc.union(df.clone()).unwrap()
            });

        // Compute domain from column
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
        // Collect single column DataFrames for all columns. Let DataFusion try to unify the types
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

        // Collect unique values in array
        let union_df = self.union_all_cols(Some(col_name))?;
        // Get distinct values, aggregate, then sort the resulting array
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

/// Create a unit DataFrame (single row, no columns) for testing
pub fn unit_dataframe() -> Result<DataFrame, AvengerChartError> {
    use datafusion::prelude::SessionContext;

    let ctx = SessionContext::new();
    let df = ctx.read_batch(datafusion::arrow::record_batch::RecordBatch::try_new(
        Arc::new(datafusion::arrow::datatypes::Schema::empty()),
        vec![],
    )?)?;
    Ok(df)
}

/// Extension trait for Expr to help with evaluation
#[async_trait]
pub trait ExprHelpers {
    /// Evaluate expression to a scalar value
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
    // As optimization, convert literal expressions to scalars starting from the left.
    // This is the common case, and can be handled very efficiently. For simplicity,
    // once we encounter the first non-literal expression, we process the rest through
    // evaluation, without checking if any later expressions are literals.
    let mut result_scalars = vec![];
    while let Some(Expr::Literal(scalar, _)) = exprs.first() {
        result_scalars.push(scalar.clone());
        exprs.remove(0);
    }

    if exprs.is_empty() {
        return Ok(result_scalars);
    }

    // Otherwise, we need to evaluate the remaining expressions
    let aliased_exprs = exprs
        .into_iter()
        .enumerate()
        .map(|(ind, e)| {
            let name = format!("value_{}", ind);
            if let Expr::Alias(alias) = e {
                // Unwrap existing alias if present
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

pub trait ScalarValueHelpers {
    fn as_i32(&self) -> Result<i32, DataFusionError>;
    fn as_f32(&self) -> Result<f32, DataFusionError>;
    fn as_f64(&self) -> Result<f64, DataFusionError>;
    fn as_f32x2(&self) -> Result<[f32; 2], DataFusionError>;
    fn as_f64x2(&self) -> Result<[f64; 2], DataFusionError>;
    fn as_scalar_string(&self) -> Result<String, DataFusionError>;
    fn negate(&self) -> Self;
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

    fn negate(&self) -> Self {
        match self {
            ScalarValue::Float32(Some(e)) => ScalarValue::Float32(Some(-*e)),
            ScalarValue::Float64(Some(e)) => ScalarValue::Float64(Some(-*e)),
            ScalarValue::Int8(Some(e)) => ScalarValue::Int8(Some(-*e)),
            ScalarValue::Int16(Some(e)) => ScalarValue::Int16(Some(-*e)),
            ScalarValue::Int32(Some(e)) => ScalarValue::Int32(Some(-*e)),
            ScalarValue::Int64(Some(e)) => ScalarValue::Int64(Some(-*e)),
            ScalarValue::UInt8(Some(e)) => ScalarValue::Int16(Some(-(*e as i16))),
            ScalarValue::UInt16(Some(e)) => ScalarValue::Int32(Some(-(*e as i32))),
            ScalarValue::UInt32(Some(e)) => ScalarValue::Int64(Some(-(*e as i64))),
            ScalarValue::UInt64(Some(e)) => ScalarValue::Int64(Some(-(*e as i64))),
            _ => self.clone(),
        }
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
    /// Convert ArrayRef into vector of ScalarValues
    fn to_scalar_vec(&self) -> Result<Vec<ScalarValue>, DataFusionError> {
        (0..self.len())
            .map(|i| ScalarValue::try_from_array(self, i))
            .collect::<Result<Vec<_>, DataFusionError>>()
    }

    /// Extract Vec<ScalarValue> for single element ListArray (as is stored inside ScalarValue::List(arr))
    fn list_el_to_scalar_vec(&self) -> Result<Vec<ScalarValue>, DataFusionError> {
        let a = self
            .as_any()
            .downcast_ref::<ListArray>()
            .ok_or(DataFusionError::Internal(
                "list_el_to_scalar_vec called on non-List type".to_string(),
            ))?;
        a.value(0).to_scalar_vec()
    }

    /// Extract length of single element ListArray
    fn list_el_len(&self) -> Result<usize, DataFusionError> {
        let a = self
            .as_any()
            .downcast_ref::<ListArray>()
            .ok_or(DataFusionError::Internal(
                "list_el_len called on non-List type".to_string(),
            ))?;
        Ok(a.value(0).len())
    }

    /// Extract data type of single element ListArray
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

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Array, Float32Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;

    #[tokio::test]
    async fn test_span_numeric_columns() {
        // Create test data with numeric columns
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Float32, false),
            Field::new("b", DataType::Float32, false),
            Field::new("c", DataType::Float32, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0])),
                Arc::new(Float32Array::from(vec![4.0, 5.0, 6.0])),
                Arc::new(Float32Array::from(vec![7.0, 8.0, 9.0])),
            ],
        )
        .unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create span expression
        let span_expr = df.span().unwrap();

        // The span expression should be a subquery that computes min/max across all numeric columns
        // When evaluated, it should return [1.0, 9.0] since 1.0 is the min and 9.0 is the max

        // To test, we evaluate the expression
        let result_df = ctx
            .read_empty()
            .unwrap()
            .select(vec![span_expr.alias("span")])
            .unwrap();
        let batches = result_df.collect().await.unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 1);

        let span_array = batches[0].column_by_name("span").unwrap();
        let list_array = span_array
            .as_any()
            .downcast_ref::<datafusion::arrow::array::ListArray>()
            .unwrap();
        let inner_array = list_array.value(0);
        let float_array = inner_array.as_any().downcast_ref::<Float32Array>().unwrap();

        assert_eq!(float_array.len(), 2);
        assert_eq!(float_array.value(0), 1.0);
        assert_eq!(float_array.value(1), 9.0);
    }

    #[tokio::test]
    async fn test_unique_values_string_columns() {
        // Create test data with string columns
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("col1", DataType::Utf8, false),
            Field::new("col2", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["A", "B", "A"])),
                Arc::new(StringArray::from(vec!["B", "C", "D"])),
            ],
        )
        .unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create unique values expression
        let unique_expr = df.unique_values().unwrap();

        // The unique values expression should return ["A", "B", "C", "D"]
        let result_df = ctx
            .read_empty()
            .unwrap()
            .select(vec![unique_expr.alias("unique_vals")])
            .unwrap();
        let batches = result_df.collect().await.unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 1);

        let unique_array = batches[0].column_by_name("unique_vals").unwrap();

        // array_agg returns a ListArray, we need to extract the inner array
        let list_array = unique_array
            .as_any()
            .downcast_ref::<datafusion::arrow::array::ListArray>()
            .unwrap();
        assert_eq!(list_array.len(), 1); // Should have one row

        let inner_array = list_array.value(0);
        let str_array = inner_array.as_any().downcast_ref::<StringArray>().unwrap();

        // Convert to vec and sort for consistent comparison
        let mut values: Vec<String> = (0..str_array.len())
            .map(|i| str_array.value(i).to_string())
            .collect();
        values.sort();

        assert_eq!(values, vec!["A", "B", "C", "D"]);
    }

    #[tokio::test]
    async fn test_span_mixed_numeric_types() {
        // Test with different numeric types (int32, float64, etc)
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("int_col", DataType::Int32, false),
            Field::new("float_col", DataType::Float64, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(datafusion::arrow::array::Int32Array::from(vec![10, 20, 30])),
                Arc::new(datafusion::arrow::array::Float64Array::from(vec![
                    5.5, 15.5, 25.5,
                ])),
            ],
        )
        .unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create span expression
        let span_expr = df.span().unwrap();

        // Should find min=5.5 and max=30.0 across both columns
        let result_df = ctx
            .read_empty()
            .unwrap()
            .select(vec![span_expr.alias("span")])
            .unwrap();
        let batches = result_df.collect().await.unwrap();

        let span_array = batches[0].column_by_name("span").unwrap();
        let list_array = span_array
            .as_any()
            .downcast_ref::<datafusion::arrow::array::ListArray>()
            .unwrap();
        let inner_array = list_array.value(0);
        let float_array = inner_array.as_any().downcast_ref::<Float32Array>().unwrap();

        assert_eq!(float_array.value(0), 5.5);
        assert_eq!(float_array.value(1), 30.0);
    }

    #[tokio::test]
    async fn test_all_values() {
        // Test all_values which should return all values (including duplicates)
        let ctx = SessionContext::new();

        let schema = Arc::new(Schema::new(vec![
            Field::new("col1", DataType::Utf8, false),
            Field::new("col2", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["A", "B", "A"])),
                Arc::new(StringArray::from(vec!["B", "C", "A"])),
            ],
        )
        .unwrap();

        let df = ctx.read_batch(batch).unwrap();

        // Create all values expression
        let all_expr = df.all_values().unwrap();

        // Should return all 6 values: ["A", "B", "A", "B", "C", "A"]
        let result_df = ctx
            .read_empty()
            .unwrap()
            .select(vec![all_expr.alias("all_vals")])
            .unwrap();
        let batches = result_df.collect().await.unwrap();

        let all_array = batches[0].column_by_name("all_vals").unwrap();

        // array_agg returns a ListArray, extract the inner array
        let list_array = all_array
            .as_any()
            .downcast_ref::<datafusion::arrow::array::ListArray>()
            .unwrap();
        assert_eq!(list_array.len(), 1);

        let inner_array = list_array.value(0);
        let str_array = inner_array.as_any().downcast_ref::<StringArray>().unwrap();

        assert_eq!(str_array.len(), 6);

        // Count occurrences
        let values: Vec<&str> = (0..str_array.len()).map(|i| str_array.value(i)).collect();

        assert_eq!(values.iter().filter(|&&v| v == "A").count(), 3);
        assert_eq!(values.iter().filter(|&&v| v == "B").count(), 2);
        assert_eq!(values.iter().filter(|&&v| v == "C").count(), 1);
    }
}
