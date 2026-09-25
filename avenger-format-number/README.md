# Number formatting

`avenger-format-number` implements valid D3 number formats with D3 locale definitions.

```rust
use avenger_format_number::{NumberLocaleRegistry, PreparedNumberFormat};

fn main() -> Result<(), avenger_format_number::FormatError> {
    let locale = NumberLocaleRegistry::with_builtins().resolve("en-US")?;
    let formatter = PreparedNumberFormat::new(
        Some("$,.2f"),
        Default::default(),
        &locale,
    )?;
    assert_eq!(formatter.format(1234.5).text, "$1,234.50");
    Ok(())
}
```

Prepare a formatter once and reuse it for binary64 values. Decimal conversion uses exact integer arithmetic for ECMAScript rounding and `ryu-js` for shortest strings. Keep original `f64` values until formatting. Casting a value through `f32` can change its label.

Locale JSON uses `decimal`, `thousands`, a cyclic `grouping` array, and a `currency` prefix/suffix pair, with optional `numerals`, `percent`, `minus`, and `nan`. The crate bundles `en-US`. Register other D3 locale definitions through `NumberLocaleRegistry::register_custom_locale_json`. The default minus is Unicode `−`. D3's `$` symbol uses the locale affixes and does not select an ISO currency or its precision.

`NumberFormatOverrides` replaces fields from a format string before defaults and zero padding are applied. For nullable fields, `Some(None)` clears the parsed value. D3 precision means fraction digits for `f`, `e`, and `%`, and significant digits for `g`, `r`, `s`, and `p`.

Resolved locales expose borrowed definitions. To customize a locale, clone `definition()`, edit the clone, and register it. Existing prepared formatters retain their resolved locale.

`prepare_number_step_format` selects precision from a supplied step's decimal order and a reference magnitude using D3's rules. The caller chooses the step. The reference is typically the largest magnitude to format. Automatic `s` formatting uses one SI unit for all labels.

`prepare_number_prefix_format` fixes a D3 SI unit using a reference value. A zero or non-finite reference selects no SI prefix.

`prepare_number_float_format` uses Vega's automatic precision rules. It intentionally trims the numeric significand before localization and padding. This preserves custom numerals, locale affixes, and field widths instead of reproducing Vega 2.1.3's trimming of the completed label, which can return blank labels for custom numerals. Explicit precision disables automatic trimming.

`S` and `L` select compact short and long patterns. Their precision counts significant digits, as in `.3~S` (`999500` becomes `1M`). The step formatter fixes a tier from the reference magnitude. It preserves explicit significant precision and otherwise chooses fraction digits from the step. Each label selects its own plural pattern.

`NumberLocaleExtensions` registers compact tiers independently of D3 definitions. Only `en-US` includes built-in patterns. Custom locales start with empty metadata and format unscaled numbers until extensions are registered. Tier exponents must be unique and in `1..=308`. Patterns contain one `{0}` placeholder, except an exact-one pattern can omit it. The plural rule is explicit: always `other`, integer one without fraction digits, or an integer part of zero or one. These rules cover the supplied English, German, French, and Japanese examples, not every CLDR plural category.

`C[CODE]` formats a currency using CLDR 48 default fraction digits, including historical and common-use codes. For example, `C[JPY]` rounds to whole yen, while `.2C[JPY]` explicitly requests two fraction digits. These are display defaults, not cash-rounding rules. `CurrencyDisplay` selects a symbol, narrow symbol, or code. Unknown symbols fall back to the code, with locale spacing between alphabetic affixes and digits. Currency names are unsupported.

Currency patterns in `NumberLocaleExtensions` use `¤` for the symbol or code and `-` for the locale minus. Each positive and negative prefix/suffix pair requires one `¤`. A `(` sign policy selects the accounting pattern. Currency metadata does not change D3's `$` format. Invalid codes, missing codes, and conflicting symbol options fail when preparing the formatter.

`FormattedNumber` contains plain text and optional mantissa and exponent parts for scientific notation. Those parts use the same locale, signs, precision, and trimming as the text. Formats with affixes, padding, or custom numerals retain their plain representation.

The supported grammar is the documented [D3 number format](https://d3js.org/d3-format). Unknown-type fallback and JavaScript object coercion are outside the contract. The numeric `c` format uses JavaScript number-to-string semantics.

The [reference generator](../tools/format-reference/README.md) pins upstream packages and records exact input bits for number fixtures. Rust tests need neither Node nor network access.
