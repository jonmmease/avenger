# Mark Effects And Evaluation Frames

## Goal Review

Several future-work threads point at the same missing runtime boundary:
authors need operations that run after mark data has been transformed and
encodings have been evaluated, but before final scenegraph marks are emitted.

Examples include:

- post-scale placement adjustments such as jitter, dodge, nudge, collision
  avoidance, and upright label rotation,
- derived scene marks such as labels, connectors, hulls, callouts, and
  annotations built from an existing mark's rendered geometry,
- compound marks that share one effect pipeline across multiple child marks,
- coordinate-sensitive mark semantics such as polar line interpolation and
  polar text orientation,
- stable mark/datum identity for tools that select or edit rendered geometry.

The shared primitive should not be a generic DataFrame transform. Data
transforms operate before scale evaluation and are appropriate for reshaping,
filtering, aggregation, binning, and calculated columns. Mark effects operate
after Avenger knows the actual prepared rows, evaluated encodings, scaled
encoding values, coordinate transform, and mark-specific geometry.

The old [adjust-api.md](adjust-api.md), [derive-api.md](derive-api.md), and
[text-mark.md](text-mark.md) notes should remain as source notes for now. This
document sketches a unifying design that could eventually replace or absorb
them.

## Current System Fit

Useful existing pieces:

- `prepare_mark_data_runtime` already applies mark/group/facet data scope,
  transforms, runtime aggregate-channel preparation, ordering, scale
  expressions, and collection into render batches.
- `CompiledMark::render_mark_data` is already the runtime mark boundary where
  prepared batches, `MarkRuntimeContext`, and the coordinate transform meet.
- `MarkRuntimeContext` exposes plot-area size, params, theme context, facet
  path, and coordinate measurement.
- `CoordinateSystemTransformCore` maps scaled coordinate channels into
  `PlotGeometry`.
- `RenderedMarkData` already has hooks for source row indices and event datum
  rows, so rendered output can preserve data lineage.
- Tools already depend on hit-test-to-data mapping, and lasso selection proves
  that rendered geometry can drive semantic selection.

Missing pieces:

- a stable post-transform prepared row table for effect code,
- a uniform way to inspect every evaluated encoding before and after scaling,
- a mark-specific but shareable representation of mutable render geometry,
- an identity model that survives adjustment and can attach to derived output,
- rules for compound marks that want to own adjustments, derivations, and
  child-mark routing,
- scheduling rules for effects relative to scale/domain/legend planning,
  base scenegraph emission, guides, tools, and hit testing.

## Terminology

### Prepared Mark Rows

Prepared mark rows are not raw input data. They are the rows after:

- inherited plot data or mark-local data has been selected,
- facet data scope has been applied,
- mark group and mark-local transforms have run,
- runtime aggregate-channel preparation has run,
- ordering channels have been applied,
- any repeated or generated runtime channels have been resolved.

This is the table the mark's encodings are actually based on. Effects should
be able to inspect it, but ordinary effects should not mutate it.

### Encoding Frame

An encoding frame contains one entry for every encoding channel the mark
supports or dynamically contributes.

Each channel entry should expose:

- the channel name or key,
- the evaluated value before scale application,
- the scaled/render value after scale application, or the same value for
  no-scale channels,
- scale metadata when a scale was used,
- whether the channel is scalar or row-varying,
- type information sufficient for typed accessors.

For example, a polar text mark could expose `r`, `theta`, `text`, `angle`,
`color`, `font_size`, `defined`, and any future coordinate-specific channels.
`theta.value` might contain the prepared data value, while `theta.scaled`
contains radians in the coordinate range.

### Geometry Frame

A geometry frame is the mark-owned mutable render geometry derived from scaled
encodings and the coordinate transform.

It should be typed, not forced into one universal point table. Possible views:

- point anchors for symbols, text, images, and annotations,
- intervals or rectangles for bars and rect marks,
- polylines or paths for line, trail, area, and path marks,
- arcs or radial sectors,
- custom coordinate or mark geometry.

