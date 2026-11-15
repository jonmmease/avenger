# D3 time locales

`avenger-format-locales-d3` holds [d3's time locales](https://d3js.org/d3-time-format#locales): the `DateTimeLocaleSpec` of a locale's day and month names, periods, and the layouts of `%c`, `%x`, and `%X` and the bundled definitions. The d3 and GoogleSQL datetime providers read them.

The bundled locale is U.S. English (`en-US`). Enable `all-locales` to include all of d3's:

```toml
avenger-format-locales-d3 = { version = "0.1", features = ["all-locales"] }
```

`bundled` selects a definition by its full name, with `-` or `_`, such as `fr-FR` or `fr_FR`. The crate holds data, not a grammar: each provider reads the layouts in its own grammar.

The locale files and license come from [d3-time-format 4.1.0](https://github.com/d3/d3-time-format/tree/76cfef1da81b70404c0226ec9db9b0d56fa09461/locale).
