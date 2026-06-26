# Typst-Owned Text/Math Engine Goal Strategy

Date: 2026-06-25

## Goal

Move the exact Typst functionality Avenger needs into `avenger-typst`, then remove the dependency on the vendored Typst crates entirely.

The new constraint is ownership, not re-vendorability. We should stop preserving upstream crate boundaries and instead build a small Avenger-owned engine for:

- whole-line text measurement
- whole-line rasterization for WGPU
- positioned plain-text runs for SVG/PDF native text
- math-span paths for SVG/PDF
- future math glyph/PDF embedding metadata
- robust international text shaping, including emoji, bidi text, and complex scripts

Non-goals:

- full Typst documents
- user Typst evaluation
- Typst stdlib, imports, modules, regex values, datetime, image loading, colors by name, layout pages, blocks, headings, lists, tables, or PDF export
- matrix math support (`mat(...)`) in the first owned engine
- easy periodic upstream re-vendoring

## Principle

Do this as an extraction with a strong oracle, then as a simplification.

The current vendored backend is the oracle. The owned backend should initially match its metrics, positioned runs, paths, raster output, and error behavior for Avenger-supported inputs. Once that is true, we can delete the vendored crates and keep shrinking the owned internals.

This should feel more like compiler bootstrapping than a refactor: lock behavior, copy functionality, prove equivalence, then cut away scaffolding.

## API Contract To Preserve

The `avenger-text` integration should keep depending on:

```rust
AvengerTypst::typeset_text_line(source, &TextLineOptions)
```

Required outputs:

- `TextLineArtifact.metrics`
- optional full-line `raster`
- optional `positioned_runs`
- math-run `paths`
- future `pdf_text` / `font_resources` fields, even if not active by default

The owned engine may remove internal Typst concepts, but should preserve the public `avenger-typst` types unless there is a deliberate API cleanup.

## Feature Support Scope

This scope was reviewed against:

- <https://typst.app/docs/reference/math/>
- <https://typst.app/docs/reference/symbols/>
- <https://typst.app/docs/reference/text/>

The right target is "Typst-inspired inline chart labels", not "embedded Typst documents". We should support most of Typst's math expression surface because that is the user-facing value, but avoid features that require Typst's evaluator, document model, block layout, table/grid machinery, or general styling language.

### Core Owned Engine Target

Support these in the first serious owned engine target, either immediately or before removing the vendored backend:

- inline equations delimited by `$...$`
- mixed regular text and math in one line, with Typst-owned baseline alignment
- plain text runs outside math with robust shaping, font fallback, bidi, complex scripts, direct Unicode symbols, direct Unicode emoji, and Typst-style named emoji aliases
- math identifiers, numbers, operators, punctuation, quoted text, and grouping
- general math symbols by direct Unicode and Typst names such as `pi`, `alpha`, `dot`, `in`, `RR`, arrows, relations, set symbols, and common aliases
- Typst shorthand replacements in math where they are common and unambiguous, including arrows, relation shorthands, ellipsis, inequality shorthands, and assignment-like forms
- superscripts, subscripts, corner attachments, grouped primes, and limits/scripts placement
- fractions, including slash syntax and `frac(...)`; vertical fraction layout is mandatory, skewed/horizontal variants can be added after parity-critical cases
- roots: `sqrt(...)` and `root(...)`
- delimiter matching and scaling: parenthesized groups, `lr(...)`, escaped delimiters, `mid(...)`, `abs(...)`, `norm(...)`, `floor(...)`, `ceil(...)`, and `round(...)`
- accents for common math accents, including dot, double dot, hat, tilde, bar/macron, vector/arrow, and related arrow/harpoon accents
- binomials as stacked fraction-like layout without a rule plus delimiters
- text operators: built-in operators like `sin`, `cos`, `tan`, `log`, `ln`, `lim`, `max`, `min`, plus `op("custom")`
- math class spacing sufficient to match Typst for ordinary, relation, binary, unary, fence, opening, closing, punctuation, and large operators
- math style sizing for display, inline, script, script-script, and cramped variants because fractions/scripts/limits need these internally anyway
- math font styles and variants: upright, italic, bold, serif, sans, mono, blackboard, calligraphic, fraktur, and script where the math font or Unicode math alphabets support them
- glyph stretching/assembly for large delimiters, radicals, accents, arrows, and explicit `stretch(...)` once the same machinery exists
- cancel/cross-cancel as a simple overlay primitive; this is cheap once math boxes and strokes exist
- output metadata for future PDF glyph embedding: font identity, glyph id, advance, transform, source range, and text-equivalence where available

