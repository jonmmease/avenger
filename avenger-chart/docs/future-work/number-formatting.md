# Number Formatting

Master design note for Avenger number formatting: scalar formatting, d3-style
specifier parsing, axis tick-set overrides, CLDR-backed locale data, custom
locale registration, localized compact decimals, and real currency formatting.

This is the formatting master plan.

Status: **Ready for staged implementation planning**. The first implementation
step should be a new `avenger-format-number` crate with fallible parsing and a
locale registry owned by that crate.

---

## 1. Design Goals

- Put the reusable formatter in a standalone `avenger-format-number` crate.
- Let that crate own built-in locale data and user locale registration.
- Preserve d3-format strings for the existing compact numeric cases.
- Add CLDR-backed locale styles as explicit extensions, not hidden behavior.
- Use CLDR/LDML number patterns as locale-source data only; normalize them into
  Avenger structs during locale generation or custom-locale registration.
- Keep locale selection outside the format string.
- Keep currency identity explicit with an ISO 4217 code.
- Let the existing d3 sign slot express accounting currency formatting.
- Avoid putting a full option language inside the format string.
- Let callers override parsed string fields with explicit options.
- Keep text/markup adapters outside the formatter crate.

The format string should choose numeric rendering. It should not encode axis
context, units, or chart locale.

---

## 2. Crate Boundary

Create a new workspace crate:

```text
avenger-format-number
```

This crate should own all reusable number-formatting behavior:

- d3-compatible specifier parsing and validation.
- Avenger extension parsing for `S`, `L`, and `C[ISO]`.
- Structured `NumberFormatOverrides`, `NumberFormatContext`, and
  resolved-format merge rules.
- Scalar formatting over locale data.
- Number tick-set helper functions for shared precision and locked SI/compact
  tiers.
- `FormattedNumber` output with renderer-facing structured typesetting
  metadata.
- Built-in locale data generated from CLDR.
- CLDR/LDML number-pattern parsing and normalization for locale generation and
  custom locale validation.
- `NumberLocaleRegistry`, locale lookup, custom locale registration, and
  base-locale inheritance.
- Currency metadata, including the global ISO 4217 code set and default fraction
  digits.

The crate should not depend on `avenger-chart`, `avenger-guides`,
`avenger-scales`, scenegraph types, or Typst. Chart and text crates should adapt
its plain/structured output into axis labels or future markup systems.

During migration, keep `avenger_scales::format_num` as a compatibility shim that
re-exports or delegates to `avenger-format-number`. New code should import the
formatter crate directly.

The locale registry belongs in this crate, not in chart or scale code. Axis,
legend, and colorbar integration should all resolve locales through the same
registry so measured labels and rendered labels cannot diverge.

---

## 3. Base Grammar

Start with the d3-format specifier grammar:

```text
[[fill]align][sign][symbol][0][width][,][.precision][~][type]
```

Avenger adds optional type parameters after the type:

```text
[[fill]align][sign][symbol][0][width][,][.precision][~][type][type-params]
```

`type-params` are only valid for Avenger extension types. A plain d3 specifier
continues to parse as before.

### Base d3 fields

| Field | Meaning |
| --- | --- |
| `fill` + `align` | Padding fill and alignment (`<`, `>`, `^`, `=`). |
| `sign` | Sign policy (`-`, `+`, space, `(`). |
| `symbol` | d3 symbol modifier: `$` locale currency affix, `#` alternate base prefix. |
| `0` | Zero padding. |
| `width` | Minimum field width. |
| `,` | Locale grouping. |
| `.precision` | Type-dependent precision. |
| `~` | Trim insignificant trailing zeros. |
| `type` | Numeric type. |

### Base d3 types

Keep d3-compatible meanings for:

```text
e f g r s % p b o d x X c n
```

Also keep the empty/default type behavior from d3.

---

## 4. Layered Formatting API

The public formatter should not force callers to choose between a compact string
specifier and programmatic controls. It should accept a string shorthand,
structured overrides, and formatting context:

```rust
pub fn format_number(
    value: f64,
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    context: NumberFormatContext<'_>,
) -> Result<FormattedNumber, FormatError>;
```

`spec` is a concise shorthand. `overrides` contains fields that can replace
parsed string fields. `context` contains things the string never owns, such as
the resolved locale, registry access, and compatibility policy.

