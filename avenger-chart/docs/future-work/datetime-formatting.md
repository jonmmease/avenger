# Datetime Formatting

Master design note for Avenger datetime formatting: LDML datetime patterns,
compact CLDR style specs such as `{datetime:medium}`, CLDR-backed locale data,
custom locale registration, a Typst-label `#datefmt` adapter, and temporal axis
tick labels that use that adapter.

This is intentionally parallel to [number-formatting.md](number-formatting.md).
The formatter should be reusable outside chart axes, while chart and text layers
adapt it into labels.

Status: **Ready for staged implementation planning**. The first implementation
step should be a new `avenger-format-datetime` crate with fallible parsing and a
locale registry story aligned with number formatting.

---

## 1. Design Goals

- Put reusable datetime formatting in a standalone crate.
- Use LDML as the only user-facing datetime field pattern language.
- Add compact style blocks such as `{date:long}`, `{time:short}`, and
  `{datetime:medium}` that resolve to CLDR LDML patterns.
- Do not support d3-time-format, strftime, or chrono format strings in the new
  formatter crate.
- Do not add CLDR skeletons in v1. Skeleton matching brings a larger data
  payload and fallback model than chart labels need.
- Keep locale and timezone selection outside the format string.
- Let callers provide structured overrides/context around the string.
- Let built-in and custom locales come from serde/JSON data.
- Preserve the distinction between naive temporal values and timezone-aware
  instants.
- Avoid ICU at runtime; use CLDR/ICU only as an offline data source.
- Add a Rust-backed Typst-label `#datefmt(value, spec, ...overrides)` adapter.
- Update temporal axis tick label customization to use Typst fragments that call
  `#datefmt(...)`.
- Do not add a typesetting signal. Datetime formatting returns plain label
  content.

The format string should choose datetime fields, style presets, and literal
text. It should not encode chart locale, axis context, timezone defaults, or
cascade policy.

---

## 2. Crate Boundary

Create a new workspace crate:

```text
avenger-format-datetime
```

This crate should own all reusable datetime-formatting behavior:

- LDML pattern parsing and validation.
- Avenger style-block parsing for CLDR date/time presets.
- Structured `DateTimeFormatOverrides`, `DateTimeFormatContext`, and
  resolved-format merge rules.
- Scalar datetime formatting over locale data.
- Built-in datetime locale data generated from CLDR.
- Datetime locale lookup, custom locale registration, and base-locale
  inheritance.
- Naive and timezone-aware value extraction using `chrono` types at the public
  crate boundary.

The crate should not depend on `avenger-chart`, `avenger-guides`,
`avenger-scales`, scenegraph types, or Typst. Chart and text crates should adapt
its plain output into axis labels or Typst-label content.

During migration, keep existing `avenger-scales` temporal formatter entry points
as compatibility shims that translate to, or otherwise delegate into,
`avenger-format-datetime`. Any legacy format-string compatibility should live in
those shims or in chart migration code, not in the new formatter crate.

### Shared Locale Infrastructure

Number and datetime formatting both need locale ids, built-in locale loading,
custom locale registration, base-locale inheritance, fallback, validation, and
offline CLDR generation. There are two reasonable implementation paths:

1. Start with separate locale modules in `avenger-format-number` and
   `avenger-format-datetime`, then extract shared pieces after both are real.
2. Create a small shared crate such as `avenger-format-locale` up front.

Recommendation: create a shared locale crate only if both formatter crates are
implemented in the same cycle. Otherwise, keep the first implementation local
and extract when the second crate lands. The shared crate should own generic
pieces such as `LocaleId`, registry merge/fallback, and serde registration; the
number and datetime crates should still own their domain-specific locale data
schemas.

---

## 3. Format Spec Grammar

The public `spec` string is an Avenger datetime format spec. It has two token
kinds:

1. LDML pattern text.
2. Avenger style blocks wrapped in braces.

Examples:

```text
yyyy-MM-dd
MMM d
HH:mm
EEEE, MMMM d, y
HH:mm XXX
{date:long}
{time:short}
{datetime:medium}
{date:medium} at {time:short}
```

The style block is not a separate dialect. It expands to a locale-provided LDML
pattern before rendering. After style expansion, the renderer formats one LDML
pattern stream.

