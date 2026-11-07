//! The check of a chart's format syntaxes against the providers of the label engine.

use avenger_chart::Chart;
use avenger_typst_label::LabelEngine;

use crate::EvalError;

/// Fails when the chart names a syntax of number or datetime patterns that the label engine's
/// provider does not read. A chart without a syntax formats with the provider's default, which
/// needs none.
pub(crate) fn check(chart: &Chart, engine: &LabelEngine) -> Result<(), EvalError> {
    let message = chart.message();
    let number = engine.number_format().map(|provider| provider.syntax());
    let datetime = engine.datetime_format().map(|provider| provider.syntax());
    for (kind, syntax, provided) in [
        ("numbers", &message.number_format_syntax, number),
        (
            "dates and datetimes",
            &message.datetime_format_syntax,
            datetime,
        ),
    ] {
        if !syntax.is_empty() && provided != Some(syntax.as_str()) {
            return Err(EvalError::MissingFormatter {
                kind,
                syntax: syntax.clone(),
                provided: provided.unwrap_or("none").to_string(),
            });
        }
    }
    Ok(())
}
