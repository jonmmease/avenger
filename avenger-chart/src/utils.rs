use crate::error::AvengerChartError;
use datafusion::functions_aggregate::expr_fn::array_agg;
use datafusion::functions_aggregate::min_max::{max, min};
use datafusion::functions_array::expr_fn::make_array;
use datafusion::logical_expr::Subquery;
use datafusion::common::Spans;
use datafusion::prelude::{col, DataFrame, Expr};
use std::sync::Arc;

pub trait DataFrameChartUtils {
    /// Return two-element array of min and max values across all of the columns in the input DataFrame
    fn span(&self) -> Result<Expr, AvengerChartError>;

    /// Return single-column DataFrame with all columns in the input DataFrame unioned (concatenated) together
    fn union_all_cols(&self, col_name: Option<&str>) -> Result<DataFrame, AvengerChartError>;

    /// Return an array expression with unique values across all of the columns in the input DataFrame
    fn unique_values(&self) -> Result<Expr, AvengerChartError>;

    /// Return an array expression with all values across all of the columns in the input DataFrame
    fn all_values(&self) -> Result<Expr, AvengerChartError>;
}

impl DataFrameChartUtils for DataFrame {
    fn span(&self) -> Result<Expr, AvengerChartError> {
        // Collect single column DataFrames for all numeric columns
        let mut union_dfs: Vec<DataFrame> = Vec::new();
        let col_name = "span_col";
        
        for field in self.schema().fields() {
            if field.data_type().is_numeric() {
                use datafusion::logical_expr::cast;
                use datafusion::arrow::datatypes::DataType;
                
                union_dfs.push(
                    self.clone()
                        .select(vec![
                            cast(col(field.name()), DataType::Float32).alias(col_name)
                        ])?
                );
            }
        }

        if union_dfs.is_empty() {
            return Err(AvengerChartError::InternalError(
                "No numeric columns found for span".to_string(),
            ));
        }

        // Union all the DataFrames
        let union_df = union_dfs.iter().skip(1).fold(union_dfs[0].clone(), |acc, df| {
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
                make_array(vec![col("min_val"), col("max_val")]).alias("span")
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
                    .select(vec![col(field.name()).alias(col_name)])?
            );
        }
        
        if union_dfs.is_empty() {
            return Err(AvengerChartError::InternalError(
                "No columns found for union".to_string(),
            ));
        }
        
        Ok(union_dfs.iter().skip(1).fold(union_dfs[0].clone(), |acc, df| {
            acc.union(df.clone()).unwrap()
        }))
    }

    fn unique_values(&self) -> Result<Expr, AvengerChartError> {
        let col_name = "vals";

        // Collect unique values in array
        let union_df = self.union_all_cols(Some(col_name))?;
        let uniques_df = union_df
            .clone()
            .distinct()?
            .aggregate(vec![], vec![array_agg(col(col_name)).alias("unique_vals")])?
            .select(vec![col("unique_vals")])?;

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
    let df = ctx
        .read_batch(datafusion::arrow::record_batch::RecordBatch::try_new(
            Arc::new(datafusion::arrow::datatypes::Schema::empty()),
            vec![],
        )?)?;
    Ok(df)
}