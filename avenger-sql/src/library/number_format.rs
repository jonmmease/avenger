//! GoogleSQL's number format models of `CAST(number AS STRING FORMAT ...)`.

use datafusion::arrow::array::ArrayRef;

/// A number format model.
pub(crate) struct NumberFormat;

impl NumberFormat {
    pub(crate) fn parse(_format: &str) -> Result<NumberFormat, String> {
        Err("avenger-sql doesn't format numbers yet".to_string())
    }
}

/// The text of the number at `row` of `array` in `format`.
pub(crate) fn format_row(
    _format: &NumberFormat,
    _array: &ArrayRef,
    _row: usize,
) -> Result<String, String> {
    Err("avenger-sql doesn't format numbers yet".to_string())
}