This gives chart authors the features they expect from "LaTeX-ish math fragments" while still keeping the implementation single-line and label-oriented.

### Stage 2 But Worth Supporting

These are useful, but should not block the owned-engine deletion path unless current baselines or real examples require them:

- `cases(...)` as a small math-only vertical stack with a delimiter, limited to label use; do not restore general grid/table layout for it
- `vec(...)` as a small column vector, also implemented as math-only vertical stacking
- under/over braces, brackets, parens, shells, and annotations; these need horizontal stretchy glyph assembly and annotation placement, so they belong after core math boxes are stable
- text underline, overline, and strike as rendering primitives on regular text spans, using font metrics for default offsets and thickness
- text subscript/superscript as regular-text span styling, preferring OpenType `subs`/`sups` features and falling back to synthesized size/baseline shifts
- text highlight as a simple rectangular background around a text span

Important distinction: supporting underline/strike/highlight as Avenger text span primitives is reasonable. Supporting Typst's evaluator, dynamic functions, show/set rules, or arbitrary code is not.

The preferred text-markup compromise is a static Typst-shaped subset. We can accept exact whitelisted forms like:

- `#underline[important]`
- `#strike[deprecated]`
- `#overline[label]`
- `#sub[n]`
- `#super[2]`
- `#highlight[warning]`
- `#emoji.face`
- `#emoji.chart.up`

Text commands should parse directly into Avenger text-span nodes. Emoji names should lower through a generated static table to Unicode emoji sequences before shaping. Neither path should call a Typst function, resolve a variable, evaluate a module, import from `emoji`, or accept arbitrary options in the first pass. Unsupported variants like `#underline(stroke: red)[important]`, `#let`, `#text(...)`, `#import emoji: face`, unknown `#name[...]` forms, and unknown `#emoji.name` forms should produce explicit unsupported-syntax errors.

### Explicitly Unsupported In The Owned Engine

Return clear parser errors for:

- matrix/table math: `mat(...)`, augmented matrices, arbitrary cell grids, and general table/grid layout
- multi-line equations, line breaks inside math, `&` alignment blocks, and block equation layout
- equation numbering, references, supplements, and document-level accessibility `alt` handling; Avenger can add accessibility metadata separately later
- arbitrary Typst code/eval: `#let`, `#set`, `#show`, `#import`, `#include`, `#eval`, `#range`, `#sym`, `#math`, and arbitrary hash expressions
- general Typst function calls outside the whitelisted math calls and whitelisted static text-span commands
- named arguments, argument spreading, and 2D semicolon argument lists except where a whitelisted function explicitly needs a small subset
- raw/code text, syntax highlighting, lorem, smart quotes, upper/lowercase transformations, and full paragraph/linebreak text layout
- document model features such as headings, lists, figures, links, footnotes, citations, bibliography, counters, state, introspection, pages, blocks, padding, columns, and layout containers

The unsupported list is as important as the supported list. It keeps the owned engine from regrowing the dependency families we are trying to remove.

## Correctness Harness First

Before major extraction, add a backend comparison test harness that runs both implementations:

- `VendorTypst` current backend
- `OwnedTypst` new backend

Compare these for representative inputs:

- metrics: width, height, baseline, ascent, descent
- positioned run count/order/kinds/source byte ranges
- plain run text and baseline offsets
- math path command counts and path bounds
- math path transforms/fill/stroke
- future PDF glyph metadata when requested
- raster dimensions and pixel difference tolerance
- error messages/categories for unsupported strict syntax

Use tolerances:

- metrics: exact or <= 0.01 px where floats differ
- paths: exact command topology if copied unchanged; otherwise bound/tolerance-based
- raster: image diff tolerance, not byte equality

Seed corpus:

- plain text only: `Hello`, `Axis Tick Spacing`, `Using count() aggregation`
- mixed literal dollar: `Price $7, score $R^2$ = 0.94`
- math only: `$x^2 + y^2$`, `$R^2$`, `$sqrt(x) / (1 + x^2)$`
- Bessel labels: `$J_0(x)$`, `$J_1(x)$`, `$J_2(x)$`
- fractions, roots, superscripts/subscripts, primes
- symbols and shorthands: `$alpha + beta -> gamma$`, `$x <= y => y >= x$`, `$x in RR$`
- operators and limits: `$sin(x)$`, `$lim_(x -> oo) f(x)$`, `$sum_(i=0)^n i$`
- delimiters: `$abs(x)$`, `$norm(v)$`, `$floor(x/2)$`, `$ceil(x/2)$`
- accents and variants: `$hat(x)$`, `$tilde(x)$`, `$arrow(v)$`, `$bb(R)$`, `$cal(P)$`, `$frak(g)$`
- cancellation and binomials: `$cancel(x)$`, `$binom(n, k)$`
- emoji and emoji sequences: `Revenue 🚀`, `Family 👨‍👩‍👧‍👦`, `Tone 👍🏽`, `Flag 🇯🇵`
- named emoji syntax: `Revenue #emoji.rocket`, `Trend #emoji.chart.up`, `Plain #emoji.face`
- bidi and mixed-direction text: `שלום world $x^2$`, `السعر $R^2$ = 0.94`
- complex scripts and shaping: `नमस्ते $x$`, `ภาษาไทย $x$`, `বাংলা $x$`
- CJK labels mixed with math: `温度 $T^2$`, `売上 $R^2$`
- text-decoration future fixtures once exposed: underline, strike, overline, text superscript/subscript, and highlight spans
- escaped dollars
- whitespace around math
- rotated-axis labels from visual baselines
- unsupported embedded Typst code, invalid math syntax, matrices, line breaks/alignment, and unsupported elements

Also keep visual baselines:

```bash
cargo test --release -p avenger-chart --features visual-tests,typst-text --test visual_regression typst_math -- --nocapture
```

The baseline reviewer instruction should remain: inspect each generated math baseline for correct math layout, line spacing, font weight, run order, legend/title/axis geometry, emoji rendering, and bidi/complex-script shaping.

## Target Module Shape

Create an owned implementation under `avenger-typst/src/owned/`:

```text
avenger-typst/src/owned/
  mod.rs
  engine.rs          # public-in-crate text line entrypoint
  syntax.rs          # strict math parser / source ranges
  ast.rs             # small Avenger math AST
  style.rs           # resolved text/math style state
  font.rs            # font loading, font book, font selection
  shape.rs           # rustybuzz shaping and glyph metrics
  inline.rs          # single-line text + math line builder
  math/
    mod.rs
    run.rs
    scripts.rs
    fraction.rs
    radical.rs
    fenced.rs
    shaping.rs
  frame.rs           # Avenger-owned frame/items/transforms
  path.rs            # frame -> MathPathArtifact
  raster.rs          # optional tiny-skia path rasterization
  pdf_glyphs.rs      # future glyph metadata extraction
```

The destination should not mirror `typst-library`, `typst-layout`, or `typst-syntax` modules long term. Those shapes are temporary scaffolding only.

## Phase 0: Freeze And Instrument

- [x] Add a vendor-vs-owned comparison harness; initially the owned backend delegates to the vendor backend.
- [x] Add fixtures for all supported math constructs and current visual baseline labels.
- [x] Add snapshot/debug output for:
  - metrics
  - positioned runs
  - path bounds
  - glyph ids and font names for future PDF embedding
- [x] Add a text-render size probe target that can build:
  - vendor backend
  - owned backend
  - cosmic backend
- [x] Keep release-mode commands as the default validation path.

Success criteria:

- We can add an owned backend and immediately see exactly where it diverges.
- The current vendored backend behavior is captured before deletions start.

## Phase 1: Add An Owned Backend Shell

- [x] Add `TypstEngineBackend::OwnedTypst` or a temporary internal feature-gated backend.
- [x] Route `AvengerTypst::typeset_text_line` to the owned shell when requested.
- [x] Keep the public `avenger-typst` API stable.
- [x] Make the owned shell call through to the vendored backend at first.

