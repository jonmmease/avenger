# Pattern Grammar API Proposal

## Goal

This note proposes an Avenger pattern-fill API inspired by He, Dykes,
Isenberg, and Isenberg's "Reframing Pattern: A Comprehensive Approach to a
Composite Visual Variable" (arXiv:2508.02639). It complements
[`pattern-fills.md`](pattern-fills.md), which surveys existing visualization
libraries and renderer implementation paths.

The core question here is API grammar: if pattern is a composite visual
variable made from repeated primitives on a host mark, how should Avenger let
authors describe the options shown in Figure 6 of the paper?

Source material:

- Paper: <https://arxiv.org/abs/2508.02639>
- PDF: <https://arxiv.org/pdf/2508.02639>
- Supplementary figures: <https://osf.io/z7ae2/>

## Reading Summary

The paper argues that "texture" is too overloaded for visualization API design.
For abstract charts, the more precise concept is `pattern`: a structured,
often repeated collection of graphical primitives that sits on or within a
host symbol. A primitive can itself be treated as a mark. A host symbol is the
mark being filled or decorated, such as an area, bar, symbol, line with width,
or map region.

The paper's design space has three major attribute groups:

1. **Spatial arrangement of primitives**: how primitive positions are generated.
   Figure 6 starts with 1D lattices, 2D lattices, and no-lattice/data-driven
   arrangements, then applies transforms and positional regularity.
2. **Appearance relationships among primitives**: how many primitive groups
   exist, their ratios, and how groups are distributed across generated
   positions.
3. **Retinal visual variables on primitives**: the primitive-level visual
   channels, such as shape, size, orientation, hue, value, opacity, and stroke.
   These may be identical across primitives or vary by group, data, or
   generated randomness.

The paper also emphasizes:

- lattice orientation and primitive orientation are distinct;
- Bertin's "granularity" is not a single primitive variable, but a combined
  change in primitive size and spacing while preserving average area value;
- data-driven arrangements, such as geographic dot maps, are still describable
  as patterns even when no regular lattice exists;
- patterns may nest, with primitives of one pattern serving as hosts for a
  second pattern;
- pattern variables interact: size, spacing, primitive shape, and color can
  create emergent area value, shade, moire, or loss of primitive identity.

For Avenger, the important implementation lesson is that a useful pattern API
should not collapse this design space into a flat enum like
`DiagonalHatch | Dots | Crosshatch`. Named hatches are good presets, but the
underlying representation should be a small grammar.

## Design Principles

- Treat patterns as **scenegraph paint resources**, not as SVG-only strings.
- Keep the first author-facing API simple, but make the serialized model
  expressive enough for Figure 6's dimensions.
- Separate host-level fill color from primitive-level pattern paint so
  redundant color+pattern encodings are natural.
- Make pattern generation deterministic: all stochastic regularity and
  distribution choices need seeds.
- Keep coordinate spaces explicit: logical pixels, plot area, host-local, and
  data coordinates should not be conflated.
- Make renderer limits visible: v1 can support a strict subset of this grammar
  in WGPU while SVG may support more.

## Two API Layers

Avenger should expose two layers:

1. **Preset/convenience API** for chart authors who want ordinary pattern
   fills.
2. **Grammar API** for design-space coverage, research experiments, and future
   custom pattern libraries.

The convenience API lowers into the grammar API.

```rust
Rect::new()
    .x("category")
    .y("value")
    .y2(0.0)
    .fill(col("category"))
    .fill_pattern(col("category"));
```

```rust
Rect::new()
    .x("category")
    .y("value")
    .y2(0.0)
    .fill("#e8eef7")
    .fill_pattern_with(col("category"), |p| {
        p.scale_with::<PatternOrdinal>(|s| {
            s.range_patterns(PatternPalette::print_safe())
        })
        .legend(|l| l.title("Category"))
    });
```

The grammar API is explicit:

