# Typst Math Typesetting

Date: 2026-06-24
Last updated: 2026-06-25

Status: partially implemented / staged follow-up plan

Source notes: this document condenses the scratch research in
`scratch/typst-math-typesetting-analysis.md` and
`scratch/typst-avenger-text-integration-plan.md`. The local Typst checkout
studied was `../typst` at commit
`c98e910391a8544b28bd5c99a6f3b1ac1ada9a84`, workspace version `0.15.0`.

## Goal

Add support for Typst-style math fragments in Avenger labels, titles, axis
labels, legend labels, and similar text-bearing chart surfaces.

The target user-facing shape is a string that may contain math spans such as:

```text
$x^2 + y^2$
```

Text outside math spans should remain normal Avenger text. Text inside math
spans should be measured and rendered by a Typst-derived math engine.

The intended scope is math fragments, not arbitrary Typst documents. In
particular, this should support LaTeX-like math use cases and should reject
embedded Typst code, file loading, imports, packages, image insertion, custom
show rules, and arbitrary content embedded inside math.

## Summary

Using Typst math for Avenger is feasible, but only for explicit math
fragments. Regular text should stay on Avenger's existing text path. A Typst
equation is a laid-out mini scene: it can contain positioned glyph runs,
fraction rules, radicals, accents, stretchy delimiters, matrices, grouped
transforms, and other shapes.

The recommended long-term plan is:

1. Keep `avenger-typst` as the low-level optional Typst math-fragment crate.
2. Parse label strings into normal text runs and `$...$` math runs.
3. Route math runs through a vendored and patched Typst math-fragment engine.
4. Return measured artifacts with width, height, baseline, ascent, descent,
   vector paths, optional rasters, and optional PDF glyph placement data.
5. Integrate through optional `avenger-text` wrappers that keep surrounding
   non-math text on the existing cosmic/native text path.

The main reason to prefer a vendored math-fragment path over a full Typst
dependency is size and control. A full Typst compile/layout path added roughly
40-42 MB to an earlier stripped native macOS arm64 probe binary. The current
vendored math-string API probe is smaller, but still adds about 12 MB stripped
without cosmic, about 14 MB stripped with cosmic, and about 15 MB stripped for
the current mixed cosmic-plus-Typst-raster path. The feature should remain
optional while the vendored subset is pruned.

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

Avenger's current text stack is optimized for normal text:

- `SceneTextMark` stores strings plus font, size, fill, alignment, baseline,
  angle, and limit properties.
- `avenger-text` measures text through a narrow `TextMeasurer` trait.
- WGPU rendering rasterizes glyphs into an atlas.
- SVG rendering emits native SVG `<text>`.
- PDF rendering emits SVG and passes it through `svg2pdf` with text embedding
  enabled, so normal Avenger text can become real embedded PDF text.

Typst math does not naturally fit a text-only glyph model. The chosen
integration is to keep `SceneTextMark` and the existing text traits for regular
text, add math-aware wrappers for mixed strings, and add a path extraction
trait for vector output. Math snippets become measured run artifacts inside
the text pipeline rather than a separate chart mark.

## Public API Sketch

Math parsing should be opt-in, not magical for every string. The selected
configuration shape is a markup mode on text options:

```rust
pub enum TextMarkupMode {
    Plain,
    TypstMathDelimited(MathDelimiterOptions),
}

pub struct TextMathConfig {
    pub mode: TextMarkupMode,
    pub math_style: MathStyle,
    pub syntax: MathSyntaxMode,
    pub limits: MathLabelLimits,
    pub error_policy: MathErrorPolicy,
}
```

High-level chart helpers such as these can still be added later, but they
should lower to the same opt-in text markup configuration:

```rust
plot.title("Plain title");
plot.title_math("sum_(i=1)^n x_i");
axis.title_math("x^2");
legend.label_format_math(...);
```

For labels that intentionally mix text and math, use a helper that parses
math spans:

```rust
pub fn typeset_math_label(
    source: &str,
    options: &MathLabelOptions,
) -> Result<MathLabelArtifact, MathLabelError>;
```

Suggested configuration:

```rust
pub struct MathLabelOptions {
    pub text_style: TextStyle,
    pub math_style: MathStyle,
    pub outputs: MathLabelOutputs,
    pub delimiters: MathDelimiterOptions,
    pub syntax: MathSyntaxMode,
    pub raster_scale: f32,
    pub limits: MathLabelLimits,
}

pub struct MathStyle {
    /// Default: New Computer Modern Math.
    pub math_font: MathFontSpec,

    /// Usually inherited from surrounding text.
    pub font_size: f32,

    pub fill: Color,

    /// Default should likely be false for chart labels.
    pub allow_display_style: bool,
}

pub enum MathFontSpec {
    NewComputerModernMath,
    Family(String),
    FontId(FontId),
}

pub struct MathLabelOutputs {
    pub paths: bool,
    pub raster: bool,

    /// Exact glyph placement and font data for a PDF text-injection pass.
    pub pdf_text_layer: bool,
}

pub struct MathDelimiterOptions {
    /// Default: '$'.
    pub delimiter: char,

    /// Default: Some('\\'), so `\$` is literal.
    pub escape: Option<char>,

    pub unmatched: UnmatchedDelimiterPolicy,
}

pub enum UnmatchedDelimiterPolicy {
    TreatAsLiteral,
    Error,
}

pub enum MathSyntaxMode {
    /// Typst math syntax, but no embedded Typst code/content.
    TypstFragmentStrict,
}

pub struct MathLabelLimits {
    pub max_source_bytes: usize,
    pub max_math_spans: usize,
    pub max_math_depth: usize,
}
```

Suggested artifact shape:

```rust
pub struct MathLabelArtifact {
    pub source: String,
    pub metrics: LabelMetrics,
    pub runs: Vec<LabelRun>,
    pub font_resources: Vec<MathFontResource>,
    pub warnings: Vec<MathLabelWarning>,
}

pub struct LabelMetrics {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    pub ascent: f32,
    pub descent: f32,
}

pub enum LabelRun {
    Text(TextRun),
    Math(MathRun),
}

pub struct MathRun {
    /// Source inside `$...$`, without delimiters.
    pub source: String,
    pub byte_range: std::ops::Range<usize>,
    pub x: f32,
    pub y: f32,
    pub metrics: LabelMetrics,
    pub paths: Option<MathPathArtifact>,
    pub raster: Option<MathRasterArtifact>,
    pub pdf_text: Option<MathPdfTextLayer>,
}

pub struct MathPathArtifact {
    pub logical_width: f32,
    pub logical_height: f32,
    pub commands: Vec<MathPathItem>,
}

pub struct MathPathItem {
    pub path: PathData,
    pub kind: MathPathKind,
    pub fill: Option<Color>,
    pub stroke: Option<Stroke>,
    pub transform: Transform,
    pub clip: Option<PathData>,
}

pub enum MathPathKind {
    /// Glyph outlines generated from a Typst text item.
    /// PDF export may omit these and inject `MathPdfTextLayer` instead.
    GlyphOutline { glyph_run: usize, glyph_index: usize },

    /// Non-text math geometry: fraction bars, radicals, cancel strokes,
    /// some stretchy constructions, etc.
    MathShape,
}

pub struct MathRasterArtifact {
    pub image: RgbaImage,
    pub scale: f32,
    pub logical_width: f32,
    pub logical_height: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}

pub struct MathFontResource {
    pub id: MathFontResourceId,
    pub family: String,
    pub postscript_name: Option<String>,
    pub face_index: u32,
    pub units_per_em: f32,

    /// Use shared storage or a font registry in real code.
    pub data: std::sync::Arc<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MathFontResourceId(pub u32);

pub struct MathPdfTextLayer {
    pub logical_width: f32,
    pub logical_height: f32,

    /// Useful for accessibility/search even if glyph-level extraction is
    /// imperfect.
    pub semantic_text: String,

    pub glyph_runs: Vec<MathPdfGlyphRun>,
}

pub struct MathPdfGlyphRun {
    pub font: MathFontResourceId,
    pub font_size: f32,
    pub fill: Color,
    pub stroke: Option<Stroke>,
    pub text: String,
    pub glyphs: Vec<MathPdfGlyph>,
}

pub struct MathPdfGlyph {
    /// Original glyph id in `font`. A PDF postprocessor can remap this into a
    /// subset CID.
    pub glyph_id: u16,

    /// Byte range into `MathPdfGlyphRun::text` for this glyph's source
    /// cluster. This mirrors Typst/krilla and feeds ToUnicode/ActualText.
    pub text_range: std::ops::Range<usize>,

    /// Glyph origin in math-run coordinates, in points, before the enclosing
    /// label/scene/page transform.
    pub x: f32,
    pub y: f32,

    pub x_advance: f32,
    pub y_advance: f32,

    /// Usually identity, but useful for transformed groups.
    pub transform: Transform,
}
```

