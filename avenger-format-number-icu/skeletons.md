# Number skeletons

`IcuNumberFormatProvider::prepare` accepts [ICU number skeletons](https://unicode-org.github.io/icu/userguide/format_parse/numbers/skeletons.html) without the `::` prefix, in long or concise form, and formats them with CLDR locale data.

## Unsupported options

Preparation returns an error for:

- notation, scale, rounding increments, percent, per-mille, currencies, and units
- `rounding-mode-unnecessary`, because formatting cannot fail
- numbering systems without decimal digits, such as `roman`

## Locales

Locales resolve through ICU4X's fallback chain, so `nb` uses Norwegian `no` data. `und` selects root data. Preparation rejects invalid tags and locales without number data, such as `xx` and `az-Cyrl`. The [`nu` key](https://www.unicode.org/reports/tr35/#Key_Type_Definitions) selects the numbering system.

## Data

Patterns and symbols come from ICU4X 2.3's compiled data. Tables generated from CLDR 48.2.1 supply what ICU4X omits, under the [Unicode license](data/LICENSE). The [reference tools](../tools/icu-number-reference/README.md) regenerate them and the test fixture.
