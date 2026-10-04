# Number skeletons

`IcuNumberFormatProvider::prepare` accepts [ICU number skeletons](https://unicode-org.github.io/icu/userguide/format_parse/numbers/skeletons.html) without the `::` prefix, in long or concise form, and formats them with CLDR locale data.

## Unsupported options

Preparation returns an error for:

- mixed units and `usage`
- `rounding-mode-unnecessary`, because formatting cannot fail
- repeated notation options, such as `scientific/*e/*ee`
- compact currency, percent, and per-mille; full-name percent and per-mille; and formal and variant currency widths
- measurement units outside length, area, duration, mass, and volume, other than `percent` and `permille`
- compound units, including `per-measure-unit`, other than `square-` and `cubic-` powers
- measurement units in locales without unit names, such as `gsw`
- numbering systems without decimal digits, such as `roman`
- decimal literals with more than 999 digits or magnitudes beyond ±1000

## Locales

Locales resolve through ICU4X's fallback chain, so `nb` uses Norwegian `no` data. `und` selects root data. Preparation rejects invalid tags and locales without number data, such as `xx` and `az-Cyrl`. The [`nu` key](https://www.unicode.org/reports/tr35/#Key_Type_Definitions) selects the numbering system.

## Known gaps

ICU4X's data omits currency symbols that equal the ISO code, so those locales show the root symbol: Italian formats `currency/USD` with `US$` where CLDR specifies `USD`. Per-currency separators, such as the Cape Verdean escudo's `$`, are not applied.

## Data

Patterns, symbols, and plural rules come from ICU4X 2.3's compiled data. Tables generated from CLDR 48.2.1 supply what ICU4X omits, under the [Unicode license](data/LICENSE). The [reference tools](../tools/icu-number-reference/README.md) regenerate them and the test fixture.