This gives us the backend switch, test harness, and feature wiring without behavior risk.

Success criteria:

- `OwnedTypst` exists as a selectable backend.
- It passes all current tests by delegation.
- No `avenger-text` changes are required.

## Phase 2: Copy The Runtime Core Into `avenger-typst`

Copy only the code needed by the current `avenger-typst/src/engine/typst.rs` imports. Start with a private compatibility core, even if it still looks Typst-ish:

Progress:

- [x] Move the owned backend shell under `avenger-typst/src/owned/`.
- [x] Split the owned backend feature from the vendored Typst oracle feature, so `OwnedTypst` can be constructed without compiling `typst-*` crates and consumers can opt into owned text without the vendor dependency path.
- [x] Lazily initialize the vendored delegate so owned fast paths can run without constructing a Typst `World`.
- [x] Add the first owned artifact path: empty text lines with metrics, optional empty paths, optional empty PDF text layer, and positioned-run output.
- [x] Copy enough runtime/layout code for a non-empty plain text line with metrics-only/positioned-run output, using embedded Atkinson face selection, `rustybuzz` shaping advances, and Typst-compatible cap-height/baseline vertical metrics.
- [x] Copy a first math runtime slice: metrics-only standalone math atoms using the owned math parser, default math font discovery, default math italic styling, math-script shaping advances, MATH italic correction, and Typst-compatible text-like cap-height vertical metrics.
- [x] Extend the owned math metrics slice to simple rows of standalone atoms, with ignored source spaces, variable-operator resolution, and Typst/TeX thin-medium-thick math class spacing for binary operators, relations, punctuation, and large operators.
- [x] Add owned path extraction for simple math rows by outlining shaped math glyphs with `ttf-parser`, preserving vendor-compatible path item topology for paths-only requests.
- [x] Add owned raster output for simple math rows behind the `raster` feature by feeding the owned path artifact through the existing tiny-skia rasterizer.
- [x] Add owned PDF glyph metadata for simple math rows, including semantic text, glyph IDs, advances, transforms, and embedded math font resource bytes.
- [x] Add owned post-superscript/subscript layout for simple math atoms, including OpenType `ssty` script glyph alternates, MATH script shifts, MATH kerning, paths, raster output, and PDF glyph metadata for labels such as `R^2`, `x_i^2`, and `J_0(x)`.
- [x] Add owned vertical slash-fraction layout for simple numerator/denominator atoms and grouped simple denominator rows, including script-style child sizing, suppressed script-style binary spacing, fraction rule path output, raster output, and PDF glyph metadata for labels such as `a / b` and `a / (b + c)`.
- [x] Add owned `frac(...)` call layout for simple numerator/denominator rows by sharing the vertical fraction machinery, with paths and PDF glyph metadata validated against the vendored oracle for labels such as `frac(x + y, z)`.
- [x] Add owned `binom(...)` call layout as a no-rule stacked fraction wrapped in stretched delimiters, with strict vendor lowering and oracle coverage for labels such as `binom(n, k)`.
- [x] Add owned `sqrt(...)` layout for simple radicand rows, including radical glyph styling, overbar shape ordering, nested script-script sizing, paths, raster output, and PDF glyph metadata for labels such as `sqrt(x) / (1 + x^2)`.
- [x] Add owned indexed `root(index, radicand)` layout for simple index and radicand rows, including script-script index sizing, radical index placement, paths, raster output, and PDF glyph metadata for labels such as `root(3, x)`.
- [x] Add owned visible parenthesized group layout for current core labels, including normal function-like groups such as `x(t)`, Typst-compatible identifier-subscript continuation such as `J_n(x)`, invisible script/fraction grouping where parentheses are only syntax, paths, raster output, and PDF glyph metadata.
- [x] Add owned simple delimiter helper calls `abs(...)`, `norm(...)`, `floor(...)`, `ceil(...)`, and `round(...)` for non-stretched single-line bodies, with strict vendor lowering and oracle coverage.
- [x] Add owned text-operator layout for `sin`, `cos`, `tan`, `log`, `ln`, `lim`, `max`, `min`, and `op("custom")`, including strict vendor lowering, operator identifier/call parsing, Typst-compatible word shaping/kerned advances, script attachment behavior, path output, and coalesced PDF glyph metadata for labels such as `sin(x)` and `lim_(x -> oo) f(x)`.
- [x] Add owned default `cancel(...)` overlay layout for one-argument calls, with strict vendor lowering, path/PDF output, and oracle coverage.
- [x] Add owned math variant/style call layout for one-argument `bold`, `upright`, `italic`, `serif`, `sans`, `cal`, `scr`, `frak`, `mono`, and `bb` calls over the current simple math subset, including strict vendor lowering, styled-content oracle traversal, path/PDF output, and oracle coverage for labels such as `bb(R)`, `cal(P)`, and `bold(alpha + 2)`.
- [x] Add owned simple top-accent layout for one-argument `hat`, `tilde`, `dot`, `ddot`, `bar`, and `arrow` calls over the current simple math subset, including MATH top-accent attachment placement, strict vendor lowering, path/PDF output, and oracle coverage for labels such as `hat(x)` and `arrow(v)`.
- [x] Add owned PDF glyph metadata for plain Atkinson-only text lines when paths/raster are not requested, including shaped glyph IDs, advances, transforms, font resource bytes, and vendor-oracle coverage for labels such as `Hello`, `Axis Tick Spacing`, and `Using count() aggregation`.
- [x] Add owned path extraction for plain Atkinson-only text lines by sharing the glyph-outline lowering helper with math, preserving vendor-compatible path topology for covered glyphs and delegating missing-glyph labels such as emoji until fallback segmentation is implemented.
- [x] Add owned raster output for plain Atkinson-only text lines behind the `raster` feature by feeding the owned path artifact through the existing tiny-skia rasterizer, while preserving non-raster-feature delegation behavior.
- [x] Extend the owned math fragment path from simple-row metrics/paths/raster/PDF metadata to scripts, fractions, roots, delimiters, grouped expressions, and text operators for the currently supported single-line subset.
- [x] Add owned mixed plain-text/math line metrics and positioned-run output for supported Atkinson + simple-math labels, including escaped-dollar source ranges and math positioned-run path artifacts, while delegating full-line paths/raster/PDF for now.
- [x] Extend owned mixed plain-text/math lines from metrics/positioned-run output to full-line paths, raster output, PDF text metadata, and font-resource merging.
- [x] Add initial owned `fontdb` face selection for non-Atkinson plain text, including path/PDF font-resource support while preserving Atkinson as the fast default face and current default missing-glyph behavior.
- [x] Add initial grapheme-based segmented fallback shaping for non-Atkinson plain text, including path/PDF lowering across multiple font resources and `rustybuzz` segment-property guessing.
- [x] Route default Atkinson plain text through segmented non-emoji fallback, so CJK/RTL/other text glyph misses can use owned fallback fonts while current color-emoji/tofu behavior remains isolated.
- [x] Preserve full Unicode grapheme clusters in owned plain-text glyph metadata for combining marks, emoji modifiers, and regional-indicator flags.
- [x] Add an initial Unicode bidi visual-run pass before owned plain-text fallback shaping, preserving logical source ranges while placing directional runs in visual order.
- [x] Add script-aware span boundaries before owned plain-text shaping, so mixed-script labels are shaped as separate Rustybuzz buffers even when one fallback face covers multiple scripts.
- [x] Extend the owned plain text line path from Atkinson-only shaping to fallback fonts, bidi, emoji, and complex-script segmentation, including owned path/PDF/raster artifact generation for missing-glyph RTL and ZWJ emoji labels without falling back to the vendored delegate.
- [x] Keep color emoji and static named emoji aliases on the owned path as Atkinson glyph-0/tofu output, matching the current vendored behavior until real color/ZWJ emoji rendering is implemented.

