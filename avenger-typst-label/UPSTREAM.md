# Upstream Traceability

This crate is a lightweight Typst-label engine. It mirrors Typst concepts and
selected source structure, but it is not intended to be a drop-in fork of the
upstream crates. Use this document when porting fixes from `../typst`.

The adapted upstream sources retain Typst’s Apache 2.0 license; see
[LICENSE-APACHE](LICENSE-APACHE). The module map identifies the retained and
modified portions.

## Module Map

| Avenger module | Upstream source | Notes |
| --- | --- | --- |
| `src/typst_syntax/*` | `../typst/crates/typst-syntax/src/*` | Mostly copied parser, AST, syntax tree, spans, paths, and reparser support. Keep private. |
| `src/typst_timing/*` | `../typst/crates/typst-timing/src/*` | No-op shim for parser instrumentation hooks. |
| `src/typst_utils/*` | `../typst/crates/typst-utils/src/*` | Parser/support utilities retained for copied syntax code. Trim aggressively when unused. |
| `src/typst_library/font/*` | `../typst/crates/typst-library/src/text/font/*` | Retained font style/weight/spec concepts. `FontWeight` intentionally keeps public `Normal`/`Bold` variants. |
| `src/typst_library/text/*` | `../typst/crates/typst-library/src/text/*` and model text elements | Retained text style, text markup options, symbols, emoji, and line content nodes. |
| `src/typst_library/math/*` | `../typst/crates/typst-library/src/math/*` and `math/ir/*` | Retained math node model, function vocabulary, options, and symbols for label-scale math. |
| `src/typst_library/visualize.rs` | `../typst/crates/typst-library/src/visualize/*` | Solid color/stroke support only. Gradients/patterns are intentionally not kept. |
| `src/typst_eval/*` | `../typst/crates/typst-eval/src/*` | Static evaluator/lowerer for label markup, read-only params, text functions, and math expressions. No scripting or document evaluation. |
| `src/typst_realize/*` | Upstream realization between eval/library/layout | Flattens retained label content into renderable text/math runs. |
| `src/typst_layout/inline/*` | `../typst/crates/typst-layout/src/inline/*` | Single-line shaping, fallback, decoration placement, and text-frame construction. |
| `src/typst_layout/math/*` | `../typst/crates/typst-layout/src/math/*` | Compact math layout split into partials named after upstream areas. These are included into one Rust module to avoid a large visibility refactor. |
| `src/typst_layout/frame.rs` | `../typst/crates/typst-library/src/layout/frame.rs` and layout frame items | Label-scale frame/metrics/artifact structs. |
| `src/typst_layout/glyph_path.rs` | Upstream glyph outline conversion in SVG/PDF/render paths | Converts font outlines into retained path commands. |
| `src/typst_svg/*` | `../typst/crates/typst-svg/src/*` plus `typst-library` layout/visualize names | Vector artifact types consumed by Avenger renderers. Transform fields use upstream `sx/ky/kx/sy/tx/ty` names. |
| `src/typst_render/*` | `../typst/crates/typst-render/src/*` | Optional tiny-skia raster lowering for label path/image artifacts. |
| `src/label/*` | Avenger-owned facade | Public API, label errors/warnings, PDF metadata, font resources, params, limits, and renderer-facing lowerers. |

## Retained Math Layout Partials

The files under `src/typst_layout/math/` are intentionally named after upstream
math layout concerns:

- `run.rs`: public math fragment entry point, retained atom/glyph/shape structs,
  and row artifact assembly.
- `row.rs`: row construction, atom composition, and dispatch into math
  constructs.
- `decorate.rs`: math underline/overline and under/over ornaments.
- `fenced.rs`: groups, delimiters, `lr`, `mid`, and stretchy glyph assembly.
- `constructs.rs`: roots, radicals, accents, cancel, and fraction entry points.
- `stack.rs`: stacked, horizontal, and skewed fraction layout.
- `scripts.rs`: scripts, limits, primes, corner slots, and math kerns.
- `atom.rs`: atom classification and math text/style lowering.
- `spacing.rs`: math-class spacing.
- `style.rs`: retained math alphabet variant mapping.
- `font.rs`: math font loading, MATH constants, shaping, and accent attachment.
- `pdf.rs`: retained PDF glyph metadata for math.
- `svg.rs`: vector path artifact lowering.

Because these files are `include!` partials, private items are still in one Rust
module. This keeps the split traceable while avoiding visibility churn.

## Math Layout Function Map

Many retained math layout functions still use `layout_simple_*` names. That is
intentional for now: the useful traceability is the source mapping below, not a
large private rename diff.

