//! Avenger's formatting functions: `#numfmt` and `#datetimefmt`.
//!
//! They format numbers and dates with the number and datetime formatters the label's world
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
use typst_syntax::Span;

/// Hook up the formatting functions, and `datetime`, which builds what `#datetimefmt`
/// formats.
pub(crate) fn define(global: &mut Scope) {
    global.define_func::<numfmt>();
    global.define_func::<datetimefmt>();
    global.define_func::<super::datetime::datetime>();
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
/// formatter. A date formats its calendar fields, a naive datetime its
/// calendar and clock fields, and a zoned datetime, an instant, is shown in the
/// formatter's timezone.
#[func]
pub fn datetimefmt(
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
        Datetime::Zoned(instant) => {
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
struct Entry<F: ?Sized> {
    pattern: String,
    formatter: Arc<F>,
}

type Slot<F> = Mutex<Option<Entry<F>>>;

/// The most recent format of each kind, reused across labels. A cache belongs to one engine's
/// providers: an engine with another provider starts a new one.
#[derive(Debug, Default)]
pub struct FormattingCache {
    number: Slot<dyn PreparedNumberFormatter>,
    date: Slot<dyn PreparedDateFormatter>,
    naive: Slot<dyn PreparedNaiveDateTimeFormatter>,
    zoned: Slot<dyn PreparedZonedDateTimeFormatter>,
}

impl FormattingCache {
    fn number(
        cache: Option<&Self>,
        provider: &Arc<dyn NumberFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        cached(cache.map(|cache| &cache.number), pattern, || provider.prepare(pattern))
    }

    fn date(
        cache: Option<&Self>,
        provider: &Arc<dyn DateTimeFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
        cached(cache.map(|cache| &cache.date), pattern, || provider.prepare_date(pattern))
    }

    fn naive(
        cache: Option<&Self>,
        provider: &Arc<dyn DateTimeFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError> {
        cached(cache.map(|cache| &cache.naive), pattern, || {
            provider.prepare_naive(pattern)
        })
    }

    fn zoned(
        cache: Option<&Self>,
        provider: &Arc<dyn DateTimeFormatProvider>,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError> {
        cached(cache.map(|cache| &cache.zoned), pattern, || {
            provider.prepare_zoned(pattern)
        })
    }
}

/// Return the slot's formatter when it was prepared for `pattern`, and otherwise prepare one
/// and keep it. Without a cache, always prepare.
fn cached<F: ?Sized, E>(
    slot: Option<&Slot<F>>,
    pattern: &str,
    prepare: impl FnOnce() -> Result<Arc<F>, E>,
) -> Result<Arc<F>, E> {
    let Some(slot) = slot else {
        return prepare();
    };
    let mut slot = slot.lock().expect("formatting cache lock");
    if let Some(entry) = slot.as_ref().filter(|entry| entry.pattern == pattern) {
        return Ok(entry.formatter.clone());
    }
    let formatter = prepare()?;
    *slot = Some(Entry {
        pattern: pattern.to_owned(),
        formatter: formatter.clone(),
    });
    Ok(formatter)
}