Needed families:

- diagnostics/source result types
- minimal engine/world/sink/route/traced structs
- style chain and style storage
- content/packed/native element machinery needed by current math/text lowering
- minimal introspection locations/tags for positioned run markers
- frame/item/abs/size/point/transform units
- text font/book/font instance/font list/text item/space/text elem
- math elements used by strict lowering
- solid color, fixed stroke, shape, curve/path geometry
- inline layout and math layout functions
- rustybuzz shaping support

Do not copy:

- document/page layout
- loading/image support
- Typst eval
- full stdlib/global module
- model/list/table/heading/figure/document elements
- PDF export
- color maps/gradients/tilings
- regex/datetime/decimal/lorem/numbering utilities

At this phase, some copied code may still use names like `Content`, `Packed`, `StyleChain`, and `Frame`. That is acceptable as a temporary bridge.

Success criteria:

- Owned backend no longer calls vendored `typst-library`, `typst-layout`, or `typst-syntax` for at least one simple case.
- Vendor-vs-owned comparison passes for plain text and simple math.

## Phase 3: Eliminate Typst Proc Macro Dependence

The vendored code uses Typst macros for elements, style access, native element metadata, and timing. In owned code, these are not worth preserving.

Strategy:

- [ ] Replace `#[elem]`-generated element types with explicit structs/enums for only the elements Avenger lowers.
- [ ] Replace generic `NativeElement` dispatch with a closed `OwnedContent` enum.
- [ ] Replace `Packed<T>` with either:
  - direct enum variants carrying span/style data, or
  - small typed structs where static dispatch is simpler.