The artifact is intentionally page-independent. Glyph coordinates are in
math-run coordinates. The PDF renderer supplies the final label, scene, and page
transform when injecting `MathPdfTextLayer`.

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
`avenger-typst/raster` feature; keep `typst-render` out of the default path.

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

## Vendor Plan

The recommended implementation is a reproducible vendored subset under a path
like:

```text
vendor/typst-avenger/
  README.md
  UPSTREAM_REV
  Cargo.toml
  crates/
    typst-syntax/
    typst-library/
    typst-layout/
    typst-utils/
    typst-macros/
    typst-timing/

tools/vendor-typst-math/
  recipe.toml
  overlays/
  patches/
  prompts/
    resolve-sync.md
```

Prefer preserving upstream Rust crate names internally. If Cargo package names
must avoid registry collisions, rename packages but keep crate names:

```toml
typst_library = { package = "avenger-typst-library", path = "vendor/typst-avenger/crates/typst-library" }
typst_layout = { package = "avenger-typst-layout", path = "vendor/typst-avenger/crates/typst-layout" }
```

Do not expose vendored Typst types from Avenger's public API.

### Vendor Inventory

Keep whole at first:

- `typst-syntax`
- `typst-utils`
- `typst-macros`
- `typst-timing`

Keep with a `math-fragment` or `avenger-minimal` feature profile:

- `typst-library` diagnostics, engine, foundations, introspection, routines,
  symbols, math modules, text/font modules, layout primitives, frame types,
  and visualize paint/shape/stroke types.
- `typst-layout/src/math/*`
- `typst-layout/src/modifiers.rs`
- `typst-layout/src/shapes.rs`
- `typst-layout/src/inline/shaping.rs`
- only enough inline layout support for math text runs
- minimal model stubs if needed to satisfy equation imports

Do not vendor:

- `typst-realize`: `avenger-typst` supplies a narrow realization routine for
  accepted math content.
- `typst-eval`: `avenger-typst` lowers the accepted math AST directly into
  Typst math content and disables general string/closure evaluation.

Do not include in the default fragment path:

- top-level `typst`
- `typst-html`
- `typst-pdf`, unless Avenger explicitly chooses to depend on Typst's full PDF
  exporter. The current plan may use `krilla` directly, but does not require
  `typst-pdf` as a dependency.
- `typst-svg`, unless used only as an early spike backend
- `typst-render`; rasterization should stay outside the vendored Typst tree and
  use `tiny-skia` behind `avenger-typst/raster`
- `typst-kit` and `typst-assets`, except temporary development use
- bibliography, raw highlighting, plugins, data loading, image loading, full
  document/page export features

### Patch Shape

Expected patches:

1. Add a math-fragment feature profile for `typst-library`.
2. Add a public or private `layout_math_fragment_frame` facade in
   `typst-layout` that wraps the existing private inline equation layout path.
3. Add Avenger-owned strict AST validation and math-content lowering.
4. Add Avenger-owned minimal routines for realization while keeping string and
   closure evaluation disabled.
5. Add an Avenger frame-to-artifact lowering layer.

The private driver can be much narrower than the public Avenger API:

```rust
pub fn layout_math_fragment(
    source: &str,
    style: &MathStyle,
) -> Result<TypstMathFrameArtifact, TypstMathError>;
```

Internally it should:

1. Parse the fragment.
2. Reject unsupported syntax.
3. Lower allowed math AST nodes directly into Typst math content.
4. Wrap content in an equation element.
5. Call the patched math layout facade.
6. Lower the resulting frame into Avenger artifacts.

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

## Vendor Sync Strategy

The vendored tree should be reproducible from:

1. an upstream Typst git revision,
2. a declarative file/patch recipe,
3. Avenger overlay files,
4. a mechanical sync command.

Example command:

```text
cargo xtask vendor-typst-math --upstream ../typst --rev c98e910391a8544b28bd5c99a6f3b1ac1ada9a84
```

The command should:

1. Verify or check out the requested upstream revision.
2. Regenerate `vendor/typst-avenger`.
3. Copy only recipe-listed crates and files.
4. Generate local `Cargo.toml` files from templates.
5. Preserve upstream relative paths where possible.
6. Record upstream source path and revision.
7. Apply overlays.
8. Apply topic-oriented patches.
9. Run `cargo fmt`.
10. Run `cargo check --release -p avenger-typst --features vendor-typst`.
11. Run a dependency deny-list check.

Expected mechanical fixes:

- Rename Cargo packages while preserving Rust crate names.
- Generate minimal manifests.
- Stub HTML hooks.
- Build a math-only routines table.
- Add `math-fragment` gates.
- Generate unsupported-feature diagnostics.

Patch failures or `cargo check` failures are a good bounded agent task. The
agent prompt should require preserving the fragment contract, keeping embedded
code/content rejected, and avoiding reintroducing full Typst dependencies such
as `typst`, `typst-eval`, `typst-realize`, `typst-html`, `typst-pdf`,
`typst-svg`, `typst-render`, `typst-kit`, `typst-assets`, `hayagriva`,
`syntect`, `wasmi`, image loaders, or data format loaders unless explicitly
approved. In particular, embedded PDF math glyphs via `MathPdfTextLayer` should
not by itself justify adding `typst-pdf`; that crate only belongs in the plan if
Avenger adopts Typst's full PDF exporter rather than using `krilla` directly.

## Size Findings

Two native macOS arm64 release probes informed the current decision.

Earlier full-Typst probe:

- Baseline no-Typst probe: about 363 KB stripped.
- Full Typst compile/layout/metrics path with embedded fonts: about 41 MB
  stripped.
- Full Typst compile/layout/render path with `typst-render`: about 42 MB
  stripped.

Current `avenger-text` feature probe in `tools/text-size-probe`:

| Variant | Raw | Stripped | Notes |
| --- | ---: | ---: | --- |
| `plain-none` | 404,928 B | 336,128 B | Minimal probe baseline. |
| `cosmic` | 2,554,048 B | 2,237,952 B | Existing regular native text measurement stack. |
| `typst-math` | 15,292,224 B | 12,312,464 B | Current `avenger-typst/vendor-typst` math-string API with paths and PDF text metadata. |
| `typst-math-raster` | 15,549,760 B | 12,533,288 B | Low-level Typst math plus `tiny-skia` path rasterization. |
| `cosmic+typst-math` | 17,293,920 B | 14,077,928 B | Native plain measurement plus Typst math paths. |
| `cosmic+typst-math-raster` | 18,795,216 B | 15,365,936 B | Expected WGPU mixed-label path: cosmic rasterization plus Typst math rasterization. |

