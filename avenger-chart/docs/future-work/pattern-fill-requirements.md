# Pattern Fill Requirements

## Status

Draft requirements and serde contract. This document defines the required
behavior, data model, chart integration, theme integration, validation rules,
and renderer-facing contracts for pattern fill overlays. It is not an
implementation plan and does not attempt to choose internal module boundaries,
task sequencing, or renderer algorithms.

This document is narrower than the full pattern grammar explored in
[`pattern-grammar-api.md`](pattern-grammar-api.md). The v1 implementation can
start with 1D stripe overlays, while the enum is shaped so 2D symbol lattice
layers can be added without changing the top-level model.

Pattern rendering is part of the renderer-facing scenegraph contract. Any
pattern layer family included in an implementation milestone must be supported
by all three render paths: `avenger-svg`, `avenger-wgpu`, and `avenger-pdf`.
A milestone can stage which marks or layer families are implemented, but it
should not ship as an SVG-only, wgpu-only, or PDF-only feature.

## V1 Non-Goals

The first implementation should stay focused on geometric mark fills with a
shared pattern ink and renderer-identical stripe geometry. These are explicitly
out of scope for v1:

- image or raster texture pattern fills;
- gradient pattern ink, even though host mark fills may already be gradients;
- per-layer ink, per-layer opacity, or data-encoded pattern ink;
- pattern fills for text glyphs, group backgrounds, line-like marks, rules,
  trails, or images;
- continuous or interpolated pattern scales;
- chart-level arc authoring, unless a chart `Arc` mark is added separately.

## Design Model

A pattern fill is an overlay on top of a normal mark fill.

```text
visible mark fill = solid/gradient fill + clipped pattern overlay
pattern overlay   = shared ink + ordered pattern layers
pattern layer     = one self-contained pattern-generating primitive family
```

The initial layer families are:

- `Stripe`: a 1D set of parallel hatch lines.
- `Symbol`: a 2D lattice of repeated symbols. This can be implemented later,
  but should be included in the type model from the start.

Crosshatch is not its own primitive. It is two `Stripe` layers.

## Serde Conventions

The Rust types and serde attributes are the source of truth for serialization.
This document should not duplicate a JSON schema. It only calls out conventions
that affect compatibility:

- Rust fields use `snake_case`.
- Serialized fields and enum variants use `kebab-case`.
- Enums with data use an internally tagged representation with a `type` field.
- `#[serde(default)]` and `skip_serializing_if` choices are part of the
  compatibility contract.
- Numeric distances are logical display pixels unless otherwise named.
- Angles are degrees.
- Colors reuse Avenger's normalized RGBA color representation where practical.

Serialized examples should be avoided unless the shape is not obvious from the
Rust type, such as CSS theme parsing.

Because scene marks derive or manually implement `Hash`, every new pattern
field must have a stable hash implementation. Pattern structs that contain
floating-point values should hash floats through the same `OrderedFloat` style
used by existing scenegraph marks rather than relying on derives that cannot
handle raw `f32` fields. `ScalarOrArray<Option<PatternFill>>` should be
registered with the existing scalar/array hash helper macro.

## Requirement Layers

Pattern support has two integration layers:

- `avenger-scenegraph`: renderer-facing mark fields that carry resolved
  `PatternFill` values.
- `avenger-chart`: authoring, channel, scale, theme, and legend interfaces that
  resolve to the scenegraph fields.

The scenegraph layer should not know about CSS themes, palette lookup, chart
scales, or chart legends. The chart layer should not require renderers to know
which data expression produced a pattern.

## Renderer Parity Requirements

The three renderers should draw visually identical patterns from the same
`PatternFill` values. The requirement is visual identity in logical scene
coordinates, with normal anti-aliasing tolerance between backends. It is not a
requirement for renderers to use the same native rendering primitive.

The implementation should keep pattern resolution in shared scenegraph or
renderer-support code wherever possible:

- resolve `PatternAnchor` to a concrete origin and clip region;
- resolve `PatternInk` to a concrete RGBA value after the host fill is known;
- expand each `PatternLayer` into renderer-neutral geometry conventions;
- union layer coverage into a non-accumulating pattern mask before applying ink
  opacity;
- apply legend-only phase shifts outside the mark's stored `PatternFill`;
- preserve layer order exactly.

SVG, wgpu, and PDF can then choose the most appropriate backend primitive. SVG
may use `<pattern>` or clipped generated paths. wgpu may expand stripes on the
CPU or in a shader. PDF may use native pattern facilities or generated vector
paths. Those choices are implementation details, but they must follow the same
geometry equations, anchor rules, non-accumulating opacity rules, clipping
rules, and legend-centering rules.

If a renderer cannot exactly express a native tiling primitive, it should expand
the pattern into the shared renderer-neutral geometry and clip it to the host
mark rather than inventing backend-specific approximations. A renderer should
not silently drop supported pattern layers. Unsupported layer families in an
early milestone should produce explicit diagnostics during lowering or
rendering.

Renderer acceptance should include the same fixture set rendered through SVG,
wgpu, and PDF, covering stripe angles, spacing, stroke width, phase, dash,
opacity, clipping, marks with adjacent same-pattern fills, legends, and facet or
plot anchors.

## Scenegraph Requirements

Pattern types should live with the scenegraph mark model, for example in
`avenger_scenegraph::marks::pattern`, and be re-exported by `avenger-chart` for
chart authors.

All fillable scene marks should expose an optional pattern overlay alongside
their existing fill paint. The overlay does not replace the fill. It is drawn
over the fill and clipped to the host mark geometry.

Scenegraph marks that use `ScalarOrArray<ColorOrGradient>` for item-wise fill
should use the analogous scalar/array representation for pattern overlays:

```rust
#[serde(default = "default_no_fill_pattern", skip_serializing_if = "is_no_fill_pattern")]
pub fill_pattern: ScalarOrArray<Option<PatternFill>>;
```