- [ ] Replace `ShowSet` with explicit style application in the Avenger lowering code.
- [ ] Remove `typst-macros` and `typst-timing`.

Preferred model:

```rust
enum OwnedContent {
    Sequence(Vec<OwnedContent>),
    Text(TextContent),
    Space(SpaceContent),
    Math(MathContent),
    Tag(TagContent),
}

enum MathContent {
    Symbol(SymbolContent),
    Text(TextContent),
    Fraction { numerator: Box<OwnedContent>, denominator: Box<OwnedContent> },
    Root { index: Option<Box<OwnedContent>>, radicand: Box<OwnedContent> },
    Attach { base: Box<OwnedContent>, top: Option<Box<OwnedContent>>, bottom: Option<Box<OwnedContent>> },
    Fenced { body: Box<OwnedContent>, left: Option<char>, right: Option<char> },
    AlignPoint,
    Primes(String),
}
```

Success criteria:

- `avenger-typst` no longer depends on `typst-macros`.
- The owned code is statically typed around Avenger-supported content, not dynamically typed around Typst's full element system.

## Phase 4: Replace Typst Syntax With An Avenger Strict Parser

Once layout is owned and proven, replace `typst-syntax` with a small parser for the strict supported subset.

Progress:

- [x] Add an initial owned line AST and static Typst-shaped parser for plain text, `$...$` math spans, exact text commands, nested static text spans, and named emoji aliases.
- [x] Route the owned parser into the owned backend for non-empty text lines.
- [x] Render named emoji aliases from the owned AST by lowering them to plain text before owned line layout.
- [x] Render static underline/strike/overline/highlight text commands from the owned AST as native text plus decoration/highlight path shapes.
- [x] Render static sub/super text commands from the owned AST once positioned runs can represent per-run font size, baseline shift, and style metadata.
- [x] Add an initial owned math AST/parser for the current core label corpus: identifiers, numbers, symbols, shorthands, groups, scripts, primes, slash fractions, whitelisted calls, string arguments, and explicit matrix rejection.
- [x] Extend the owned math parser symbol subset with common Typst-style set, relation, arithmetic, quantifier, and dotted arrow symbol names, while keeping unknown dotted names out of the symbol fast path.
- [x] Route API-level strict math validation through the owned parser when the owned feature is available, so public validation no longer depends on `typst-syntax` and matrix syntax is rejected before backend dispatch.
- [ ] Add the owned math parser for the strict supported subset.

Keep:

- Typst-like math syntax accepted by current Avenger examples
- exact whitelisted Typst-shaped text commands outside math, lowered directly to Avenger span nodes
- source byte ranges
- useful invalid syntax errors
- escaped dollar handling remains in Avenger delimiter parsing, not the math parser

Line parser target:

- plain text runs
- escaped dollars and escaped hash signs
- math spans delimited by `$...$`
- static text span commands: `#underline[...]`, `#strike[...]`, `#overline[...]`, `#sub[...]`, `#super[...]`, and `#highlight[...]`
- static named emoji aliases: `#emoji.<name>` with dotted Typst emoji names resolved through a generated table
- nested static text spans if they are cheap to support and preserve clear source ranges
- explicit errors for unknown `#...` commands, unsupported options, missing brackets, and unterminated spans