Approximate stripped deltas over `plain-none`:

- `cosmic`: +1.90 MB
- `typst-math`: +11.98 MB
- `typst-math-raster`: +12.20 MB
- `cosmic+typst-math`: +13.74 MB
- `cosmic+typst-math-raster`: +15.03 MB

The direct `typst-math-raster` increment over `typst-math` is only about
0.22 MB stripped. The larger `cosmic+typst-math-raster` increment also reflects
exercising cosmic glyph rasterization, not just adding `tiny-skia`.

Duplicate-dependency checks from `cargo tree -d` show useful future alignment
targets:

- `png` 0.17 through `tiny-skia` and `png` 0.18 through `image`/Typst;
- `bitflags` 1.x through `tiny-skia` and 2.x through Typst/cosmic/image;
- `kurbo` 0.11 through `svgtypes` and 0.13 through vendored Typst;
- in the combined cosmic path, `read-fonts`/`skrifa` stacks from cosmic/swash
  and Typst are still split.

This supports the current decision:

- Keep cosmic as the regular text path.
- Keep Typst math optional for now.
- Study vendored Typst size reduction before considering math support as a
  default feature.
- Do not add `typst-render` or `typst-pdf` for first-stage math rendering.
- Align shared dependencies such as `rustybuzz`, `ttf-parser`, and `fontdb`
  where possible, but expect the Typst math layout subset itself to dominate
  binary size until deeper pruning lands.

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

Phase 1: low-level math fragment crate.

- Keep `avenger-typst` optional and independent from `avenger-text`.
- Vendor/pin the first Typst subset.
- Implement strict math fragment parsing and validation.
- Add math-frame layout facade.
- Lower frames to metrics, path artifacts, optional raster artifacts, and
  `MathPdfTextLayer`.
- Add unit tests for scripts, fractions, roots, matrices, cases, accents, large
  delimiters, color, baseline alignment, and unsupported syntax rejection.

Phase 2: optional `avenger-text` integration.

- Add `typst-math` and `typst-math-raster` feature flags.
- Keep cosmic as the regular native text backend.
- Add math-aware `TextMeasurer` and `TextRasterizer` wrappers that split
  opt-in `$...$` math spans from surrounding plain text.
- Add a `TextPathExtractor` trait.
- Implement `TextPathExtractor` for cosmic and for math-aware mixed labels.
- Cache artifacts by source/style/font/output scale/fork revision.

Phase 3: path-first renderer integration.

- Emit regular text as native SVG `<text>`.
- Emit math spans as paths in SVG.
- Let current `svg2pdf` convert that SVG to PDF. This keeps regular text
  embedded by `svg2pdf` and keeps math z-order correct because math paths are
  emitted at the text mark's display-list position.
- Rasterize math path artifacts for WGPU atlas entries behind
  `typst-math-raster`.

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
- Implement the reproducible vendor sync command.

## Open Questions

- Which chart-level helpers should be added on top of opt-in text markup:
  `title_math`, `axis_title_math`, typed label content, or only configuration
  knobs at first?
- Should display-style math ever be enabled in chart labels, or should all math
  delimiters be treated as inline by default?
- Which math font choices should be supported beyond New Computer Modern Math?
- Which z-order-preserving embedded PDF route should follow the overlay
  prototype: direct krilla PDF emission, `svg2pdf`/`krilla-svg` callback API, or
  content-stream replacement?
- How small does the vendored fragment path need to become before it is
  acceptable as an optional Avenger feature?

## Recommendation

Use a vendored, patchable Typst math-fragment engine rather than the full Typst
crate for the production path. The helper should return metrics plus backend
payloads, not rendered text strings. SVG and normal PDF can use paths for only
the math spans. WGPU can rasterize those paths. Embedded PDF text can later use
the same returned glyph placement and font resources, but it should not become
the default for general text marks until it preserves the text mark's z-order.

Keep the full scratch analysis as background, but treat this document as the
working plan.
