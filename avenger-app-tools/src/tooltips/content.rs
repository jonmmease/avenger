//! What a tooltip says about its instance: the values of its row, formatted as an axis formats its
//! ticks.

use std::{collections::HashMap, sync::Arc};

use arrow_array::{Array, ArrayRef};
use avenger_common::types::TextSyntaxMode;
use avenger_format::PrecisionSharing;
use avenger_geometry::data::Datum;
use avenger_typst_label::{bind, LabelEngine, LabelError, LabelValue, LabelValues};

use super::error::AvengerTooltipError;

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
    pub label_format: String,
    /// Typst markup for the value, where `#value` is the value as `label_format` formats it, such
    /// as `#value km`, as an axis's `tick_label` is. Without it, the value is the formatted value.
    pub label: Option<String>,
}

impl TooltipField {
    /// The column, titled by its name, with its value formatted by the provider's default.
    pub fn new(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            title: None,
            label_format: String::new(),
            label: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_label_format(mut self, pattern: impl Into<String>) -> Self {
        self.label_format = pattern.into();
        self
    }

    pub fn with_label(mut self, markup: impl Into<String>) -> Self {
        self.label = Some(markup.into());
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

/// The function of a column, a row, the field's `label_format`, and the text engine that gives
/// the values that the field's label binds for that row, such as the parts of a struct, or `None`
/// for a column whose value binds as `value`.
type ValuesFn = dyn Fn(&ArrayRef, usize, &str, &LabelEngine) -> Result<Option<LabelValues>, LabelError>
    + Send
    + Sync;

/// The values that the labels of a target's fields bind for a row, in place of the row's value
/// as `value`: a field without markup shows the `value` that they hold.
#[derive(Clone)]
pub struct FieldValues(Arc<ValuesFn>);

impl FieldValues {
    pub fn new(
        values: impl Fn(&ArrayRef, usize, &str, &LabelEngine) -> Result<Option<LabelValues>, LabelError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self(Arc::new(values))
    }
}

impl std::fmt::Debug for FieldValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FieldValues")
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

/// The fields that a tooltip of `datum`, a row of the mark `mark`, shows: `fields`, or else every
/// column of its rows, titled by `titles` or by its name.
pub(crate) fn fields_of(
    fields: Option<&[TooltipField]>,
    titles: &HashMap<(String, String), String>,
    mark: &str,
    datum: &Datum,
) -> Vec<TooltipField> {
    match fields {
        Some(fields) => fields.to_vec(),
        None => (datum.rows.schema().fields().iter())
            .map(|field| {
                let column = TooltipField::new(field.name());
                match titles.get(&(mark.to_string(), field.name().clone())) {
                    Some(title) => column.with_title(title.as_str()),
                    None => column,
                }
            })
            .collect(),
    }
}

/// The value of `datum`'s row in `field`'s column, formatted as an axis formats its ticks, or
/// the values that `values` gives for it, and empty for a null value.
pub(crate) fn value(
    datum: &Datum,
    field: &TooltipField,
    values: Option<&FieldValues>,
    engine: &LabelEngine,
) -> Result<Text, AvengerTooltipError> {
    let column = (datum.rows.column_by_name(&field.column))
        .ok_or_else(|| AvengerTooltipError::MissingColumn(field.column.clone()))?;
    if column.is_null(datum.row) {
        return Ok(Text::plain(""));
    }
    let given = match values {
        Some(values) => (values.0)(column, datum.row, &field.label_format, engine)?,
        None => None,
    };
    if let Some(mut given) = given {
        return Ok(match &field.label {
            Some(label) => Text::markup(bind(label, &given)?),
            None => match given.shift_remove("value") {
                Some(LabelValue::Str(text)) => Text::plain(text),
                Some(LabelValue::Markup(markup)) => Text::markup(markup),
                Some(LabelValue::None) | None => Text::plain(""),
                Some(value) => Text::markup(bind(
                    "#value",
                    &LabelValues::from([("value".to_string(), value)]),
                )?),
            },
        });
    }
    let value = column.slice(datum.row, 1);
    let formatted = engine.format_array(&value, &field.label_format, PrecisionSharing::PerValue)?;
    let formatted = formatted.into_iter().next().unwrap_or(LabelValue::None);
    let bound = |template: &str, value: LabelValue| {
        bind(template, &LabelValues::from([("value".to_string(), value)]))
    };
    Ok(match (&field.label, formatted) {
        (Some(label), value) => Text::markup(bound(label, value)?),
        (None, LabelValue::Str(text)) => Text::plain(text),
        (None, LabelValue::Markup(markup)) => Text::markup(markup),
        (None, value) => Text::markup(bound("#value", value)?),
    })
}