| Avenger function(s) | Upstream source/function(s) | Notes |
| --- | --- | --- |
| `layout_simple_row`, `layout_simple_nodes_as_atom*` in `row.rs` | `../typst/crates/typst-layout/src/math/run.rs` (`layout_aligned_row`, `row_into_line_frame`) and `../typst/crates/typst-layout/src/math/mod.rs` (`layout_into_fragments`, `layout_into_fragment`) | Retained single-row dispatcher and row-to-frame assembly. Multiline/table alignment is intentionally not kept. |
| `layout_simple_node_with_mid_target` in `row.rs` | `../typst/crates/typst-layout/src/math/mod.rs` (`layout_realized`) plus the individual `typst-layout/src/math/*.rs` element layout functions | Compact dispatcher from retained `MathNode` to the supported construct layouts. |
| Math-class propagation and spacing in `row.rs`/`spacing.rs` | `../typst/crates/typst-library/src/math/ir/process.rs` (`spacing`) and `../typst/crates/typst-library/src/math/ir/item.rs` (`MathClass`, `MathItem::class`) | Avenger keeps the class-spacing subset needed for one-line labels, including explicit spacing suppressing automatic spacing. |
| `layout_simple_fraction`, `layout_simple_fraction_call`, `layout_simple_fraction_nodes` in `constructs.rs` | `../typst/crates/typst-layout/src/math/fraction.rs` (`layout_fraction`) | Vertical fractions mirror OpenType MATH numerator/denominator shifts and gaps. |
| `layout_simple_stack_nodes`, `layout_simple_no_rule_stack`, `layout_simple_horizontal_fraction_nodes`, `layout_simple_skewed_fraction_nodes` in `stack.rs` | `../typst/crates/typst-layout/src/math/fraction.rs` (`layout_fraction`, `layout_skewed_fraction`) | Retained stack, no-rule stack/binom, horizontal fraction, and skewed fraction formulas. |
| `layout_simple_sqrt`, `layout_simple_root`, `layout_simple_radical` in `constructs.rs` | `../typst/crates/typst-layout/src/math/radical.rs` (`layout_radical`) | Retains radical gap/thickness/degree placement formulas without upstream frame/style machinery. |
| `layout_simple_accent` in `constructs.rs` | `../typst/crates/typst-layout/src/math/accent.rs` (`layout_accent`) | Retains top/bottom accent placement, flattened accent behavior, and accent attachment points. |
| `layout_simple_cancel` in `constructs.rs` | `../typst/crates/typst-layout/src/math/cancel.rs` (`layout_cancel`, `draw_cancel_line`, `default_angle`) | Retains diagonal/cross cancel line geometry and literal stroke options. |
| `layout_simple_attach*`, `layout_simple_script_attach_parts`, `layout_simple_limit_attach_parts` in `scripts.rs` | `../typst/crates/typst-layout/src/math/scripts.rs` (`layout_scripts`, `layout_primes`, `layout_attachments`, script/limit shift helpers) and `../typst/crates/typst-library/src/math/attach.rs` (`Limits`) | Retains scripts, primes, corner slots, default display limits, forced `scripts(...)`, and forced `limits(...)`. |
| `layout_simple_group`, `layout_simple_delimited_*`, `layout_simple_lr_call` in `fenced.rs` | `../typst/crates/typst-layout/src/math/fenced.rs` (`layout_fenced`) and `../typst/crates/typst-layout/src/math/fragment/glyph.rs` (`stretch`) | Retains group/delimiter/lr layout and stretchy delimiter sizing for one-line labels. |
| `layout_simple_mid_call` in `decorate.rs` with helpers in `fenced.rs` | `../typst/crates/typst-layout/src/math/fenced.rs` and `../typst/crates/typst-library/src/math/lr.rs` | Retains `mid` as a stretchy middle delimiter between surrounding `lr` content. |
| `layout_simple_line_call`, `layout_simple_under_over_call` in `decorate.rs` | `../typst/crates/typst-layout/src/math/scripts.rs` for under/over attachment placement, plus retained decoration path construction | Retains math underline/overline and under/over ornaments as label path artifacts. |
| `layout_simple_operator_call`, `layout_simple_variant_call`, `layout_simple_stretch_call` in `decorate.rs` | `../typst/crates/typst-layout/src/math/text.rs`, `../typst/crates/typst-layout/src/math/shaping.rs`, and `../typst/crates/typst-layout/src/math/fragment/glyph.rs` | Retains operator text, math alphabet variants, and explicit stretch calls for supported axes. |
| `layout_math_text`, glyph shaping helpers, variant/assembly helpers in `font.rs` | `../typst/crates/typst-layout/src/math/text.rs` (`layout_text`, `layout_number`, `layout_glyph`), `../typst/crates/typst-layout/src/math/shaping.rs`, and `../typst/crates/typst-layout/src/math/fragment/glyph.rs` (`glyph_construction`, `assemble`, `stretch_axes`) | Retains direct math glyph shaping, glyph variants, and MATH assembly support needed for radicals, delimiters, accents, and large operators. |
| `pdf.rs` math glyph metadata helpers | Upstream PDF export path through Typst's frame/glyph metadata and `typst-pdf`/krilla integration | Avenger exposes a compact PDF text-layer artifact rather than upstream frames. |
| `svg.rs` math path artifact lowering | `../typst/crates/typst-svg/src/path.rs` and `../typst/crates/typst-svg/src/shape.rs` | Retains path commands, transform names, stroke cap/join/dash/miter, and rectangle winding relevant to SVG/PDF/raster output. |

