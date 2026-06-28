# Upstream Traceability

This crate is a lightweight Typst-label engine. It mirrors Typst concepts and
selected source structure, but it is not intended to be a drop-in fork of the
upstream crates. Use this document when porting fixes from `../typst`.

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
- Text and math parameters are read-only external values, not Typst variables
  from `#let`.
- SVG/PDF/raster lowerers expose Avenger artifact structs instead of upstream
  frame/render APIs.
