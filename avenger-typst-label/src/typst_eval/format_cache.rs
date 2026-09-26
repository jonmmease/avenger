use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, NumberFormatError, NumberFormatProvider,
    PreparedDateFormatter, PreparedNaiveDateTimeFormatter, PreparedNumberFormatter,
    PreparedZonedDateTimeFormatter,
};

use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct Entry<P: ?Sized, F: ?Sized> {
    provider: Arc<P>,
    pattern: String,
    formatter: Arc<F>,
}

type Slot<P, F> = Mutex<Option<Entry<P, F>>>;

/// The most recent format of each kind is reused across parameterized labels. Entries hold their
/// provider, so providers match by identity and a dropped provider's address is never reused.
#[derive(Debug, Default)]
pub(crate) struct FormattingCache {
    number: Slot<dyn NumberFormatProvider, dyn PreparedNumberFormatter>,
    date: Slot<dyn DateTimeFormatProvider, dyn PreparedDateFormatter>,
    naive: Slot<dyn DateTimeFormatProvider, dyn PreparedNaiveDateTimeFormatter>,
    zoned: Slot<dyn DateTimeFormatProvider, dyn PreparedZonedDateTimeFormatter>,
}

impl FormattingCache {
    pub(crate) fn number(
        cache: Option<&Self>,
        provider: &Arc<dyn NumberFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        cached(cache.map(|cache| &cache.number), provider, pattern, || {
            provider.prepare(pattern)
        })
    }

    pub(crate) fn date(
        cache: Option<&Self>,
        provider: &Arc<dyn DateTimeFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        cached(cache.map(|cache| &cache.date), provider, pattern, || {
            provider.prepare_date(pattern)
        })
    }

    pub(crate) fn naive(
        cache: Option<&Self>,
        provider: &Arc<dyn DateTimeFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        cached(cache.map(|cache| &cache.naive), provider, pattern, || {
            provider.prepare_naive(pattern)
        })
    }

    pub(crate) fn zoned(
        cache: Option<&Self>,
        provider: &Arc<dyn DateTimeFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        cached(cache.map(|cache| &cache.zoned), provider, pattern, || {
            provider.prepare_zoned(pattern)
        })
    }
}

/// Return the slot's formatter when it was prepared by `provider` for `pattern`, and otherwise
/// prepare one and keep it. Without a cache, always prepare.
fn cached<P: ?Sized, F: ?Sized, E>(
    slot: Option<&Slot<P, F>>,
    provider: &Arc<P>,
    pattern: &str,
    prepare: impl FnOnce() -> Result<Arc<F>, E>,
) -> Result<Arc<F>, E> {
    let Some(slot) = slot else {
        return prepare();
    };
    let mut slot = slot.lock().expect("formatting cache lock");
    if let Some(entry) = slot
        .as_ref()
        .filter(|entry| Arc::ptr_eq(&entry.provider, provider) && entry.pattern == pattern)
    {
        return Ok(entry.formatter.clone());
    }
    let formatter = prepare()?;
    *slot = Some(Entry {
        provider: provider.clone(),
        pattern: pattern.to_owned(),
        formatter: formatter.clone(),
    });
    Ok(formatter)
}
