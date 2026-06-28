# Typst Math Typesetting

Date: 2026-06-24
Last updated: 2026-06-26

Status: partially implemented in the active text-engine branch; vendor-route
research superseded by the `avenger-typst-label` text engine.

Current direction: `avenger-typst-label` owns the strict Typst-style text/math subset
directly. `avenger-text` should always use that engine for regular text
and `$...$` math fragments; the old cosmic-text and HTML canvas text backends
are no longer the target architecture. The active path should not depend on
`vendor-typst`, `typst-library`, `typst-layout`, or `typst-syntax`. Sections
below that discuss vendoring or optional Typst features are retained as
historical research, not as the implementation plan.

Source notes: this document condenses the scratch research in
`scratch/typst-math-typesetting-analysis.md` and
`scratch/typst-avenger-text-integration-plan.md`. The local Typst checkout
studied was `../typst` at commit
`c98e910391a8544b28bd5c99a6f3b1ac1ada9a84`, workspace version `0.15.0`.

## Goal

Use the Avenger Typst-style text engine for Avenger labels, titles, axis labels,
legend labels, and similar text-bearing chart surfaces, including Typst-style
math fragments.

The target user-facing shape is a string that may contain math spans such as:

```text
$x^2 + y^2$
```

Text outside math spans should be shaped, measured, rasterized, and exported by
the same `avenger-typst-label` line engine as text inside math spans. This keeps
mixed strings such as `Price $7, score $R^2$ = 0.94` positioned by one line
layout pass instead of by summing independently measured fragments.

The intended scope is math fragments, not arbitrary Typst documents. In
particular, this should support LaTeX-like math use cases and should reject
embedded Typst code, file loading, imports, packages, image insertion, custom
show rules, and arbitrary content embedded inside math.

## Summary

Using Typst-style text/math for Avenger is feasible as the default text path,
provided the supported syntax remains a strict subset rather than arbitrary
Typst documents. A math span is a laid-out mini scene: it can contain positioned
glyph runs, fraction rules, radicals, accents, stretchy delimiters, grouped
transforms, and other shapes. A mixed text label should therefore be laid out as
one text line by `avenger-typst-label`, not as separately positioned plain and math
fragments.

The current long-term plan is:

1. Keep `avenger-typst-label` as the low-level Avenger Typst-style text/math crate.
2. Make `avenger-text::TextEngine` the single concrete text engine.
3. Always parse supported static text markup and `$...$` math spans.
4. Return measured artifacts with width, height, baseline, ascent, descent,
   vector paths, rasters, native plain-run metadata, and future PDF glyph
   placement data.
5. Use whole-line Typst raster entries for WGPU.
6. Use hybrid SVG/PDF export: native SVG `<text>` for regular runs and paths
   for math/decorations, then let `svg2pdf` embed the native text.

The main reason to prefer the Avenger subset over a full Typst dependency is size,
control, and predictable syntax errors for unsupported document/evaluator
features. Earlier vendored/full-Typst size measurements in this document are
historical and should be regenerated with the current engine before making release
decisions.

## Typst Math Syntax

Typst uses `$...$` for math. It does not use LaTeX's `$$...$$` convention.

Important delimiter behavior:

- `$x^2$` is inline math.
- `$ x^2 $` is block/display math because Typst treats whitespace after the
  opening dollar and before the closing dollar as block-level equation syntax.
- For Avenger label use, default behavior should probably force inline math
  unless an option explicitly allows display-style math.

This should be documented as "Typst-style math fragments" rather than "LaTeX
math", because Typst's math language is similar but not identical.

## Current Avenger Boundaries

The active text stack is now centered on a single concrete text engine:

- `SceneTextMark` stores strings plus font, size, fill, alignment, baseline,
  angle, and limit properties.
- `avenger-text::TextEngine` measures text, produces whole-line rasters, and
  extracts hybrid text/path output.
- WGPU rendering stores whole text lines in the atlas instead of individual
  cosmic-text glyph entries.
- SVG rendering emits native SVG `<text>` for regular runs and paths for math
  and static decoration shapes.
