# D3 number formatting

`avenger-format-number-d3` formats `f64` values with [D3 number patterns](https://d3js.org/d3-format) and locale definitions. It implements the [number formatter interface](../avenger-format/README.md) from `avenger-format`.

```rust
use avenger_format::NumberFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;

fn main() -> Result<(), avenger_format::NumberFormatError> {
    let provider = D3NumberFormatProvider::new().with_locale("en-US");
    let formatter = provider.prepare("$,.2f")?;
    assert_eq!(formatter.format(1234.5).text, "$1,234.50");
    Ok(())
}
```

Prepare a formatter once and reuse it for multiple values. Set grouping, signs, currency symbols, padding, and precision in the pattern. D3's `$` symbol uses the selected locale's currency prefix and suffix.

The default locale is U.S. English (`en-US`). Enable `all-locales` to include all bundled D3 locales:

```toml
avenger-format-number-d3 = { version = "0.1", features = ["all-locales"] }
```

Select a locale with `.with_locale("de-DE")`. Names accept hyphens or underscores. Add custom `NumberLocaleSpec` definitions with `with_custom_locale()`. Custom definitions take precedence over bundled definitions and work without `all-locales`.

`with_precision()` selects how `format()` labels a single value when the pattern omits precision:

- `FromSpecifier` uses D3's default precision.
- `Automatic` uses Vega's automatic precision and trimming.

`format_ticks()` labels tick values together, as Vega labels axes:

- `TickSpacing::Uniform` follows Vega's `formatSpan`. Precision follows the tick step, and `s` patterns share one SI unit chosen from the largest magnitude. Vega reads the step and magnitude from the scale domain; this crate infers them from the tick values.
- `TickSpacing::Varying` follows Vega's `formatFloat`, which Vega uses for log axes.

```rust
use avenger_format::{NumberFormatProvider, TickSpacing};
use avenger_format_number_d3::D3NumberFormatProvider;

fn main() -> Result<(), avenger_format::NumberFormatError> {
    let formatter = D3NumberFormatProvider::new().prepare("s")?;
    let labels: Vec<String> = formatter
        .format_ticks(&[0.0, 5e5, 1e6], TickSpacing::Uniform)
        .into_iter()
        .map(|label| label.text)
        .collect();
    assert_eq!(labels, ["0.0M", "0.5M", "1.0M"]);
    Ok(())
}
```

Explicit precision in the pattern takes precedence in every mode.

Automatic trimming removes trailing zeros before localization and padding. This intentionally differs from Vega 2.1.3 so that custom numerals, locale affixes, and field widths are preserved.

`FormattedNumber` contains the label text and optional mantissa and exponent parts for scientific notation. Formats with affixes, padding, or custom numerals use the text representation.

Keep values as `f64` until formatting. Converting through `f32` can change the label. Prepared formatters also accept NaN and infinity.

The locale files and license come from [d3-format 3.1.2](https://github.com/d3/d3-format/tree/ebdc2d530277df379157f82fee6ea5623d179bd7/locale). The [reference generator](../tools/d3-vega-reference/README.md) produces compatibility fixtures from D3 and Vega.
