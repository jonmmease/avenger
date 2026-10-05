pub mod band;
pub mod numeric;
pub mod opts;
pub mod point;

use arrow::{
    array::{ArrayRef, AsArray},
    compute::cast,
    datatypes::{DataType, Float64Type},
};
use avenger_common::value::ScalarOrArray;
use avenger_format::{PreparedNumberFormatter, TickSpacing};
use avenger_scales::error::AvengerScaleError;

/// Label numeric values with the axis formatter.
fn number_labels(
    values: &ArrayRef,
    format: &dyn PreparedNumberFormatter,
    spacing: TickSpacing,
) -> Result<ScalarOrArray<String>, AvengerScaleError> {
    let values = cast(values, &DataType::Float64)?;
    let values: Vec<f64> = values
        .as_primitive::<Float64Type>()
        .iter()
        .map(|value| value.unwrap_or(f64::NAN))
        .collect();
    let labels = format.format_ticks(&values, spacing);
    Ok(ScalarOrArray::new_array(
        labels.into_iter().map(|label| label.text).collect(),
    ))
}
