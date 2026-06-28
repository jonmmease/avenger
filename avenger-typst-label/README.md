# avenger-typst-label

`avenger-typst-label` is a lightweight adaptation of Typst for making labels. It
implements the subset of Typst behavior needed for compact single-line labels,
including regular text, inline math, emoji, simple text markup, SVG/PDF-friendly
metadata, and raster output.

The implementation is owned by this crate, but the syntax and layout behavior
should stay Typst-shaped. The parser front end and small parser support modules
are currently copied from upstream Typst into private `src/syntax`, `src/timing`,
and `src/utils` modules. Those copied modules are intentionally treated as an
intermediate extraction point: this crate lowers their AST into a compact label
representation and does not expose Typst parser types in the public API.

## Kept Functionality

The crate keeps the parts of Typst that are useful for compact labels:

- Single-line text layout with font fallback, shaping, bidi support, script
  segmentation, emoji, and explicit font weight/style selection.
- `$...$` math spans inside text lines, using Typst syntax and escaping rules for
  literal dollar signs and other markup characters.
- Strict Typst-like math fragments for identifiers, numbers, operators,
  grouping, shorthand symbols, string literals, scripts, primes, fractions,
  roots, binomial-style calls, accents, cancellation, common functions, and
  operator-sized constructs such as sums.
- A small text-markup subset outside math, currently including bracketed
  commands such as underline/strike-like spans and named emoji aliases such as
  `#emoji.face`.
- OpenType MATH-table based math positioning where available, including math
  constants, glyph variants, italic correction, top accent attachment, and
  script-style shaping.
- Full-line metrics with width, height, baseline, ascent, and descent.
- Optional positioned run output so SVG/PDF can emit native `<text>` for plain
  text and paths or future PDF glyph metadata for math runs.
- Path output for vector renderers.
- Optional raster output through the `raster` feature and `tiny-skia`.
- Optional PDF text-layer metadata for future direct PDF math embedding.

The crate intentionally excludes full Typst document features:

- No Typst evaluator, `#let`, imports, dynamic code execution, content blocks, or
  package loading.
- No public Typst `SyntaxNode`, content, element, style-chain, or frame tree.
- No page layout, paragraphs, wrapping, justification, tables, matrices, or
  multiline math.
- No general Typst SVG/PDF/render backends.
- No compatibility promise for unsupported Typst syntax beyond returning clear
  errors.

## Public API Shape

The current top-level entry point is `AvengerTypst`; future API cleanup should
rename it to a Typst-label-oriented name.

- `typeset_math_fragment` accepts one math fragment without delimiters and
  returns metrics plus requested path, raster, and PDF metadata artifacts.
- `typeset_math_string` splits a string into text and math runs. The intended
  long-term behavior is canonical Typst `$...$` syntax rather than
  crate-specific delimiter policy.
- `typeset_text_line` lays out a complete single line containing plain text,
  static text markup, emoji aliases, and math spans. This is the primary API used
  by Avenger renderers and text measurement.

Output requests are explicit. Callers can ask only for metrics, or can also
request positioned runs, paths, raster images, and PDF text metadata.

## Relationship To Upstream Typst

`avenger-typst-label` should be treated as a behavioral subset of Typst, not as a
source-level fork. The closest upstream source areas are:

| Module | Typst source area | Relationship |
| --- | --- | --- |
| `src/syntax/*` | `crates/typst-syntax/src/*` | Private copied parser/AST module, pending trim to the single-line label subset |
| `src/timing/*` | `crates/typst-timing/src/*` | Private copied timing support used by parser macros; expected to shrink to no-op or minimal hooks |
| `src/utils/*` | `crates/typst-utils/src/*` | Private copied parser support utilities; expected to shrink to only parser-required helpers |
| `src/delimiter.rs` | Typst markup/math delimiter behavior | Temporary compatibility scanner; should be replaced by canonical Typst parsing/escaping |
| `src/engine/syntax.rs` | `crates/typst-syntax/src/parser.rs` markup/code parsing | Lowers copied Typst parser AST into the label text-markup IR; no Typst evaluator |
| `src/engine/math/syntax.rs` | `crates/typst-syntax/src/parser.rs` math parsing | Lowers copied Typst parser AST into the supported label math fragment IR |
| `src/engine/math/ast.rs` | `crates/typst-syntax/src/ast.rs` and `typst-library/src/math/ir` | Compact label math AST |
| `src/engine/math/metrics.rs` | `typst-library/src/math/ir/*` and `typst-layout/src/math/*` | Consolidated subset of Typst math layout behavior |
| `src/engine/font.rs` | `typst-layout/src/inline/shaping.rs` and Typst text/font modules | Label font fallback and shaping pipeline |
| `src/engine/inline.rs` | `typst-layout/src/inline/*` | Single-line inline layout for labels |
| `src/engine/glyph_path.rs` | `typst-svg`, `typst-render`, and glyph outline helpers | Glyph outline lowering |
| `src/paths.rs` | Typst frame/path export concepts | Vector artifact model |
| `src/raster.rs` | `typst-render` | `tiny-skia` path rasterization |
| `src/pdf.rs` | `typst-pdf` text/glyph embedding concepts | PDF glyph metadata; not Typst's PDF backend |

Most layout file names are still inherited from the initial extraction rather
than preserved Typst names. The copied parser/support modules are the exception.
The intended direction is to reorganize this crate into Typst-shaped modules
that still stay focused on single-line labels.

## Extraction Process

The current crate came from a staged extraction:

1. Study the upstream Typst crates to identify the minimum pieces needed for
   labels: math parsing, math layout, font shaping, glyph outlines,
   metrics, rasterization, and SVG/PDF-friendly artifacts.
2. Prototype a vendored path by copying relevant Typst code and patching it
   enough to compile independently.
3. Remove the evaluator and document model, keeping only static text and math
   fragments.
4. Replace Typst's general syntax, content, style-chain, frame, render, SVG, and
   PDF machinery with compact label-oriented structs and APIs.
5. Collapse the remaining backend abstraction after the label engine was
   sufficient.
6. Add release-mode unit and visual tests to lock down the supported subset.

This means future traceability should rely on both source notes for the copied
parser modules and behavior tests for the label layout engine. When changing
math behavior, compare against upstream Typst for representative supported
examples, but keep the implementation small and local to this crate.

## Traceability Guidelines

When adding or changing functionality:

- Prefer naming tests after the supported Typst syntax or layout behavior.
- Add comments only where an algorithm depends on a specific Typst/OpenType MATH
  convention.
- If a new feature corresponds closely to Typst source, mention the upstream file
  in the module or test comment.
- Keep unsupported Typst syntax strict: accept the subset intentionally, and
  return `MathTypesetError::UnsupportedSyntax` for unsupported constructs.
- Avoid reintroducing Typst evaluator, document, page-layout, or renderer
  dependencies unless label rendering explicitly needs them.