Common geometry fields can still be shared where they exist: display-space
anchors, extents, angles, local coordinate bases, defined flags, and clipping
state.

### Geometry Space

`GeometrySpace` is a mark semantic option, not itself an effect.

```rust
pub enum GeometrySpace {
    Coordinate,
    Display,
}
```

`GeometrySpace::Coordinate` means the mark constructs or interprets its
geometry in the coordinate frame before projection into display space.

`GeometrySpace::Display` means the mark constructs or interprets its geometry
after projection into local plot-area display coordinates.

Examples:

- `Line<Polar>` with coordinate geometry space interpolates in scaled
  `r/theta` before projecting, so constant radius yields arcs and constant
  theta yields radial segments.
- `Line<Polar>` with display geometry space projects the data vertices and
  connects them with display-space chords.
- `Text<Polar>` with coordinate geometry space interprets `angle = 0` as the
  local coordinate-space zero direction, naturally radial outward for polar.
  `angle = 90` is tangential.
- `Text<Polar>` with display geometry space keeps current display-space angle
  semantics, where `angle = 0` is screen-horizontal.

### Identity Frame

Effects and derived output need stable identity. The frame should carry:

- compiled mark index and optional public mark id,
- facet path and child-frame path where relevant,
- source row lineage,
- event datum rows or a way to construct them,
- parent/source datum rows for derived marks,
- detail/group partition identity for continuous geometry,
- generated instance ids for geometry that no longer maps one-to-one to source
  rows.

This identity model is also needed by tools, hit testing, editable selections,
and annotation workflows.

## Proposed Core Boundary

The shared runtime substrate is a `MarkEvaluationFrame`.

```rust
pub struct MarkEvaluationFrame {
    pub rows: PreparedMarkRows,
    pub encodings: EncodingFrame,
    pub geometry: MarkGeometryFrame,
    pub identity: MarkIdentityFrame,
}
```

Sketch:

```rust
pub struct PreparedMarkRows {
    pub batch: Option<RecordBatch>,
    pub scalar_batch: RecordBatch,
    pub len: usize,
}

pub struct EncodingFrame {
    pub channels: IndexMap<ChannelKey, EncodingColumn>,
}

pub struct EncodingColumn {
    pub key: ChannelKey,
    pub value: EncodedValues,
    pub scaled: EncodedValues,
    pub scale_name: Option<String>,
    pub uses_scale: bool,
}

pub enum MarkGeometryFrame {
    Point(PointGeometryFrame),
    Rect(RectGeometryFrame),
    Path(PathGeometryFrame),
    Text(TextGeometryFrame),
    Arc(ArcGeometryFrame),
    Compound(CompoundGeometryFrame),
    Custom(Box<dyn CustomGeometryFrame>),
}
```

The exact storage can be Arrow arrays, `ScalarOrArray` columns, or typed
vectors. The important contract is semantic: effect code can see prepared rows
and all channel values, while marks remain responsible for interpreting and
emitting their own geometry.

Compound marks should be able to implement this same boundary. A compound mark
may expose one public evaluation frame while internally owning named child
frames for symbols, text, leaders, intervals, or other generated pieces. The
compound frame should identify a primary geometry when there is one, and it
should expose named child geometry when effects or derivations need more
specific targets.

## Effects

Effects are runtime operations over a `MarkEvaluationFrame`.

There are two related categories:

- **Adjustments** produce post-scale/effect-stage output handles that authors
  can route into ordinary mark encoding methods, much like data transforms.
- **Derivations** read the current mark's frame and emit additional output.

The public adjustment authoring shape should mirror data transforms:

```rust
pub trait MarkAdjustment: Clone + Send + Sync + 'static {
    type Output;

    fn into_compiled_and_output(
        self,
        ctx: MarkAdjustmentCompileContext,
    ) -> Result<(Box<dyn CompiledMarkAdjustment>, Self::Output), AvengerChartError>;
}

pub trait CompiledMarkAdjustment: Send + Sync {
    fn apply(
        &self,
        frame: &mut MarkEvaluationFrame,
        context: &MarkEffectContext<'_>,
    ) -> Result<(), AvengerChartError>;
}
```