### LDML Pattern Text

Support LDML apostrophe quoting:

```text
MMM d 'at' HH:mm
''yy
```

Repeated field letters control width, padding, and text-vs-number behavior using
LDML rules for the supported field set. Literal text passes through unchanged
except for style blocks and apostrophe-quoted regions.

Because old d3/strftime strings are easy to type accidentally, an unquoted `%`
followed by an ASCII alphabetic character should be a parse error with a message
that d3/strftime syntax is not accepted. Users who need a literal percent before
a letter can quote it as LDML literal text.

### Style Blocks

Support these v1 style blocks:

```text
{date}
{date:short}
{date:medium}
{date:long}
{date:full}

{time}
{time:short}
{time:medium}
{time:long}
{time:full}

{datetime}
{datetime:short}
{datetime:medium}
{datetime:long}
{datetime:full}
```

When the style length is omitted, the formatter uses structured overrides or
context defaults:

| Block | Default source |
| --- | --- |
| `{date}` | `DateTimeFormatOverrides.date_style`, then context default. |
| `{time}` | `DateTimeFormatOverrides.time_style`, then context default. |
| `{datetime}` | `DateTimeFormatOverrides.datetime_style`, then context default. |

`{datetime:medium}` means: render the locale's medium date pattern, medium time
pattern, and medium date-time glue. The glue pattern replaces `{1}` with the
date result and `{0}` with the time result, matching CLDR date-time pattern
ordering.

Do not add skeleton style blocks in v1. In particular, do not support
`{skeleton:...}` or LDML skeleton matching. If users need an exact shape, they
should write an LDML pattern. If they need a locale preset, they should use a
style block.

Mixed-style date-time blocks such as `{datetime:date=long,time=short}` can be a
future extension. V1 should keep the style block grammar deliberately small.

Malformed or unknown style blocks are errors. Literal braces in LDML pattern
text should be apostrophe-quoted.

---

## 4. LDML Field Subset

V1 should support the LDML fields needed for chart labels and CLDR date/time
presets in the built-in locale allowlist.

| LDML field | Runtime behavior |
| --- | --- |
| `G` | Era name using locale era widths. |
| `y` | Calendar year. |
| `M` | Format-context month number/name by width. |
| `L` | Standalone month number/name by width. |
| `d` | Day of month. |
| `D` | Day of year. |
| `E` | Format-context weekday name by width. |
| `e` | Locale weekday number/name by width. |
| `c` | Standalone weekday number/name by width. |
| `q` | Format-context quarter number/name by width. |
| `Q` | Standalone quarter number/name by width. |
| `a` | AM/PM day period. |
| `h` | 1-12 hour cycle. |
| `H` | 0-23 hour cycle. |
| `K` | 0-11 hour cycle. |
| `k` | 1-24 hour cycle. |
| `m` | Minute. |
| `s` | Second. |
| `S` | Fractional second digits. |
| `X`, `x` | ISO-style numeric timezone offsets for zoned inputs. |
| `Z`, `O` | Localized numeric timezone offsets for zoned inputs. |

Timezone fields require timezone-aware input. For naive input, `X`, `x`, `Z`,
and `O` are validation errors.

LDML timezone-name fields (`z`, `v`, and `V`) should parse as known but
unsupported fields in v1. They should not silently produce timezone names or
invent abbreviations. If CLDR long/full style patterns for a built-in locale use
timezone-name fields, locale generation must either exclude those styles or
normalize them to supported numeric offset fields as an explicit generation-time
choice.

Unsupported LDML fields should fail during parse, generation, or locale
registration. They should not pass through as ambiguous literal text.

Open extension candidates:

- flexible day periods beyond AM/PM;
- locale week-year and week-number fields;
- timezone names backed by real timezone-name data;
- alternate calendars;
- mixed-style date-time blocks.

---

## 5. Layered Formatting API

The public formatter should accept a string shorthand, structured overrides, and
formatting context. Because Avenger supports both naive time scales and
timezone-aware time scales, expose two public entry points over one shared
parser/renderer:

```rust
pub enum NaiveDateTimeInput {
    Date(chrono::NaiveDate),
    DateTime(chrono::NaiveDateTime),
}

pub type ZonedDateTimeInput = chrono::DateTime<chrono::Utc>;

pub fn format_naive_datetime(
    value: NaiveDateTimeInput,
    spec: Option<&str>,
    overrides: DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<FormattedDateTime, DateTimeFormatError>;

pub fn format_zoned_datetime(
    value: ZonedDateTimeInput,
    spec: Option<&str>,
    overrides: DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<FormattedDateTime, DateTimeFormatError>;
```

Use `chrono` and `chrono_tz` types directly in v1. Avenger wrapper types can be
added later if the crate needs to support another time stack without changing
the user-facing chart API.

`spec` is an LDML pattern with optional Avenger style blocks. `overrides`
contains explicit caller choices. `context` contains things the string never
owns, such as resolved locale, timezone, registry access, and compatibility
policy.

```rust
pub struct FormattedDateTime {
    pub text: String,
}
```

There is no `needs_typesetting` flag for datetime formatting. The result is
plain label content; the Typst-label adapter should escape it as ordinary text.

The two entry points should share a core implementation after the input is
lowered to resolved calendar fields:

```rust
pub struct DateTimeFields {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanosecond: u32,
    pub offset: Option<FixedOffset>,
    pub epoch_millis: Option<i64>,
}
```

`format_naive_datetime` formats fields exactly as supplied. It must not attach
UTC, local time, or the chart timezone to a naive value. Date-only input must be
lowered to naive midnight for calendar field rendering, and must still stay on
the naive path with no timezone offset or absolute epoch instant.

`format_zoned_datetime` formats an absolute instant in the resolved display
timezone. Its extracted fields include the display offset and the original epoch
instant. This is the entry point for Arrow timestamps with timezone metadata and
for scale values that are semantically instants.

LDML fields that require an offset have input requirements:

| Fields | Naive entry point | Zoned entry point |
| --- | --- | --- |
| `X`, `x`, `Z`, `O` | Error: no timezone offset. | Numeric display timezone offset. |

Merge order:

1. Parse the string specifier into pattern fields and style blocks.
2. Resolve style blocks through the locale and structured style overrides.
3. Apply every explicitly supplied override.
4. Fill unspecified fields from context and locale defaults.
5. Validate the resolved configuration against the input kind.

Recommended override fields:

| Option | Meaning |
| --- | --- |
| `timezone` | Display timezone override for `format_zoned_datetime`; invalid for `format_naive_datetime`. |
| `date_style` | Length used by `{date}` and by default style selection when `spec` is absent. |
| `time_style` | Length used by `{time}` and by default style selection when `spec` is absent. |
| `datetime_style` | Length used by `{datetime}` and by default style selection when `spec` is absent. |
| `first_day_of_week` | Reserved for locale week fields if they are added later. |

Recommended context fields:

| Context | Meaning |
| --- | --- |
| `locale` | Resolved datetime locale data. |
| `timezone` | Default display timezone for timezone-aware input. |
| `registry` | Optional locale registry access for adapters. |
| `compatibility` | Policy for strict parsing and migration diagnostics. |

Locale selection is context, not a format-string field. A Typst or chart adapter
may accept `locale: "fr-CA"`, but it should resolve the locale before calling
the core scalar formatter.

Timezone selection is also outside the string. LDML timezone fields render the
timezone offset for the resolved display timezone when formatting a
timezone-aware value; they do not choose the timezone.

### Timezone Context And Precedence

Avenger already has chart-level temporal context:

```rust
pub struct TimeContext {
    pub timezone: Option<String>,
    pub week_start: Option<WeekStart>,
}
```

`TimeContext::resolved_timezone()` currently defaults to `UTC`, and child plots
resolve missing fields from their parent context. During scale building,
temporal scales receive a default `timezone` option from the resolved
`TimeContext`; explicit channel scale options and plot-level scale overrides are
then applied after that default.

The datetime formatting integration should reuse this existing source of truth.
For timezone-aware tick values, timezone precedence should be:

1. Explicit `#datefmt(..., timezone: "...")` or direct formatter override.
2. The resolved temporal scale `timezone` option.
3. The resolved chart `TimeContext`.
4. `UTC`.

Naive date and timestamp values ignore this timezone chain. Passing a timezone
override for a naive value is an error.

