# Pattern Fills

## Goal

Add support for pattern fills on marks that already support filled interiors.
This is primarily an accessibility and print/export feature: patterns can
differentiate categories when color is unavailable, unreliable, or already used
for another channel. A good design should work in both `avenger-wgpu` and
`avenger-svg`, preserve the existing scenegraph rendering boundary, and leave a
path for pattern legends and redundant color+pattern encodings.

This document is a research and design note, not an implementation plan.

## Current Avenger Context

Avenger currently represents mark paint with `ColorOrGradient`. Filled
scenegraph marks such as rects, paths, symbols, arcs, areas, and groups carry
fill and stroke paint plus a per-mark `gradients` registry. The SVG backend
emits native gradient definitions in `<defs>` and resolves paint attributes as
`url(#...)`. The WGPU backend builds a gradient texture atlas and encodes
gradient references through sentinel values in vertex color attributes.

That suggests a scenegraph-level paint extension rather than an
`avenger-chart`-only feature. Chart authoring can add pattern channels and
scales later, but both SVG and WGPU need a common serialized pattern model.

## Existing Library Survey

### Matplotlib

Matplotlib exposes hatching as a compact string property, with symbols such as
slashes, bars, crosses, dots, and rings. Repeating characters increases density.
Hatches are supported for many filled polygon-like artists and work across
several file/rendering backends, but not every backend supports them.

Useful precedent: a short symbolic pattern language is convenient for static
authoring, but it is too limited to be the only Avenger representation.

Source: <https://matplotlib.org/stable/gallery/shapes_and_collections/hatch_style_reference.html>

### Bokeh

Bokeh separates hatch styling from fill color. Glyphs can carry
`hatch_pattern`, `hatch_color`, `hatch_alpha`, `hatch_weight`,
`hatch_scale`, and `hatch_extra`. The `hatch_extra` hook allows custom
textures. Bokeh also has mappers for hatch patterns, so pattern can behave like
another encodable visual channel rather than a raw paint string.

Useful precedent: keep pattern selection separate from fill color so redundant
color+pattern encodings are natural.

Sources:

- <https://docs.bokeh.org/en/latest/docs/user_guide/styling/visuals.html>
- <https://docs.bokeh.org/en/latest/docs/reference/plotting/figure.html>

### Plotly

Plotly exposes `marker.pattern` on filled traces. The core controls include
`shape`, `fgcolor`, `bgcolor`, `fgopacity`, `size`, `solidity`, and
`fillmode`. Shape values include common hatches and dots. Plotly added this for
bars and later expanded support to several area-like traces.

Useful precedent: foreground/background composition and "overlay versus
replace" modes are important enough to model explicitly.

Sources:

- <https://plotly.com/python/pattern-hatching-texture/>
- <https://plotly.com/python-api-reference/generated/plotly.graph_objects.bar.marker.html>

### ggpattern

`ggpattern` adds patterned versions of ggplot2 geoms that have filled regions.
It provides many pattern aesthetics and scales, supports more than a dozen
built-in patterns through `gridpattern`, and allows user-defined patterns.

Useful precedent: the full design space is broad. Avenger should avoid making
the v1 channel a closed string enum that cannot grow into parameterized or
custom patterns.

Source: <https://cran.r-project.org/web/packages/ggpattern/refman/ggpattern.html>

### Highcharts

Highcharts treats pattern fills as a kind of color option, analogous to linear
and radial gradients. A `pattern-fill.js` module allows a pattern object
wherever a color option is accepted. It supports custom SVG patterns and image
patterns.

Useful precedent: integrating patterns into the existing paint slot is elegant
for low-level rendering, even if chart authoring also benefits from separate
pattern channels.

Source: <https://www.highcharts.com/docs/chart-design-and-style/pattern-fills>

### Apache ECharts

ECharts uses the term `decal` for parametric repeating imagery, distinct from
image patterns. Decals are tied to accessibility: `aria.decal.show` can apply a
default set, and `aria.decal.decals` can customize the generated patterns.

Useful precedent: automatic redundant pattern assignment can be framed as an
accessibility mode, not only as explicit visual styling.

Sources:

- <https://echarts.apache.org/handbook/en/best-practices/aria/>
- <https://github.com/apache/echarts/issues/13263>

### Chart.js

Chart.js accepts `CanvasPattern` and `CanvasGradient` objects in color fields.
The docs point users to `CanvasRenderingContext2D.createPattern` or libraries
such as Patternomaly.

Useful precedent: renderer-native pattern objects are powerful, but they do not
by themselves define a serializable chart grammar or portable legend behavior.