Marks would get an `.adjust(...)` family analogous to `.transform(...)`:

```rust
impl<C> Text<C> {
    pub fn adjust<A, F>(self, adjustment: A, f: F) -> Self
    where
        A: MarkAdjustment,
        F: FnOnce(Self, A::Output) -> Self;
}
```

The closure is important. It keeps the mark API explicit: the adjustment stage
can create or mutate effect-stage columns, and the author decides which mark
encoding channels should consume those outputs. This is the same ergonomic
pattern as:

```rust
Rect::new().transform(Bin::new(col("value")), |mark, bin| {
    mark.x(bin.start()).x2(bin.end())
})
```

but the outputs are post-scale/effect values rather than DataFusion columns.

Sketch:

```rust
Text::<Polar>::new()
    .r("r")
    .theta("theta")
    .text("label")
    .geometry_space(GeometrySpace::Coordinate)
    .adjust(KeepUpright::new(), |text, upright| {
        text.angle(upright.angle())
    });
```

`KeepUprightOutput::angle()` would reference the adjusted angle produced by
the adjustment stage. It is not a raw DataFusion expression and should not
participate in scale-domain inference.

This implies a new channel-value category or equivalent internal reference
type for effect outputs:

```rust
pub enum ChannelValue {
    Scaled { /* existing */ },
    Value { /* existing */ },
    Conditional { /* existing */ },
    EffectOutput {
        stage_id: MarkEffectStageId,
        output: EffectOutputKey,
    },
}
```

The exact representation may differ, but the semantic distinction matters:
effect outputs are evaluated after prepared data, scale application, and
initial geometry construction. They can feed mark encodings only in the render
pipeline.

Derivations can follow a similar output-handle pattern, but their closure would
configure a child/derived mark rather than reconfigure the parent mark:

```rust
pub trait MarkDerivation: Clone + Send + Sync + 'static {
    type Output;

    fn into_compiled_and_output(
        self,
        ctx: MarkDerivationCompileContext,
    ) -> Result<(Box<dyn CompiledMarkDerivation>, Self::Output), AvengerChartError>;
}

pub trait CompiledMarkDerivation: Send + Sync {
    fn create_frame(
        &self,
        source: &MarkEvaluationFrame,
        context: &MarkEffectContext<'_>,
    ) -> Result<DerivedMarkFrame, AvengerChartError>;
}
```

The source frame passed to a derivation should include prepared rows,
evaluated encodings, scaled encodings, and final source geometry after source
adjustments. The derived mark frame should be render-stage output by default:
it can emit scenegraph marks and participate in hit testing, but it does not
contribute to scale-domain inference, legends, axes, or guide planning.

If a derived mark needs to create new domains, scales, legends, or guide
entries, that should be an explicit heavier tier: a compiled-child derivation
or an ordinary layered mark created before scale planning.

`MarkEffectContext` should expose stable runtime inputs without leaking
facade-owned layout internals:

```rust
pub struct MarkEffectContext<'a> {
    pub mark_type: &'a str,
    pub geometry_space: GeometrySpace,
    pub plot_width: f32,
    pub plot_height: f32,
    pub facet_path: &'a [ScalarValue],
    pub params: &'a IndexMap<String, ScalarValue>,
    pub coord: &'a dyn CoordinateSystemTransformCore,
    pub measurement: &'a dyn CoordMeasurement,
}
```

Future versions may add text measurement, scene R-tree queries, materialized
data lookup, resource request sinks, or app/session services through explicit
capability traits.

### Geometry Query Context

Some effects only need their input frame. Smart label placement needs more:
the derived label mark needs access to the source mark's geometry and possibly
other rendered geometry, such as trend lines that labels should avoid.

The query surface should be explicit and targetable:

```rust
pub struct MarkEffectGeometryContext<'a> {
    pub input: &'a GeometryIndex,
    pub source: Option<&'a GeometryIndex>,
    pub base_scene: Option<&'a GeometryIndex>,
    pub derived_so_far: Option<&'a GeometryIndex>,
}

pub enum GeometryObstacleTarget {
    Input,
    Source,
    BaseScene,
    Marks(SceneGeometryTarget),
    PreviousDerived,
}
```

`input` is the geometry of the mark currently being adjusted. For a label
placement adjustment on a derived text mark, `input` is the candidate label
geometry. `source` is the parent mark that produced the derived labels. A
target such as `Marks(SceneGeometryTarget)` should be resolved at compile time
from public mark ids or paths rather than relying on fragile display names at
runtime.

The existing scene-query R-tree is a useful foundation, but label placement
needs a more general geometry index than hit testing. Obstacles may include
non-interactive marks, guide-like geometry, selected named marks, or only
marks before/after a particular rendering phase.

## Evaluation Order

The default v1 order should respect declared adjustment order and channel
bindings. Conceptually:

```text
compile authoring marks
  -> scale/domain/legend planning from ordinary encodings
  -> prepare mark rows
  -> evaluate ordinary encoding values
  -> apply scales to ordinary scaled encodings
  -> build an initial mark evaluation frame
     -> for compound marks, initialize named child frames as part of that frame
  -> for each adjustment stage in declaration order:
       -> apply the adjustment to the current frame
       -> resolve mark channels bound to this stage's output handle
       -> update dependent geometry in the frame
  -> emit base scenegraph marks
  -> build geometry indexes for base marks
  -> run derivation effects from source frames
  -> apply adjustments to derived frames
  -> append derived scenegraph marks with explicit ordering/z-index
```

This gives derived labels and connectors the adjusted positions of their
parent marks. For example, a connector derived from jittered symbols should
anchor to the jittered symbol positions.

Adjustment chaining should work like transform chaining: each closure updates
the mark configuration that subsequent stages see. For example, a dodge stage
can write adjusted `x`, and a later nudge stage can consume that current `x`
when computing its own output.

For compound marks, the current frame is the compound frame. The compound mark
owns how effect outputs are routed to named child frames, and child geometry
updates happen before scenegraph emission. Child marks may also support their
own internal effects, but the compound mark should present one coherent
scheduling boundary to callers.

Effects and v1 derived marks should not affect scale domains, legends, guide
generation, or layout measurement. They are render-stage modifications. If
later effects need to participate in scale or layout planning, they should use
a heavier compiled-child derivation tier with explicit scheduling.

## Authoring Sketches

### Polar Line Geometry Space

```rust
Line::<Polar>::new()
    .r("r")
    .theta("theta")
    .geometry_space(GeometrySpace::Coordinate);
```

The line mark builds its path from the geometry frame. Coordinate space means
subdivide/interpolate in scaled `r/theta` before projection. Display space
means project vertices and connect display-space chords.

### Polar Text Orientation

```rust
Text::<Polar>::new()
    .r("r")
    .theta("theta")
    .text("label")
    .geometry_space(GeometrySpace::Coordinate)
    .angle(0.0);
```

The text geometry frame can carry a local coordinate basis. For polar, the
zero-degree basis is radial outward and the ninety-degree basis is tangential.

### Upright Labels

```rust
Text::<Polar>::new()
    .r("r")
    .theta("theta")
    .text("label")
    .geometry_space(GeometrySpace::Coordinate)
    .adjust(KeepUpright::new(), |text, upright| {
        text.angle(upright.angle())
    });
```

`KeepUpright` should not need to know whether `theta` came from hours, radians,
dates, or categories. It can use either `theta.scaled` or the local display
basis from the text geometry frame. The output handle exposes the adjusted
angle, and the closure explicitly routes it into the mark's `angle` channel.

### Chained Adjustments

```rust
Text::<Cartesian>::new()
    .x("x")
    .y("y")
    .text("label")
    .adjust(Dodge::x().by("group"), |text, dodge| {
        text.x(dodge.x())
    })
    .adjust(Nudge::new(4.0, -4.0), |text, nudge| {
        text.x(nudge.x()).y(nudge.y())
    });
```