```rust
let pattern = Pattern::new()
    .arrangement(
        Lattice::two_d()
            .cell(UnitCell::rect(px(10.0), px(10.0)))
            .rotate(deg(15.0)),
    )
    .placement(Placement::regular())
    .groups(
        PrimitiveGroups::new()
            .group("a", |g| g.ratio(1.0).fill("#8e2a8d"))
            .group("b", |g| g.ratio(1.0).fill("#d07a1f"))
            .distribution(GroupDistribution::interleaved()),
    )
    .primitive(Primitive::square().size(px(3.0)))
    .fit(PatternFit::clip_to_host());

Rect::new()
    .x("x")
    .y("y")
    .y2(0.0)
    .fill("#fff7ed")
    .fill_pattern(lit(pattern));
```

## Serializable Grammar

The grammar should serialize to a backend-neutral spec. Names below are
sketches, not final API commitments.

```rust
pub struct PatternSpec {
    pub arrangement: ArrangementSpec,
    pub placement: PlacementSpec,
    pub groups: PrimitiveGroupsSpec,
    pub primitive: PrimitiveSpec,
    pub variables: PrimitiveVariablesSpec,
    pub fit: PatternFitSpec,
}

pub enum ArrangementSpec {
    Lattice(LatticeSpec),
    DataDriven(DataDrivenArrangementSpec),
    Nested(Box<PatternSpec>),
}

pub struct LatticeSpec {
    pub dimensionality: LatticeDim,
    pub unit_cell: UnitCellSpec,
    pub transform: Affine2Spec,
    pub space: PatternSpace,
}

pub enum LatticeDim {
    OneD,
    TwoD,
}

pub enum UnitCellSpec {
    Segment { spacing: LengthSpec },
    Rect { width: LengthSpec, height: LengthSpec },
    Oblique { a: LengthSpec, b: LengthSpec, theta: AngleSpec },
    Hex { spacing: LengthSpec },
    Basis { u: Vec2Spec, v: Vec2Spec },
}

pub struct PlacementSpec {
    pub regularity: PositionalRegularitySpec,
    pub anchor: PrimitiveAnchor,
}

pub enum PositionalRegularitySpec {
    Regular,
    Jitter {
        axes: AxesSpec,
        range: Vec2Spec,
        dispersion: f32,
        distribution: RandomDistribution,
        seed: u64,
    },
    Algorithm {
        name: String,
        params: serde_json::Value,
    },
}

pub struct PrimitiveGroupsSpec {
    pub groups: Vec<PrimitiveGroupSpec>,
    pub ratio: GroupRatioSpec,
    pub distribution: GroupDistributionSpec,
}

pub struct PrimitiveGroupSpec {
    pub id: String,
    pub variables: PrimitiveVariablesSpec,
}

pub enum GroupRatioSpec {
    Equal,
    Weights(Vec<f32>),
    Encoded(ChannelValue),
}

pub enum GroupDistributionSpec {
    Stacked { axis: DistributionAxis },
    Interleaved,
    Random { seed: u64 },
    BlueNoise { seed: u64 },
    ByData { expr: ChannelValue },
}

pub enum PrimitiveSpec {
    Line(LinePrimitiveSpec),
    Rect(RectPrimitiveSpec),
    Circle(CirclePrimitiveSpec),
    Path(PathPrimitiveSpec),
    Symbol(SymbolPrimitiveSpec),
    Pattern(Box<PatternSpec>),
}

pub struct PrimitiveVariablesSpec {
    pub shape: Option<PrimitiveValue<ShapeSpec>>,
    pub size: Option<PrimitiveValue<SizeSpec>>,
    pub orientation: Option<PrimitiveValue<AngleSpec>>,
    pub fill: Option<PrimitiveValue<PaintSpec>>,
    pub stroke: Option<PrimitiveValue<PaintSpec>>,
    pub stroke_width: Option<PrimitiveValue<LengthSpec>>,
    pub opacity: Option<PrimitiveValue<f32>>,
}

pub enum PrimitiveValue<T> {
    Constant(T),
    ByGroup(IndexMap<String, T>),
    Encoded(ChannelValue),
    Generated(GeneratedValueSpec<T>),
}

pub struct GeneratedValueSpec<T> {
    pub base: T,
    pub regularity: VariableRegularitySpec,
}

pub enum VariableRegularitySpec {
    Regular,
    Jitter {
        range: f32,
        dispersion: f32,
        seed: u64,
    },
    CategoricalEntropy {
        alternatives: Vec<String>,
        entropy: f32,
        seed: u64,
    },
}

pub struct PatternFitSpec {
    pub origin: PatternOrigin,
    pub units: PatternUnits,
    pub transform: Affine2Spec,
    pub edge: PatternEdgePolicy,
}
```

