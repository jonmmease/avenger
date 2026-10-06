//! Avenger's formatting functions: `#numfmt` and `#datefmt`.
//!
//! They format a label's parameters with the number and datetime formatters the label's world
//! provides, the same `avenger-format` formatters that axes and legends use.

use std::sync::{Arc, Mutex};

use avenger_format::{
    DateTimeFormatError, DateTimeFormatProvider, NumberFormatError, NumberFormatProvider,
    NumberTypesetting, PreparedDateFormatter, PreparedNaiveDateTimeFormatter,
    PreparedNumberFormatter, PreparedZonedDateTimeFormatter,
};
use ecow::eco_format;

use crate::typst_library::diag::{SourceResult, bail};
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{
    Content, Datetime, NativeElement, Scope, Str, SymbolElem, func, one_line,
};
use crate::typst_library::math::{AttachElem, EquationElem};
use crate::typst_library::text::TextElem;
use crate::typst_syntax::Span;

/// Hook up the formatting functions.
pub(crate) fn define(global: &mut Scope) {
    global.define_func::<numfmt>();
    global.define_func::<datefmt>();
}

func! {
/// Formats a number with the label's number formatter.
///
/// The pattern is in the formatter's syntax, such as `{",.2f"}` for a D3
/// formatter. Without a pattern, the formatter's default applies. Scientific
/// notation is set as a power of ten.
#[func]
pub fn numfmt(
    engine: &mut Engine,
    span: Span,
    /// The number to format.
    value: f64,
    /// The pattern.
    #[default]
    pattern: Str,
) -> SourceResult<Content> {
    if !value.is_finite() {
        bail!(span, "cannot format a number that is not finite");
    }
    let Some(provider) = engine.world.number_format() else {
        bail!(span, "number formatting is not configured");
    };
    let formatted = FormattingCache::number(engine.world.formatting_cache(), provider, &pattern)
        .map(|formatter| formatter.format(value));
    let formatted = match formatted {
        Ok(formatted) => formatted,
        Err(err) => bail!(span, "{err}"),
    };
    Ok(match formatted.typesetting {
        NumberTypesetting::Plain => TextElem::packed(one_line(&formatted.text)).spanned(span),
        NumberTypesetting::Exponent { mantissa, exponent, .. } => {
            scientific_notation(&mantissa, exponent, span)
        }
    })
}
}

func! {
/// Formats a date or datetime with the label's datetime formatter.
///
/// The pattern is in the formatter's syntax, such as `{"%b %d"}` for a D3
/// formatter. A date formats its calendar fields, a datetime without a
/// timezone its calendar and clock fields, and a UTC datetime is shown in the
/// formatter's timezone.
#[func]
pub fn datefmt(
    engine: &mut Engine,
    span: Span,
    /// The date or datetime to format.
    value: Datetime,
    /// The pattern.
    pattern: Str,
) -> SourceResult<Content> {
    let Some(provider) = engine.world.datetime_format() else {
        bail!(span, "datetime formatting is not configured");
    };
    let cache = engine.world.formatting_cache();
    let formatted = match value {
        Datetime::Date(date) => {
            FormattingCache::date(cache, provider, &pattern).and_then(|f| f.format(date))
        }
        Datetime::Naive(datetime) => FormattingCache::naive(cache, provider, &pattern)
            .and_then(|f| f.format(datetime)),
        Datetime::Utc(instant) => {
            FormattingCache::zoned(cache, provider, &pattern).and_then(|f| f.format(instant))
        }
    };
    match formatted {
        Ok(text) => Ok(TextElem::packed(one_line(&text)).spanned(span)),
        Err(err) => bail!(span, "{err}"),
    }
}
}

/// A number in scientific notation, as `$#mantissa times 10^(#exponent)$` sets it, built
/// directly so that localized mantissas ("1,2", Arabic-Indic digits) never become math
/// punctuation. A minus sign is a symbol, so it stays unary at the start of the row.
fn scientific_notation(mantissa: &str, exponent: i32, span: Span) -> Content {
    let text = |text: &str| TextElem::packed(text).spanned(span);
    let minus = || SymbolElem::packed('−').spanned(span);
    let mantissa = match mantissa.strip_prefix(['-', '−']) {
        Some(rest) => minus() + text(rest),
        None => text(mantissa),
    };
    let exponent = if exponent < 0 {
        minus() + text(&eco_format!("{}", exponent.unsigned_abs()))
    } else {
        text(&eco_format!("{exponent}"))
    };
    let power = AttachElem::new(text("10"))
        .with_t(Some(exponent))
        .pack()
        .spanned(span);
    let body = mantissa + SymbolElem::packed('×').spanned(span) + power;
    EquationElem::new(body).pack().spanned(span)
}

#[derive(Debug)]
struct Entry<P: ?Sized, F: ?Sized> {
    provider: Arc<P>,
    pattern: String,
    formatter: Arc<F>,
}

type Slot<P, F> = Mutex<Option<Entry<P, F>>>;

/// The most recent format of each kind, reused across labels. Entries hold their provider, so
/// providers match by identity and a dropped provider's address is never reused.
#[derive(Debug, Default)]
pub struct FormattingCache {
    number: Slot<dyn NumberFormatProvider, dyn PreparedNumberFormatter>,
    date: Slot<dyn DateTimeFormatProvider, dyn PreparedDateFormatter>,
    naive: Slot<dyn DateTimeFormatProvider, dyn PreparedNaiveDateTimeFormatter>,
    zoned: Slot<dyn DateTimeFormatProvider, dyn PreparedZonedDateTimeFormatter>,
}

impl FormattingCache {
    fn number(
        cache: Option<&Self>,
        provider: &Arc<dyn NumberFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        cached(cache.map(|cache| &cache.number), provider, pattern, || {
            provider.prepare(pattern)
        })
    }

    fn date(
        cache: Option<&Self>,
        provider: &Arc<dyn DateTimeFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        cached(cache.map(|cache| &cache.date), provider, pattern, || {
            provider.prepare_date(pattern)
        })
    }

    fn naive(
        cache: Option<&Self>,
        provider: &Arc<dyn DateTimeFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        cached(cache.map(|cache| &cache.naive), provider, pattern, || {
            provider.prepare_naive(pattern)
        })
    }

    fn zoned(
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
    if let Some(entry) = slot.as_ref().filter(|entry| {
        Arc::ptr_eq(&entry.provider, provider) && entry.pattern == pattern
    }) {
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