Source: <https://www.chartjs.org/docs/latest/general/colors.html>

### D3, PatternFills, And Textures.js

D3 users commonly define SVG `<pattern>` elements manually and fill marks with
`url(#pattern-id)`. Irene Ros's PatternFills and Textures.js provide reusable
SVG pattern definitions and APIs around this approach.

Useful precedent: SVG has a strong native pattern primitive, but hand-authored
pattern defs are SVG-specific and can become hard to port to canvas/GPU paths.

Sources:

- <https://iros.github.io/patternfills/sample_d3.html>
- <https://github.com/iros/patternfills>
- <https://riccardoscalco.it/textures/>

### Vega And Deneb

Vega does not currently provide native cross-renderer pattern fill support; the
long-running Vega issue asks for scenegraph-level pattern fill/stroke support.
Deneb documents pattern fills as an SVG-only extension and warns that they are
not Vega features and will not work in Canvas.

Useful precedent: adding pattern fills only in the SVG path would create a
backend portability problem that Avenger should avoid.

Sources:

- <https://github.com/vega/vega/issues/1372>
- <https://deneb-viz.github.io/docs/1.5/pattern-fills>

## Literature Survey

### No Canonical Pattern Palette Yet

I did not find a widely accepted named set of categorical pattern fills
analogous to the Okabe-Ito color palette. The literature is stronger on
describing pattern design spaces, measuring texture perception, and giving
task-specific design guidance than on prescribing one universal finite palette.

For Avenger, that means a built-in cycle should be presented as a pragmatic
default, not as an "optimal" perceptual palette. It should be validated with
small visual tasks and be easy to override.

### Black-And-White Categorical Textures

He, Zhong, Isenberg, and Isenberg study 2D black-and-white textures for
categorical visualization. They distinguish geometric textures from iconic
textures, summarize texture attributes, collect expert designs, and evaluate
textured charts. Their results are directly relevant to bar, pie/arc, and map
marks: texture can work for categorical differentiation, but design quality and
chart type matter.

Implication for Avenger: v1 should start with geometric textures and swatches.
Iconic or semantically meaningful patterns are promising but should probably be
custom/user-provided rather than baked into a generic default palette.

Sources:

- <https://arxiv.org/abs/2307.10089>
- <https://github.com/tingying-he/design-characterization-for-black-and-white-textures-in-visualization>

### Perceptually Uniform Texture Density

Schulz et al. study stippling, hatching, and triangle textures for information
encoding. They use multidimensional scaling to recover perceptual spaces and
construct perceptually uniform density levels. For hatching, they find a strong
separation between one-direction hatching and crosshatching, and advise against
mixing those families for one scalar field. They also find non-linearity near
very sparse and very dense extremes and recommend sigmoid-like mappings for
density.

Implication for Avenger: if pattern density becomes an ordered or quantitative
channel, density should not be linearly mapped from data without perceptual
calibration. Categorical pattern shape and ordered pattern density are different
problems.

Source: <https://arxiv.org/html/2308.03644>

### Internal Patterns In Bar Charts

Wong and Ruchikachorn study internal patterns for value estimation in bar
charts. Their short-paper result is a useful caution: patterns can help or
hinder, depending on structure and complexity. They recommend intentional,
countable, structured patterns when the pattern is expected to aid quantitative
reading.

Implication for Avenger: default patterns should not add misleading horizontal
reference lines or dense noise to bars. Legends and examples should steer users
toward pattern as a categorical/redundant channel first.

Source: <https://diglib.eg.org/items/1a6c33c7-efa6-4787-9dbe-422351e89554>

### Semantically Resonant Patterns

Lu et al. propose a methodology for semantically resonant abstract patterns:
patterns that intuitively evoke the category they represent, analogous to
semantically resonant colors. They use workshops with design experts and
non-design participants to produce and evaluate design strategies.

Implication for Avenger: the core API should allow named or custom pattern
sets, so domain packages can supply meaningful pattern palettes. The default
library should not try to infer semantic pattern assignments from arbitrary
category names in v1.

Source: <https://arxiv.org/abs/2505.14816>

### Pattern As A Composite Visual Variable

He, Dykes, Isenberg, and Isenberg reframe pattern as a composite visual variable
made from structured groups of graphic primitives. Their system separates:

- spatial arrangement of primitives,
- appearance relationships among primitives,
- retinal variables that characterize each primitive.

Implication for Avenger: a pattern grammar should not be just a list of names.
Even if v1 exposes a small set of named built-ins, the serialized pattern model
should be capable of representing primitive shape, arrangement, spacing,
orientation, foreground/background paint, opacity, and density.