- PDF rendering continues to emit SVG and pass it through `svg2pdf` with text
  embedding enabled, so regular text runs become real embedded PDF text while
  math remains vector paths for now.

Typst math does not naturally fit a glyph-only text model, but it does fit a
text-line artifact model. Math snippets remain part of `SceneTextMark` strings
rather than becoming separate chart marks. The one remaining service trait in
the chart stack is `TextMeasurementService`, retained only so derived mark
adjustments can reuse the evaluation-level text measurement cache; it is not a
backend-selection seam.

## Public API Shape

Math parsing is now part of the Typst-label engine used by Avenger text. The
public configuration surface stays small: chart and scene APIs opt labels into
Typst markup, while `avenger-text` owns fallback, truncation, measurement cache,
and renderer integration policy. The lower-level `avenger-typst-label` crate is
a frame-first label engine:

```rust
pub struct LabelEngine;

impl LabelEngine {
    pub fn compile(
        &self,
        source: &str,
        options: &LabelOptions,
    ) -> Result<CompiledLabel, LabelError>;

    pub fn compile_text(
        &self,
        text: &str,
        options: &LabelOptions,
    ) -> Result<CompiledLabel, LabelError>;
}

pub struct LabelOptions {
    pub text: TextStyle,
    pub math: MathStyle,
    pub params: LabelParams,
    pub limits: LabelLimits,
}
```

`compile` uses canonical Typst label markup, including `$...$` math spans and
Typst escaping. `compile_text` is the literal fast path used for plain
data-derived labels and fallback rendering; it is equivalent to escaping the
source and compiling markup, but avoids the markup parser.

The compiled artifact is page-independent and reused by all lowerers:

```rust
pub struct CompiledLabel {
    pub source: String,
    pub frame: LabelFrame,
    pub metrics: LabelMetrics,
    pub flags: LabelFlags,
    pub warnings: Vec<LabelWarning>,
}

pub struct LabelFrame {
    pub size: Size,
    pub baseline: f32,
    pub items: Vec<(Point, LabelFrameItem)>,
}

pub enum LabelFrameItem {
    Text(TextItem),
    Shape(ShapeItem),
    Image(ImageItem),
    Group(GroupItem),
}
```

Raster, SVG/vector, and PDF output are derived from that same frame:

```rust
pub fn rasterize(
    label: &CompiledLabel,
    options: &RasterOptions,
) -> Result<RasterImage, LabelError>;

pub fn svg_items(
    label: &CompiledLabel,
    options: &SvgOptions,
) -> Result<SvgLabel, LabelError>;

pub fn pdf_items(
    label: &CompiledLabel,
    options: &PdfOptions,
) -> Result<PdfLabel, LabelError>;
```

PDF artifacts carry font resources, glyph runs, paths, and draw order. The PDF
renderer supplies the final label, scene, and page transform when converting
these frame-local items into krilla operations.

## Font Choice

There is a real font choice for math. Typst defaults equations to New Computer
Modern Math. That is a good default for Avenger because it is expected by Typst
math layout and supports the OpenType MATH features Typst needs.

The initial implementation should:

- Default to New Computer Modern Math.
- Let callers choose another math-capable font later.
- Treat the math font separately from the surrounding text font.
- Include enough font resource metadata in output artifacts for PDF embedding:
  font bytes, face index, units per em, family, and PostScript name if known.

Avenger already carries some overlapping font dependencies:

- `ttf-parser = 0.25.1`
- `fontdb = 0.23.0`
- `rustybuzz = 0.20.1` transitively through SVG/PDF rendering

The studied Typst checkout used compatible `rustybuzz` and `ttf-parser`
versions, so the eventual dependency cost should be lower than the raw full
Typst probe suggests for normal Avenger builds that already include SVG/PDF
text support.

## Backend Behavior

### Measurement

The primary contract is metrics:

- width
- height
- baseline
- ascent
- descent

