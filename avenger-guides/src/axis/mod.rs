//! Axes prepared once and drawn at any length, as whole axes or as parts that a layout places.

mod band;
mod continuous;
mod marks;
mod nested_band;
mod opts;
mod prepared;

pub use nested_band::NestedBandAxisLevelConfig;
pub use opts::{AxisConfig, AxisStyle, AxisTickInterval};
pub use prepared::{Axis, AxisPart};

use crate::error::AvengerGuidesError;
use avenger_common::types::TextSyntaxMode;
use avenger_typst_label::{bind, escape_text, LabelValue, LabelValues};

/// Tick label text and how it reads: each tick's `tick_label` markup with `#tick` bound to the
/// tick, or else the ticks themselves, which are text or markup, as plain text unless one of
/// them is markup.
pub(crate) fn tick_label_text(
    ticks: Vec<LabelValue>,
    tick_label: Option<&str>,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    if let Some(template) = tick_label {
        let text = ticks
            .into_iter()
            .map(|tick| bind(template, &LabelValues::from([("tick".into(), tick)])))
            .collect::<Result<_, _>>()?;
        return Ok((text, TextSyntaxMode::TypstMarkup));
    }
    let markup = ticks
        .iter()
        .any(|tick| matches!(tick, LabelValue::Markup(_)));
    let text = ticks
        .into_iter()
        .map(|tick| match tick {
            LabelValue::Markup(markup) => markup,
            LabelValue::Str(text) if markup => escape_text(&text),
            LabelValue::Str(text) => text,
            tick => unreachable!("formatted ticks are text or markup, not {tick:?}"),
        })
        .collect();
    let syntax = if markup {
        TextSyntaxMode::TypstMarkup
    } else {
        TextSyntaxMode::Plain
    };
    Ok((text, syntax))
}