Math parser target:

- identifiers
- numbers
- operators
- groups
- superscripts/subscripts
- primes and grouped primes
- fractions
- roots
- parenthesized/fenced groups
- delimiter helper functions
- symbols and common shorthands
- accents
- binomials
- text operators
- math variants and style functions
- cancel/cross-cancel
- function-like math constructs Avenger supports today

Explicit parser errors:

- matrix/table math (`mat(...)`) is unsupported in the first owned engine
- embedded Typst code remains unsupported
- multi-line equations and alignment points are unsupported
- arbitrary Typst function calls and named/spread arguments are unsupported unless whitelisted
- outside-math `#...` syntax is unsupported unless it is one of the exact static text-span commands

Use current `lower_math_source` behavior as the compatibility target.

Success criteria:

- `avenger-typst` no longer depends on `typst-syntax` or `typst-utils` for parsing.
- Strict syntax tests still pass.
- Differential corpus matches current vendor output for supported syntax.

## Phase 5: Collapse Styles And World

With the parser and content model owned, remove the remaining compiler-world abstractions:

- [ ] Delete `World`, `Library`, `Engine`, `Routines`, `Route`, `Sink`, `Traced` equivalents unless one is still doing real work.
- [ ] Replace style chains with a compact resolved style struct:

```rust
struct ResolvedTextStyle {
    family: String,
    size: f32,
    weight: FontWeight,
    style: FontStyle,
    fill: Color,
    line_height: Option<f32>,
}

struct ResolvedMathStyle {
    family: String,
    size: f32,
    fill: Color,
    display: MathDisplayStyle,
}
```

- [ ] Make line layout receive resolved styles directly.
- [ ] Make math layout receive a small `MathLayoutContext` directly.

Success criteria:

- There is no `LibraryBuilder`, no global module, no std module, no built-in rules map, and no Typst-style evaluation context in `avenger-typst`.

## Phase 6: Dependency Deletion Pass

Remove dependencies as their owning abstractions disappear.

Expected removals:

- `typst-library`
- `typst-layout`
- `typst-syntax`
- `typst-utils`
- `typst-macros`
- `typst-timing`
- `regex`, `regex-automata`, `regex-syntax`, `aho-corasick`
- `time`
- `roxmltree`, `xmlwriter`
- `flate2`, `png`, `kamadak-exif`, `moxcms` from the Typst path
- `lipsum`
- `rust_decimal`
- `codex`, `chinese-number`
- `hypher` if we do not hyphenate chart labels

Likely keep:

- `rustybuzz`
- `ttf-parser`
- `fontdb` or an Avenger font registry equivalent
- `unicode-math-class`
- `unicode-script`, bidi, grapheme/word segmentation, variation-selector, and emoji/font-fallback support required for international labels
- ICU segmentation/properties/provider data as needed for robust complex-script and emoji handling
- `tiny-skia` behind raster feature
- `kurbo`/`lyon` only if still used by owned path conversion
- `serde` optional for public API

Success criteria:

- `cargo tree -p avenger-typst --features ...` has no `typst-*` crates.
- `text-render-probe` Typst mode is materially closer to cosmic, with a written size report.

## Phase 7: Simplify Layout To Avenger's Actual Needs

Once we own the code and have deleted dependency families, optimize the layout model around labels:

- single line only
- no page/block layout
- no line wrapping
- no hyphenation
- no embedded images
- no user-defined show rules
- no Typst `#` code
- no full document measurement
- no matrix/table math support

Preserve:

- bidi text
- combining marks
- variation selectors
- emoji, emoji ZWJ sequences, regional-indicator flags, skin-tone modifiers, and color font fallback
- complex scripts and script-specific shaping
- font fallback
- math axis / italic correction / glyph assembly
- baseline alignment between plain text and math
- line-height breathing room

This phase can be incremental, but robust international shaping is a requirement. Do not remove bidi, segmentation, script, emoji, or font-fallback machinery unless the owned engine has an equivalent replacement and fixtures prove it.

## Phase 8: Remove The Vendored Crates

Only after the owned backend is complete:

- [ ] Remove `vendor-typst` feature from `avenger-typst`.
- [ ] Remove vendored Typst crate dependencies from `avenger-typst/Cargo.toml`.
- [ ] Remove workspace references if no other crate needs them.
- [ ] Delete or archive `vendor/typst-avenger` as appropriate.
- [ ] Update docs and feature names so users see `typst-text`/owned Typst behavior, not a vendor implementation detail.

Success criteria:

- Full build/test path passes without vendored Typst crates.
- SVG/PDF still emit native text for plain spans and paths for math spans.
- WGPU raster still renders math labels correctly.
- Visual baselines are reviewed and accepted.

## Recommended Milestones

### Milestone A: Proved Owned Shell

- backend switch exists
- comparison harness exists
- owned backend delegates to vendor
- all tests pass

### Milestone B: Owned Layout For Core Cases

- plain text and simple math are laid out by owned copied code
- comparison harness passes for core corpus
- vendored backend still available as oracle

### Milestone C: No Typst Proc Macros

- owned content model is closed and explicit
- `typst-macros` removed
- strict subset still matches vendor

### Milestone D: No Typst Parser

- small Avenger parser replaces `typst-syntax`
- parser tests cover current examples and invalid syntax

### Milestone E: No Vendored Typst Crates

- `avenger-typst` depends on no `typst-*` crates
- text-render probe remeasured
- chart visual baselines reviewed

## Validation Commands

Core:

```bash
cargo test --release -p avenger-typst --features raster
cargo test --release -p avenger-text --features typst-text-raster
cargo test --release -p avenger-svg --features typst-text
cargo test --release -p avenger-pdf --features typst-text
```

Chart visuals:

```bash
cargo test --release -p avenger-chart --features visual-tests,typst-text --test visual_regression typst_math -- --nocapture
```

Size:

```bash
tools/text-render-probe/measure.sh
cargo bloat --release -p text-render-probe --bin text-render-probe --no-default-features --features typst --crates -n 0
cargo tree -p text-render-probe --no-default-features --features typst --duplicates --target all
```

## Decision Points

### Matrix Support

We do not need `mat(...)` for default chart-label math. Drop it early. Matrix support keeps table/grid-style math layout alive, and table/grid machinery is exactly the sort of dependency we are trying to remove.

Recommendation: keep a clear parser error for matrices in the first owned-minimal engine. If a future user case requires matrices, implement a small math-only matrix layout as a separate feature instead of restoring Typst's general grid machinery.

### Emoji, Bidi, And Complex Script Text

International labels are important. The owned engine must support robust shaping for emoji, bidi text, combining marks, variation selectors, CJK, Indic scripts, Arabic shaping, Thai, and other complex scripts that chart authors can reasonably use in labels.

Recommendation: keep robust shaping via `rustybuzz` plus the Unicode/ICU/font-fallback support needed to match current behavior. Do not treat bidi, segmentation, script detection, or emoji support as optional size wins.

### Font Bundling

The current Atkinson payload is about 700 KiB per embedded copy. It is not the biggest problem, but duplication should go.

Recommendation: use one shared embedded font registry and let the owned Typst engine consume the same font bytes as the rest of `avenger-text`.

### Rasterization

`tiny-skia` is currently the pragmatic raster path for WGPU. It is not the largest cost.

Recommendation: keep it optional, but do not block the owned-engine extraction on replacing it.

## How To Work Safely

Commit at each milestone:

1. test harness / no behavior change
2. owned backend shell / delegation
3. first copied owned core / simple cases
4. proc macro removal
5. parser replacement
6. dependency deletion
7. final vendor removal

Every commit should compile in release mode and include either:

- a behavior-equivalence test improvement, or
- a dependency/size reduction with measured output.

Do not delete the vendored backend until the owned backend has passed comparison tests across the corpus and chart baselines.

## Summary

The cleanest strategy is not to fork Typst smaller. It is to use the current vendored implementation as a temporary oracle, extract the exact Avenger text/math runtime into `avenger-typst`, then collapse it into an Avenger-native line layout and math layout engine.

The major technical shift is replacing Typst's open-ended `Content`/`Packed`/style/rule/eval framework with a closed Avenger-owned AST and frame model. That is where the size drops become dramatic, because it lets us remove the Typst crates and the dependency families they keep alive.