The best engine boundary is "layout this math fragment to a Typst `Frame`" and
then derive all payloads from that frame. Typst frames already carry width,
height, baseline, ascent, and descent. A public/forked `layout_math_fragment`
facade should return those values directly.

### Rasterization

Raster payloads can be derived from the path artifact instead of depending on
Typst's renderer in the minimal path. Use `tiny-skia` behind an optional
`avenger-typst-label/raster` feature; keep `typst-render` out of the default path.

Important raster details:

- Cache by source, style, math font, Typst fork revision, output kind, and
  raster scale.
- Render at device scale for WGPU and draw at logical size.
- Re-render at higher scale only when zoom or device scale makes it necessary.
- Preserve origin/baseline offsets in the raster artifact so atlas placement can
  align math snippets with surrounding text.
- Do not implement text truncation by cutting Typst source strings.

### SVG

For SVG, use vector paths for math spans and keep surrounding text as native
SVG `<text>`.

This means math glyphs are not selectable in SVG, but the output is faithful and
works with Avenger's renderer. Typst's own SVG exporter makes the same tradeoff:
it outputs glyph shapes rather than SVG text so rendering is stable across
systems.

Using paths only for content inside math delimiters is acceptable and keeps the
rest of the label on Avenger's normal text path.

### PDF Through Current `svg2pdf` Path

Avenger currently renders SVG, parses it with `svg2pdf::usvg::Tree::from_str`,
and calls `svg2pdf::to_pdf` with `embed_text: true`. That works for normal SVG
`<text>` because `svg2pdf` receives `usvg::Node::Text`, can resolve fonts, can
subset fonts, and can write real PDF text.

Typst SVG does not help with PDF text embedding in this path. Typst's SVG
backend lowers text to glyph definitions and `<use>` references backed by paths
or image glyph frames. Once that reaches `usvg`, it is vector geometry, not
text. `svg2pdf` cannot infer the original font-backed text from those paths.

Synthetic SVG `<text>` from Typst math data is also fragile because SVG would
ask `usvg` and rustybuzz to shape the Unicode text again. That loses Typst's
exact glyph IDs, math variants, offsets, stretchy delimiters, and non-glyph math
geometry.

There is a useful intermediate PDF strategy that matches the old
`vl-convert-pdf` manual text embedding approach:

1. Lower Typst math frames into path items and PDF glyph runs.
2. Mark glyph-outline paths as `MathPathKind::GlyphOutline`.
3. For normal SVG/PDF vector fallback, emit all paths.
4. For PDF text embedding, omit all visible math from the intermediate SVG
   passed to `svg2pdf`.
5. Keep the omitted math in a sidecar containing `MathPdfTextLayer`,
   `font_resources`, final math transforms, and any non-glyph `MathShape` paths
   such as fraction bars and radicals.
6. Let `svg2pdf` convert the SVG without math.
7. Post-process the PDF and append real PDF text for math glyphs, plus PDF path
   operators for non-glyph math shapes.

The useful parts to reuse are the same boundaries `vl-convert-pdf` used:
convert non-text SVG with `svg2pdf`, write Type0/CID font resources, subset
font data, write `/ToUnicode` maps, then append a PDF text content stream with
explicit glyph positioning.

Typst's current PDF backend is built on `krilla`, and that is the cleaner model
for the math overlay. Typst converts `TextItem` into `krilla::text::Font` plus
glyphs implementing `krilla::text::Glyph`, then calls `Surface::draw_glyphs`.
Krilla handles Type0/CID fonts, `Identity-H`, font subsetting, CID remapping,
`FontFile2`/`FontFile3`, `/ToUnicode`, and `/ActualText` for clustered or
ambiguous glyph mappings. Math glyphs are not special in Typst PDF output:
math layout produces normal text frame items for glyphs and shape frame items
for rules, radicals, and similar geometry.

The first embedded-PDF prototype should therefore try a krilla overlay before
hand-porting font embedding code:

1. Convert the SVG-without-math to PDF with existing `svg2pdf`.
2. Load that PDF with `hayro_syntax::Pdf::new` and wrap it as a
   `krilla::pdf::PdfDocument`.
