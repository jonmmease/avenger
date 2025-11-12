use crate::error::AvengerChartError;
use datafusion::common::ScalarValue;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::Expr;

/// Helper utilities for extracting distinct facet keys from DataFusion `DataFrame`s.
///
/// Facet transforms only require the unique values that appear in a facet channel.
/// These helpers perform the minimal queries needed to retrieve those values while leaving
/// the original `DataFrame` untouched for subplot rendering.
pub struct FacetKeyExtractor;

impl FacetKeyExtractor {
    /// Extract all distinct values for a single facet dimension.
    ///
    /// The supplied expression should resolve to the column used by the facet channel.
    pub async fn extract_keys(
        df: &DataFrame,
        expr: &Expr,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let distinct_df = df.clone().select(vec![expr.clone()])?.distinct()?;

        let batches = distinct_df.collect().await?;
        Ok(Self::scalar_column_to_vec(&batches, 0)?)
    }

    /// Extract all distinct combinations for a two-dimensional (row/column) facet.
    ///
    /// The returned vector preserves the order emitted by DataFusion.
    pub async fn extract_key_pairs(
        df: &DataFrame,
        row_expr: &Expr,
        col_expr: &Expr,
    ) -> Result<Vec<(ScalarValue, ScalarValue)>, AvengerChartError> {
        let distinct_df = df
            .clone()
            .select(vec![row_expr.clone(), col_expr.clone()])?
            .distinct()?;

        let batches = distinct_df.collect().await?;
        Self::pair_columns_to_vec(&batches, 0, 1)
    }

    fn scalar_column_to_vec(
        batches: &[datafusion::arrow::record_batch::RecordBatch],
        column_index: usize,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let mut values = Vec::new();
        for batch in batches {
            let column = batch.column(column_index);
            for row in 0..batch.num_rows() {
                values.push(ScalarValue::try_from_array(column, row)?);
            }
        }
        Ok(values)
    }

    fn pair_columns_to_vec(
        batches: &[datafusion::arrow::record_batch::RecordBatch],
        first_index: usize,
        second_index: usize,
    ) -> Result<Vec<(ScalarValue, ScalarValue)>, AvengerChartError> {
        let mut values = Vec::new();
        for batch in batches {
            let first_col = batch.column(first_index);
            let second_col = batch.column(second_index);
            for row in 0..batch.num_rows() {
                let first = ScalarValue::try_from_array(first_col, row)?;
                let second = ScalarValue::try_from_array(second_col, row)?;
                values.push((first, second));
            }
        }
        Ok(values)
    }
}