Scenegraph marks whose fill has already been partitioned to a scalar style can
use a scalar option:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub fill_pattern: Option<PatternFill>;
```

The initial scene mark support should be:

| Scene mark | Field shape | Notes |
| --- | --- | --- |
| `SceneRectMark` | `ScalarOrArray<Option<PatternFill>>` | Primary v1 target for bars and rects. |
| `SceneArcMark` | `ScalarOrArray<Option<PatternFill>>` | Needed for pies, donuts, and radial sectors. |
| `SceneSymbolMark` | `ScalarOrArray<Option<PatternFill>>` | Pattern clips to the host symbol shape. This is independent of `SymbolPatternLayer`. |
| `ScenePathMark` | `ScalarOrArray<Option<PatternFill>>` | Pattern clips to each path item. |
| `SceneAreaMark` | `Option<PatternFill>` | Area compilation already partitions varying scalar styles. |

`SceneGroup` background fills and `SceneTextMark` glyph fills are out of scope
for the initial pattern-fill requirements and the first follow-on implementation
after geometric filled marks. Line-like marks (`SceneLineMark`, `SceneRuleMark`, and
`SceneTrailMark`) do not get `fill_pattern` because they are stroke-oriented.
Images do not get `fill_pattern`.

Scenegraph drawing order for a filled mark should be:

1. Fill paint.
2. Pattern overlay, clipped to the same host geometry.
3. Stroke paint.

`None` means no overlay. `Some(PatternFill { layers: [] })` is valid but should
render the same as no overlay. Serialization should use `kebab-case` field
names, so scenegraph JSON uses `fill-pattern`.

Renderers must resolve `PatternAnchor` at draw time:

- `PatternAnchor::Plot` uses the current plot or facet plot area coordinate
  context supplied by chart lowering.
- `PatternAnchor::Mark` uses the current mark item bounding box.
- `PatternAnchor::Chart` uses the current scene or chart viewport.

The scenegraph mark data only carries the requested anchor. It should not carry
legend-specific phase shifts or palette metadata.
Any scenegraph metadata needed to resolve `PatternAnchor::Plot` should lower to
a neutral renderer-facing reference frame, such as a rectangular plot region in
absolute scene coordinates on display-list items. `SceneGroup` may author that
region in group-local coordinates as long as display-list construction
translates it to the absolute coordinates consumed by renderers. It must not
expose chart scale, legend, palette, or expression semantics to renderers.

## Avenger Chart Requirements

`avenger-chart` should expose a first-class `fill_pattern` channel on the
chart marks that lower to the scenegraph marks above:

| Chart mark | Supported |
| --- | --- |
| `Rect` | Yes |
| `Arc` | No in v1; no chart-level arc authoring mark exists today. |
| `Symbol` | Yes |
| `PathMark` | Yes |
| `Area` | Yes |
| `Text` | Later |
| `Line`, `Rule`, `Trail`, `Image` | No |

For marks that support both `fill` and `fill_pattern`, the interfaces should
compose:

```rust
Rect::new()
    .fill(col("category"))
    .fill_pattern(col("category"));
```

Using the same expression for `fill` and `fill_pattern` is the normal
redundant-encoding case. Using separate expressions is allowed and creates
separate scales and legends unless guide merging rules say otherwise:

```rust
Rect::new()
    .fill(col("temperature"))
    .fill_pattern(col("scenario"));
```

Direct literal patterns should be possible without a scale:

```rust
let hatch = PatternFill::new()
    .with_layer(StripePatternLayer::new(45.0, 16.0, 1.25));

Rect::new()
    .fill("#f8fafc")
    .fill_pattern(hatch);
```

Disabling a pattern explicitly should also be possible:

```rust
Rect::new()
    .fill("#f8fafc")
    .fill_pattern(None::<PatternFill>);
```

Because `PatternFill` is structured data, the chart integration should use a
pattern-specific channel value wrapper rather than forcing patterns through
`ScalarValue`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PatternChannelValue {
    /// A direct renderer-facing pattern value. `None` disables the overlay.
    Value {
        value: Option<PatternFill>,
    },

    /// A data expression resolved through the fill-pattern scale.
    Channel {
        value: ChannelValue,
    },
}

impl From<PatternFill> for PatternChannelValue {
    fn from(value: PatternFill) -> Self {
        Self::Value { value: Some(value) }
    }
}

impl From<Option<PatternFill>> for PatternChannelValue {
    fn from(value: Option<PatternFill>) -> Self {
        Self::Value { value }
    }
}

impl From<Expr> for PatternChannelValue {
    fn from(value: Expr) -> Self {
        Self::Channel {
            value: value.into(),
        }
    }
}
```

The mark builder method should therefore be pattern-specific:

```rust
impl<C> Rect<C> {
    pub fn fill_pattern<V: Into<PatternChannelValue>>(self, value: V) -> Self;

    pub fn fill_pattern_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<PatternChannelValue>,
        F: FnOnce(PatternChannelConfig) -> PatternChannelConfig;
}
```

`PatternChannelConfig` should mirror the scale, domain-coordination, and legend
configuration surface used by other channels, but its default scale range is a
pattern range:

```rust
pub struct PatternChannelConfig {
    value: PatternChannelValue,
}

impl PatternChannelConfig {
    pub fn scale(self, scale: Scale<Auto>) -> Self;
    pub fn scale_name(self, name: impl Into<String>) -> Self;
    pub fn domain_group(self, group: impl Into<String>) -> Self;
    pub fn legend(self, legend: Legend) -> Self;
    pub fn no_legend(self) -> Self;
}

pub enum ScaleRange {
    // existing variants...
    Pattern(Vec<Option<PatternFill>>),
}

impl Scale<Auto> {
    pub fn range_patterns(mut self, values: Vec<Option<PatternFill>>) -> Self;
}
```

Chart specs should use `fill_pattern` as the channel name. The serde-tagged
`PatternChannelValue` distinguishes literal pattern values from scaled channel
values, so no separate chart-channel wire format is needed.

### Chart Lowering Storage Requirements

The existing chart data-context model stores ordinary visual encodings in
`IndexMap<String, ChannelValue>`. That is still the right model for scalar
expressions and scale-bearing inputs, but `PatternFill` is structured renderer
data and should not be coerced into `ScalarValue`, JSON strings, or synthetic
dataframe columns.

The chart lowering model should add a typed sidecar for pattern channels, for
example:

```rust
pub struct DataContext {
    // existing scalar, expression-backed channels
    pub channels: IndexMap<String, ChannelValue>,

    // new structured pattern channels
    pub pattern_channels: IndexMap<String, PatternChannelValue>,
}
```

