use avenger_chart_core::{AvengerChartError, CompiledMarkCore, detail_array_column_name};
use datafusion::{
    arrow::{array::ArrayRef, record_batch::RecordBatch},
    common::ScalarValue,
};

pub(crate) struct DetailColumns {
    arrays: Vec<ArrayRef>,
}

impl DetailColumns {
    pub(crate) fn from_mark_data(
        mark: &dyn CompiledMarkCore,
        data: &RecordBatch,
    ) -> Result<Self, AvengerChartError> {
        let Some(details) = mark.state().details.as_deref() else {
            return Ok(Self { arrays: Vec::new() });
        };
        let mut arrays = Vec::with_capacity(details.len());
        for (index, field) in details.iter().enumerate() {
            let alias = detail_array_column_name(index);
            let array = data.column_by_name(&alias).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "{} mark detail field '{field}' was not projected into array data as '{alias}'",
                    mark.mark_type()
                ))
            })?;
            arrays.push(array.clone());
        }
        Ok(Self { arrays })
    }

    pub(crate) fn key_for_row(&self, row: usize) -> Result<Vec<ScalarValue>, AvengerChartError> {
        self.arrays
            .iter()
            .map(|array| {
                ScalarValue::try_from_array(array, row).map_err(AvengerChartError::DataFusionError)
            })
            .collect()
    }
}