3. Create a new krilla PDF, draw the base page as a page-sized XObject, then
   draw math shapes and `MathPdfTextLayer` glyph runs on top.
4. Verify regular SVG text remains selectable/searchable after base-page import.
   If it does not, fall back to direct content-stream postprocessing.

This gives selectable/searchable math glyphs without rewriting the whole PDF
renderer while avoiding duplicate visible math from `svg2pdf`. The caveat is
draw order: appended PDF math may appear above earlier content. That should be
treated as a prototype or opt-in mode, not the default for general text marks,
because math must match the z-order of the surrounding text and may need to be
covered by later marks.

The cleaner long-term solution is a direct PDF renderer integration, likely via
krilla or a similar layer, where math glyphs and math shapes can be written in
the correct draw order.

Z-order-preserving options:

- Keep path-based math in SVG/PDF as the first production path. Since math paths
  are emitted exactly where the text mark appears in the SVG/display list, this
  preserves draw order.
- Add direct krilla PDF emission for ordered Avenger display-list items, using
  krilla paths for math shapes and `Surface::draw_glyphs` for pre-shaped math
  glyphs.
- Or extend `svg2pdf`/`krilla-svg` with a placeholder/callback API that lets
  Avenger inject pre-shaped math glyphs during SVG-to-PDF conversion at the
  placeholder node's transform, clip, and paint state.
- Avoid relying on final-page overlay as the default. It can put math above
  marks that should occlude the text.

### Possible `svg2pdf` Contribution

`svg2pdf` already contains much of the machinery needed for math PDF text:
positioned glyph iteration, font resource lookup, font subsetting, CID remap,
PDF text operators, and ToUnicode maps. The missing piece is an API that accepts
pre-shaped glyph runs plus font resources, instead of requiring those glyphs to
come from `usvg::Text`.

A useful upstream contribution would be a public low-level text embedding API
parallel to the existing `usvg::Text` path. For z-order correctness, the API
should also support placeholder/callback insertion during SVG traversal, so a
caller can draw pre-shaped glyphs at the placeholder node's current transform,
clip, opacity, and paint state. Avenger's `MathPdfTextLayer` is shaped around
that possible boundary.

## Fragment Language Boundary

Allow:

- Typst math parser support via `parse_math`.
- `$x$` inline math and optionally `$ x $` display-style math.
- Native math syntax: `/`, `_`, `^`, primes, roots, delimiters, accents,
  matrices, cases, over/under constructs, alignment points, and shorthands.
- Typst math symbols and names such as `alpha`, `sum`, `integral`, `arrow.r`.
- A whitelist of native math functions such as `sqrt`, `frac`, `binom`, `mat`,
  `cases`, `cancel`, `bold`, `italic`, `bb`, `cal`, `frak`, and `mono`.

Reject:

- Embedded code syntax: `#...`, `#{...}`, `#let`, `#import`, `#include`,
  `#context`, loops, package imports, and file reads.
- Embedded arbitrary content/layout inside math, such as `#box(...)`,
  `#image(...)`, and `#rect(...)`.
- Show/set rules and user-defined functions.
- Raw code highlighting, bibliography/citations, plugins, image/SVG/PDF
  loading, data loading, and document/page constructs.

A simple validation rule is: reject syntax that enters Typst code mode before
evaluation, except for escaped characters that should render literally.

## Superseded Vendor Plan

Historical note: the implementation no longer follows the vendor route. The
old plan was to copy and patch a small Typst subset, then periodically sync it
from upstream. That path was replaced by the `avenger-typst-label` parser, text
layout, math layout, artifact lowering, and strict unsupported-syntax errors.

## Frame-To-Artifact Lowering

After Typst produces a frame:

- Traverse `FrameItem::Group`, push transforms, recurse into the child frame,
  and apply clips.
- Traverse `FrameItem::Text`, outline each shaped glyph with
  `FontInstance::ttf().outline_glyph`, scale from font units to Typst points,
  apply glyph offsets/advances/transforms, and emit `MathPathKind::GlyphOutline`.