`DataContext` itself is a live authoring/lowering structure, not a serde
contract. `CompiledDataContext` should carry the same sidecar with
`#[serde(default, skip_serializing_if = "IndexMap::is_empty")]` so older
serialized specs remain readable and newly serialized specs do not gain empty
pattern-channel fields. The sidecar should be specific enough to avoid
pretending that all future structured channels are ordinary `ScalarValue`s, but
it can be generalized later if more structured visual channels appear.

Scale, domain, and legend discovery should not need to know where every channel
is physically stored. Add an iterator or adapter such as
`scale_channels()`/`legend_channels()` that yields:

- existing `channels` entries;
- `pattern_channels` entries whose value is `PatternChannelValue::Channel`;
- no entry for literal `PatternChannelValue::Value`, because literals do not
  build scales.

During compilation:

- `PatternChannelValue::Value { value }` lowers directly to the scenegraph
  `fill_pattern` field.
- `PatternChannelValue::Channel { value }` participates in discrete domain
  resolution through the wrapped `ChannelValue`, then maps each domain value to
  `Option<PatternFill>` through `ScaleRange::Pattern`.
- pattern outputs stay out of DataFusion scalar evaluation. Only the input
  expression used to compute the discrete domain participates in query and
  extent logic.
- `fill` and `fill_pattern` using the same expression should share the same
  resolved domain ordering so theme fill palettes and pattern palettes remain
  index-aligned. This should be an explicit domain-coordination rule, not just
  a legend-merge check after two independent domains happen to match.

### Pattern Scale Semantics

`ScaleRange::Pattern` should be treated as range material, parallel to
`ScaleRange::Color`. It should not encode scale behavior by itself.

The current scale model separates:

- the scale type, which decides how input values are mapped into the range
  (`ordinal`, `quantize`, `quantile`, `threshold`, `linear`, and so on);
- the scale range, which provides the output values available to the scale
  (`Numeric`, `Discrete`, `Color`, and proposed `Pattern`).

For v1, `Pattern` should mean a discrete pattern range:

```rust
pub enum ScaleRange {
    // existing variants...
    Pattern(Vec<Option<PatternFill>>),
}
```

This is enough for ordinal pattern encodings:

- categorical input + `ordinal` scale maps ordered domain entries to pattern
  entries by index;
- numeric input + `quantize`, `quantile`, or `threshold` scale maps numeric
  values into discrete pattern bins;
- ordered categorical input can use the existing domain ordering machinery so
  denser patterns correspond to larger ordered values.

An ordinal density palette can therefore be represented as a normal pattern
range whose entries get progressively denser, for example by decreasing stripe
spacing while holding angle, stroke width, and ink constant. A theme can define
this today as an explicit `fill-pattern-discrete` list for a known
cardinality.

Parameterized pattern generators and continuous numeric pattern scales are
future work, not v1 requirements. A future theme function such as
`stripe-density(angle 45deg, spacing 24px 8px, stroke-width 1px)` could generate
the concrete `Vec<Option<PatternFill>>` needed by an ordinal or quantize scale
after cardinality and ordering are known. A future continuous pattern scale
would require a scale evaluation path parallel to color scaling, for example a
`scale_to_pattern` operation that can ask a continuous scale type to turn input
values into concrete `PatternFill` values. That interpolation is only
well-defined for compatible pattern families, such as stripe-density ramps
where layers differ only in spacing, phase, stroke width, or similar numeric
fields. Arbitrary `PatternFill` values should not be interpolated implicitly.

Chart compilation rules:

- A literal `PatternChannelValue::Value { value }` lowers directly to a scalar
  scenegraph `fill_pattern`.
- A scaled `PatternChannelValue::Channel { value }` builds a discrete pattern
  scale. Continuous pattern scales are out of scope for v1.
- If no explicit pattern range is provided, theme resolution queries
  `fill-pattern-discrete`.
- If `fill` and `fill_pattern` use the same expression and compatible discrete
  domains, chart lowering should use the same domain ordering so color and
  pattern palette entries remain paired.
- If the resolved pattern for an item is `None`, the scenegraph item receives no
  overlay.
- Pattern channel values should be excluded from numeric extent computation
  except for their scale-domain input expression, just like other non-position
  visual channels.

Legend rules belong to `avenger-chart`, not `PatternFill`:

- If `fill` and `fill_pattern` redundantly encode the same expression with
  compatible discrete domains, they should share one merged legend. The sample
  shows the resolved fill color with the resolved pattern overlay.
- If only `fill_pattern` is present, render a pattern legend using the theme's
  neutral legend fill background.
- Pattern legend rectangles should default to larger chips than fill-only color
  rectangles, similar to how line-dash legends use wider samples than
  stroke-only line legends.
- `PatternLegendConfig` controls sample size and legend-local centering. Merged
  fill + pattern legends use pattern legend sample sizing.

## Core Types

```rust
pub type Px = f32;
pub type Deg = f32;
pub type Alpha = f32;
pub type Rgba = [f32; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PatternFill {
    #[serde(default)]
    pub anchor: PatternAnchor,

    #[serde(default)]
    pub ink: PatternInk,

    #[serde(default)]
    pub layers: Vec<PatternLayer>,
}
```

### Defaults

```rust
impl Default for PatternFill {
    fn default() -> Self {
        Self {
            anchor: PatternAnchor::Plot,
            ink: PatternInk::AutoContrast { opacity: 0.18 },
            layers: Vec::new(),
        }
    }
}
```

An empty `layers` list is valid and equivalent to no pattern overlay. Chart
authoring helpers should usually avoid constructing empty pattern fills.

## Anchor

