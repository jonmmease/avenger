# D3 locales

`avenger-format-locales-d3` holds d3's locales and the bundled definitions:

- [d3's time locales](https://d3js.org/d3-time-format#locales): the `DateTimeLocaleSpec` of a locale's day and month names, periods, and the layouts of `%c`, `%x`, and `%X`. The d3 and GoogleSQL datetime providers read them.
- [d3's number locales](https://d3js.org/d3-format#locales): the `NumberLocaleSpec` of a locale's decimal point, group separator and sizes, currency affixes, numerals, percent sign, minus sign, and NaN. The d3 and GoogleSQL number providers read them.

The bundled locale is U.S. English (`en-US`). Enable `all-locales` to include all of d3's:

```toml
avenger-format-locales-d3 = { version = "0.1", features = ["all-locales"] }
```

`bundled` selects a time definition, and `bundled_number` a number definition, by its full name, with `-` or `_`, such as `fr-FR` or `fr_FR`. `BUNDLED` and `BUNDLED_NUMBER` list the bundled time and number definitions as JSON. The crate holds data, not a grammar: each provider reads the layouts in its own grammar and applies the symbols by its own rules.

```rust
use avenger_format_locales_d3::bundled_number;

let en_us = bundled_number("en_US").unwrap();
assert_eq!((en_us.decimal.as_str(), en_us.thousands.as_str()), (".", ","));
```

The time locale files and license, in `locales/`, come from [d3-time-format 4.1.0](https://github.com/d3/d3-time-format/tree/76cfef1da81b70404c0226ec9db9b0d56fa09461/locale). The number locale files and license, in `locales/number/`, come from [d3-format 3.1.2](https://github.com/d3/d3-format/tree/ebdc2d530277df379157f82fee6ea5623d179bd7/locale).
