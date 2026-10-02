# Number formatting

`avenger-format-number-d3` implements valid D3 number formats with D3 locale definitions.

```rust
use avenger_format::NumberFormatProvider;
use avenger_format_number_d3::{D3NumberFormatConfig, D3NumberFormatProvider};

fn main() -> Result<(), avenger_format::NumberFormatError> {
    let config = D3NumberFormatConfig::new().with_locale("en-US");
    let formatter = D3NumberFormatProvider.prepare(&config, "$,.2f")?;
    assert_eq!(formatter.format(1234.5).text, "$1,234.50");
    Ok(())
}
```

Prepare a formatter once and reuse it for binary64 values. Decimal conversion uses exact integer arithmetic for ECMAScript rounding and `ryu-js` for shortest strings. Keep original `f64` values until formatting. Casting a value through `f32` can change its label.

Locale JSON uses `decimal`, `thousands`, a cyclic `grouping` array, and a `currency` prefix/suffix pair, with optional `numerals`, `percent`, `minus`, and `nan`. The crate bundles `en-US`. Register other D3 locale definitions through `NumberLocaleRegistry::register_custom_locale_json`. The default minus is Unicode `−`. D3's `$` symbol uses the locale affixes and does not select an ISO currency or its precision.

D3 precision means fraction digits for `f`, `e`, and `%`, and significant digits for `g`, `r`, `s`, and `p`.

Resolved locales expose borrowed definitions. To customize a locale, clone `definition()`, edit the clone, and register it. Existing prepared formatters retain their resolved locale.

`prepare_number_step_format` selects precision from a supplied step's decimal order and a reference magnitude using D3's rules. The caller chooses the step. The reference is typically the largest magnitude to format. Automatic `s` formatting uses one SI unit for all labels.

`prepare_number_prefix_format` fixes a D3 SI unit using a reference value. A zero or non-finite reference selects no SI prefix.

`prepare_number_float_format` uses Vega's automatic precision rules. It intentionally trims the numeric significand before localization and padding. This preserves custom numerals, locale affixes, and field widths instead of reproducing Vega 2.1.3's trimming of the completed label, which can return blank labels for custom numerals. Explicit precision disables automatic trimming.

`FormattedNumber` contains plain text and optional mantissa and exponent parts for scientific notation. Those parts use the same locale, signs, precision, and trimming as the text. Formats with affixes, padding, or custom numerals retain their plain representation.

The supported grammar is the documented [D3 number format](https://d3js.org/d3-format). Unknown-type fallback and JavaScript object coercion are outside the contract. The numeric `c` format uses JavaScript number-to-string semantics.

The [reference generator](../tools/format-reference/README.md) pins upstream packages and records exact input bits for number fixtures. Rust tests need neither Node nor network access.

`D3NumberFormatProvider` implements the provider interface in `avenger-format` with `D3NumberFormatConfig`. Pass an explicit pattern string to `prepare()`. Set grouping, signs, symbols, padding, and explicit precision in the pattern.

Use `with_locale()` and `with_custom_locale()` to configure the selected locale and typed `NumberLocaleSpec` definitions. The provider treats hyphens and underscores as equivalent in locale names, including custom definitions. An exact custom name takes precedence when both forms are registered.

`with_precision()` selects a `D3NumberPrecision` policy. `FromSpecifier` uses ordinary D3 precision. `Automatic` chooses Vega's precision and trimming when the pattern omits precision. `Step { step, reference_value }` infers precision from numeric spacing and coordinates SI units. Both numeric inputs must be finite. Explicit precision in the pattern is preserved in every mode. The caller chooses the pattern and tick spacing.