The anchor controls where the repeated pattern coordinate system starts.

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatternAnchor {
    /// Default. Pattern coordinates are shared across the plot area and marks
    /// clip the same infinite pattern. Adjacent same-pattern bars visually
    /// continue through gaps.
    #[default]
    Plot,

    /// Pattern coordinates are local to each mark's bounding box. The pattern
    /// restarts for every mark instance.
    Mark,

    /// Pattern coordinates are shared across the full chart or scene viewport.
    Chart,
}
```

Recommended default: `Plot`.

`Mark` is useful for tiny swatches or glyph-like marks where the pattern should
be self-contained.

## Ink

Pattern ink is independent of data color. It describes the hatch/symbol paint
used by all layers in a `PatternFill`.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PatternInk {
    /// Choose dark or light neutral ink from the rendered host fill luminance.
    AutoContrast {
        #[serde(default = "default_pattern_opacity")]
        opacity: Alpha,
    },

    /// Explicit ink color. Alpha in `color` is multiplied by `opacity`.
    Solid {
        color: Rgba,
        #[serde(default = "default_pattern_opacity")]
        opacity: Alpha,
    },
}

impl Default for PatternInk {
    fn default() -> Self {
        Self::AutoContrast {
            opacity: default_pattern_opacity(),
        }
    }
}

fn default_pattern_opacity() -> Alpha {
    0.18
}
```

Validation:

- `opacity` must be in `[0, 1]`.
- `AutoContrast` resolves per rendered mark item after the host fill is known.
  A single `PatternFill` can therefore produce different concrete ink colors
  for different items when the host fill is item-wise.
- `AutoContrast` should choose a neutral dark ink for light host fills and a
  neutral light ink for dark host fills using the existing relative-luminance
  helper. The exact dark/light neutral values and threshold should be shared by
  all renderers.
- If the host fill is a gradient, `AutoContrast` should not inspect every pixel
  of the gradient in v1. It should use a documented deterministic fallback,
  such as the first gradient stop or average stop luminance, and all renderers
  must use the same fallback.
- Pattern ink is shared across all layers in a `PatternFill`; per-layer ink is
  out of scope.
- Pattern ink is not a data channel in this model.
- Pattern ink opacity is non-accumulating within a `PatternFill`. Renderers
  should union the coverage of all pattern layers into an isolated pattern mask
  and apply `PatternInk` opacity once when compositing the pattern over the host
  fill. Overlapping layers, such as crosshatch intersections, should not become
  darker than non-overlapping pattern strokes.

## Layers

`PatternLayer` is the intended extension point. Each variant owns its full
configuration; invalid cross-family fields are not shared at the top level.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PatternLayer {
    Stripe(StripePatternLayer),
    Symbol(SymbolPatternLayer),
}
```

## Stripe Layer

A stripe layer is a 1D lattice of parallel lines.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StripePatternLayer {
    /// Line orientation in degrees.
    pub angle: Deg,

    /// Distance between neighboring stripe centerlines.
    pub spacing: Px,

    /// Stroke width of each stripe.
    pub stroke_width: Px,

    /// Offset along the stripe normal. Useful for palette tuning and
    /// deterministic alignment.
    #[serde(default)]
    pub phase: Px,

    /// Optional dash pattern applied along each stripe line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<StripeDash>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StripeDash {
    pub length: Px,
    pub gap: Px,

    /// Offset along the stripe direction in the shared pattern coordinate
    /// system. This does not restart for each clipped stripe segment.
    #[serde(default)]
    pub phase: Px,
}
```

Validation:

- `spacing > 0`.
- `stroke_width > 0`.
- `stroke_width < spacing` is recommended and should be warned on if violated.
- `dash.length > 0` and `dash.gap > 0`.
- Angles may be any finite degree value, but categorical palettes should prefer
  a small set such as `0`, `45`, `90`, and `135`.

### Stripe Geometry Conventions

Stripe geometry must be specified once and shared by all renderers. A stripe
layer is defined in logical display pixels in the resolved pattern coordinate
system.

Angle convention:

- `angle` is the direction of each stripe centerline.
- Angles are degrees clockwise from the positive x axis in screen/display
  coordinates, where x increases rightward and y increases downward.
- `0` means horizontal stripes.
- `90` means vertical stripes.
- `45` means stripes running down and right.
- `135` means stripes running down and left.

For an angle `theta`, define:

```text
d = [cos(theta), sin(theta)]       // unit direction along the stripe
n = [-sin(theta), cos(theta)]      // unit normal between stripes
```

Given a resolved pattern origin `origin`, stripe centerlines satisfy:

```text
dot(n, p - origin) = phase + k * spacing
```

where `k` is any integer. `spacing` is the perpendicular distance between
neighboring stripe centerlines. Positive `phase` moves centerlines in the
positive `n` direction. Renderers may reduce `phase` modulo `spacing` for
efficiency, but the visual result must be unchanged.

Stroke convention:

- `stroke_width` is centered on each stripe centerline.
- Stroke caps are butt caps for generated stripe segments.
- The visual result should be equivalent whether a renderer emits stroked
  paths, filled quads, or shader coverage.
- Stripe geometry is clipped to the host mark geometry after layer generation.

Coverage generation:

- Determine the host clip bounds in the resolved pattern coordinate system.
- Project the bounds corners onto `n`.
- Generate every integer `k` whose centerline could intersect the bounds after
  expanding by `stroke_width / 2`.
- For each centerline, generate a segment long enough to cover the expanded
  bounds in direction `d`, then clip to the host geometry.
- Filled stripe quads from all layers in a `PatternFill` represent one coverage
  mask. If the shared helper returns a compound vector path, subpaths must use
  a fill-rule/winding convention that makes overlapping stripe quads additive
  coverage, not holes. A nonzero-fill compound path should therefore append all
  quad subpaths with consistent winding. Equivalent implementations may use a
  true boolean union or an isolated opacity group/mask, but they must produce
  the same non-accumulating coverage result.

Dash convention, when `dash` is present:

```text
dash_period = dash.length + dash.gap
visible when mod(dot(d, p - origin) - dash.phase, dash_period) < dash.length
```

`dash.phase` is measured in the shared pattern coordinate system, not from the
start of each generated or clipped stripe segment. A phase of `0` starts each
stripe's dash pattern at its intersection with the dash-origin line through
`origin`, where `dot(d, p - origin) = 0`. That dash-origin line is perpendicular
to the stripes and parallel to `n`.

Positive `dash.phase` moves the start of each visible dash interval along the
positive stripe direction `d`. All stripes in a layer share the same dash
origin and phase, so dashed hatches remain seamless across adjacent marks when
the pattern anchor is shared. Dashes do not restart at mark boundaries, clip
boundaries, renderer-generated segment endpoints, or legend sample edges.

