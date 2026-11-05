//! Axes prepared once and drawn at any length, as whole axes or as parts that a layout places,
//! and polar axes, drawn at any size inside a polar plot.

mod band;
mod continuous;
mod marks;
mod nested_band;
mod opts;
mod polar;
mod prepared;

pub use nested_band::NestedBandAxisLevelConfig;
pub use opts::{AxisConfig, AxisStyle, AxisTickInterval, AxisTicks, LabelThinning};
pub use polar::{
    arc_path, upright, PolarAxis, PolarAxisConfig, PolarAxisKind, PolarFrame, PolarLabel,
    PolarLabelOrient, PolarShape,
};
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
    label_text(ticks, tick_label, "tick")
}

/// Label text and how it reads: each value's `template` markup with the value bound to `name`,
/// as `#tick` binds a tick, or else the values themselves, which are text or markup, as plain
/// text unless one of them is markup.
pub(crate) fn label_text(
    values: Vec<LabelValue>,
    template: Option<&str>,
    name: &str,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    if let Some(template) = template {
        let text = values
            .into_iter()
            .map(|value| bind(template, &LabelValues::from([(name.into(), value)])))
            .collect::<Result<_, _>>()?;
        return Ok((text, TextSyntaxMode::TypstMarkup));
    }
    let markup = values
        .iter()
        .any(|value| matches!(value, LabelValue::Markup(_)));
    let text = values
        .into_iter()
        .map(|value| match value {
            LabelValue::Markup(markup) => markup,
            LabelValue::Str(text) if markup => escape_text(&text),
            LabelValue::Str(text) => text,
            value => unreachable!("formatted values are text or markup, not {value:?}"),
        })
        .collect();
    let syntax = if markup {
        TextSyntaxMode::TypstMarkup
    } else {
        TextSyntaxMode::Plain
    };
    Ok((text, syntax))
}
