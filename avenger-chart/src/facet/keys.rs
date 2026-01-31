use datafusion::{
    arrow::record_batch::RecordBatch, common::ScalarValue, dataframe::DataFrame, logical_expr::Expr,
};

use crate::{error::AvengerChartError, facet::scalar_cmp::scalar_total_cmp};

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
    /// Values are sorted to ensure deterministic facet ordering.
    pub async fn extract_keys(
        df: &DataFrame,
        expr: &Expr,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let distinct_df = df.clone().select(vec![expr.clone()])?.distinct()?;

        let batches = distinct_df.collect().await?;
        let mut values = Self::scalar_column_to_vec(&batches, 0)?;

        // Sort values to ensure deterministic facet ordering
        values.sort_by(scalar_total_cmp);

        Ok(values)
    }

    fn scalar_column_to_vec(
        batches: &[RecordBatch],
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
}