The grammar mirrors Figure 6:

- `ArrangementSpec` chooses the lattice or no-lattice basis.
- `LatticeSpec::transform` covers scale, translate, rotate, and shear.
- `PlacementSpec::regularity` covers regular and irregular primitive placement.
- `PrimitiveGroupsSpec` covers grouping, ratios, and distributions.
- `PrimitiveVariablesSpec` covers identical and varied primitive-level visual
  variables.
- `PatternFitSpec` covers placing the generated pattern on the host symbol.

## Figure 6 As Grammar

The paper's Figure 6 can be read as a pipeline:

```text
choose lattice
  -> transform lattice
  -> place primitives
  -> assign primitive groups / ratios / distributions
  -> vary primitive retinal variables
  -> place generated pattern on host
```

In Avenger terms:

```rust
Pattern::new()
    .arrangement(...)
    .placement(...)
    .groups(...)
    .primitive(...)
    .variables(...)
    .fit(...);
```

### 1D Lattice

Figure 6's 1D examples are line-based patterns: a lattice extends in one
direction, and the primitive is typically a line segment or an effectively
infinite line.

```rust
let diagonal_lines = Pattern::new()
    .arrangement(
        Lattice::one_d()
            .cell(UnitCell::segment(px(8.0)))
            .rotate(deg(25.0)),
    )
    .placement(Placement::regular())
    .primitive(
        Primitive::line()
            .length(Length::infinite())
            .stroke_width(px(1.5)),
    )
    .variables(|v| v.stroke("#222").opacity(1.0))
    .fit(PatternFit::clip_to_host());
```

A denser version changes lattice scale, not primitive width:

```rust
let dense_lines = diagonal_lines.clone()
    .with_arrangement(|a| a.scale(0.65));
```

That distinction matters because primitive width changes patterned area value,
while lattice spacing changes density.

### 2D Lattice

Figure 6's 2D examples place point-like primitives on a two-dimensional
lattice.

```rust
let square_grid = Pattern::new()
    .arrangement(
        Lattice::two_d()
            .cell(UnitCell::rect(px(10.0), px(10.0)))
            .rotate(deg(0.0)),
    )
    .placement(Placement::regular())
    .primitive(Primitive::square().size(px(3.0)))
    .variables(|v| v.fill("#222"))
    .fit(PatternFit::clip_to_host());
```

An oblique or sheared lattice should be represented as a lattice transform, not
as primitive rotation:

```rust
let oblique_grid = square_grid.clone()
    .with_arrangement(|a| {
        a.shear_x(0.35)
            .scale_xy(1.0, 0.85)
    });
```

### No Lattice

The paper's no-lattice row covers data-driven or algorithmic arrangements. In
Avenger, this should be a separate arrangement mode, not "a lattice with a lot
of jitter".

```rust
let data_driven = Pattern::new()
    .arrangement(
        Arrangement::data_driven()
            .x(col("longitude"))
            .y(col("latitude"))
            .space(PatternSpace::Data),
    )
    .primitive(Primitive::circle().radius(px(1.5)))
    .variables(|v| {
        v.size(col("population"))
            .fill(col("tax_rate"))
    })
    .fit(PatternFit::clip_to_host());
```

For non-geographic layouts, the same API can use generated positions:

```rust
let stipple = Pattern::new()
    .arrangement(
        Arrangement::algorithm("blue_noise")
            .param("density", 0.35)
            .seed(12),
    )
    .primitive(Primitive::circle().radius(px(1.2)))
    .variables(|v| v.fill("#222"))
    .fit(PatternFit::clip_to_host());
```

### Transform Lattice

Figure 6 separates choosing a lattice from transforming it. Avenger should do
the same.

```rust
Lattice::two_d()
    .cell(UnitCell::rect(px(8.0), px(8.0)))
    .translate(px(2.0), px(0.0))
    .scale_xy(1.4, 0.8)
    .rotate(deg(30.0))
    .shear_x(0.25);
```

The transform target must be clear:

- `arrangement.transform(...)` transforms lattice points only;
- `primitive.orientation(...)` rotates primitives inside the lattice;
- `fit.transform(...)` transforms the whole completed pattern on the host.

This distinction is one of the best reasons to use a grammar instead of a flat
pattern enum.

### Place Primitives

Figure 6 distinguishes regular and irregular primitive placement. In the
grammar, this is a placement rule applied after lattice points are generated.

```rust
let regular = Placement::regular();

let jittered = Placement::jitter()
    .axes(Axes::xy())
    .range(px2(3.0, 3.0))
    .dispersion(0.8)
    .distribution(RandomDistribution::Normal)
    .seed(7);

let x_only_jitter = Placement::jitter()
    .axes(Axes::x())
    .range(px2(3.0, 0.0))
    .dispersion(0.6)
    .seed(7);
```

Use jitter for positional regularity. Do not represent this by changing the
unit cell unless the intended data variable is spacing/density.

### Grouping And Ratios

Figure 6's "grouping, ratios" column means that primitives can belong to
appearance groups, and group counts can vary.

```rust
let two_groups_equal = PrimitiveGroups::new()
    .group("a", |g| g.fill("#8e2a8d"))
    .group("b", |g| g.fill("#d07a1f"))
    .ratio([1.0, 1.0])
    .distribution(GroupDistribution::stacked_x());

let two_groups_1_to_3 = PrimitiveGroups::new()
    .group("a", |g| g.fill("#8e2a8d"))
    .group("b", |g| g.fill("#d07a1f"))
    .ratio([1.0, 3.0])
    .distribution(GroupDistribution::stacked_x());
```

For data-dependent ratios:

```rust
let category_mix = PrimitiveGroups::from_columns()
    .group("A", |g| g.ratio(col("share_a")).fill("#8e2a8d"))
    .group("B", |g| g.ratio(col("share_b")).fill("#d07a1f"))
    .distribution(GroupDistribution::interleaved());
```

This opens the door to waffle-chart-like patterns inside each host mark, but
also raises legend and perceptual-load questions.

### Distributions

Figure 6's "distributions" examples decide which lattice slots receive each
group after group ratios are known.

```rust
let grouped = GroupDistribution::stacked_x();
let interleaved = GroupDistribution::interleaved();
let random = GroupDistribution::random().seed(19);
let blue_noise = GroupDistribution::blue_noise().seed(19);
```

These should not be overloaded with positional regularity:

- distribution assigns group labels to slots;
- placement moves slots in space.

### Identical Retinal Mapping

Figure 6's "identical mapping" column uses one primitive-level retinal variable
consistently across all primitives.

```rust
let identical_shape = Pattern::new()
    .arrangement(Lattice::two_d().cell(UnitCell::rect(px(10.0), px(10.0))))
    .placement(Placement::regular())
    .primitive(Primitive::symbol(SymbolShape::Square))
    .variables(|v| {
        v.size(px(3.0))
            .fill("#222")
            .orientation(deg(0.0))
    });
```

Each variable should be independently addressable:

```rust
variables.shape(SymbolShape::Circle);
variables.size(px(3.0));
variables.orientation(deg(45.0));
variables.fill("#222");
variables.value(0.65);
```

### Varied Retinal Mapping

Figure 6's "varied mapping" column varies a primitive-level retinal variable
across primitives. This should be represented as a generated or encoded
primitive value.