The nudge stage sees the current adjusted `x` from the previous dodge stage.
The output handles make that dataflow visible in the authoring API rather than
hiding it inside an opaque mutating pass.

### Beeswarm Plots

Beeswarm plots are a good example of an adjustment that should run after scale
evaluation. The data value controls the main axis position, while the swarm
axis is chosen in pixels to avoid overlapping symbols.

For a vertical beeswarm, authors might write:

```rust
Symbol::<Cartesian>::new()
    .x("category")
    .y("value")
    .size(36.0)
    .adjust(Beeswarm::x().group_by("category"), |symbol, swarm| {
        symbol.x(swarm.x())
    });
```

Or for a horizontal beeswarm:

```rust
Symbol::<Cartesian>::new()
    .x("value")
    .y("category")
    .size(36.0)
    .adjust(Beeswarm::y().group_by("category"), |symbol, swarm| {
        symbol.y(swarm.y())
    });
```

`Beeswarm` reads the current scaled positions, symbol size/radius, grouping
values, and optional ordering channel from the mark evaluation frame. It then
emits adjusted display-space positions through its output handle. The adjusted
swarm positions should not contribute to scale domains or axes; the domain
still comes from the original `x`/`y` encodings.

A beeswarm adjustment can also compose with ordinary encodings and later
effects:

```rust
Symbol::<Cartesian>::new()
    .x("group")
    .y("score")
    .fill("group")
    .size("weight")
    .adjust(Beeswarm::x().group_by("group").padding_px(2.0), |symbol, swarm| {
        symbol.x(swarm.x())
    })
    .adjust(Nudge::new(0.0, -1.0), |symbol, nudge| {
        symbol.y(nudge.y())
    });
```

This is a useful spike candidate because it exercises scaled encodings,
effect-stage output handles, grouping, deterministic layout, point extents,
and the rule that post-scale adjustments do not feed back into guide/domain
planning.

### Compound Marks As Effect Hosts

Compound marks should be allowed to support `adjust` and `derive` directly.
They are still marks from the author's point of view, even when they render as
several child scene marks internally.

The public API should therefore allow a compound mark to expose effect-aware
methods that route outputs into its children:

```rust
LabeledSymbol::<Cartesian>::new()
    .id("points")
    .x("x")
    .y("y")
    .label("name")
    .leader_lines(true)
    .adjust(
        PlaceLabels::new()
            .avoid(GeometryObstacleTarget::Source)
            .avoid(GeometryObstacleTarget::Marks(trend_line_target))
            .avoid(GeometryObstacleTarget::PreviousDerived),
        |mark, placed| {
            mark
                .label_x(placed.text_x())
                .label_y(placed.text_y())
                .label_visible(placed.text_visible())
                .leader_x0(placed.leader_x0())
                .leader_y0(placed.leader_y0())
                .leader_x1(placed.leader_x1())
                .leader_y1(placed.leader_y1())
                .leader_visible(placed.leader_visible())
        },
    );
```

`LabeledSymbol` can build an initial compound frame with at least:

- a primary symbol child frame,
- a label text child frame,
- an optional leader line child frame,
- shared identity that maps every emitted child back to the same source datum.

The placement adjustment runs once against the compound frame. Its output can
move the text child, toggle text visibility, and toggle or position the leader
child. A convenience API such as `.smart_labels(...)` can be sugar over this
same internal adjustment, but it should not require a separate label-placement
runtime model.

Derivations from a compound mark should read the compound's final frame after
its adjustments. The compound source handle can expose primary geometry and
named child geometry so a derived mark can choose what it anchors to:

```rust
LabeledSymbol::<Cartesian>::new()
    .id("points")
    .x("x")
    .y("y")
    .label("name")
    .smart_labels(PlaceLabels::new())
    .derive(Voronoi::new().clip_to_plot_area(true), |cell, source| {
        cell
            .x(source.primary_x())
            .y(source.primary_y())
            .datum(source.datum())
            .interactive(true)
    });
```