Merge order:

1. Parse the string specifier into format fields.
2. Apply every explicitly supplied override over the parsed string fields.
3. Fill unspecified fields from type-specific defaults.
4. Validate the resolved configuration.

This makes the string useful for user-authored defaults while still letting
guide code impose shared tick-set decisions:

```rust
format_number(value, Some(".3S"), overrides.with_precision(axis_precision), context)
format_number(value, Some(",.2C[EUR]"), overrides.with_currency(currency), context)
```

In the first example, the axis-provided `precision` overrides `.3` from the
string. In the second, the explicit `currency` option overrides `EUR` from the
string if the guide or channel supplies a different ISO code.

Recommended override fields:

| Option | Overrides |
| --- | --- |
| `type` or `style` | d3/Avenger type character. |
| `precision` | `.precision`, using the resolved type's d3 precision semantics. |
| `group` | `,` grouping flag. |
| `trim` | `~` trim flag. |
| `sign` | d3 sign slot, including accounting for `C[...]`. |
| `symbol` | d3 symbol slot, mainly `$` and `#` compatibility. |
| `width`, `fill`, `align`, `zero` | Padding fields. |
| `currency` | `C[ISO]` type parameter. |
| `currency_display` | Display choice, if added as an option. |

Use optional fields for this override type. `None` means "use the string or
default"; `Some(false)` must be able to turn off a string flag such as `,` or
`~`.

Recommended context fields:

| Context | Meaning |
| --- | --- |
| `locale` | Resolved `ResolvedNumberLocale` used for symbols, grouping, compact, and currency display. |
| `registry` | Optional `NumberLocaleRegistry` access for lookup-heavy adapters. |
| `compatibility` | Policy for d3 compatibility cases such as `$`. |

`locale` is context rather than a string override because the string never
contains a locale.

### Digit Controls

Do not represent precision, fraction digits, and significant digits as
independent fields with implicit precedence. Normalize caller intent to one
digit-control enum:

```rust
pub enum DigitSpec {
    Auto,
    Precision(u8),      // d3 .precision semantics for the resolved type
    Fraction(u8),       // exact fraction digits
    Significant(u8),    // exact significant digits
}
```

The d3 string only fills `DigitSpec::Precision`. Programmatic callers may choose
the clearer `Fraction` or `Significant` forms. If an adapter receives more than
one digit-control argument, it should return an error instead of choosing a
surprising precedence order.

### Typesetting Signal

The scalar formatter should return enough information for chart code to decide
whether labels need the richer text/typesetting path, without making
`avenger-format-number` depend on Typst or any scenegraph text backend.

```rust
pub struct FormattedNumber {
    pub text: String,
    pub typesetting: NumberTypesetting,
}

pub enum NumberTypesetting {
    Plain,
    Exponent {
        mantissa: String,
        exponent: i32,
        marker: ExponentMarker,
    },
}

pub enum ExponentMarker {
    LowerE,
}
```

`NumberTypesetting::Plain` means the plain `text` string is sufficient.
`NumberTypesetting::Exponent` means the formatted result would benefit from a
richer text layout path and carries the semantic parts needed by adapters so
they do not need to reparse localized plain text.

The first implementation should use exponent metadata only for exponent
notation:

- `e`: always returns `NumberTypesetting::Exponent`.
- `g`, `n`, and empty/default type: return `NumberTypesetting::Exponent` only
  when the actual formatted result uses exponent notation.
- all other d3 and Avenger extension types: return `NumberTypesetting::Plain` by
  default.

Do not expose uppercase `E` as a public v1 type. It is not part of d3-format's
type set, and it is reserved for a possible future engineering or scientific
extension. Existing `avenger_scales::format_num` behavior for uppercase `E`
should be treated as legacy compatibility in the shim, not as part of the new
crate's public grammar.

Axis, legend, and colorbar code can then format every label and choose one text
path for the whole label mark:

```rust
let labels: Vec<FormattedNumber> = ticks.iter().map(format_tick).collect();
let use_typesetting = labels
    .iter()
    .any(|label| !matches!(label.typesetting, NumberTypesetting::Plain));
```