```rust
let varied_size = Pattern::new()
    .arrangement(Lattice::two_d().cell(UnitCell::rect(px(9.0), px(9.0))))
    .primitive(Primitive::square())
    .variables(|v| {
        v.size(
            Generated::around(px(3.0))
                .jitter(range(2.0), dispersion(0.7))
                .seed(4),
        )
        .fill("#222")
    });

let varied_orientation = Pattern::new()
    .arrangement(Lattice::two_d().cell(UnitCell::rect(px(9.0), px(9.0))))
    .primitive(Primitive::rect().size(px2(2.0, 5.0)))
    .variables(|v| {
        v.orientation(
            Generated::around(deg(45.0))
                .jitter(range_deg(25.0), dispersion(0.8))
                .seed(5),
        )
        .fill("#222")
    });
```

For data-driven primitives, varied mapping can be a real encoding:

```rust
let by_data = Pattern::new()
    .arrangement(Arrangement::data_driven().x(col("x")).y(col("y")))
    .primitive(Primitive::circle())
    .variables(|v| {
        v.size(col("population"))
            .fill(col("category"))
    });
```

### Place On Host

Figure 6's final step is applying the completed pattern to the host symbol.
This should be explicit because host anchoring affects cross-mark consistency.

```rust
PatternFit::new()
    .origin(PatternOrigin::PlotArea)
    .units(PatternUnits::LogicalPx)
    .edge(PatternEdgePolicy::Clip);
```

Origin options:

- `PlotArea`: one global pattern phase inside each plot area. Good for maps,
  bars, and facets when identical categories should align visually.
- `HostBbox`: pattern phase starts at each host mark's bounding box. Good for
  local symbols, but equal categories can look different across differently
  sized marks.
- `Canvas`: phase is stable across the entire scenegraph.

Edge policies:

- `Clip`: draw partial primitives at the boundary.
- `OmitPartial`: drop primitives that cross the boundary.
- `Inset`: keep primitives away from the boundary by a margin.
- `Halo`: reserve or draw a border halo to prevent edge fragments from merging
  with host outlines.

## Pattern-Local Primitive Frame

The most Avenger-native way to make the grammar powerful is to define a
pattern-local primitive frame. Pattern generation emits conceptual primitive
rows before rendering:

```text
host_index
slot_index
slot_u
slot_v
slot_x
slot_y
group
random_0
random_1
primitive_x
primitive_y
```

Primitive variables can then be constants, group lookup values, generated
values, or expressions over this frame. This mirrors how ordinary mark channels
and mark effects already treat rows and item-level derived values.

Example:

```rust
Pattern::new()
    .arrangement(Lattice::two_d().cell(UnitCell::rect(px(8.0), px(8.0))))
    .placement(Placement::regular())
    .groups(PrimitiveGroups::from_ratio([1.0, 3.0]))
    .primitive(Primitive::square())
    .variables(|v| {
        v.fill(when(col("group").eq(lit("a")), lit("#8e2a8d"))
            .otherwise(lit("#d07a1f")))
         .size(lit(3.0))
    });
```

The expression model should be internal at first. A public expression surface
can come later if there is demand for advanced pattern programming.

## Integration With Chart Marks

Marks that support `fill` should gain a `fill_pattern` channel. The fill color
continues to define the host/background paint. The pattern defines foreground
primitives over that background.

```rust
Rect::new()
    .fill(col("region"))
    .fill_pattern(col("region"));
```

For separate fields:

```rust
Rect::new()
    .fill(col("temperature"))
    .fill_pattern(col("scenario"));
```

For direct pattern literals:

```rust
Rect::new()
    .fill("#f8fafc")
    .fill_pattern(Pattern::lines().angle(deg(45.0)).spacing(px(8.0)));
```

Channel config:

```rust
define_channel_config!(PatternChannelConfig);
```

Scale range:

```rust
pub enum ScaleRange {
    // existing variants...
    Pattern(Vec<PatternSpec>),
}
```

The default pattern scale should be categorical, not continuous. Ordered
pattern density should be a separate, later design because the paper is clear
that spacing, granularity, density, and average area value are entangled.

