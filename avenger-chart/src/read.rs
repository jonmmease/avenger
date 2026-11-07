//! Reading a chart: every constructor infers the chart's absent scales, checks the chart against
//! the rules of the format, then rewrites it into the canonical form.

use crate::{infer, normalize, validate, Chart, ChartError};

/// `chart`, with its absent scales inferred, checked, and in the canonical form.
pub(crate) fn read(mut chart: Chart) -> Result<Chart, ChartError> {
    infer::infer_scales(&mut chart.message, &chart.tables, &chart.values);
    validate::check(&chart)?;
    // The rules that span messages compare specs and options without the fields that a reader
    // ignores, and with their keyed lists in order.
    normalize::local(&mut chart);
    validate::check_shared(&chart)?;
    normalize::tree(&mut chart);
    Ok(chart)
}