Dashes use the same butt-cap convention as stripe segment ends. A separate dash
front angle, per-stripe stagger, or full 2D dash lattice is out of scope for v1;
if needed later, it should be added explicitly rather than inferred from backend
path start behavior.

Renderers should not apply backend-specific device-pixel snapping to stripe
centerlines. Patterns are defined in logical scene coordinates and then
rasterized or emitted by each backend. Pixel snapping can be considered later as
an explicit shared rendering option, but it is not part of v1.

For `PatternAnchor::Mark`, the origin is the normalized host item bounding-box
minimum corner. For `PatternAnchor::Plot`, the origin is the plot-area minimum
corner. For `PatternAnchor::Chart`, the origin is the chart or scene viewport
minimum corner. Negative-size host geometry must be normalized before resolving
mark-local origins and clip bounds.

## Symbol Layer

A symbol layer is a 2D lattice of repeated mark-language symbols. It is not
required for the first renderer implementation, but including it in the enum
keeps the requirements from baking in a stripe-only world.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SymbolPatternLayer {
    pub lattice: SymbolLattice2d,
    pub symbol: PatternSymbol,
    pub paint: SymbolPaint,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SymbolLattice2d {
    /// First basis vector spacing and angle.
    pub u_spacing: Px,
    pub u_angle: Deg,

    /// Second basis vector spacing and angle.
    pub v_spacing: Px,
    pub v_angle: Deg,

    #[serde(default)]
    pub u_phase: Px,

    #[serde(default)]
    pub v_phase: Px,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PatternSymbol {
    /// Shape string accepted by the mark-language symbol parser. This lowers
    /// through `SymbolShape::from_vega_str`, so pattern symbols and symbol
    /// marks share one vocabulary.
    pub shape: SymbolShapeSpec,

    /// Symbol size in display pixels. This should mean the same thing as
    /// symbol mark size after lowering, or be explicitly documented if not.
    pub size: Px,

    #[serde(default)]
    pub rotation: Deg,
}

/// Serialized as a string such as `"circle"`, `"triangle-up"`, or an SVG path
/// string if custom path symbols are enabled for pattern rendering.
pub type SymbolShapeSpec = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SymbolPaint {
    /// Filled with pattern ink, no symbol stroke.
    Filled,

    /// No fill, stroked with pattern ink.
    Open {
        stroke_width: Px,
    },
}
```

Validation:

- `u_spacing > 0` and `v_spacing > 0`.
- `symbol.shape` must parse with `SymbolShape::from_vega_str`, including named
  shapes and SVG path strings.
- `symbol.size > 0`.
- For `Open`, `stroke_width > 0`.
- Symbol paint uses `PatternFill::ink`; it does not introduce a per-symbol
  data color.

The recognized named shapes should match the symbol mark parser: `circle`,
`square`, `cross`, `diamond`, `triangle`, `triangle-up`, `triangle-down`,
`triangle-left`, `triangle-right`, `arrow`, `wedge`, `star`, `wye`, `pentagon`,
`cushion`, and `concave-square`. Custom SVG path strings can use the same field
unless implementation shows that supporting them is significantly more complex
than named shapes.

## Legend Rendering

Legend samples need enough area to show the actual pattern, but Avenger should
also support fixed-size legend chips. Legend sample geometry is a property of
legend construction and theme defaults, not `PatternFill`. The pattern object
should not carry legend-only fields.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PatternLegendConfig {
    #[serde(default = "default_pattern_legend_sample_size")]
    pub sample_size: [Px; 2],
}

impl Default for PatternLegendConfig {
    fn default() -> Self {
        Self {
            sample_size: default_pattern_legend_sample_size(),
        }
    }
}

fn default_pattern_legend_sample_size() -> [Px; 2] {
    [32.0, 32.0]
}
```

The default pattern legend sample should be visibly larger than the existing
fill-only color sample. If the current rect legend implementation represents
size as symbol area rather than explicit width and height, pattern legends
should add an explicit sample width/height path instead of trying to overload
the color-only `symbol-size` meaning.

Merged `fill` + `fill_pattern` legends use the pattern sample size and draw the
resolved fill first, then the pattern overlay, then any sample outline. A
pattern-only legend uses a neutral theme background so the ink remains visible.
The primary legend title and event metadata should prefer `fill` when `fill` is
present in the merged legend; otherwise they use `fill_pattern`. The merged
legend item should still retain the complete set of participating channels for
interaction and accessibility metadata.

Legend merging should reuse the existing discrete `MergeKey` concept:
normalized expression, exact discrete domain values, and mark index. Add
`fill_pattern` to the rect legend renderer's evaluable and mergeable channel
set. A redundant `fill` + `fill_pattern` legend merges only when both channels
have discrete scales and the same merge key. If the pattern range contains a
`None` entry, the merged item draws only the fill for that domain value.

Legend construction should render a legend-local display copy of the pattern
with the center of the legend sample treated as the pattern origin. The adjusted
copy is only for the legend sample and must not mutate the `PatternFill` used by
marks. This does not require another `PatternAnchor`: the legend still uses
`PatternAnchor::Mark` because the sample is local to the legend chip. The
centering behavior is a legend-construction rule.

For stripe layers, if the renderer defines stripe centerlines by:

```text
dot(stripe_normal(angle), point - origin) = phase + k * spacing
```

then a legend sample of size `[w, h]` can be centered by rendering a display-only
copy with:

```text
legend_phase = phase + dot(stripe_normal(angle), [w / 2, h / 2])
```

modulo `spacing`.

This makes `phase: 0` place a stripe centerline through the legend center.
For a crosshatch, both stripe layers receive their own centered phase adjustment,
so the crossing point lands at the legend center. `phase: spacing / 2` instead
centers the gap between neighboring stripes.

For 2D symbol layers, the same rule applies conceptually: shift the legend-local
lattice origin so the legend sample center is the lattice origin, then preserve
the layer's `u_phase` and `v_phase` relative to that centered origin.

Theme defaults should coordinate legend square size and palette spacing. For a
32px square legend chip, categorical stripe palettes should prefer spacings such
as `16px` and `8px`, but that is a palette-design guideline rather than a
renderer contract.

## CSS Theme Integration

This section is the theming-specific part of the pattern requirements. The Rust
types and serde attributes define the canonical in-memory and serialized model;
CSS only defines how theme authors write pattern palette entries that lower into
those types.

Pattern palettes need to be theme-level resources, not only Rust-side builder
values. This should mirror how fill palettes are currently specified in CSS
themes with discrete range properties such as `fill-discrete`.

The channel name should be `fill_pattern` in Rust and serialized chart specs.
The CSS theme property should use the existing underscore-to-hyphen convention:
`fill-pattern-discrete`.

```css
mark[type="rect"][cardinality="4"] {
    fill-discrete:
        #000000, #E69F00, #CC79A7, #56B4E9;

    fill-pattern-discrete:
        {
            ink: { type: solid; color: white; opacity: 0.28; };
            layers: [
                { type: stripe; angle: 0deg; spacing: 16px; stroke-width: 1.25px; }
            ];
        },
        {
            ink: { type: solid; color: black; opacity: 0.16; };
            layers: [
                { type: stripe; angle: 45deg; spacing: 16px; stroke-width: 1.25px; }
            ];
        },
        {
            ink: { type: solid; color: black; opacity: 0.13; };
            layers: [
                { type: stripe; angle: 45deg; spacing: 16px; stroke-width: 1px; },
                { type: stripe; angle: 135deg; spacing: 16px; stroke-width: 1px; }
            ];
        },
        {
            ink: { type: auto-contrast; opacity: 0.22; };
            layers: [
                {
                    type: symbol;
                    lattice: {
                        u-spacing: 14px;
                        u-angle: 0deg;
                        v-spacing: 14px;
                        v-angle: 90deg;
                    };
                    symbol: { shape: circle; size: 7px; rotation: 0deg; };
                    paint: { type: open; stroke-width: 1.5px; };
                }
            ];
        };

    pattern-legend-size: 32px;
}
```

Theme range resolution should treat `fill-pattern-discrete` like
`fill-discrete`:

- Try mark-specific contexts before the generic `mark` context.
- Honor cardinality-specific selectors and use the existing "smallest palette
  at least as large as the requested cardinality, otherwise largest available"
  fallback rule.
- Map domain categories to fill colors and pattern fills by the same discrete
  scale index when both `fill-discrete` and `fill-pattern-discrete` are used.
- Allow palette cycling only as an explicit fallback behavior; for accessibility
  palettes, prefer cardinality-matched definitions.
- Resolve CSS values and variables before producing renderer-facing
  `PatternFill` values.

The renderer-facing resolved theme shape stays fully structured:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PatternPalette {
    pub entries: Vec<Option<PatternFill>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ThemePatternConfig {
    #[serde(default)]
    pub legend: PatternLegendConfig,
}
```

`None` entries allow a theme palette to reserve an index with no pattern while
keeping alignment with a fill palette. A fully redundant accessibility palette
should generally use `Some(PatternFill)` for every entry.

CSS theme input should lower directly to `Option<PatternFill>` entries. It does
not need a separate reference-resolution layer for v1:

```rust
pub enum ThemePatternPaletteEntry {
    None,
    Inline(PatternFill),
}
```

### CSS Pattern Entry Syntax

The CSS object syntax should mirror the serde field and variant names, with CSS
declaration syntax replacing Rust/JSON field syntax:

```text
<pattern-entry> =
    none
  | {
        anchor?: plot | mark | chart;
        ink?: <ink-block>;
        layers: [ <layer-block># ];
    }

<ink-block> =
    { type: auto-contrast; opacity?: <number>; }
  | { type: solid; color: <color>; opacity?: <number>; }

<layer-block> =
    { type: stripe; angle: <angle>; spacing: <length>; stroke-width: <length>; phase?: <length>; dash?: <dash-block>; }
  | { type: symbol; lattice: <symbol-lattice-block>; symbol: <symbol-block>; paint: <symbol-paint-block>; }

<dash-block> =
    { length: <length>; gap: <length>; phase?: <length>; }

<symbol-lattice-block> =
    {
        u-spacing: <length>;
        u-angle: <angle>;
        v-spacing: <length>;
        v-angle: <angle>;
        u-phase?: <length>;
        v-phase?: <length>;
    }

<symbol-block> =
    {
        shape: <symbol-shape>;
        size: <length>;
        rotation?: <angle>;
    }

<symbol-shape> =
    circle | square | cross | diamond |
    triangle | triangle-up | triangle-down | triangle-left | triangle-right |
    arrow | wedge | star | wye | pentagon | cushion | concave-square |
    <string-token containing an SVG path>

<symbol-paint-block> =
    { type: filled; }
  | { type: open; stroke-width: <length>; }

```

### Required CSS Parser Features

The theme parser needs a real CSS token parser for `fill-pattern-discrete`; it
should not parse these values with ad hoc string splitting. Required features:

- Reuse the existing theme selector and cascade machinery, including
  cardinality-specific contexts such as `mark[type="rect"][cardinality="4"]`.
- Recognize `fill-pattern-discrete` as a discrete range property for the
  `fill_pattern` channel, following the existing underscore-to-hyphen channel
  name convention.
- Support the complete `PatternFill` field surface immediately, rather than a
  minimal `ink` and `layers` subset.
- Parse a top-level comma-separated palette list where commas inside `{ ... }`
  blocks, `[ ... ]` lists, color functions, or quoted strings do not split the
  palette.
- Parse `none` as a palette entry that lowers to `None`.
- Parse nested CSS simple blocks: the pattern object, `ink`, `dash`, `lattice`,
  `symbol`, and `paint`.
- Parse `layers: [ ... ];` as an ordered bracket list of layer blocks. Layer
  order is semantically significant and must be preserved.
- Parse declarations inside blocks with CSS declaration syntax:
  `name: value;`. Missing optional declarations use the Rust/serde defaults.
- Map CSS hyphenated field names to the same serde names used by the Rust
  structs, including `stroke-width`, `u-spacing`, `u-angle`, `v-spacing`,
  `v-angle`, `u-phase`, and `v-phase`.
- Parse `type` identifiers for tagged enum variants and reject variants that
  are not present in the Rust type model.
- Parse lengths into display pixels. For v1, accepting `px` and unitless numeric
  values is sufficient; supporting additional absolute CSS length units can be
  added if the theme system already normalizes them.
- Parse angles into degrees. At minimum support `deg`; supporting `rad`,
  `grad`, and `turn` is useful if the existing CSS parser already exposes that
  conversion.
- Parse opacity values as numbers and validate the `[0, 1]` range during
  lowering.
- Parse colors for `ink.color` through the existing theme color resolver, so
  theme variables and supported CSS color functions behave the same as they do
  for `fill-discrete`.
- Parse `symbol.shape` as either an identifier for named symbol shapes or a
  quoted string for custom SVG path syntax.
- Parse `pattern-legend-size` as a theme-level property. A single length means a
  square sample; two lengths can represent `[width, height]` if rectangular
  legend samples are later needed.
- Produce path-aware diagnostics for invalid nested values, for example
  `fill-pattern-discrete[2].layers[1].spacing must be greater than zero`.
- Reject unknown fields, duplicate fields, missing required fields, invalid
  units, and invalid enum variants rather than silently ignoring them.

Out of scope for v1 CSS parsing:

- Named pattern references or a separate pattern registry.
- CSS inheritance or cascading inside a pattern object.
- Per-layer CSS properties outside the inline `layers` array.
- Data-dependent pattern expressions in CSS.
- Continuous pattern ranges.

Lowering should produce the same structured `PatternFill` values that Rust
builders produce. The symbol CSS block lowers directly to `SymbolPatternLayer`.
`symbol.shape` uses the same shape strings accepted by the symbol mark parser.
`paint` uses `PatternFill::ink`; it does not introduce a color of its own.
`pattern-legend-size` belongs to `ThemePatternConfig`, not to any individual
`PatternFill`.

### CSS Parser Ownership And Integration

Pattern CSS parsing should be owned by `avenger-chart-core::theme`, where the
current selector matching, cascade, cardinality lookup, color resolution, and
unit conversion already live. Renderers should only receive resolved
`PatternFill` values. High-level chart marks should not parse CSS strings.

The preferred implementation model is:

- Extend `ThemeValue` with structured values, for example
  `Object(IndexMap<String, ThemeValue>)` and `Array(Vec<ThemeValue>)`, or an
  equivalent internal representation scoped to the theme parser.
- Teach the existing CSS declaration parser to preserve nested curly blocks as
  object values and square bracket blocks as ordered arrays. Existing
  comma-separated list parsing should remain responsible for splitting
  top-level palette entries.
- Add a theme-lowering module, for example `theme::pattern`, that converts
  `ThemeValue` objects into `Option<PatternFill>`, performs path-aware
  validation, and resolves colors/lengths/angles through the same helpers used
  by ordinary theme properties.
- Add `ScaleRange::Pattern(Vec<Option<PatternFill>>)` and route
  `fill-pattern-discrete` through a pattern-specific range constructor rather
  than through the current string-based `create_scale_range` path.
- Keep cardinality-specific lookup in `Theme::get_range_for_channel`; the only
  pattern-specific branch should be the conversion from the selected
  `ThemeValue::List` to `ScaleRange::Pattern`.
- Parse `pattern-legend-size` with the same typed theme-property accessors used
  for other legend properties, returning `PatternLegendConfig` or an equivalent
  legend-theme value.

This keeps CSS grammar ownership centralized in the theme crate and makes
pattern CSS a typed theme feature rather than a renderer feature or a string
mini-language hidden in chart code.

## Serde-Owned Shape

The `PatternFill`, `PatternLayer`, `PatternInk`, and nested layer structs above
define the public serialization shape through serde. A separate hand-written
JSON schema is intentionally out of scope for this document. If examples are
needed for implementation tests, they should live next to the tests so they
cannot drift from the derived serde behavior.

## Okabe-Ito Stripe Palette Sketch

The draft Okabe-Ito redundant encoding palette can be represented entirely with
stripe layers and a 32px legend chip by using only `16px` and `8px` spacings.

| Color | Pattern layers |
| --- | --- |
| Black | `Stripe { angle: 0, spacing: 16, stroke_width: 1.25 }` |
| Orange | `Stripe { angle: 45, spacing: 16, stroke_width: 1.25 }` |
| Sky blue | `Stripe { angle: 135, spacing: 16, stroke_width: 1.25 }` |
| Bluish green | `Stripe { angle: 0, spacing: 8, stroke_width: 1.1 }` |
| Yellow | `Stripe { angle: 90, spacing: 16, stroke_width: 1.25 }` |
| Blue | `Stripe { angle: 45, spacing: 8, stroke_width: 1.1 }` |
| Vermillion | `Stripe { angle: 135, spacing: 8, stroke_width: 1.1 }` |
| Reddish purple | two `Stripe` layers at `45` and `135`, `16px` spacing, `1px` stroke |

The closest hue families receive opposite angle and different spacing:

- sky blue vs blue: `135/16` vs `45/8`
- orange vs vermillion: `45/16` vs `135/8`

## Validation Summary

Pattern construction should reject:

- non-finite numeric values;
- negative or zero spacing;
- negative or zero stroke widths;
- `opacity` outside `[0, 1]`;
- symbol shape strings that fail `SymbolShape::from_vega_str`;
- `PatternLegendConfig.sample_size` values less than or equal to zero.

Pattern construction should warn, but still render, when stripe widths are
greater than or equal to stripe spacing. This produces dense or nearly solid
texture, which may be intentional in some themes.

Palette builders should additionally warn when default legend sample sizes and
pattern spacings produce visually unbalanced centered samples. For a default
accessibility palette, prefer spacings that produce symmetric legend chips at
the configured `pattern-legend-size`.

Theme palette builders should additionally validate that:

- `fill-pattern-discrete` entries lower to `PatternFill` values or `None`;
- paired `fill-discrete` and `fill-pattern-discrete` palettes use compatible
  cardinality and ordering when intended for redundant encoding.

## Visual Baseline Suite

The pattern implementation should have a small visual baseline suite that
exercises the full renderer-facing contract without creating a combinatorial
matrix. Each baseline should render the same logical scene through SVG, wgpu,
and PDF. PDF output should be rasterized to the same logical viewport before
comparison.

Use deterministic dimensions, simple backgrounds, and minimal text so failures
mostly indicate pattern geometry, clipping, color, or legend regressions rather
than font/layout drift.

| Baseline | Purpose | Required features |
| --- | --- | --- |
| `pattern_stripe_geometry_matrix` | Verifies stripe angle, spacing, stroke width, and phase conventions. | A grid of square chips with `0`, `45`, `90`, and `135` degree stripes; at least two spacings; at least one non-zero stripe `phase`; visible mark outlines for measuring crossings. |
| `pattern_anchor_and_dash_phase` | Verifies shared pattern coordinates, mark-local restarts, ordered layers, and global dash phase. | Two rows of adjacent bars: one `PatternAnchor::Plot`, one `PatternAnchor::Mark`; a dashed diagonal stripe whose dashes continue across plot-anchored bars; a crosshatch sample built from two stripe layers. |
| `pattern_filled_mark_clipping` | Verifies that supported filled mark geometries clip the same pattern correctly. | The same stripe pattern clipped to rect, arc, symbol host shape, path, and area marks, each with a contrasting stroke drawn above the pattern. If a milestone stages marks, include the marks in that milestone and add the rest as they land. |
| `pattern_ink_opacity` | Verifies ink resolution, non-accumulating opacity, and fill independence. | Light and dark host fills using `AutoContrast`, plus explicit solid ink examples; include at least one transparent or low-opacity fill to catch incorrect alpha composition; include overlapping stripe layers where intersections must not become darker than single-layer strokes. |
| `pattern_theme_scale_legend` | Verifies chart lowering, CSS theme palettes, discrete pattern ranges, paired fill/pattern ordering, and merged legends. | A categorical bar chart where `fill` and `fill_pattern` use the same expression and theme-defined `fill-discrete` plus `fill-pattern-discrete`; include one `None` pattern entry; the legend uses one merged fill+pattern guide with larger pattern chips and centered samples. |
| `pattern_facet_plot_anchor` | Verifies plot-anchor resolution in faceted or repeated plot contexts. | Two facets with the same plot-anchored pattern and matching data positions; pattern continuity is local to each facet plot area, not global across the full chart, unless `PatternAnchor::Chart` is explicitly used. |

The symbol-layer baseline is gated by the layer milestone. If 2D symbol layers
are included in a milestone, add these baselines:

| Baseline | Purpose | Required features |
| --- | --- | --- |
| `pattern_symbol_layer_lattice` | Verifies the 2D lattice pattern layer contract. | Open and filled symbol patterns using the mark-language symbol vocabulary, non-orthogonal or phase-shifted lattice vectors, and clipping to at least one non-rectangular host mark. |
| `pattern_symbol_stripe_opacity_union` | Verifies final-state non-accumulating opacity across mixed layer families. | A `PatternFill` with at least one stripe layer and one symbol layer whose coverage overlaps visibly; overlap regions must not become darker than stripe-only or symbol-only coverage. |

These baselines should be paired with non-visual unit tests for parser
diagnostics, validation failures, serde defaults, channel lowering, scale range
construction, and legend merge-key grouping. The visual suite should stay
focused on user-visible geometry and renderer parity.

## Before Implementation Planning

Before writing an implementation plan, specify these remaining inputs:

- Layer milestone scope: whether the first implementation renders only stripe
  layers, or also renders 2D symbol layers. Whichever layer families are in a
  milestone must be implemented in SVG, wgpu, and PDF before that milestone is
  considered complete.
- Mark milestone scope: whether the first implementation must cover all listed
  geometric filled scene marks, or whether it should stage `SceneRectMark`
  first and expand once renderer behavior is proven.
- Anchor metadata contract: how `PatternAnchor::Plot` is represented for
  standalone scenegraphs, facets, nested groups, and exported/replayed
  scenegraphs.
- Default palette commitment: whether the Okabe-Ito stripe palette sketch above
  is an initial theme default, an optional accessibility theme, or only a
  design reference.
- Non-visual test matrix: unit tests for parsing, validation, serde defaults,
  chart lowering, scale range construction, CSS diagnostics, and legend
  merge-key grouping.

## Resolved Decisions

- CSS block syntax should support the full `PatternFill` field surface
  immediately.
- `PatternFill::ink` is shared across all layers. Per-layer ink is not part of
  these requirements.
- `symbol.shape` should accept custom SVG path strings through
  `SymbolShape::from_vega_str` unless implementation proves this is
  significantly more complex than named shapes.
- `SceneGroup` background fills and `SceneTextMark` glyph fills are out of scope
  for the initial implementation and the first follow-on pass after geometric
  filled marks.
- SVG, wgpu, and PDF must all support any pattern layer family included in a
  shipped milestone and must use shared geometry conventions to produce
  visually identical output.
- Stripe geometry uses display-coordinate angles, normal-projected spacing,
  centered strokes, butt caps, no backend-specific pixel snapping, and the phase
  equation documented in "Stripe Geometry Conventions".
- CSS parser ownership belongs in `avenger-chart-core::theme`, with typed
  nested values lowered to `ScaleRange::Pattern` after existing selector,
  cascade, color, unit, and cardinality handling.
- Chart lowering should store pattern channels in a typed sidecar rather than
  forcing `PatternFill` through `ScalarValue`.
- `DataContext` receives a plain live sidecar; serde defaults and
  `skip_serializing_if` apply to the compiled/serialized data-context form.
- Redundant `fill` + `fill_pattern` encodings with the same expression and
  compatible discrete domains must share resolved domain ordering before range
  lookup, then share one merged legend. Pattern legends use larger samples than
  fill-only color legends.
- Pattern ink opacity is non-accumulating within a `PatternFill`; overlapping
  layer coverage should not darken where layers intersect.
- `AutoContrast` resolves per rendered item in shared code after host fill is
  known, including a deterministic gradient-host fallback shared by all
  renderers.
- Chart-level `Arc` authoring is not a v1 pattern API target unless a chart arc
  mark is added; scenegraph `SceneArcMark` pattern rendering remains in scope.
- No-pattern fields and empty pattern-channel sidecars should be skipped during
  serialization to preserve existing baselines.
- The concise visual baseline suite above is the renderer-parity acceptance
  target for v1 pattern features.

## Open Questions

- The implementation plan needs to verify how missing `PatternAnchor::Plot`
  reference frames are reported.