## Preset API

Common hatches should be named constructors that lower into the grammar:

```rust
Pattern::lines()
    .angle(deg(45.0))
    .spacing(px(8.0))
    .stroke_width(px(1.5))
    .foreground("#222");

Pattern::crosshatch()
    .spacing(px(8.0))
    .stroke_width(px(1.25));

Pattern::dots()
    .spacing(px2(8.0, 8.0))
    .radius(px(1.5));

Pattern::grid()
    .spacing(px2(8.0, 8.0))
    .stroke_width(px(1.0));
```

These should be implemented as grammar builders, not special renderer-only
branches. For example:

```rust
impl Pattern {
    pub fn lines() -> PatternBuilder {
        Pattern::new()
            .arrangement(Lattice::one_d().cell(UnitCell::segment(px(8.0))))
            .placement(Placement::regular())
            .primitive(Primitive::line().length(Length::infinite()))
            .variables(|v| v.stroke("#222").stroke_width(px(1.5)))
            .fit(PatternFit::clip_to_host())
    }
}
```

## Renderer Implications

The SVG backend can emit the grammar as native `<pattern>` definitions for the
regular lattice subset. More advanced data-driven or nested patterns may need
to lower to explicit clipped primitives or to rasterized pattern tiles.

The WGPU backend should start with procedural support for the preset subset:
line, crosshatch, dots, grid, checker, and maybe simple square lattices. A
full grammar implementation needs either:

- a richer per-vertex paint id plus pattern metadata table, or
- a pattern atlas/mask atlas with repeat metadata.

The grammar should therefore advertise renderer support:

```rust
pub enum PatternSupportLevel {
    ProceduralBuiltin,
    SvgNativeOnly,
    ExplicitPrimitiveExpansion,
    RasterFallback,
    Unsupported,
}
```

This avoids a trap where SVG silently supports a design that WGPU cannot
render.

## Suggested V1 Slice

Start with a strict, useful subset:

- `ArrangementSpec::Lattice` only.
- `LatticeDim::OneD` and `LatticeDim::TwoD`.
- `UnitCellSpec::Segment` and `UnitCellSpec::Rect`.
- affine scale, rotate, translate; defer shear.
- `PlacementSpec::Regular`; optionally deterministic jitter.
- one primitive group, plus simple two-group equal/weighted groups.
- line, square, circle primitives.
- constant primitive variables and by-group primitive variables.
- plot-area or host-bbox origin.
- clip edge policy only.

This covers most practical hatching/dot/grid fills and enough of Figure 6 to
validate the model. It intentionally defers data-driven arrangements, nested
patterns, arbitrary paths, image patterns, and public primitive-frame
expressions.

## Open Questions

- Should `fill_pattern` be a separate channel, or should `fill` accept a
  `Paint::Pattern` value directly? The separate channel is better for
  redundant color+pattern encodings; the paint value is cleaner for
  scenegraph internals.
- Should pattern legends combine with color legends when both map the same
  field?
- Should patterns be clipped to rounded rect corners and arbitrary symbols at
  the pattern stage or by normal scenegraph clipping?
- How much of the grammar should be public Rust API versus serialized internal
  spec?
- Should data-driven patterns be a pattern feature or just ordinary nested
  marks clipped by a host? The paper's theory says they are patterns, but the
  implementation may be cleaner as clipped generated marks.
- How should pattern support be exposed to Python bindings without requiring
  users to build large Rust-style nested builders?

## Recommendation

Use the Figure 6 pipeline as the mental model and serialized grammar, but ship
the first public API as named presets plus a small builder:

```rust
Pattern::lines()
Pattern::dots()
Pattern::grid()
Pattern::new()
    .arrangement(...)
    .primitive(...)
    .variables(...)
```

Avoid committing v1 to data-driven and nested patterns, but keep those concepts
in the spec shape. The paper's value for Avenger is not that every pattern
variant should be implemented immediately; it gives us the vocabulary to keep
the easy hatch API from becoming a dead end.