Sources:

- <https://arxiv.org/abs/2508.02639>
- <https://vdl.sci.utah.edu/publications/2025_vis_reframing-pattern/>

### Texture And Color Together

Healey and Enns combine simple texture patterns with perceptually uniform
colors for multivariate scientific visualization. This older work supports the
same broad motivation: texture can reserve hue for another channel or combine
with hue for redundant encoding.

Implication for Avenger: the useful product feature is not "replace color with
patterns"; it is "compose pattern with color without losing legend and backend
parity."

Source: <https://healey.csc.ncsu.edu/publications/15833.pdf>

## Proposed Pattern Grammar

The low-level model should be a paint extension with a separate pattern
registry, analogous to gradients:

```rust
pub enum Paint {
    Color([f32; 4]),
    GradientIndex(u32),
    PatternIndex(u32),
}

pub struct PatternPaint {
    pub background: Option<PatternBackground>,
    pub foreground: [f32; 4],
    pub opacity: f32,
    pub definition: PatternDefinition,
    pub tile: PatternTile,
}

pub enum PatternDefinition {
    Lines(LinePattern),
    Dots(DotPattern),
    Grid(GridPattern),
    Checker(CheckerPattern),
    Glyph(GlyphPattern),
    Image(ImagePattern),
}
```

The exact names can change. The important split is:

- `Paint` references a reusable pattern definition from scenegraph marks.
- `PatternPaint` owns foreground/background composition.
- `PatternDefinition` describes primitive geometry.
- `PatternTile` describes spacing, size, rotation, phase, and origin policy.

The chart authoring layer can expose a simpler v1 surface:

```rust
Rect::new()
    .x("x")
    .y("y")
    .y2(0.0)
    .fill(col("group"))
    .fill_pattern(col("group"));
```

For v1, `fill_pattern` should be categorical only. Pattern density, pattern
size, and semantically resonant custom palettes can follow after the rendering
contract is proven.

## Chart API Questions

### Separate Channel Or Paint Value

There are two plausible user-facing models:

- `fill(pattern(...))`: pattern is a paint value that replaces color.
- `fill(...) + fill_pattern(...)`: pattern overlays or composes with color.

The second model fits accessibility and redundant encoding better. The first
model fits renderer internals better. A reasonable compromise is separate chart
channels that lower into scenegraph `Paint::PatternIndex` values whose pattern
definition carries background paint.

### Pattern Scales

Add a `PatternChannelConfig` and eventually a `ScaleRange::Pattern`. The
default range should be a short, conservative categorical cycle:

- no pattern / diagonal lines,
- opposite diagonal lines,
- horizontal lines,
- vertical lines,
- cross,
- diagonal cross,
- dots,
- grid/checker.

The exact cycle needs visual testing. Avoid claiming optimality.

### Legends

Pattern legends need swatches rendered by the same scenegraph path as marks.
Open policy decisions:

- If the same field maps to color and pattern, should one combined legend key
  show both encodings?
- If different fields map to color and pattern, should Avenger render separate
  legends?
- Should pattern legends default to larger swatches than color legends because
  texture needs area?

### Defaults

Defaults should be restrained:

- pattern foreground: near-black with configurable opacity,
- pattern background: inherited fill color by default,
- minimum screen-space spacing to avoid moire,
- no automatic pattern channel unless an accessibility mode or explicit channel
  asks for it.

## SVG Rendering

SVG has native `<pattern>` support, so the SVG path should emit pattern
definitions through the existing `SvgDefs` mechanism:

- deduplicate pattern definitions by hash/equality,
- emit `<pattern>` in `<defs>`,
- fill marks with `fill="url(#svg-pattern-N)"`,
- keep gradients and clips independent,
- render legend swatches as ordinary rect/symbol scenegraph marks.

Use `patternUnits="userSpaceOnUse"` for v1 so pattern spacing remains stable in
screen/logical pixels. Object-bounding-box patterns scale with each mark and can
make identical categories look different across small and large marks. A later
option can add object-local patterns if a use case needs them.

For built-ins, SVG definitions can be simple:

- line hatches: a line in a tile, with `patternTransform` for rotation,
- dots: circle primitives in a tile,
- checker/grid: rect or line primitives,
- glyph patterns: path or text/glyph-like primitives, if supported later.

The PDF renderer currently goes through SVG. Pattern support should therefore
include a `svg2pdf` verification step. If a native SVG pattern does not survive
PDF conversion for a target pattern type, the fallback should be rasterizing the
pattern tile into an embedded image pattern rather than changing chart
semantics.