If `use_typesetting` is true, the chart/text adapter is responsible for escaping
plain labels and converting exponent metadata into the chosen rich text syntax.
The formatter crate only reports the structured need; it does not emit markup.

### Typst-Label `#numfmt` Adapter

Add a Rust-backed `#numfmt` function to the Typst label layer after the core
formatter API exists. This is an adapter over `avenger-format-number`; it should
not move formatting or locale logic into `avenger-typst-label`.

The function should accept a value, a format string, and named arguments for the
same override fields supported by `NumberFormatOverrides`:

```typst
#numfmt(value, ".2f")
#numfmt(value, ".3S", precision: precision)
#numfmt(value, ",.2C[EUR]", currency: "USD", currency_display: "code")
```

The first positional argument is the value to format. The second positional
argument is the enhanced d3-style format string. The named arguments are mapped
to structured overrides before calling `format_number`:

| Argument | Override |
| --- | --- |
| `style` | Override the resolved d3/Avenger type. |
| `precision` | `DigitSpec::Precision`. |
| `fraction_digits` | `DigitSpec::Fraction`. |
| `significant_digits` | `DigitSpec::Significant`. |
| `group` | Grouping flag. |
| `trim` | Trim flag. |
| `sign` | Sign policy. |
| `symbol` | d3 symbol slot, if exposed. |
| `width`, `fill`, `align`, `zero` | Padding fields. |
| `currency` | Currency code override. |
| `currency_display` | Currency display override. |

If more than one digit-control argument is supplied, `#numfmt` should return an
error rather than picking a hidden precedence order.

`#numfmt` returns label content, not a raw unescaped string. It should call
`format_number`, inspect `FormattedNumber::typesetting`, and automatically apply
the math-capable typesetting path to exponent output when
`NumberTypesetting::Exponent` is returned. Users should not have to wrap
exponent-producing `#numfmt` calls in explicit math delimiters just to get
proper exponent typesetting.

Plain results should be escaped as ordinary label content. Results that need
typesetting should be converted by the Typst-label adapter into the appropriate
math/rich-text representation from the structured exponent metadata. The exact
conversion can start with exponent notation only, matching the initial
`NumberTypesetting` rules.

This likely requires additional `avenger-typst-label` call machinery: the
current function whitelist may need to accept a positional parameter/numeric
value, a positional string format specifier, and named arguments for this
function specifically.

### Tick Label Format Strings

Axis tick label customization should move to Typst label fragments that call
`#numfmt(...)`. The user-facing string should describe the whole tick label, and
numeric formatting should happen inside that string through the `#numfmt`
function.

Preferred shape:

```typst
#numfmt(value, ".2f")
#numfmt(value, ".1S")
#numfmt(value, ",.2C[USD]")
#numfmt(value, ".2f") kg
$#numfmt(value, ".2e") m/s^2$
```

The axis label context should bind `value` for each tick. It may also bind
set-level values such as `precision` when the axis has computed a shared
precision for the tick set:

```typst
#numfmt(value, ".3S", precision: precision)
```

This makes the enhanced d3-style specifier an argument to `#numfmt`, not the
entire tick label API. Units, currency text outside the numeric formatter,
styling, and future math labels belong in the surrounding Typst fragment.

Migration rule:

- Existing bare numeric format strings such as `.2f` or `,.0%` can be accepted
  temporarily as compatibility shorthand.
- The compatibility shorthand should desugar to `#numfmt(value, "<spec>")`.
- New examples and public API docs should prefer the Typst fragment form.

This keeps a single path for regular tick labels and rich tick labels: the chart
formats the Typst fragment for each tick, `#numfmt` returns label content, and
the resulting `NumberTypesetting` metadata decides whether the label mark needs
the math-capable text path.

---

## 5. Avenger Extension Types

Add three CLDR-backed extension types:

| Type | Meaning | Locale data required |
| --- | --- | --- |
| `S` | Compact short decimal | `ResolvedNumberLocale.compact_short` |
| `L` | Compact long decimal | `ResolvedNumberLocale.compact_long` plus plural data when needed |
| `C[ISO]` | Currency | `ResolvedNumberLocale.currency` plus global ISO currency metadata |

Examples:

```text
.1S          # compact short: 1.2M / 1,2 Mio. / 123万
.1L          # compact long: 1.2 million / 1,2 Millionen
,.2C[EUR]    # currency, EUR
,.0C[JPY]    # currency, JPY, precision override
(,.2C[USD]   # currency, USD, accounting sign
```

The uppercase extension types avoid collisions with d3:

- d3 `s` remains SI-prefix formatting.
- Avenger `S` is CLDR localized compact short.
- Avenger `L` is CLDR localized compact long.
- Avenger `C[...]` is CLDR currency formatting.

`E` remains reserved. It is not part of d3-format's public type set, and Avenger
should not add it in v1. A future extension can decide whether `E` means
engineering notation, uppercase scientific notation, or remains unsupported.

---

## 6. Currency Parameters

Currency formatting requires a currency code because locale controls display,
not the semantic currency of the value.

```text
C[USD]
C[EUR]
C[JPY]
```

Rules:

- The code is a global ISO 4217 code, not a locale-local option.
- The valid set comes from the checked-in CLDR/ISO currency metadata.
- Locale controls symbol/name localization, placement, spacing, grouping,
  native digits, accounting patterns, and currency display names.
- Currency code controls which currency is being formatted and the default
  fraction digits.
- If a localized symbol/name is missing, fall back to the ISO code.

`C` without a code is valid only when a separate currency override is supplied:

```rust
format_number(value, Some(",.2C"), overrides.with_currency("EUR"), context)
```

Without `C[ISO]` or a structured `currency` override, `C` is an error.

If both `C[EUR]` and `currency: "USD"` are supplied, the explicit option wins
and the resolved currency is USD. This follows the general override rule above.

### Accounting

Reuse d3's sign slot:

```text
,.2C[USD]    # standard currency negative pattern
(,.2C[USD]   # accounting currency pattern
```

For non-currency types, `(` keeps its d3 meaning: generic parenthesized
negative values. For `C[...]`, `(` means use the locale's CLDR accounting
currency pattern.

This avoids adding a second currency-sign mini-syntax such as `C[USD(]`.

### Currency Display

The grammar does not encode `symbol | code | name | narrow-symbol`. Default to
symbol display. Display selection is a structured override only:

```rust
format_number(
    value,
    Some(",.2C[EUR]"),
    overrides.with_currency_display(CurrencyDisplay::Code),
    context,
)
```

Do not add display choices to `C[...]` in v1.

Do not add `currency: auto` in v1. Region-default currencies can be ambiguous
over time and are not the semantic currency of a numeric value. If an adapter
adds a future convenience path, it must be explicit and should resolve to a
normal ISO code before calling `format_number`.

---

## 7. Locale Selection

Do not embed locale in the format string.

Locale should come from chart context, guide context, or another adapter outside
the formatter crate. At the `avenger-format-number` layer, the locale is already
resolved in `NumberFormatContext`:

```rust
format_number(value, Some(".1S"), overrides, en_us_context)
format_number(value, Some(".1S"), overrides, de_de_context)
format_number(value, Some(",.2C[USD]"), overrides, de_de_context)
```

The same format string renders differently under different locales:

```text
.1S, locale en-US -> 1.2M
.1S, locale de-DE -> 1,2 Mio.
.1S, locale ja-JP -> 123万

,.2C[EUR], locale en-US -> €1,234.56
,.2C[EUR], locale de-DE -> 1.234,56 €
```

Locale data is provided by the planned `NumberLocaleRegistry`, not by ad hoc
strings.

---

## 8. Locale Registry And Registration

The extension types require a real locale registry owned by
`avenger-format-number`. CLDR/ICU should be used as a build-time data source, not
as a runtime dependency.

Runtime model:

```rust
pub struct NumberLocaleRegistry {
    builtins: BTreeMap<LocaleId, NumberLocaleSpec>,
    custom: BTreeMap<LocaleId, NumberLocaleSpec>,
}
```

The registry should support:

- Lookup by BCP-47-like locale id, such as `en-US` or `de-DE`.
- User-defined custom locale ids.
- JSON/serde registration of custom locales.
- Partial custom locales with `base` inheritance.
- Validation that inherited/resolved locales contain required fields.
- A default chart/rendering locale, plus explicit per-call override.

Custom locale registration should use the same serde schema as built-ins. A user
can override one field and inherit the rest:

```json
{
  "base": "en-US",
  "decimal": ",",
  "group": ".",
  "grouping": { "primary": 3, "secondary": 2, "min_digits": 1 }
}
```

Reference a locale by registered id:

```rust
let locale = registry.resolve("de-DE")?;
let context = NumberFormatContext::new(locale);
format_number(value, Some(".1S"), overrides, context)?;
```

or through chart-level locale configuration. Inline locale dictionaries can be a
later convenience, but the registry path should come first.

### Locale Data Schema

Use two locale data shapes:

- `NumberLocaleSpec`: serde/JSON input, partial, user-authorable, may include
  `base`.
- `ResolvedNumberLocale`: complete, merged, validated runtime data used by the
  formatter.

The custom-locale schema should be partial at every level so small JSON
overrides are ergonomic:

```rust
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberLocaleSpec {
    pub base: Option<LocaleId>,
    pub decimal: Option<String>,
    pub group: Option<String>,
    pub grouping: Option<GroupingSpec>,
    pub minus: Option<String>,
    pub plus: Option<String>,
    pub percent: Option<String>,
    pub permille: Option<String>,
    pub nan: Option<String>,
    pub infinity: Option<String>,
    pub digits: Option<[String; 10]>,
    pub decimal_patterns: Option<DecimalPatternSpec>,
    pub percent_patterns: Option<DecimalPatternSpec>,
    pub currency: Option<CurrencyFormatSpec>,
    pub compact_short: Option<Vec<CompactTierSpec>>,
    pub compact_long: Option<Vec<CompactTierSpec>>,
    pub plural: Option<PluralRulesSpec>,
}
```

After resolution, the formatter should receive complete runtime data:

```rust
pub struct ResolvedNumberLocale {
    pub decimal: String,
    pub group: String,
    pub grouping: Grouping,
    pub minus: String,
    pub plus: String,
    pub percent: String,
    pub permille: String,
    pub nan: String,
    pub infinity: String,
    pub digits: Option<[String; 10]>,
    pub decimal_patterns: DecimalPatterns,
    pub percent_patterns: DecimalPatterns,
    pub compact_short: Vec<CompactTier>,
    pub compact_long: Vec<CompactTier>,
    pub currency: CurrencyFmt,
    pub plural: Option<PluralRules>,
}
```

Locale-backed fields needed by this grammar:

- Decimal separator, grouping separator, grouping rules, and minimum grouping
  digits.
- Minus sign, percent sign, NaN/infinity strings, and native digits.
- Compact short/long tiers, including non-1000 tiers such as CJK `10^4` and
  lakh/crore.
- Compact suffixes/patterns, with plural categories where needed.
- Currency symbols, names, placement patterns, accounting patterns, and per-ISO
  fraction digits.
- Region default currency data is not required for v1 because `currency: auto`
  is not part of v1.

`compact-short`, `compact-long`, and `currency` should not be marked supported
until the relevant registry data exists.

### CLDR/LDML Number Pattern Normalization

CLDR/LDML number patterns are locale-source syntax, not Avenger's public user
format-string syntax.

Use CLDR/LDML number patterns when generating built-in locales and when
validating custom locale objects that provide patterns. Parse them once during
locale generation or registration, normalize them into Avenger runtime structs,
and store only the normalized representation in `ResolvedNumberLocale`.

Runtime formatting should not repeatedly interpret raw CLDR patterns. Runtime
should use normalized data for:

- positive and negative decimal affixes;
- positive and negative percent affixes;
- currency positive and negative affixes;
- currency accounting positive and negative affixes;
- currency placement and spacing;
- grouping sizes and minimum grouping digits;
- minimum and maximum integer/fraction/significant digits where a locale pattern
  supplies them;
- compact decimal tiers and plural-sensitive suffixes/patterns.

Supported locale-ingestion pattern subset:

- positive and negative subpatterns separated by `;`;
- digit placeholders `0` and `#`;
- significant-digit placeholder `@` only if needed by compact data;
- decimal and grouping separators;
- percent and per-mille scaling signs;
- exponent marker `E` only for locale data validation, not as a public Avenger
  type;
- currency sign `¤` repetitions for symbol, ISO code, localized name, and
  narrow symbol where the CLDR source provides them;