## Parity Validation

Use Rust tests, not GitHub Actions wiring, to validate upstream parity for this
crate. The focused command is:

```bash
cargo test --release -p avenger-typst-label --features raster,upstream-png-parity --test upstream_png_parity -- --nocapture
```

The test harness is offline and reads checked-in PNG references from
`tests/fixtures/upstream_png/ref`. The reference generator is the only path that
requires `../typst`; fixture operation and failure artifacts are documented in
`tests/fixtures/upstream_png/README.md`.

## Porting Rules

- Start from upstream behavior and tests, then reduce to the label subset.
- Keep parser changes as close as possible to `typst-syntax`; add comments when
  diverging.
- Keep function vocabulary/options in `typst_library::{text,math}::call`.
- Keep syntax dispatch in `typst_eval`.
- Keep Avenger policy, limits, params, public errors, and output APIs in
  `label`.
- For PDF behavior, prefer matching upstream Typst's glyph metadata strategy,
  but expose only the compact `label::pdf` artifact model.
- Do not reintroduce the full evaluator, document model, page layout, package
  loading, or upstream renderer crates without a deliberate size/behavior
  decision.

## Deliberate Divergences

- `FontWeight` is public and ergonomic for Avenger themes: `Normal`, `Bold`, or
  `Number(u16)`. Upstream Typst stores weight as a numeric newtype. Convert at
  font-selection boundaries.
- `src/label` owns the public API and is intentionally not upstream-shaped.
- Math matrices, cases, vectors, multiline math, and general scripting are out
  of scope.
- Block display equations are out of scope. Explicit display-style math inside
  a single-line label, such as `$display(sum_(i=0)^n)$`, is retained and should
  follow upstream large-operator variant and limit-placement behavior.
- Text and math parameters are read-only external values, not Typst variables
  from `#let`.
- SVG/PDF/raster lowerers expose Avenger artifact structs instead of upstream
  frame/render APIs.

## September 2026 correctness audit

The fixes below follow upstream commit `c98e910391a8544b28bd5c99a6f3b1ac1ada9a84`.
The portable PNG corpus compares against upstream output. `tests/upstream_audit.rs`
checks geometry, shaping metadata, font instances, UTF-8 ranges, and frame order.

| Finding | Corrected behavior | Upstream source |
| --- | --- | --- |
| U01 | Align math rows by baseline and keep rules outside expression bounds. | `typst-layout/src/math/run.rs` |
| U02 | Stretch radicals with MATH variants and vertical assemblies. | `typst-layout/src/math/radical.rs` |
| U03 | Place nested/tall accents using base ascent, flattened and dotless glyphs. | `typst-layout/src/math/accent.rs` |
| U04 | Shape decorated text with fallback, script segmentation, and line-level bidi. | `typst-layout/src/inline/{prepare,shaping}.rs` |
| U05 | Keep nested decorations within their realized text ranges, including case expansion. | `typst-layout/src/rules.rs` |
| U06 | Shape quoted math text as text and preserve spaces between text fragments. | `typst-layout/src/math/text.rs` |
| U07 | Attach scripts relative to composite base bounds. | `typst-layout/src/math/scripts.rs` |
| U08 | Select absolute math size categories and stop at scriptscript size. | `typst-library/src/math/style.rs` |
| U09 | Propagate cramped style through roots, denominators, and top accents. | `typst-library/src/math/ir/resolve.rs` |
| U10 | Retain operator spacing in scripts and use each adjacent item's size. | `typst-library/src/math/ir/process.rs` |
| U11 | Require GSUB script-feature coverage for every character, including spaces. | `typst-layout/src/inline/shaping.rs` |
| U12 | Preserve small positive sizes and signed script offsets; use upstream metric defaults. | `typst-library/src/text/shift.rs` |
| U13 | Carry resolved variable-font coordinates through shaping, metrics, outlines, and export resources. | `typst-library/src/text/font/variations.rs` |
| U14 | Shape variation selectors with their base glyphs. | `typst-library/src/math/style.rs` |
| U15 | Stretch font ornaments for under/over braces and attach annotations. | `typst-library/src/math/ir/resolve.rs` |
| U16 | Preserve painter order across paths, glyphs, and bitmap images. | `typst-render/src/lib.rs` |
| U17 | Interpret named markup colors using the Typst palette. | `typst-library/src/visualize/color.rs` |
| U18 | Saturate strong weight arithmetic before clamping. | `typst-library/src/text/font/variant.rs` |

The SVG text adapter outlines variable-font runs so downstream SVG engines do
not reshape them at default axis coordinates. PDF font resources retain the same
coordinates and distinguish instances in the renderer's font cache. These tests
cover the retained single-line subset; they do not establish full Typst document
compatibility.