## WGPU Rendering

WGPU has no native pattern fill. The two viable paths are:

### Procedural Built-Ins

Implement built-in line, dot, grid, checker, and cross patterns directly in
WGSL using fragment coordinates, tile parameters, and derivative-aware
antialiasing.

Advantages:

- crisp output at high DPI,
- no atlas upload for simple patterns,
- easy recoloring from foreground/background uniforms or attributes,
- good parity with SVG for the built-in grammar.

Costs:

- current vertex color encoding is too small for pattern id, colors, spacing,
  rotation, opacity, and origin;
- `MultiVertex` and shader inputs would need a richer paint representation;
- branching in the fragment shader needs profiling.

### Pattern Tile Atlas

Rasterize each distinct pattern paint into a small RGBA tile on the CPU, pack
tiles into an atlas, and sample with `fract(...)` based on fragment coordinates.

Advantages:

- supports more custom pattern types earlier,
- shader can be simpler,
- aligns with existing gradient/image atlas concepts.

Costs:

- recoloring creates more distinct atlas entries unless the tile stores a mask,
- small tiles can blur or shimmer under scaling,
- atlas sampling needs metadata for subrect, tile size, origin, and transform,
- repeated sampling cannot simply use existing image texture coordinates because
  the pattern must repeat inside arbitrary tessellated fills.

### Recommended WGPU Direction

Start with procedural built-ins and keep custom image/SVG patterns out of v1.
The WGPU renderer already passes fragment pixel coordinates and mark bounding
boxes for gradient evaluation. Pattern evaluation can similarly use fragment
coordinates plus tile metadata. This keeps the first implementation portable to
WebGPU/WebGL limits and makes the built-in SVG and WGPU outputs easier to
compare.

The implementation probably needs a new paint encoding instead of overloading
`vec4 color` further. Candidate designs:

- add a `paint_kind` plus packed paint indices to `MultiVertex`,
- keep solid colors in vertex attributes and store pattern parameters in a small
  uniform/storage-like table indexed by paint id,
- or use a metadata texture, similar in spirit to the current gradient atlas,
  but with enough precision for tile sizes and transforms.

## Open Decisions

- Should pattern phase be chart-global, mark-local, or configurable?
- Should pattern spacing be in logical pixels, device pixels, or mark-local
  normalized units?
- Should v1 support pattern strokes, or only fills?
- Should `ColorOrGradient` be generalized to `Paint`, or should a new enum be
  introduced while preserving compatibility?
- How should pattern opacity combine with mark `opacity` and color alpha?
- What is the minimum default tile size that avoids moire in PNG, SVG, and PDF?
- How should pattern scales interact with color scales when both map the same
  field?
- How should custom user patterns be serialized across Rust, Python, WASM, SVG,
  and WGPU?

## Suggested V1 Scope

1. Add scenegraph support for `Paint::PatternIndex` and a `patterns` registry on
   filled scene marks.
2. Add a small built-in geometric pattern set: diagonal lines, opposite
   diagonal lines, horizontal lines, vertical lines, cross, diagonal cross,
   dots, and grid/checker.
3. Add SVG native pattern rendering with `userSpaceOnUse` tiles.
4. Add WGPU procedural rendering for the same built-ins.
5. Add chart-level `fill_pattern` categorical channel and pattern legend
   swatches.
6. Add visual baselines for rect, symbol, path, area, arc/pie, legend keys,
   facets, SVG export, PNG export, and PDF conversion if enabled.

Leave these out of v1:

- image patterns,
- arbitrary SVG pattern imports,
- semantic pattern inference,
- quantitative density scales,
- animated pattern phase,
- pattern fills for strokes,
- object-bounding-box pattern units.

## Validation Checklist

- Pattern swatches remain distinguishable in grayscale and under common
  color-vision-deficiency simulations.
- Pattern scale is stable across different mark sizes, facets, and exported
  PNG/SVG/PDF outputs.
- Small marks degrade gracefully rather than showing noisy fragments.
- Rounded rects, clipped paths, polar arcs, and symbols clip pattern primitives
  correctly.
- Combined color+pattern legends show the same composition as data marks.
- WGPU and SVG visual baselines match closely enough for the built-in pattern
  set.
- Dense patterns do not create unacceptable moire at 1x and high-DPI scales.

## Readiness

Ready for design spike. The scenegraph boundary is clear and SVG rendering is
straightforward, but WGPU paint encoding and chart legend semantics should be
prototyped before committing to public API names.