- quoted literal affixes.

Unsupported CLDR pattern features should fail during generation or custom locale
registration rather than rendering ambiguous literals at runtime. This includes
padding escapes and any syntax that the normalized Avenger runtime structs do
not represent.

### Built-In Locale Generation

Generate built-in locale data offline when CLDR is bumped, then check the
result into the repository:

```text
xtask gen-locales
      |
      | load CLDR/ICU data, normalize to Avenger structs
      v
avenger-format-number/locales/<tag>.json
      |
      | include at compile time
      v
NumberLocaleRegistry::with_builtins()
```

Do not fetch or parse CLDR from `build.rs` on every downstream compile. A small
`build.rs` that transcodes checked-in JSON into Rust constants is acceptable;
networked or heavyweight extraction is not.

Extraction source options:

- `icu_provider_source`, as used by ICU4X datagen. This is fastest to bootstrap
  and can be a build-only tool dependency.
- Direct `cldr-json` parsing. This avoids ICU coupling but requires more custom
  parser code.

Push normalization to generation time. Parse CLDR/LDML decimal, percent,
currency, accounting, and compact patterns; validate that they use only the
supported ingestion subset; pre-resolve compact tiers into
`{ exponent, suffix_per_plural_category }`; strip CLDR `{0}` placeholders once;
record grouping and minimum grouping digits; bake default numbering-system
digits; and record per-currency symbols, names, placement patterns, accounting
patterns, and default fraction digits.

Runtime should do only registry lookup, option resolution, rounding, grouping,
digit substitution, and pattern interpolation.

### Font-Aware Built-Ins

The first built-in locale set should match the fonts Avenger ships. A locale is
displayable only when every glyph its formatter can emit is present in the font's
character map.

The old prototype found that Latin-script locales are a good first cut when
using Latin digits. Some non-Latin locales can still format ordinary numbers
with Latin digits, but their compact suffixes may need either additional font
coverage or an ASCII SI fallback. Keep this as a generation-time decision rather
than a runtime surprise.

---

## 9. Precision Semantics

Use d3 precision semantics where possible:

- `e`, `f`, `%`, and `C[...]`: precision is exact fraction digits.
- `g`, `r`, `s`, `p`, `S`, and `L`: precision is significant digits.
- Explicit `DigitSpec::Precision` overrides string precision and may come from
  axis tick context.
- Omitted precision may be filled from axis tick context by supplying that same
  explicit override.
- `~` trims insignificant trailing zeros after formatting.

Currency exception:

- If precision is omitted for `C[...]`, use the currency's CLDR fraction digits.
- If precision is supplied, it overrides the currency default.

Examples:

```text
C[USD]      # USD default fraction digits, normally 2
.0C[USD]    # force no fraction digits
C[JPY]      # JPY default fraction digits, normally 0
.2C[JPY]    # force two fraction digits
.2~C[USD]   # explicitly trim insignificant trailing zeros
```

For currency, `~` should be honored only when explicitly supplied by the string
or structured `trim` override. Omitted `~` preserves the resolved currency
fraction digits.

---

## 10. Tick-Set Context

The format string does not encode tick-set context.

Axis code may provide:

- computed `DigitSpec::Precision`
- locale
- currency code if a guide/channel declares currency semantics
- locked SI or CLDR compact tier

Examples:

```rust
format_number(tick, Some(".1S"), overrides.with_precision(axis_precision), context)
format_number(tick, Some(",.2C[EUR]"), overrides.with_currency(axis_currency), context)
```

For SI and compact labels, axis code should lock the tier across the tick set
when formatting axis ticks. A formatter should never produce mixed units like
`900k, 1.0M, 1.1M` for a single axis when a locked tier is requested.

Do not add explicit scale locking to the scalar `format_number` API. D3 has the
`s` type for per-value SI-prefix formatting and a separate `formatPrefix`
function that fixes the SI prefix from an external reference value, but the d3
format string grammar itself has no equivalent slot.

Add a separate set-level helper in `avenger-format-number`:

```rust
pub fn prepare_number_tick_format(
    values: &[f64],
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    context: NumberFormatContext<'_>,
) -> Result<PreparedNumberTickFormat, FormatError>;

pub struct PreparedNumberTickFormat {
    pub resolved: ResolvedNumberFormat,
    pub tick_overrides: NumberFormatOverrides,
}
```

