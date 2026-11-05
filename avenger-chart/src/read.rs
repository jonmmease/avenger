//! Reading a chart: every constructor checks the chart against the rules of the format, then
//! rewrites it into the canonical form.

use crate::{normalize, validate, Chart, ChartError};

/// `chart`, checked and in the canonical form.
pub(crate) fn read(mut chart: Chart) -> Result<Chart, ChartError> {
    validate::check(&chart)?;
    normalize::normalize(&mut chart);
    Ok(chart)
}