This keeps compound marks from becoming opaque macros. They can provide rich
domain-specific behavior while still participating in the same adjustment,
derivation, identity, obstacle-query, and event-datum contracts as simpler
marks.

### Derived Labels

```rust
Symbol::<Cartesian>::new()
    .x("x")
    .y("y")
    .derive(Label::new(), |label, parent| {
        label
            .text(parent.encoding("name"))
            .x(parent.x())
            .y(parent.y())
            .adjust(PlaceLabels::new()
                .avoid(GeometryObstacleTarget::Source)
                .avoid(GeometryObstacleTarget::Marks(trend_line_target))
                .avoid(GeometryObstacleTarget::PreviousDerived),
                |label, placed| {
                    label
                        .x(placed.x())
                        .y(placed.y())
                        .visible(placed.visible())
            })
    });
```

This derivation reads the parent symbol frame and emits text scene marks. In
v1, those text marks do not create new scales or legends. They inherit parent
row identity unless the derivation creates aggregate or synthetic instances.
The placement adjustment runs on the derived text frame, but it can query the
source mark, selected named marks, and previously accepted derived labels as
obstacles.

### Labels With Leader Lines

Smart labels often need more than text. A displaced label may need a leader
line back to its source point, while a nearby label may not. This can be
modeled as a composite mark containing `Text` plus `Rule` or `Line`, with
leader-line rendering optional per row or disabled globally.

The important requirement is that one placement result can feed multiple
child marks inside the composite. A placement adjustment should be able to
expose handles such as:

```rust
pub struct PlacedLabelOutput {
    pub fn text_x(&self) -> EffectExpr;
    pub fn text_y(&self) -> EffectExpr;
    pub fn text_visible(&self) -> EffectExpr;
    pub fn leader_x0(&self) -> EffectExpr;
    pub fn leader_y0(&self) -> EffectExpr;
    pub fn leader_x1(&self) -> EffectExpr;
    pub fn leader_y1(&self) -> EffectExpr;
    pub fn leader_visible(&self) -> EffectExpr;
}
```

Then a composite label mark can route the same placement output into both its
text child and its leader-line child:

```rust
Symbol::<Cartesian>::new()
    .x("x")
    .y("y")
    .derive(Label::new(), |label, point| {
        label
            .text(point.encoding("name"))
            .anchor_x(point.x())
            .anchor_y(point.y())
            .adjust(PlaceLabels::new(), |label, placed| {
                label
                    .text_x(placed.text_x())
                    .text_y(placed.text_y())
                    .text_visible(placed.text_visible())
                    .leader_x0(placed.leader_x0())
                    .leader_y0(placed.leader_y0())
                    .leader_x1(placed.leader_x1())
                    .leader_y1(placed.leader_y1())
                    .leader_visible(placed.leader_visible())
            })
    });
```

The exact authoring shape can be cleaner than this sketch, but the model is:
derive a composite label mark, run placement once, then route placement
outputs into the composite's child encodings. `leader_visible` can be computed
per row, for example only when the text has been displaced beyond a threshold.
A global option such as `leader_lines(false)` can omit the leader child or bind
it to constant false.

Keeping text and leader lines as child marks of a composite is useful because
it preserves normal styling, z-index, hit testing, and future customization
without forcing the effect system to invent a special label-layer concept. The
runtime requirement is that composite marks can share effect-stage outputs
among their children and preserve identity from the source mark through each
emitted child mark.

The same requirement applies when the composite is not only a derived `Label`,
but a primary compound mark such as `LabeledSymbol`. In that case the compound
mark owns the symbol, text, and leader children from the start and can run its
own smart-label adjustment before emitting any child scene marks.

### Voronoi Geometry

Voronoi cells are another useful derived mark. A point mark can render visible
symbols while a derived Voronoi mark emits polygon hit regions from the same
scaled point positions.