V1 should accept `UTC` and valid IANA timezone ids, parsed with `chrono_tz::Tz`.
Do not support `"local"` in the new formatter. Existing scale code currently
accepts `"local"` as a UTC fallback placeholder; the formatting migration should
not carry that behavior forward.

---

## 6. Locale Selection

Do not embed locale in the format string.

Locale should come from chart context, guide context, or another adapter outside
the formatter crate. At the `avenger-format-datetime` layer, the locale is
already resolved in `DateTimeFormatContext`:

```rust
format_naive_datetime(value, Some("MMM d"), overrides, en_us_context)
format_zoned_datetime(value, Some("MMM d"), overrides, fr_fr_context)
format_zoned_datetime(value, Some("{datetime:medium}"), overrides, de_de_context)
```

The same format string renders differently under different locales:

```text
MMM d, locale en-US -> Jan 5
MMM d, locale fr-FR -> 5 janv.

{date:long}, locale en-US -> January 5, 2026
{date:long}, locale de-DE -> 5. Januar 2026
```

Locale data is provided by the planned registry, not by ad hoc strings.

---

## 7. Locale Registry And Registration

Style blocks and locale text fields require a real locale registry owned by
`avenger-format-datetime` or by a shared locale crate. CLDR/ICU should be used
as a build-time data source, not as a runtime dependency.

Use two locale data shapes:

- `DateTimeLocaleSpec`: serde/JSON input, partial, user-authorable, may include
  `base`.
- `ResolvedDateTimeLocale`: complete, merged, validated runtime data used by the
  formatter.

Runtime model:

```rust
pub struct DateTimeLocaleRegistry {
    builtins: BTreeMap<LocaleId, DateTimeLocaleSpec>,
    custom: BTreeMap<LocaleId, DateTimeLocaleSpec>,
}
```

The registry should support:

- lookup by BCP-47-like locale id, such as `en-US` or `de-DE`;
- user-defined custom locale ids;
- JSON/serde registration of custom locales;
- partial custom locales with `base` inheritance;
- recursive base resolution with cycle detection;
- validation that resolved locales contain required fields;
- explicit per-chart registries, not global mutable state;
- a default chart/rendering locale, plus explicit per-call override.

Example custom locale:

```json
{
  "base": "en-US",
  "months": {
    "abbrev": ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
               "Jul", "Aug", "Sept", "Oct", "Nov", "Dec"]
  },
  "date_patterns": {
    "long": "d MMMM y"
  }
}
```

Reference a locale by registered id:

```rust
let locale = registry.resolve("fr-FR")?;
let context = DateTimeFormatContext::new(locale, timezone);
format_zoned_datetime(value, Some("MMM d"), overrides, context)?;
```

or through chart-level locale configuration. Inline locale dictionaries can be a
later convenience, but the registry path should come first.

### Locale Data Schema

The custom-locale schema should be partial at every level so small JSON
overrides are ergonomic:

```rust
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DateTimeLocaleSpec {
    pub base: Option<LocaleId>,
    pub months: Option<Widths12Spec>,
    pub months_standalone: Option<Widths12Spec>,
    pub weekdays: Option<Widths7Spec>,
    pub weekdays_standalone: Option<Widths7Spec>,
    pub quarters: Option<Widths4Spec>,
    pub quarters_standalone: Option<Widths4Spec>,
    pub eras: Option<Widths2Spec>,
    pub day_periods: Option<DayPeriodsSpec>,
    pub date_patterns: Option<LengthsSpec>,
    pub time_patterns: Option<LengthsSpec>,
    pub datetime_glue: Option<LengthsSpec>,
    pub first_day_of_week: Option<Weekday>,
    pub digits: Option<[String; 10]>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Widths12Spec {
    pub narrow: Option<[String; 12]>,
    pub abbrev: Option<[String; 12]>,
    pub wide: Option<[String; 12]>,
}
```

Use analogous partial spec structs for weekdays, quarters, eras, day periods,
and length patterns. After resolution, the formatter should receive complete
runtime locale data:

```rust
pub struct ResolvedDateTimeLocale {
    pub months: Widths12,
    pub months_standalone: Widths12,
    pub weekdays: Widths7,
    pub weekdays_standalone: Widths7,
    pub quarters: Widths4,
    pub quarters_standalone: Widths4,
    pub eras: Widths2,
    pub day_periods: DayPeriods,
    pub date_patterns: Lengths,
    pub time_patterns: Lengths,
    pub datetime_glue: Lengths,
    pub first_day_of_week: Weekday,
    pub digits: Option<[String; 10]>,
}
```

Validation should check array lengths, digit count, supported LDML pattern
syntax in built-in and custom preset strings, missing required fields after
inheritance, and invalid fallback cycles.

### CLDR Pattern Handling

CLDR date/time presets are LDML patterns, so the same parser should handle both
user-authored patterns and locale-provided patterns. The locale-generation
pipeline should parse and normalize built-in CLDR patterns into a compact
Avenger-owned pattern AST, then check the normalized locale data into the
repository.

Custom locale registration should parse and validate LDML patterns at
registration time. Unsupported LDML fields should fail there, before rendering.

Date-time glue patterns should replace `{0}` with the rendered time pattern and
`{1}` with the rendered date pattern.

Do not implement skeleton matching, alternate calendars, or plural pattern
selection in v1.

### Digit Substitution

Keep digit substitution as a runtime capability, but do not make it part of the
first built-in locale requirement.

Recommendation:

- Built-in locales in the first pass should use ASCII digits.
- Custom locale JSON may provide `digits: ["0", ..., "9"]` replacements.
- When `digits` is present, apply it to formatter-produced numeric tokens after
  padding and rounding, before concatenating with literal text.
- Do not substitute digits inside user-authored literal text or locale name
  strings.
- Validate that exactly ten nonempty digit strings are provided.

This keeps Latin-ext built-ins predictable while leaving a clean path for
custom or future non-Latin numbering systems.

### Built-In Locale Generation

Generate built-in datetime locale data offline when CLDR is bumped, then check
the result into the repository:

```text
xtask gen-datetime-locales
      |
      | load CLDR/ICU data, normalize to Avenger structs
      v
avenger-format-datetime/locales/<tag>.json
      |
      | include at compile time
      v
DateTimeLocaleRegistry::with_builtins()
```

Do not fetch or parse CLDR from `build.rs` on every downstream compile. A small
`build.rs` that transcodes checked-in JSON into Rust constants is acceptable;
networked or heavyweight extraction is not.

Extraction source options:

- `icu_provider_source` / ICU4X datagen. This is fastest to bootstrap and can be
  a build-only tool dependency.
- Direct `cldr-json` parsing. This avoids ICU coupling but requires more custom
  parser code.

The old prototype found that runtime ICU data/code is far larger than the subset
Avenger needs. The retained plan is still: use ICU/CLDR as an offline oracle,
commit compact serde data, and ship zero ICU runtime dependency.

Built-in extraction should start with:

- Gregorian calendar only.
- No skeletons.
- A built-in locale allowlist whose rendered symbols are representable by the
  Latin-ext character set that Avenger can ship/render by default.
- ASCII digits for built-ins in the first pass.
- Date/time preset patterns and date-time glue.
- Month, weekday, quarter, era, and day-period names.
- Week data only if a supported field or future extension needs it.

Push normalization to generation time. Runtime should do only registry lookup,
timezone conversion, style expansion, LDML field rendering, digit
substitution, and pattern interpolation.

---

## 8. Typst-Label `#datefmt` Adapter

Add a Rust-backed `#datefmt` function to the Typst label layer after the core
formatter API exists. This is an adapter over `avenger-format-datetime`; it
should not move formatting or locale logic into `avenger-typst-label`.

The function should accept a value, a format string, and named arguments for the
same override/context choices supported by `DateTimeFormatOverrides` and the
chart adapter:

```typst
#datefmt(value, "MMM d")
#datefmt(value, "y")
#datefmt(value, "{date:long}", locale: "fr-FR")
#datefmt(value, "{datetime:medium}")
#datefmt(value, "HH:mm XXX", timezone: "America/New_York")
```

The first positional argument is the datetime value. The second positional
argument is an LDML-based Avenger datetime spec. Named arguments are mapped to
structured overrides/context before calling the appropriate formatter entry
point:

| Argument | Meaning |
| --- | --- |
| `locale` | Resolve a registered locale id for this call. |
| `timezone` or `tz` | Display timezone override for zoned values. |
| `date_style` | Default length for `{date}` or when `spec` is absent. |
| `time_style` | Default length for `{time}` or when `spec` is absent. |
| `datetime_style` | Default length for `{datetime}` or when `spec` is absent. |

`#datefmt` returns label content, not a raw unescaped string. The result is plain
label content; no math or rich typesetting path is required.

The Typst-facing API should stay as one function. The adapter should inspect the
bound tick value's temporal kind and dispatch to `format_naive_datetime` or
`format_zoned_datetime`. Passing `timezone` for a naive value should be a
validation error, not a silent conversion.

This likely requires additional `avenger-typst-label` call machinery: the
current function whitelist may need to accept a positional datetime/numeric
value, a positional string format specifier, and named arguments for this
function specifically.

---

## 9. Tick Label Format Strings

Temporal axis tick label customization should move to Typst label fragments that
call `#datefmt(...)`. The user-facing string should describe the whole tick
label, and datetime formatting should happen inside that string through the
`#datefmt` function.

Preferred shape:

```typst
#datefmt(value, "MMM d")
#datefmt(value, "HH:mm")
#datefmt(value, "{date:medium}")
#datefmt(value, "{datetime:medium}")
#datefmt(value, "MMM d") UTC
```

The axis label context should bind `value` for each tick. It may also bind
set-level or tick-classification values later, such as `interval` or
`boundary_level`, if the temporal cascade needs them.

This makes the LDML specifier an argument to `#datefmt`, not the entire tick
label API. Extra text, styling, and custom labels belong in the surrounding
Typst fragment.

Migration rule:

- New examples and public API docs should prefer Typst fragments with
  `#datefmt(value, "<LDML-or-style-spec>")`.
- Existing bare temporal format strings should not be treated as d3/strftime
  specs by the new formatter crate.
- If old chart APIs need temporary compatibility, that compatibility should live
  in adapter code and should emit migration diagnostics.
- The bound `value` must preserve whether the source scale is naive or
  timezone-aware so `#datefmt` can call the correct formatter entry point.

---

## 10. Axis Tick Formatting

Tick placement stays with the temporal scale. The datetime formatter owns only
labeling.

The current temporal formatter uses hardcoded interval-specific patterns. Replace
that with calls into `avenger-format-datetime`.

V1 axis behavior:

- Use the existing tick placement logic.
- Add a temporal tick-label field parallel to numeric tick label fragments.
- Accept Typst fragments with `#datefmt(value, spec, ...overrides)`.
- Use LDML patterns and style blocks for datetime specs.
- Use locale from scale/chart context unless overridden.
- For timezone-aware values, use timezone from scale/chart context unless
  overridden.
- Keep output plain; no typesetting signal is needed.

Later cascade behavior:

- Use the tick interval and tick boundary classification to choose a spec per
  tick.
- Preserve multi-scale temporal labeling behavior, but express the default
  specs in LDML, such as `y`, `MMM`, `MMM d`, `HH:mm`, `mm:ss`, and `SSS`.
- Keep more advanced context strategies, such as edge labels or tiered labels,
  as a later axis-level feature.

The formatter crate should not know about chart axes or tick arrays. If Avenger
needs a set-aware temporal helper later, it should live as a small utility over
the scalar formatter, not as a hidden side effect of the main entry points.

---

## 11. Compatibility Rules

Recommended compatibility rules:

- LDML fields keep their documented Avenger v1 meanings.
- Unknown or unsupported LDML fields are errors.
- Unknown or malformed `{...}` style blocks are errors.
- D3, strftime, and chrono pattern syntaxes are not accepted by the new
  formatter crate.
- Unquoted `%` followed by an ASCII alphabetic character is an error to catch
  accidental d3/strftime usage.
- CLDR skeletons are not supported in v1.
- Timezone-name fields are unsupported until real timezone-name data is added.
- Gregorian calendar only for v1.

---

## 12. Test Plan

Add compatibility tests for the LDML formatter surface before adding broad
locale coverage:

- one golden test for every supported LDML field;
- field-width tests for numeric padding, abbreviated names, wide names, narrow
  names, and fractional seconds;
- apostrophe quoting and literal text tests;
- style-block tests for `{date:*}`, `{time:*}`, and `{datetime:*}`;
- explicit tests proving `{skeleton:...}` is rejected;
- locale-dependent field tests for months, weekdays, quarters, eras,
  day periods, date patterns, time patterns, and date-time glue;
- percent-guard tests proving d3/strftime-looking strings are rejected;
- timezone-context tests showing chart timezone formatting and UTC formatting
  produce the expected differences;
- timezone-precedence tests covering `#datefmt` override, scale option,
  `TimeContext`, and UTC default;
- invalid-timezone tests proving `"local"` is rejected by the new formatter;
- date-only tests proving `NaiveDate` formats through the naive-midnight path
  and never through timezone-aware conversion;
- naive-input tests proving timezone offset fields fail instead of silently
  treating the value as UTC or local time;
- zoned-input tests proving calendar fields and timezone fields use the display
  timezone while the input remains an absolute instant;
- custom-digit tests proving digit substitution applies only to generated
  numeric tokens;
- custom-locale tests showing partial serde overrides merge over `base`;
- fixture tests comparing representative outputs with CLDR/ICU outputs for the
  supported LDML subset and built-in locale allowlist.

When importing or porting external fixtures, preserve license attribution as
needed. The goal is LDML-based formatting semantics plus Avenger style blocks,
not d3-compatible formatting APIs.

---

## 13. Implementation Plan

- [ ] Create `avenger-format-datetime`.
- [ ] Decide whether shared locale infrastructure should be extracted now or
      after both number and datetime formatter crates exist.
- [ ] Implement the fallible LDML parser for the v1 field subset.
- [ ] Implement Avenger style-block parsing and expansion.
- [ ] Add the percent guard so d3/strftime-looking specs fail clearly.
- [ ] Define chrono-backed `NaiveDateTimeInput`, `ZonedDateTimeInput`, and the
      shared `DateTimeFields` lowering model.
- [ ] Implement scalar datetime formatting for every supported LDML field and
      field width.
- [ ] Add date-only lowering to naive midnight.
- [ ] Add input-kind validation so naive inputs reject timezone fields, timezone
      overrides, and any future timezone-only fields.
- [ ] Add `DateTimeFormatOverrides`, `DateTimeFormatContext`, and resolved-format
      merge rules.
- [ ] Integrate the formatter with existing chart `TimeContext` and temporal
      scale `timezone` options, preserving the documented precedence.
- [ ] Add `DateTimeLocaleSpec`, `ResolvedDateTimeLocale`, and locale registry
      support with base inheritance and validation.
- [ ] Validate custom locale LDML patterns at registration time.
- [ ] Add custom-locale digit substitution for generated numeric tokens.
- [ ] Add a minimal built-in locale set, starting with `en-US` and constrained
      by Latin-ext character coverage.
- [ ] Add offline CLDR extraction for the datetime locale data.
- [ ] Add CLDR-backed style patterns and date-time glue.
- [ ] Add custom locale registration from JSON/serde objects.
- [ ] Migrate current temporal formatter call sites to the new crate through
      compatibility shims.
- [ ] Add the Typst-label `#datefmt(value, spec, ...overrides)` adapter.
- [ ] Update temporal axis tick label strings to use Typst fragments with
      `#datefmt(value, "<LDML-or-style-spec>")`.
- [ ] Add temporal cascade formatting as a later axis-level feature.

---

## 14. Open Decisions

- Should the crate be named `avenger-format-datetime`,
  `avenger-format-temporal`, or part of a broader `avenger-format` family?
- Should shared locale registry mechanics live in a dedicated
  `avenger-format-locale` crate?
- Which exact Latin-ext-compatible built-in locale allowlist should ship first?
- Should LDML timezone-name fields become real localized names later, or should
  v1 keep rejecting them?
- Should mixed-style date-time blocks such as
  `{datetime:date=long,time=short}` be added after v1?
- Should locale week-year and week-number fields be added after v1, and should
  they use CLDR week data or only Avenger `TimeContext.week_start`?
- How much of the temporal cascade belongs in the formatter crate versus axis
  code?
