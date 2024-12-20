use arrow::array::{Array, ArrayRef, AsArray, BooleanArray, RecordBatch};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Float32Type};
use async_trait::async_trait;
use datafusion::common::tree_node::{Transformed, TreeNode, TreeNodeRewriter};
use datafusion::common::{Column, DFSchema, ExprSchema, ParamValues, ScalarValue};
use datafusion::error::DataFusionError;
use datafusion::functions_aggregate::expr_fn::array_agg;
use datafusion::functions_aggregate::min_max::{max, min};
use datafusion::functions_array::make_array::make_array;
use datafusion::logical_expr::expr::Placeholder;
use datafusion::logical_expr::utils::expr_to_columns;
use datafusion::logical_expr::{Expr, ExprSchemable, SortExpr, Subquery, TryCast};
use datafusion::optimizer::simplify_expressions::SimplifyInfo;
use datafusion::physical_expr::execution_props::ExecutionProps;
use datafusion::prelude::{array_sort, ident, lit, DataFrame, SessionContext};
use lazy_static::lazy_static;
use std::collections::HashSet;
use std::convert::TryFrom;
use std::sync::Arc;

use crate::error::AvengerChartError;

lazy_static! {
    pub static ref UNIT_RECORD_BATCH: RecordBatch = RecordBatch::try_from_iter(vec![(
        "__unit__",
        Arc::new(BooleanArray::from(vec![true])) as ArrayRef
    )])
    .unwrap();
    pub static ref UNIT_SCHEMA: DFSchema =
        DFSchema::try_from(UNIT_RECORD_BATCH.schema().as_ref().clone()).unwrap();
}

// Create a parameter reference expression
pub fn param<S: Into<String>>(name: S) -> Expr {
    Expr::Placeholder(Placeholder::new(format!("${}", name.into()), None))
}

#[async_trait]
pub trait ExprHelpers {
    fn columns(&self) -> Result<HashSet<Column>, DataFusionError>;
    fn apply_params(self, params: &ParamValues) -> Result<Expr, DataFusionError>;
    async fn eval_to_scalar(
        &self,
        ctx: &SessionContext,
        params: Option<&ParamValues>,
    ) -> Result<ScalarValue, DataFusionError>;
    async fn eval_to_f32(
        &self,
        ctx: &SessionContext,
        params: Option<&ParamValues>,
    ) -> Result<f32, AvengerChartError>;
    fn try_cast_to(
        self,
        cast_to_type: &DataType,
        schema: &dyn ExprSchema,
    ) -> Result<Expr, DataFusionError>;
}

#[async_trait]
impl ExprHelpers for Expr {
    fn columns(&self) -> Result<HashSet<Column>, DataFusionError> {
        let mut columns: HashSet<Column> = HashSet::new();
        expr_to_columns(self, &mut columns)?;
        Ok(columns)
    }

    /// Apply param replacements directly to an expression
    /// This isn't usually needed if the params are applied to the DataFrame containing this
    /// expression, but some expression operations, like casting, require the full schema and so
    /// the params must be filled in first.
    fn apply_params(self, params: &ParamValues) -> Result<Expr, DataFusionError> {
        let mut replacer = ExprParamReplacer::new(params);
        let transformed = self.rewrite(&mut replacer)?;
        Ok(transformed.data)
    }

    async fn eval_to_scalar(
        &self,
        ctx: &SessionContext,
        params: Option<&ParamValues>,
    ) -> Result<ScalarValue, DataFusionError> {
        if !self.columns()?.is_empty() {
            return Err(DataFusionError::Internal(format!(
                "Cannot eval_to_scalar for Expr with column references: {self:?}"
            )));
        }
        let df = ctx.read_batch(UNIT_RECORD_BATCH.clone())?;

        // Normalize params
        let params = params
            .cloned()
            .unwrap_or_else(|| ParamValues::Map(Default::default()));

        let res = df
            .select(vec![self.clone().alias("value")])?
            .with_param_values(params)?
            .collect()
            .await?;
        let row = res.get(0).unwrap();
        let col = row.column_by_name("value").unwrap();
        let scalar = ScalarValue::try_from_array(col, 0)?;
        Ok(scalar)
    }

    async fn eval_to_f32(
        &self,
        ctx: &SessionContext,
        params: Option<&ParamValues>,
    ) -> Result<f32, AvengerChartError> {
        // Must apply params before looking up the schema, or DataFusion errors
        let expr = if let Some(params) = params {
            self.clone().apply_params(params)?
        } else {
            self.clone()
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

    fn try_cast_to(
        self,
        cast_to_type: &DataType,
        schema: &dyn ExprSchema,
    ) -> Result<Expr, DataFusionError> {
        // Based on cast_to, using TryCast instead of Cast
        let this_type = self.get_type(schema)?;
        if this_type == *cast_to_type {
            return Ok(self);
        }
        Ok(Expr::TryCast(TryCast::new(
            Box::new(self),
            cast_to_type.clone(),
        )))
    }
}

pub struct ExprParamReplacer<'a> {
    params: &'a ParamValues,
}

impl<'a> ExprParamReplacer<'a> {
    pub fn new(params: &'a ParamValues) -> Self {
        Self { params }
    }
}

impl<'a> TreeNodeRewriter for ExprParamReplacer<'a> {
    type Node = Expr;