This helper should compute shared precision and, for `s`, `S`, and `L`, a locked
SI or compact tier. Axis, legend, and colorbar code can then format each scalar
through the prepared format. The scalar API stays simple, and the set-level
logic has one home instead of being reimplemented in each guide.

---

## 11. Compatibility Rules

### d3 `$` symbol

d3's `$` symbol slot is compatibility behavior. It means "apply the locale's
currency affix" to an existing numeric type; it does not carry an ISO currency
code.

```text
$,.2f
```

Keep `$` as a simple d3 compatibility affix. Do not desugar it to `C[...]`, and
do not use `$` as a currency type code. The CLDR-aware currency path is
`C[ISO]` or a `currency` override.

Recommended errors:

- `$` with `C[...]`.
- `$` with a `currency` override.

Both cases should ask the user to use `C[USD]` or the structured `currency`
override instead.

### `#` symbol

Keep `#` as the d3 alternate-base modifier for `b`, `o`, `x`, and `X`. Do not
reuse the symbol slot for Avenger extensions.

### Invalid combinations

Recommended errors:

- `C[]` with no currency code.
- `C` with no structured `currency` override.
- `C[EUR]` with `#`.
- `C[EUR]` with `$`.
- `$` with a `currency` override.
- `S` or `L` with `#`.
- `C[EUR]` before currency registry support exists.
- `S` or `L` before compact registry support exists.

---

## 12. Implementation Plan

1. Create `avenger-format-number`.
2. Make parsing fallible; invalid specs must not panic.
3. Implement the d3-compatible parser core.
4. Add structured `NumberFormatOverrides`, `NumberFormatContext`, and
   resolved-format merge rules.
5. Extend the type parser with optional `type-params`.
6. Add extension types `S`, `L`, and `C[ISO]`.
7. Add structured `FormattedNumber::typesetting` metadata for exponent output.
8. Move locale data structs, built-in locale data, and `NumberLocaleRegistry`
   into the new crate.
9. Add `NumberLocaleSpec` and `ResolvedNumberLocale`.
10. Add CLDR/LDML number-pattern parsing and normalization for locale ingestion.
11. Add custom locale registration with `base` inheritance and validation.
12. Add offline CLDR extraction for decimal, percent, compact, and currency data.
13. Implement `S` and `L` using registry compact tiers.
14. Implement `C[ISO]` using registry currency metadata.
15. Add `prepare_number_tick_format` for shared precision and locked SI/compact
   tiers.
16. Route axis/legend/colorbar label marks through the richer text path when any
   formatted label returns non-plain `NumberTypesetting`.
17. Migrate chart, guide, scale, legend, and colorbar call sites onto the shared
   fallible formatter.
18. Add the Typst-label `#numfmt(value, spec, ...overrides)` adapter once the
   core formatter and label-call machinery can support the required arguments.
19. Update axis tick label format strings to use Typst fragments with
   `#numfmt(value, "<spec>")`, keeping bare d3-style specs only as compatibility
   shorthand during migration.

---

## 13. Resolved Decisions And Remaining Questions

- `~` is honored for currency only when explicitly supplied.
- `C` without `[...]` is allowed only when a structured `currency` override is
  supplied.
- `currency: auto` is not part of v1.
- `currency_display` remains a structured override only in v1.
- `E` is not part of the public v1 grammar. Reserve it for a future extension.
- Enhanced d3 strings remain the concise formatter specifier inside `#numfmt`.
  The preferred public axis-label customization is the Typst fragment
  `#numfmt(value, "<spec>")`; bare specs are compatibility shorthand.
- Invalid combinations should be strict errors in the new crate. Preserve legacy
  quirks only in `avenger_scales::format_num` compatibility shims.
- The first locale generation pass should prefer `icu_provider_source` or ICU4X
  datagen for bootstrap speed, while keeping the generated compact Avenger data
  checked into the repo and keeping ICU out of runtime dependencies.

Remaining questions:

- Which built-in locale allowlist should ship with the default font bundle?
- How much of the existing `avenger_scales::format_num` behavior should remain
  available through the compatibility shim after chart code migrates?
