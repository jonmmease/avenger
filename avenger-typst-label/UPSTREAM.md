# Upstream Traceability

This crate implements a subset of Typst behavior for single-line labels. It
combines copied syntax code, adapted layout algorithms, and Avenger-owned
implementations of evaluation, content, frames, and output artifacts. The
`typst_*` module names identify corresponding responsibilities in Typst. They
do not imply that each module is copied or can accept upstream patches directly.

Copied and adapted Typst sources retain Typst's Apache 2.0 license. See
[LICENSE-APACHE](LICENSE-APACHE).

## Reference Revisions

The source comparison and September 2026 behavior audit use Typst commit
[`c98e910391a8544b28bd5c99a6f3b1ac1ada9a84`](https://github.com/typst/typst/tree/c98e910391a8544b28bd5c99a6f3b1ac1ada9a84).
This is an audit baseline, not a verified import revision for every adapted file.
The operator follow-up below records a later fix from Typst 0.15.1.

Upstream paths beginning with `crates/` refer to that Typst repository.
Avenger paths beginning with `src/` are relative to this crate. The development
checkout used by the reference generator is `../typst` from the Avenger
repository root.

The files `src/typst_library/symbols/sym.txt` and
`src/typst_library/symbols/emoji.txt` contain the same bytes as
`src/modules/{sym,emoji}.txt` in the `codex` 0.3.0 crate used by the audit baseline.
The resolver in `src/typst_library/symbols.rs` is Avenger's implementation.

## Module Map

| Avenger module | Relationship | Upstream reference or Avenger scope |
| --- | --- | --- |
| `src/typst_syntax/*` | Mostly copied | `crates/typst-syntax/src/*`. Module paths and formatting differ. Package manifest parsing is removed. |
| `src/typst_timing/*` | Avenger shim | No-op replacements for parser hooks in `crates/typst-timing/src/*`. |
| `src/typst_utils/*` | Copied/adapted subset | Selected support code from `crates/typst-utils/src/*` for the copied parser. |
| `src/typst_library/font/*` | Avenger types | Font concepts from `crates/typst-library/src/text/font/*`. `MathFontSpec` and `MathFontBytesId` serve Avenger's font API. |
| `src/typst_library/foundations.rs` | Avenger types | Compact `Value` and `Scope` for external parameters. They replace the general values and scopes in `crates/typst-library/src/foundations/*`. |
| `src/typst_library/text/*` | Avenger content and option model | Supported behavior from `crates/typst-library/src/text/*` and `crates/typst-library/src/model/*`. |
| `src/typst_library/math/*` | Avenger syntax-oriented math model | Function vocabulary from `crates/typst-library/src/math/*`. `MathAst` and `MathNode` differ from the resolved `MathItem` representation in `crates/typst-library/src/math/ir/*`. |
| `src/typst_library/symbols.rs` and `symbols/*` | Copied data, Avenger resolver | `codex` 0.3.0 data and modifier behavior from `crates/typst-library/src/foundations/symbol.rs`. |
| `src/typst_library/visualize.rs` | Avenger color type | RGBA values only. Stroke/path types live in `src/typst_svg/mod.rs`. |
| `src/typst_eval/*` | Avenger static evaluator | Uses the copied AST to lower supported markup and math. Compare `crates/typst-eval/src/*` and `crates/typst-library/src/math/ir/resolve.rs` for behavior. |
| `src/typst_eval/format_cache.rs` | Avenger formatting integration | Caches D3 number and datetime formatters. There is no corresponding Typst module. |
| `src/typst_realize/*` | Avenger static realization | Flattens Avenger label content into runs. Shares the general role of `crates/typst-realize/src/*`, not its document realization machinery. |
| `src/typst_layout/inline/*` | Avenger single-line implementation | Shaping, fallback, bidi, and decoration behavior informed by `crates/typst-layout/src/inline/*` and `crates/typst-layout/src/rules.rs`. |
| `src/typst_layout/math/*` | Adapted algorithms and Avenger composition | Math layout formulas from `crates/typst-layout/src/math/*` and resolution rules from `crates/typst-library/src/math/ir/*`, implemented over Avenger types. |
| `src/typst_layout/frame.rs` and `line.rs` | Avenger layout artifacts and composition | Internal metrics, options, and positioned runs. These are not copies of Typst's `Frame`. |
| `src/typst_layout/glyph_path.rs` | Avenger outline adapter | Converts `ttf-parser` outlines into Avenger path commands. Compare `crates/typst-svg/src/path.rs` for outline conventions. |
| `src/typst_svg/*` | Avenger vector artifact types | Uses conventions from `crates/typst-library/src/layout/transform.rs` and `crates/typst-svg/src/path.rs`. Does not contain Typst's SVG serializer. |
| `src/typst_render/*` | Avenger raster implementation | Lowers label paths/images through `tiny-skia`. Compare `crates/typst-render/src/*` for rendering behavior. |
| `src/label/*` | Avenger public API | Label frames, errors, font resources, external parameters, limits, PDF metadata, and output conversion. |

## Math Layout Organization

The files under `src/typst_layout/math/` group Avenger's implementation by
concern. Their boundaries differ from upstream:

- `run.rs`: internal math entry point, atom/glyph/shape types, and artifacts.
- `row.rs`: row composition and construct dispatch.
- `decorate.rs`: lines, ornaments, operators, variants, explicit stretch, and `mid`.
- `fenced.rs`: groups, delimiters, `lr`, glyph variants, and glyph assembly.
- `constructs.rs`: roots, radicals, accents, cancel, and fraction entry points.
- `stack.rs`: stacked, horizontal, and skewed fractions.
- `scripts.rs`: scripts, limits, primes, corner slots, and math kerns.
- `atom.rs`: atom classification, operator text layout, and style dispatch.
- `spacing.rs`: math-class spacing.
- `style.rs`: math alphabet variant mapping.
- `font.rs`: math fonts, MATH constants, glyph/text shaping, and accent attachment.
- `pdf.rs`: Avenger PDF glyph metadata.
- `svg.rs`: Avenger vector path artifacts.

These files are `include!` partials in one private Rust module. The function map
below identifies comparison points for manual ports, not matching module APIs.

## Math Layout Function Map

The `layout_simple_*` functions implement the supported behavior over Avenger
nodes, font access, and metrics. Upstream functions use different content,
style, and frame types. A port may need changes in both evaluation and layout.

| Avenger function(s) | Upstream comparison point(s) | Notes |
| --- | --- | --- |
| `layout_simple_row`, `layout_simple_nodes_as_atom*` in `row.rs` | `crates/typst-layout/src/math/run.rs` (`layout_aligned_row`, `row_into_line_frame`) and `crates/typst-layout/src/math/mod.rs` (`layout_into_fragments`, `layout_into_fragment`) | Avenger single-row dispatcher and row-to-frame assembly. Multiline/table alignment is intentionally not kept. |
| `layout_simple_node_with_mid_target` in `row.rs` | `crates/typst-layout/src/math/mod.rs` (`layout_realized`) plus the individual `typst-layout/src/math/*.rs` element layout functions | Compact dispatcher from Avenger `MathNode` to the supported construct layouts. |
| Math-class propagation and spacing in `row.rs`/`spacing.rs` | `crates/typst-library/src/math/ir/process.rs` (`spacing`) and `crates/typst-library/src/math/ir/item.rs` (`MathItem::class`, `MathItem::lclass`, `MathItem::rclass`) | Avenger keeps the class-spacing subset needed for one-line labels, including explicit spacing suppressing automatic spacing. |
| `layout_simple_fraction`, `layout_simple_fraction_call`, `layout_simple_fraction_nodes` in `constructs.rs` | `crates/typst-layout/src/math/fraction.rs` (`layout_fraction`) | Vertical fractions mirror OpenType MATH numerator/denominator shifts and gaps. |
| `layout_simple_stack_nodes`, `layout_simple_no_rule_stack`, `layout_simple_horizontal_fraction_nodes`, `layout_simple_skewed_fraction_nodes` in `stack.rs` | `crates/typst-layout/src/math/fraction.rs` (`layout_fraction`, `layout_skewed_fraction`) | Retained stack, no-rule stack/binom, horizontal fraction, and skewed fraction formulas. |
| `layout_simple_sqrt`, `layout_simple_root`, `layout_simple_radical` in `constructs.rs` | `crates/typst-layout/src/math/radical.rs` (`layout_radical`) | Retains radical gap/thickness/degree placement formulas without upstream frame/style machinery. |
| `layout_simple_accent` in `constructs.rs` | `crates/typst-layout/src/math/accent.rs` (`layout_accent`) | Retains top/bottom accent placement, flattened accent behavior, and accent attachment points. |
| `layout_simple_cancel` in `constructs.rs` | `crates/typst-layout/src/math/cancel.rs` (`layout_cancel`, `draw_cancel_line`, `default_angle`) | Retains diagonal/cross cancel line geometry and literal stroke options. |
| `layout_simple_attach*`, `layout_simple_script_attach_parts`, `layout_simple_limit_attach_parts` in `scripts.rs` | `crates/typst-layout/src/math/scripts.rs` (`layout_scripts`, `layout_primes`, `layout_attachments`, script/limit shift helpers) and `crates/typst-library/src/math/attach.rs` (`Limits`) | Retains scripts, primes, corner slots, default display limits, forced `scripts(...)`, and forced `limits(...)`. |
| `layout_simple_group`, `layout_simple_delimited_*`, `layout_simple_lr_call` in `fenced.rs` | `crates/typst-layout/src/math/fenced.rs` (`layout_fenced`) and `crates/typst-layout/src/math/fragment/glyph.rs` (`stretch`) | Retains group/delimiter/lr layout and stretchy delimiter sizing for one-line labels. |
| `layout_simple_mid_call` in `decorate.rs` with helpers in `fenced.rs` | `crates/typst-layout/src/math/fenced.rs` and `crates/typst-library/src/math/lr.rs` | Retains `mid` as a stretchy middle delimiter between surrounding `lr` content. |
| `layout_simple_line_call` in `decorate.rs` | `crates/typst-layout/src/math/line.rs` (`layout_line`) | Adapts underline/overline gaps, thickness, and placement. |
| `layout_simple_under_over_call` in `decorate.rs` | `crates/typst-library/src/math/ir/resolve.rs` (`resolve_underoverspreader`), `crates/typst-layout/src/math/accent.rs` and `scripts.rs` | Builds a stretched accent and optional annotation using Avenger atoms. |
| `layout_simple_operator_call` in `decorate.rs`, `layout_operator_atom` in `atom.rs` | `crates/typst-library/src/math/ir/resolve.rs` (`resolve_op`) and `crates/typst-layout/src/math/text.rs` (`layout_text`) | Applies operator spacing after body layout. See the 0.15.1 follow-up below. |
| `layout_simple_variant_call`, `layout_simple_stretch_call` in `decorate.rs` | `crates/typst-library/src/math/ir/resolve.rs` (`resolve_symbol`, `resolve_stretch`), `crates/typst-layout/src/math/shaping.rs`, and `crates/typst-layout/src/math/fragment/glyph.rs` | Adapts math alphabet variants and explicit stretch calls for supported axes. |
| `layout_math_text` and glyph shaping helpers in `font.rs` | `crates/typst-layout/src/math/text.rs` (`layout_text`, `layout_number`, `layout_glyph`) and `crates/typst-layout/src/math/shaping.rs` | Avenger text/glyph shaping through `rustybuzz`, `fontdb`, and `ttf-parser`. |
| `stretch_single_glyph_variant`, `assemble_glyph_from_math_parts` in `fenced.rs` | `crates/typst-layout/src/math/fragment/glyph.rs` (`glyph_construction`, `assemble`, `stretch_axes`) | Adapts variant selection and OpenType MATH assembly. |
| `pdf.rs` math glyph metadata helpers | Upstream PDF export path through Typst's frame/glyph metadata and `typst-pdf`/krilla integration | Avenger exposes a compact PDF text-layer artifact rather than upstream frames. |
| `svg.rs` math path artifact lowering | `crates/typst-svg/src/path.rs` and `crates/typst-svg/src/shape.rs` | Avenger path conversion using corresponding outline, stroke, and winding conventions. |

## Parity Validation

From the Avenger repository root, run the geometry regressions and PNG
comparisons with:

```bash
cargo test --release -p avenger-typst-label --features raster,upstream-png-parity --locked --test upstream_audit --test upstream_png_parity
```

The PNG test is offline and reads checked-in references from
`tests/fixtures/upstream_png/ref`. CI invokes the Rust tests. Reference generation
explicitly invokes upstream Typst through Cargo or `TYPST_BIN`.
The generator currently checks for the sibling checkout even when `TYPST_BIN`
is set. It does not verify that the checkout or binary matches the recorded
revision. The separate size probe in `tools/upstream-typst-probe` also depends
on the sibling checkout. Fixture operation and failure artifacts are documented
in `tests/fixtures/upstream_png/README.md`.

The PNG comparator crops each image to ink bounds and applies per-case dimension
and similarity tolerances. It cannot validate the label's external baseline or
logical advance width. The geometry tests cover selected invariants and glyph
metadata. Neither suite establishes complete Typst compatibility. The audit SHA
is recorded for the `audit-*` references. The other references have upstream test
attribution but no separately recorded generating revision.

## Porting Rules

- Start from upstream behavior and tests, then reduce to the label subset.
- Keep parser changes as close as possible to `typst-syntax`. Document deliberate
  differences.
- Keep function vocabulary/options in `typst_library::{text,math}::call`.
- Keep syntax dispatch in `typst_eval`.
- Keep public parameter and output APIs in `label`. Parameter evaluation and
  D3 formatting are Avenger extensions implemented across `label`, `typst_eval`,
  `typst_realize`, and the formatting crates.
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
  from `#let`. `numfmt` and `datefmt` use Avenger's D3 formatting and localization
  rules rather than Typst formatting behavior.
- SVG/PDF/raster lowerers expose Avenger artifact structs instead of upstream
  frame/render APIs.

## September 2026 correctness audit

The fixes below address gaps in Avenger's supported behavior found by comparison
with the audit baseline. They are not a list of upstream commits cherry-picked
into this crate. The portable PNG corpus compares against upstream output. `tests/upstream_audit.rs`
checks geometry, shaping metadata, font instances, UTF-8 ranges, and frame order.

| Finding | Corrected behavior | Upstream source |
| --- | --- | --- |
| U01 | Align math rows by baseline and keep rules outside expression bounds. | `crates/typst-layout/src/math/run.rs` |
| U02 | Stretch radicals with MATH variants and vertical assemblies. | `crates/typst-layout/src/math/radical.rs` |
| U03 | Place nested/tall accents using base ascent, flattened and dotless glyphs. | `crates/typst-layout/src/math/accent.rs` |
| U04 | Shape decorated text with fallback, script segmentation, and line-level bidi. | `crates/typst-layout/src/inline/{prepare,shaping}.rs` |
| U05 | Keep nested decorations within their realized text ranges, including case expansion. | `crates/typst-layout/src/rules.rs` |
| U06 | Shape quoted math text as text and preserve spaces between text fragments. | `crates/typst-layout/src/math/text.rs` |
| U07 | Attach scripts relative to composite base bounds. | `crates/typst-layout/src/math/scripts.rs` |
| U08 | Select absolute math size categories and stop at scriptscript size. | `crates/typst-library/src/math/style.rs` |
| U09 | Propagate cramped style through roots, denominators, and top accents. | `crates/typst-library/src/math/ir/resolve.rs` |
| U10 | Retain operator spacing in scripts and use each adjacent item's size. | `crates/typst-library/src/math/ir/process.rs` |
| U11 | Require GSUB script-feature coverage for every character, including spaces. | `crates/typst-layout/src/inline/shaping.rs` |
| U12 | Preserve small positive sizes and signed script offsets; use upstream metric defaults. | `crates/typst-library/src/text/shift.rs` |
| U13 | Carry resolved variable-font coordinates through shaping, metrics, outlines, and export resources. | `crates/typst-library/src/text/font/variations.rs` |
| U14 | Shape variation selectors with their base glyphs. | `crates/typst-library/src/math/ir/resolve.rs` (`resolve_symbol`) and `crates/typst-layout/src/math/shaping.rs` |
| U15 | Stretch font ornaments for under/over braces and attach annotations. | `crates/typst-library/src/math/ir/resolve.rs` |
| U16 | Preserve painter order across paths, glyphs, and bitmap images. | `crates/typst-render/src/lib.rs` |
| U17 | Interpret named markup colors using the Typst palette. | `crates/typst-library/src/visualize/color.rs` |
| U18 | Saturate strong weight arithmetic before clamping. | `crates/typst-library/src/text/font/variant.rs` |

The SVG text adapter outlines variable-font runs so downstream SVG engines do
not reshape them at default axis coordinates. PDF font resources retain the same
coordinates and distinguish instances in the renderer's font cache. These tests
cover the retained single-line subset. They do not establish full Typst document
compatibility.

## Operator compatibility follow-up

Operator behavior also follows Typst's
[`math.op` correction](https://github.com/typst/typst/commit/d07469fe8c9c643a18c119c2a35a8dfde00af884)
in 0.15.1. Custom operators receive their spacing class after body layout, which
preserves glyph baselines, large-operator variants, and explicit stretching.
Predefined text operators use the same text shaping as quoted operator bodies.

The `custom_operators_*` tests in `tests/upstream_audit.rs` compare predefined and
custom operators, including script sizes and explicit attachment modes, and check
that wrapping a body in `op` preserves its geometry. The PNG corpus continues to
use the audit baseline recorded above.
