# D3 time locales

`avenger-format-locales-d3` holds [d3's time locales](https://d3js.org/d3-time-format#locales): the `DateTimeLocaleSpec` of a locale's day and month names, periods, and the layouts of `%c`, `%x`, and `%X`, the bundled definitions, a registry of bundled and custom definitions, and the resolution of BCP 47 tags. The d3 datetime provider reads them, and so can other providers of `%` formats.

The specification takes optional fields that d3 does not have: `standaloneMonths` and `standaloneShortMonths`, the month names that stand alone, such as Russian `март`, where `months` holds the names in a date, `марта`. A definition without them uses `months` and `shortMonths`. The bundled d3 definitions have none of the optional fields.

The bundled locale is U.S. English (`en-US`). Enable `all-locales` to include all of d3's:

```toml
avenger-format-locales-d3 = { version = "0.1", features = ["all-locales"] }
```

`resolve_tag` selects a name for a tag: the same language and region, without regard to case and with `_` for `-`, or else the language's likely region from CLDR, such as `pt` to `pt-BR`, or else the first name of the language. The crate holds data, not a grammar: each provider reads the layouts in its own grammar.

The locale files and license come from [d3-time-format 4.1.0](https://github.com/d3/d3-time-format/tree/76cfef1da81b70404c0226ec9db9b0d56fa09461/locale).
