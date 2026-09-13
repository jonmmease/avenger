use super::markup::PreparedDatefmt;
use avenger_format::{
    DateTimeFormatConfig, DateTimeFormatError, DateTimeFormatRegistry, DateTimeFormatRequest,
    FormattedNumber, NumberFormatConfig, NumberFormatError, NumberFormatRegistry,
    NumberFormatRequest, PreparedNumberFormatter,
};

use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct NumberEntry {
    request: NumberFormatRequest,
    config: NumberFormatConfig,
    registry_id: usize,
    formatter: Arc<dyn PreparedNumberFormatter>,
}
#[derive(Debug)]
struct DateTimeEntry {
    request: DateTimeFormatRequest,
    config: DateTimeFormatConfig,
    registry_id: usize,
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
        request: &NumberFormatRequest,
        config: &NumberFormatConfig,
        registry: &NumberFormatRegistry,
    ) -> Result<FormattedNumber, NumberFormatError> {
        let mut slot = self.number.lock().expect("formatting cache lock");
        if !slot.as_ref().is_some_and(|entry| {
            entry.request == *request
                && entry.config == *config
                && entry.registry_id == registry.cache_id()
        }) {
            *slot = Some(NumberEntry {
                request: request.clone(),
                config: config.clone(),
                registry_id: registry.cache_id(),
                formatter: registry.prepare(config, request)?,
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
        request: &DateTimeFormatRequest,
        config: &DateTimeFormatConfig,
        registry: &DateTimeFormatRegistry,
    ) -> Result<String, DateTimeFormatError> {
        let mut slot = self.datetime.lock().expect("formatting cache lock");
        if !slot.as_ref().is_some_and(|entry| {
            matches!(entry.formatter, PreparedDatefmt::Instant(_)) == value.is_instant()
                && entry.request == *request
                && entry.config == *config
                && entry.registry_id == registry.cache_id()
        }) {
            *slot = Some(DateTimeEntry {
                request: request.clone(),
                config: config.clone(),
                registry_id: registry.cache_id(),
                formatter: value.prepare(config, request, registry)?,
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
