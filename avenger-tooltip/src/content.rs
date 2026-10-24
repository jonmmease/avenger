//! What a tooltip says about its instance: the values of its row, formatted as an axis formats its
//! ticks.

use arrow_array::Array;
use avenger_common::types::TextSyntaxMode;
use avenger_format::PrecisionSharing;
use avenger_geometry::data::Datum;
use avenger_typst_label::{bind, LabelEngine, LabelValue, LabelValues};

use crate::error::AvengerTooltipError;

/// A column of an instance's row that a tooltip shows: its title, and how its value reads, as an
/// axis's ticks read.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipField {
    pub column: String,
    /// The field's title, or `None` for the column's name.
    pub title: Option<String>,
    /// The pattern for a numeric or temporal value, in the syntax of the text engine's number or
    /// datetime provider, such as `".2f"` or `"%b %d"`, or "" for the provider's default, as an
    /// axis's `tick_format` is. Other values, such as strings, show as text.
    pub value_format: String,
    /// Typst markup for the value, where `#value` is the value as `value_format` formats it, such
    /// as `#value km`, as an axis's `tick_label` is. Without it, the value is the formatted value.
    pub value_label: Option<String>,
}

impl TooltipField {
    /// The column, titled by its name, with its value formatted by the provider's default.
    pub fn new(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            title: None,
            value_format: String::new(),
            value_label: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_value_format(mut self, pattern: impl Into<String>) -> Self {
        self.value_format = pattern.into();
        self
    }

    pub fn with_value_label(mut self, markup: impl Into<String>) -> Self {
        self.value_label = Some(markup.into());
        self
    }

    pub(crate) fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.column)
    }
}

impl From<&str> for TooltipField {
    fn from(column: &str) -> Self {
        Self::new(column)
    }
}

impl From<String> for TooltipField {
    fn from(column: String) -> Self {
        Self::new(column)
    }
}

/// A line of a tooltip's text, and how it reads.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Text {
    pub(crate) text: String,
    pub(crate) syntax: TextSyntaxMode,
}

impl Text {
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            syntax: TextSyntaxMode::Plain,
        }
    }

    fn markup(text: String) -> Self {
        Self {
            text,
            syntax: TextSyntaxMode::TypstMarkup,
        }
    }
}

/// The fields that a tooltip of `datum` shows: `fields`, or else every column of its rows.
pub(crate) fn fields_of(fields: Option<&[TooltipField]>, datum: &Datum) -> Vec<TooltipField> {
    match fields {
        Some(fields) => fields.to_vec(),
        None => (datum.rows.schema().fields().iter())
            .map(|field| TooltipField::new(field.name()))
            .collect(),
    }
}

/// The value of `datum`'s row in `field`'s column, formatted as an axis formats its ticks, and
/// empty for a null value.
pub(crate) fn value(
    datum: &Datum,
    field: &TooltipField,
    engine: &LabelEngine,
) -> Result<Text, AvengerTooltipError> {
    let column = (datum.rows.column_by_name(&field.column))
        .ok_or_else(|| AvengerTooltipError::MissingColumn(field.column.clone()))?;
    if column.is_null(datum.row) {
        return Ok(Text::plain(""));
    }
    let value = column.slice(datum.row, 1);
    let formatted = engine.format_array(&value, &field.value_format, PrecisionSharing::PerValue)?;
    let formatted = formatted.into_iter().next().unwrap_or(LabelValue::None);
    let bound = |template: &str, value: LabelValue| {
        bind(template, &LabelValues::from([("value".to_string(), value)]))
    };
    Ok(match (&field.value_label, formatted) {
        (Some(label), value) => Text::markup(bound(label, value)?),
        (None, LabelValue::Str(text)) => Text::plain(text),
        (None, LabelValue::Markup(markup)) => Text::markup(markup),
        (None, value) => Text::markup(bound("#value", value)?),
    })
}