- While traversing text, also emit `MathPdfGlyphRun` data: font resource id,
  font bytes, face index, original glyph id, glyph origin, font size,
  fill/stroke, run text, and each glyph's byte range into that text for
  ToUnicode/ActualText.
- Traverse `FrameItem::Shape`, convert Typst geometry to Avenger `PathData`,
  preserving fill rule, fill, stroke, caps, joins, and dashes.
- Ignore `FrameItem::Tag`.
- Reject `FrameItem::Image` and `FrameItem::Link` in strict mode.
- Compute metrics from the frame's width, height, baseline, ascent, and descent.

SVG/PDF vector output consumes paths. WGPU can consume rasters derived from
those same paths. PDF post-processing can consume the glyph layer.

## Superseded Vendor Sync Strategy

Historical note: the vendored tree has been removed from the active dependency
path. Future sync work should happen as targeted owned-engine improvements:
copy the relevant upstream algorithm or metric rule into `avenger-typst-label`, add
unit tests and visual baselines, and keep the public strict-subset contract.

## Size Findings

Two native macOS arm64 release probes informed the current decision.

Earlier full-Typst probe:

- Baseline no-Typst probe: about 363 KB stripped.
- Full Typst compile/layout/metrics path with embedded fonts: about 41 MB
  stripped.
- Full Typst compile/layout/render path with `typst-render`: about 42 MB
  stripped.

Current WGPU text-render probe in `tools/text-render-probe` after removing the
vendored Typst crates:

| Backend | Binary bytes | PNG bytes | Notes |
| --- | ---: | ---: | --- |
| `cosmic` | 8,998,848 | 56,378 | Existing WGPU text path. |
| `typst` | 8,332,624 | 42,692 | Avenger Typst-style text/math path, no `cosmic-text`. |

In this low-level probe the Avenger Typst path is about 665 KB smaller than the
cosmic path while also supporting math syntax. Re-run the probe before making
release decisions because the exact number depends on platform, feature set,
and link profile.

This supports the current decision:

- Make the Avenger Typst-style path the default Avenger text engine.
- Remove cosmic-text and HTML canvas text backends from the core path.
- Do not add `typst-render` or `typst-pdf` for first-stage math rendering.
- Keep shared dependencies such as `rustybuzz`, `ttf-parser`, and `fontdb`
  aligned where possible.
- Continue measuring text-size and text-render probes before release because
  exact size depends on platform, feature set, and link profile.

## Upstream Discussion

No exact maintainer thread was found asking for "Typst as a math-only Rust
renderer for chart labels." Adjacent discussions and docs:

- Standalone equations with auto-sized pages:
  <https://github.com/typst/typst/discussions/893>
- Smaller library-facing compiler request:
  <https://github.com/typst/typst/issues/4653>
- In-memory/library usage through custom worlds:
  <https://github.com/typst/typst/discussions/1160>
- Library splitting and hard coupling:
  <https://github.com/typst/typst/issues/5664>
- Intermediate layout layer request:
  <https://github.com/typst/typst/issues/8453>
- Plotting with Typst:
  <https://github.com/typst/typst/discussions/1444>
- Selectable SVG text request:
  <https://github.com/typst/typst/issues/4702>
- Typst SVG docs explaining why SVG text becomes glyph shapes:
  <https://typst.app/docs/reference/svg/>
- Typst math docs:
  <https://typst.app/docs/reference/math/>
  <https://typst.app/docs/reference/math/equation/>
- `svg2pdf` text embedding request/history:
  <https://github.com/typst/svg2pdf/issues/21>
- Old `vl-convert-pdf` SVG-to-PDF implementation with manual text/font
  embedding:
  <https://docs.rs/vl-convert-pdf/latest/vl_convert_pdf/>
  <https://docs.rs/vl-convert-pdf/latest/src/vl_convert_pdf/lib.rs.html>