```rust
Symbol::<Cartesian>::new()
    .id("points")
    .x("x")
    .y("y")
    .fill("category")
    .derive(Voronoi::new().clip_to_plot_area(true), |cell, point| {
        cell
            .x(point.x())
            .y(point.y())
            .datum(point.datum())
            .fill("transparent")
            .stroke("transparent")
            .interactive(true)
    });
```

`Voronoi` reads the source mark's final scaled/adjusted point anchors and
computes one polygon per source instance. The derived polygons inherit source
row identity so hover, tooltips, lasso, and point-selection style queries can
resolve back to the same datum as the visible symbol.

For interactive use, the derived datum should include the parent mark's datum,
not just the derived polygon's own geometry fields. A hover over a Voronoi
cell should expose the same data fields as a hover over the source point, plus
any derived fields such as cell area, polygon path, or nearest-neighbor
metadata if the derivation chooses to add them.

As a render-stage derived mark, Voronoi cells should not affect scale domains,
axes, legends, or guide planning. They can affect hit testing and event datum
rows. If authors want visible Voronoi diagrams as primary data geometry, they
can still use the same derivation with visible stroke/fill styling, but the
domain remains owned by the source point encodings.

This example also motivates derived geometry that is not a simple point,
rect, or text label. The effect frame should allow derivations to emit custom
path/polygon geometry while preserving source mark identity.

## Interaction With Existing Future Work

### Adjustments

[adjust-api.md](adjust-api.md) correctly identifies the missing boundary
between scaled channel evaluation and final scenegraph creation. This document
generalizes the proposed point table into a mark evaluation frame that exposes
all encodings and typed geometry views. The public authoring shape should
mirror data transforms: each adjustment returns an output handle that a closure
routes into mark encoding methods.

Initial built-in adjustments could include:

- `Nudge`: display-space pixel offset for point/text geometry,
- `Jitter`: deterministic display-space or coordinate-space random offset,
- `Dodge`: grouped offset along a selected scaled channel or basis direction,
- `KeepUpright`: text/symbol/image angle adjustment based on display basis,
- `AvoidCollisions`: later, once text measurement and scene queries exist.

### Derivations

[derive-api.md](derive-api.md) correctly separates scenegraph derivation from
compiled child mark derivation. This design keeps v1 to scenegraph derivation
from a parent frame. A future compiled-child tier can be added for derived
marks that need scale, legend, guide, or layout participation.

### Text

[text-mark.md](text-mark.md) notes that smart placement needs measured text
bounds. The effect context should eventually provide a text measurement
service, but simple orientation and nudge adjustments can come first.

### Tools

[tools.md](tools.md) calls out stable mark/datum identity and
hit-test-to-data mapping. The identity frame should be designed with tools in
mind, especially lasso selection, hover/tooltips, editable brushes, and
annotation drawing.

### Transforms

[transform-system.md](transform-system.md) remains separate. Data transforms
produce prepared mark rows before encoding and scale evaluation. Mark effects
consume prepared mark rows plus evaluated/scaled encodings after that point.

### Materialization

Async rasterized marks, M4 lines, and map tiles need resource/materialization
requests and cached runtime products. That is adjacent but separate from mark
effects. Both systems should share context concepts such as mark identity,
plot-area size, scales, params, coordinate transform, and request/session
capabilities, but materialization is not an adjustment or derivation by
itself.

### Custom Coordinates And Specialized Layouts

Hierarchical visualizations, Sankey/alluvial diagrams, parallel coordinates,
radar charts, and other custom coordinates may produce custom geometry frames.
The mark effect model should avoid assuming Cartesian point marks. It should
allow coordinate or mark crates to define typed geometry views and expose only
the capabilities their effects support.

## Crate Boundary

The frame contracts likely belong in `avenger-chart-core` if external mark and
coordinate crates should implement effects. Built-in effect implementations
could live in a peer crate such as `avenger-chart-effects` or in the mark
implementation crates that need them first.

The top-level `avenger-chart` facade may still own execution details that
depend on layout/runtime services, such as text measurement caches,
scenegraph R-tree queries, materialization request sinks, and session caches.

## Alternate Paradigms

