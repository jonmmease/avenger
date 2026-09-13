# Number formatting

`avenger-format-number` implements valid D3 number formats with D3 locale definitions. Its `S`, `L`, and `C` extensions provide compact short, compact long, and currency metadata formatting. The crate has no chart or Typst dependency.

```rust
use avenger_format_number::{NumberFormatContext, NumberLocaleRegistry, PreparedNumberFormat};

fn main() -> Result<(), avenger_format_number::FormatError> {
    let locale = NumberLocaleRegistry::with_builtins().resolve("de-DE")?;
    let formatter = PreparedNumberFormat::new(
        Some("$,.2f"),
        Default::default(),
        NumberFormatContext::new(&locale),
    )?;
    assert_eq!(formatter.format(1234.5).text, "1.234,50 €");
    Ok(())
}
```

Prepare a formatter once and reuse it for binary64 values. Decimal conversion uses exact integer arithmetic for ECMAScript rounding and `ryu-js` for shortest strings. Keep original `f64` values until formatting. Casting a value through `f32` can change its label.

Locale JSON uses `decimal`, `thousands`, a cyclic `grouping` array, and a `currency` prefix/suffix pair, with optional `numerals`, `percent`, `minus`, and `nan`. Register upstream definitions directly through `NumberLocaleRegistry`. The bundled definitions cover `en-US`, `de-DE`, `fr-FR`, and `ja-JP`. The default minus is Unicode `−`. D3's `$` symbol uses the locale affixes and does not select an ISO currency or its precision.

`NumberLocaleExtensions` stores normalized compact tiers and currency patterns separately. `register_extensions` replaces that metadata for a named locale without changing standard D3 output. `C[JPY]` uses the currency's default fraction digits unless the format overrides them. Compact patterns contain one `{0}` placeholder. Values below the first tier retain their magnitude. Runtime LDML number-pattern ingestion is removed.

Resolved locales expose borrowed definitions and extension metadata. To customize a locale, clone `definition()` or `extensions()`, edit the clone, and register it. Existing prepared formatters retain their resolved locale.

`prepare_number_span_format` implements Vega's domain/count-based precision selection. `prepare_number_float_format` implements Vega's automatic floating-point labels. `prepare_number_prefix_format` fixes a D3 SI unit using a reference value. Existing native tick sets can use `prepare_number_tick_format`. These adapters preserve upstream behavior independently of Avenger's compact and currency metadata.

`FormattedNumber` contains plain text and optional exponent parts for Typst. Those parts use the same locale, signs, precision, and trimming as the text. Formats with affixes, padding, or custom numerals retain their plain representation.

The supported grammar is the documented [D3 number format](https://d3js.org/d3-format). Unknown-type fallback and JavaScript object coercion are outside the contract. The numeric `c` format uses JavaScript number-to-string semantics. A frontend can pass string values through for `c` before calling numeric formatting.

The [reference generator](../tools/format-reference/README.md) pins upstream packages and records exact input bits for number fixtures. Rust tests need neither Node nor network access.