- Local Typst/krilla PDF embedding code studied:
  `../typst/crates/typst-pdf/src/text.rs`,
  `~/.cargo/registry/src/.../krilla-0.8.2/src/surface.rs`,
  `~/.cargo/registry/src/.../krilla-0.8.2/src/content.rs`,
  `~/.cargo/registry/src/.../krilla-0.8.2/src/text/cid.rs`,
  `~/.cargo/registry/src/.../krilla-0.8.2/src/text/group.rs`.

Possible upstream contributions:

- Typst or `typst-layout`: expose a stable "layout math expression to frame"
  API with metrics.
- Typst: expose a smaller fragment-oriented library surface for embedding.
- `svg2pdf`/`krilla-svg`: expose a low-level pre-shaped glyph-run PDF embedding
  API and a z-order-preserving placeholder/callback API during SVG traversal.
- Typst SVG: optionally emit selectable SVG text, although this is less useful
  for exact math because math layout depends on glyph IDs and MATH variants.

## Implementation Phases

Phase 1: text engine as default.

- Own the Typst-style text/math parser and strict subset validation in
  `avenger-typst-label`.
- Route `avenger-text::TextEngine` measurement, rasterization, and path
  extraction through the default engine.
- Remove cosmic-text, HTML canvas text measurement, and text-backend feature
  gates from the core path.
- Use whole-line raster atlas entries for WGPU.

Phase 2: hybrid SVG/PDF output.

- Emit regular text runs as native SVG `<text>`.
- Emit math spans and static decoration shapes as paths.
- Let current `svg2pdf` convert that hybrid SVG to PDF. This keeps regular text
  embedded by `svg2pdf` and keeps math z-order correct because math paths are
  emitted at the text mark's display-list position.
- Keep PDF glyph metadata in the text artifacts for future direct math
  font embedding.

Phase 3: validation and size work.

- Run and review all chart visual baselines because every label now uses the
  Typst path.
- Verify emoji, bidi, complex scripts, escaped dollars, unsupported syntax
  errors, SVG native text, PDF selectable text, and WGPU whole-line rasters.
- Continue text-size and text-render probes.
- Add dependency deny-list and API/visual/metric guardrails.

Phase 4: embedded PDF math text.

- Preserve `MathPdfTextLayer` from frame lowering.
- Prototype a krilla overlay by converting SVG-without-math to PDF, importing
  it as a page XObject, and drawing math shapes/glyphs on top.
- Treat the overlay only as a validation prototype or opt-in mode.
- Before embedded PDF math becomes default for general text marks, implement a
  z-order-preserving route: direct krilla display-list emission, an upstream
  `svg2pdf`/`krilla-svg` placeholder callback, or a robust content-stream
  replacement mechanism.
- Subset/embed math fonts and write ToUnicode/ActualText mappings using
  `MathPdfGlyphRun::text` and glyph `text_range`.

Phase 5: prune and sync.

- Remove unrelated Typst dependencies.
- Replace broad eval/realize paths with math-only variants if size is still too
  large.
- Add dependency deny-list and API/visual/metric guardrails.

## Open Questions

- Which chart-level helpers should be added on top of default text markup:
  `title_math`, `axis_title_math`, typed label content, or no extra helpers?
- Should display-style math ever be enabled in chart labels, or should all math
  delimiters be treated as inline by default?
- Which math font choices should be supported beyond New Computer Modern Math?
- Which z-order-preserving embedded PDF route should follow the overlay
  prototype: direct krilla PDF emission, `svg2pdf`/`krilla-svg` callback API, or
  content-stream replacement?
- Which additional size-reduction opportunities remain after removing the full
  Typst/vendored path and cosmic-text?

## Recommendation

Use an owned, patchable Typst-style text/math subset rather than the full Typst
crate for the production path. `avenger-text::TextEngine` should return metrics
plus backend payloads, not rendered text strings. WGPU should rasterize whole
text lines. SVG/PDF should use native text for regular runs and paths for math
and decorations. Embedded PDF math text can later use the same returned glyph
placement and font resources, but it should not become the default until it
preserves the text mark's z-order.

Keep the full scratch analysis as background, but treat this document as the
working plan.
