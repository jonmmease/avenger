use super::markup::PreparedDatefmt;
use avenger_format::{
    DateTimeFormatBinding, DateTimeFormatError, FormattedNumber, NumberFormatBinding,
    NumberFormatError, PreparedNumberFormatter,
};

use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct NumberEntry {
    pattern: String,
    binding_id: usize,
    formatter: Arc<dyn PreparedNumberFormatter>,
}
#[derive(Debug)]
struct DateTimeEntry {
    pattern: String,
    binding_id: usize,
    formatter: PreparedDatefmt,
}

/// The most recent number and datetime formats are reused across parameterized labels.
#[derive(Debug, Default)]
pub(crate) struct FormattingCache {
    number: Mutex<Option<NumberEntry>>,
    datetime: Mutex<Option<DateTimeEntry>>,
}
impl FormattingCache {
    pub(crate) fn number(
        &self,
        value: f64,
        pattern: &str,
        binding: &NumberFormatBinding,
    ) -> Result<FormattedNumber, NumberFormatError> {
        let mut slot = self.number.lock().expect("formatting cache lock");
        if !slot
            .as_ref()
            .is_some_and(|entry| entry.pattern == pattern && entry.binding_id == binding.cache_id())
        {
            *slot = Some(NumberEntry {
                pattern: pattern.to_owned(),
                binding_id: binding.cache_id(),
                formatter: binding.prepare(pattern)?,
            });
        }
        Ok(slot
            .as_ref()
            .expect("prepared number formatter")
            .formatter
            .format(value))
    }
    pub(super) fn datetime(
        &self,
        value: super::markup::DatefmtValue,
        pattern: &str,
        binding: &DateTimeFormatBinding,
    ) -> Result<String, DateTimeFormatError> {
        let mut slot = self.datetime.lock().expect("formatting cache lock");
        if !slot.as_ref().is_some_and(|entry| {
            matches!(entry.formatter, PreparedDatefmt::Instant(_)) == value.is_instant()
                && entry.pattern == pattern
                && entry.binding_id == binding.cache_id()
        }) {
            *slot = Some(DateTimeEntry {
                pattern: pattern.to_owned(),
                binding_id: binding.cache_id(),
                formatter: value.prepare(binding, pattern)?,
            });
        }
        value.format(
            &slot
                .as_ref()
                .expect("prepared datetime formatter")
                .formatter,
        )
    }
}