    fn f_down(&mut self, node: Self::Node) -> Result<Transformed<Self::Node>, DataFusionError> {
        if let Expr::Placeholder(Placeholder { id, .. }) = node {
            let value = self.params.get_placeholders_with_values(&id)?;
            Ok(Transformed::yes(Expr::Literal(value)))
        } else {
            Ok(Transformed::no(node))
        }
    }
}

pub trait DataFrameChartUtils {
    /// Return two-element float32 array of min and max values across all of the columns in the input DataFrame
    fn span(&self) -> Result<Expr, AvengerChartError>;

    /// Return single-column DataFrame with all columns in the input DataFrame unioned (concatenated) together
    fn union_all_cols(&self, col_name: Option<&str>) -> Result<DataFrame, AvengerChartError>;

    /// Return DataFrame with single column of unique values across all of the columns in the input DataFrame
    fn uniques(
        &self,
        sort_ascending: Option<bool>,
        col_name: Option<&str>,
    ) -> Result<DataFrame, AvengerChartError>;

    // fn scalar_aggregate(&self, expr: Expr) -> Result<Expr, AvengerChartError>;
}

impl DataFrameChartUtils for DataFrame {
    fn span(&self) -> Result<Expr, AvengerChartError> {
        // Collect single column DataFrames for all numeric columns
        let mut union_dfs: Vec<DataFrame> = Vec::new();
        let col_name = "span_col";
        for field in self.schema().fields() {
            if field.data_type().is_numeric() {
                union_dfs.push(
                    self.clone()
                        .select(vec![ident(field.name())
                            .try_cast_to(&DataType::Float32, self.schema())?
                            .alias(col_name)])?
                        .clone(),
                );
            }
        }

        if union_dfs.is_empty() {
            return Err(AvengerChartError::InternalError(
                "No numeric columns found for span".to_string(),
            ));
        }

        // Union all the DataFrames
        let union_df = union_dfs.iter().fold(union_dfs[0].clone(), |acc, df| {
            acc.union(df.clone()).unwrap()
        });

        // Compute domain from column
        let df = union_df
            .clone()
            .aggregate(
                vec![],
                vec![
                    min(ident(col_name)).alias("min_val"),
                    max(ident(col_name)).alias("max_val"),
                ],
            )?
            .select(vec![
                make_array(vec![ident("min_val"), ident("max_val")]).alias("span")
            ])?;

        let subquery = Subquery {
            subquery: Arc::new(df.logical_plan().clone()),
            outer_ref_columns: vec![],
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
                    .select(vec![ident(field.name()).alias(col_name)])?
                    .clone(),
            );
        }
        Ok(union_dfs.iter().fold(union_dfs[0].clone(), |acc, df| {
            acc.union(df.clone()).unwrap()
        }))
    }

    fn uniques(
        &self,
        sort_ascending: Option<bool>,
        col_name: Option<&str>,
    ) -> Result<DataFrame, AvengerChartError> {
        let col_name = col_name.unwrap_or("vals");

        // Collect unique values in array
        let union_df = self.union_all_cols(Some(col_name))?;
        let uniques_df = union_df.clone().distinct()?;

        if let Some(sort_ascending) = sort_ascending {
            Ok(uniques_df.sort(vec![SortExpr {
                expr: ident(col_name),
                asc: sort_ascending,
                nulls_first: false,
            }])?)
        } else {
            Ok(uniques_df)
        }
    }

    // fn scalar_aggregate(&self, expr: Expr) -> Result<Expr, AvengerChartError> {
    //     let df = self
    //         .clone()
    //         .aggregate(vec![], vec![expr.alias("val")])?
    //         .select(vec![ident("val")])?;
    //     let subquery = Subquery {
    //         subquery: Arc::new(df.logical_plan().clone()),
    //         outer_ref_columns: vec![],
    //     };
    //     Ok(Expr::ScalarSubquery(subquery))
    // }
}

pub trait ScalarValueUtils {
    fn as_f32(&self) -> Result<f32, AvengerChartError>;
    fn as_f32_2(&self) -> Result<[f32; 2], AvengerChartError>;
}

impl ScalarValueUtils for ScalarValue {
    fn as_f32(&self) -> Result<f32, AvengerChartError> {
        match self {
            ScalarValue::Int16(Some(val)) => Ok(*val as f32),
            ScalarValue::Int32(Some(val)) => Ok(*val as f32),
            ScalarValue::Int64(Some(val)) => Ok(*val as f32),
            ScalarValue::Float32(Some(val)) => Ok(*val),
            ScalarValue::Float64(Some(val)) => Ok(*val as f32),
            _ => Err(AvengerChartError::InternalError(format!(
                "ScalarValue is not convertable to f32: {:?}",
                self
            ))),
        }
    }

    fn as_f32_2(&self) -> Result<[f32; 2], AvengerChartError> {
        match self {
            ScalarValue::List(list) if list.data_type().is_numeric() => {
                let element = list.value(0);
                let element = cast(&element, &DataType::Float32)?;
                let array = element.as_primitive::<Float32Type>();
                if array.len() != 2 {
                    return Err(AvengerChartError::InternalError(format!(
                        "ScalarValue is not convertable to f32: {:?}",
                        self
                    )));
                }

                let min = array.value(0);
                let max = array.value(1);
                Ok([min, max])
            }
            _ => Err(AvengerChartError::InternalError(format!(
                "ScalarValue is not convertable to f32: {:?}",
                self
            ))),
        }
    }
}
