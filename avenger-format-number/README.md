# Number formatting

`avenger-format-number` implements valid D3 number formats with D3 locale definitions.

```rust
use avenger_format_number::{NumberLocaleRegistry, PreparedNumberFormat};

fn main() -> Result<(), avenger_format_number::FormatError> {
    let locale = NumberLocaleRegistry::with_builtins().resolve("de-DE")?;
    let formatter = PreparedNumberFormat::new(
        Some("$,.2f"),
        Default::default(),
        &locale,
    )?;
    assert_eq!(formatter.format(1234.5).text, "1.234,50 €");
    Ok(())
}
```

Prepare a formatter once and reuse it for binary64 values. Decimal conversion uses exact integer arithmetic for ECMAScript rounding and `ryu-js` for shortest strings. Keep original `f64` values until formatting. Casting a value through `f32` can change its label.

Locale JSON uses `decimal`, `thousands`, a cyclic `grouping` array, and a `currency` prefix/suffix pair, with optional `numerals`, `percent`, `minus`, and `nan`. Register upstream definitions directly through `NumberLocaleRegistry`. The bundled definitions cover `en-US`, `de-DE`, `fr-FR`, and `ja-JP`. The default minus is Unicode `−`. D3's `$` symbol uses the locale affixes and does not select an ISO currency or its precision.

`NumberFormatOverrides` replaces fields from a format string before defaults and zero padding are applied. For nullable fields, `Some(None)` clears the parsed value. D3 precision means fraction digits for `f`, `e`, and `%`, and significant digits for `g`, `r`, `s`, and `p`.

Resolved locales expose borrowed definitions. To customize a locale, clone `definition()`, edit the clone, and register it. Existing prepared formatters retain their resolved locale.

`prepare_number_span_format` selects precision from a domain and requested tick count using Vega's rules. `prepare_number_prefix_format` fixes a D3 SI unit using a reference value. A zero or non-finite reference selects no SI prefix.

`prepare_number_float_format` uses Vega's automatic precision rules. It intentionally trims the numeric significand before localization and padding. This preserves custom numerals, locale affixes, and field widths instead of reproducing Vega 2.1.3's trimming of the completed label, which can return blank labels for custom numerals. Explicit precision disables automatic trimming.

`FormattedNumber` contains plain text and optional mantissa and exponent parts for scientific notation. Those parts use the same locale, signs, precision, and trimming as the text. Formats with affixes, padding, or custom numerals retain their plain representation.

The supported grammar is the documented [D3 number format](https://d3js.org/d3-format). Unknown-type fallback and JavaScript object coercion are outside the contract. The numeric `c` format uses JavaScript number-to-string semantics.

The [reference generator](../tools/format-reference/README.md) pins upstream packages and records exact input bits for number fixtures. Rust tests need neither Node nor network access.