- **Data transforms before scaling**: good for reshaping, filtering,
  aggregating, binning, and summaries that should participate in domains and
  guides. They are the wrong level for pixel-sized jitter, display-space
  nudges, collision avoidance, or label placement based on rendered geometry.
- **Scenegraph post-processing**: can move final marks, but it loses channel,
  scale, source-row, facet, and coordinate context. It also makes legends,
  guides, hit testing, event datums, and composite mark children harder to
  reason about.
- **Per-mark builder options only**: simplest for narrow built-in features,
  but it blocks a reusable adjustment and derivation ecosystem. A built-in
  `Label` mark should be sugar over the same frame/effect machinery that
  external marks can use.
- **Macro-expanded compound marks**: convenient for authoring, but too opaque
  if expansion erases the public source mark, source datum identity, shared
  placement outputs, or the ability to derive from the compound as a single
  semantic mark.
- **Explicit layered marks only**: already works when derived data is easy to
  compute before plotting. It is not enough for derived marks that depend on
  adjusted source geometry, text measurement, or scene obstacle queries.

## Readiness

Ready for a design spike, not a full implementation plan.

A narrow spike should prove:

1. A mark can construct a `MarkEvaluationFrame` from prepared rows,
   evaluated encodings, scaled encodings, and initial geometry.
2. A simple adjustment can mutate the frame before scenegraph emission.
3. Source row and event datum identity survive the adjustment.
4. The frame shape does not force unrelated marks into point geometry.

Good first spikes:

- `Nudge` for `Text<Cartesian>` or `Symbol<Cartesian>`, because it exercises
  post-scale pixel offsets with minimal coordinate complexity.
- `KeepUpright` for `Text<Polar>` after polar text exists, because it proves
  scaled coordinate access and local coordinate bases.
- `GeometrySpace` for `Line<Polar>`, because it proves coordinate-vs-display
  geometry construction without requiring the full effect system.
- `LabeledSymbol` as a compound mark with symbol, text, and optional leader
  children, because it proves compound frames, shared placement outputs,
  child routing, and datum identity across multiple emitted scene marks.

## Decisions Needed

- Whether adjustments and derivations share one low-level effect trait, while
  retaining transform-like public authoring APIs.
- Whether compound marks expose one `MarkEvaluationFrame` with named child
  frames, or a separate `CompoundMarkEvaluationFrame` type.
- How compound-level adjustments are ordered relative to child-level
  adjustments when both are present.
- How derived marks select a compound source's primary geometry versus named
  child geometry.
- Whether prepared mark rows are stored as full `RecordBatch` values,
  selected expression columns, or lazy accessors over the prepared DataFrame.
- Whether encoding columns store both unscaled and scaled values eagerly, or
  compute one side lazily.
- How to represent scalar channels, row-varying channels, conditional
  channels, and dynamic/indexed channel keys uniformly.
- How to represent effect-stage output references in mark encodings without
  confusing them with DataFusion expressions or scale-domain inputs.
- How chained adjustment stages resolve dependencies when later stages read
  channels written by earlier stages.
- How much of `MarkEvaluationFrame` is core extension API versus facade-owned
  runtime detail.
- How effect outputs are ordered relative to base marks, guides, legends,
  debug overlays, and other derived output.
- How derived render-stage marks are explicitly excluded from scale-domain,
  legend, axis, and guide planning.
- How effects interact with clipping, hit testing, event datum rows, and
  scene query results.
- How compound child marks preserve source datum identity while still exposing
  child-specific hit-test geometry and event metadata.
- How geometry indexes are built and filtered for source marks, named obstacle
  marks, non-interactive geometry, guide geometry, and previously emitted
  derived marks.
- How text measurement, collision detection, and scene R-tree queries are
  exposed without making ordinary effects depend on renderer internals.
- Whether any effects are allowed to request layout refinement, or whether
  render-stage effects must never affect layout.
- How deterministic randomness is seeded, configured, serialized, and tested
  for jitter or stochastic label-placement effects.
- How effects are serialized, named, and versioned for external crates.
