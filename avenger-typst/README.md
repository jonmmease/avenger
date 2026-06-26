# avenger-typst

`avenger-typst` is Avenger's standalone Typst-style text and math typesetting
engine. It implements the subset of Typst behavior needed by Avenger chart
labels, titles, legends, guides, SVG/PDF export, and raster/WGPU text atlases.

This crate is **not** a mechanically vendored or trimmed copy of Typst. It began
from an investigation and vendoring experiment against the upstream Typst
project, but the current code is an Avenger-owned implementation that keeps a
small, tested subset of Typst's text/math semantics.

## Kept Functionality

The crate keeps the parts of Typst that are useful for compact chart text:

- Single-line text layout with font fallback, shaping, bidi support, script
  segmentation, emoji, and explicit font weight/style selection.
- `$...$` math spans inside text lines, with escaped dollars treated as literal
  text.
- Strict Typst-like math fragments for identifiers, numbers, operators,
  grouping, shorthand symbols, string literals, scripts, primes, fractions,
  roots, binomial-style calls, accents, cancellation, common functions, and
  operator-sized constructs such as sums.
- A small static text markup subset outside math, currently including bracketed
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
- No full Typst `SyntaxNode`, content, element, style-chain, or frame tree.
- No page layout, paragraphs, wrapping, justification, tables, matrices, or
  multiline math.
- No general Typst SVG/PDF/render backends.
- No compatibility promise for unsupported Typst syntax beyond returning clear
  errors.

## Public API Shape

The top-level entry point is `AvengerTypst`.

- `typeset_math_fragment` accepts one math fragment without delimiters and
  returns metrics plus requested path, raster, and PDF metadata artifacts.
- `typeset_math_string` splits a string into plain and math runs using configured
  delimiters.
- `typeset_text_line` lays out a complete single line containing plain text,
  static text markup, emoji aliases, and math spans. This is the primary API used
  by Avenger renderers and text measurement.

Output requests are explicit. Callers can ask only for metrics, or can also
request positioned runs, paths, raster images, and PDF text metadata.

## Relationship To Upstream Typst

`avenger-typst` should be treated as a behavioral subset of Typst, not as a
source-level fork. The closest upstream source areas are:

| Avenger module | Typst source area | Relationship |
| --- | --- | --- |
| `src/delimiter.rs` | Typst markup/math delimiter behavior | Avenger-specific delimiter scanner for chart labels |
| `src/engine/syntax.rs` | `typst-syntax` markup/code parsing | Small static text-markup parser; no Typst evaluator |
| `src/engine/math/syntax.rs` | `crates/typst-syntax/src/parser.rs` math parsing | Hand-written parser for the supported fragment subset |
| `src/engine/math/ast.rs` | `typst-syntax` math AST and `typst-library/src/math/ir` | Avenger-owned compact AST |
| `src/engine/math/metrics.rs` | `typst-library/src/math/ir/*` and `typst-layout/src/math/*` | Consolidated subset of Typst math layout behavior |
| `src/engine/font.rs` | `typst-layout/src/inline/shaping.rs` and Typst text/font modules | Avenger-owned font fallback and shaping pipeline |
| `src/engine/inline.rs` | `typst-layout/src/inline/*` | Single-line inline layout for Avenger labels |
| `src/engine/glyph_path.rs` | `typst-svg`, `typst-render`, and glyph outline helpers | Avenger-owned glyph outline lowering |
| `src/paths.rs` | Typst frame/path export concepts | Avenger artifact model |
| `src/raster.rs` | `typst-render` | Avenger `tiny-skia` path rasterization |
| `src/pdf.rs` | `typst-pdf` text/glyph embedding concepts | Metadata for Avenger's PDF path; not Typst's PDF backend |

Most file names are therefore Avenger names, not preserved Typst names. The
largest difference is that Typst separates parsing, evaluation, math IR,
frame-based layout, rendering, SVG, and PDF export into separate crates, while
`avenger-typst` folds the small supported path into one crate with Avenger
artifact types.

## Extraction Process

The current crate came from a staged extraction:

1. Study the upstream Typst crates to identify the minimum pieces needed for
   chart labels: math parsing, math layout, font shaping, glyph outlines,
   metrics, rasterization, and SVG/PDF-friendly artifacts.
2. Prototype a vendored path by copying relevant Typst code and patching it
   enough to compile independently.
3. Remove the evaluator and document model, keeping only static text and math
   fragments.
4. Replace Typst's general syntax, content, style-chain, frame, render, SVG, and
   PDF machinery with Avenger-owned structs and APIs.
5. Collapse the remaining backend abstraction after the Avenger-owned engine was
   sufficient.
6. Add release-mode unit and visual tests to lock down the subset Avenger uses.

This means future traceability should rely on behavior and tests, not mechanical
re-vendoring. When changing math behavior, compare against upstream Typst for
representative supported examples, but keep the implementation small and local
to Avenger.

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
  dependencies unless Avenger explicitly needs them.

