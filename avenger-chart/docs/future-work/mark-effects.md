# Mark Effects Follow-Up

## Current State

The v1 mark-effects system is implemented. User-facing behavior is documented
in [`../../book/src/docs/mark-effects.md`](../../book/src/docs/mark-effects.md).

Implemented pieces:

- post-scale item accessors: `item.channel("x")`, `item.data("field")`, and
  `item.bbox().left()` / `right()` / `top()` / `bottom()`,
- expression-only `.adjust(...)` stages using regular DataFusion scalar
  expressions over the item frame,
- transform-backed `.adjust_transform(...)` stages with serializable compiled
  transforms and typed output handles,
- built-in adjustment transforms: `Nudge`, `Jitter`, and `Dodge`,
- primitive effect storage and compiled-plot serialization through
  `PrimitiveMarkEffects`,
- render-stage derived primitive output for built-in `Symbol`, `Rule`, `Rect`,
  and `Text` marks,
- public `.derive(...)` source hosts for `Symbol<Cartesian>` and
  `Rect<Cartesian>`,
- derived marks with inherited source-row and event-datum lineage,
- a scene-aware adjustment-transform path that can provide text measurement and
  a facet-local base scene containing non-derived data marks only,
- internal primitive effects for the `BoxPlot` outlier `Symbol<Cartesian>`
  child through `BoxPlot::outliers(...)`,
- baselines for expression adjustments, transform adjustments, primitive
  derived marks, test-only fixed labels, faceted fixed labels, and box-plot
  outlier child effects.

Current public scope is intentionally narrow:

- effect methods are inherent methods on built-in primitive marks only;
- `MarkGroup`, `Subplot`, external marks, and compound/statistical mark
  builders are not public effect hosts;
- derived output is one level deep and cannot contain mark-local data,
  mark-local data transforms, external marks, `MarkGroup`, or compound output;
- derived primitive marks are real scene marks for rendering and hit testing,
  but they are excluded from scale-domain, guide, legend, and layout planning.

## Remaining Work

### Hit Testing And Pointer Semantics

Derived marks currently inherit source datum identity and participate in normal
scenegraph hit testing when they emit geometry. That is useful for labels and
halos, but the policy is still minimal: a derived label or halo can win a hit
over its source mark if it is topmost in the hit query.

Future work should decide whether effects need:

- mark-level or derived-child pointer-event controls,
- a default "interactive alias" policy for derived decorations,
- a source-priority hit mode where derived children report the source mark
  target path,
- explicit event-datum rows for generated items that are not one-to-one with a
  source item.

### Voronoi As A Derived Path Adjustment

The preferred first Voronoi shape is still:

```rust
Symbol::<Cartesian>::new()
    .x("x")
    .y("y")
    .derive(|point| {
        PathMark::<Cartesian>::new()
            .adjust_transform(Voronoi::new(), |path, cell| {
                path.path(cell.path()).defined(cell.defined())
            })
    });
```

That keeps `.derive(...)` structural: the derived output is an ordinary
primitive path, while the richer geometry computation lives in
`.adjust_transform(...)`.

Prerequisites:

- `PathMark<Cartesian>` as derived output, if it is not already added to the
  sealed `IntoDerivedPrimitiveMark` set,
- a source-frame-aware `Voronoi` adjustment transform,
- deterministic clipping to the current plot area / facet cell,
- a clear event identity policy for one generated cell per source item,
- visual baselines for unfaceted and faceted cases.

### Richer Label Placement

The test-only fixed-label baseline transform proves text measurement, source
anchors, and facet-local base-scene obstacle queries. It does not solve general
label placement and is not part of the public API.

Next useful transforms:

- label-label avoidance within the current plot area,
- configurable candidate positions,
- optional leader-line output fields,
- priority / importance columns,
- named obstacle target sets beyond the base non-derived data scene.

Avoid making this a layout refinement pass until a concrete example shows that
single-pass render-stage placement is insufficient.

### Regular Compound Marks With Internal Derived Primitives

The valuable compound-mark direction is not public `.derive(...)` returning
`MarkGroup`. It is regular compound marks, such as a future `LabeledPoints`,
owning internal primitive children that use the same effect machinery.

The intended model:

- the compound mark owns a primary primitive child, such as `Symbol`;
- internal derived primitive children, such as `Text` and optional leader
  `Rule`, read the primary child's post-scaled item frame;
- all child output remains ordinary built-in primitive scene marks;
- the compound mark decides which child target paths are public and how source
  identity is reported;
- public `.derive(...)` continues to accept built-in primitive marks only.

This is the encapsulation payoff from the v1 system. It should be designed as
compound-mark lowering, not by widening the public derived-output type.

### Shared Compound-Level Outputs

Some compound marks need one adjustment result to drive multiple child marks.
The classic example is a label placement result that positions text, toggles
text visibility, and optionally draws or hides a leader line.

Do not add a general compound evaluation frame until a real compound mark needs
shared outputs. When it does, decide whether the internal API is:

- one primary primitive item frame plus named child frames,
- a `CompoundMarkEvaluationFrame` that owns shared generated columns,
- or compound-specific style/config hooks that apply effects directly to
  generated primitive children.

### Custom Compound And External Mark Contracts

External marks and custom compound marks are intentionally outside v1. Before
opening the contracts, answer:

- which frame types belong in `avenger-chart-core`,
- which geometry/indexing services can be exposed without facade layout
  internals,
- how compiled effect specs remain serializable across crate boundaries,
- how external marks declare supported item channels and item grain,
- how custom marks opt into derived output or source-frame access.

### Scene Access Targets

Adjustment transforms can request plot-area metadata, text measurement, source
item frames, and a base plot-area scene. The base scene excludes guides,
legends, titles, debug overlays, and derived marks.

Future obstacle targets may include:

- the adjusted mark's source geometry,
- previous derived output,
- named marks or public target paths,
- guides and legends,
- the full final scenegraph.

Each target should be added only with a motivating transform and baseline. The
base non-derived data scene is the simplest default and remains the right v1
contract.

### Polar And Geometry Space

`Line<Polar>`, `Text<Polar>`, and `GeometrySpace` v1 are implemented as
separate mark geometry semantics. `Text<Polar>` adjustments operate on the
post-projection display-space text frame, including the final display-space
`angle`; see
[`polar-geometry-space.md`](polar-geometry-space.md) and
[`polar-line-implementation-plan.md`](polar-line-implementation-plan.md).

Future polar mark effects may still need access to:

- scaled `r` / `theta` values,
- display-space anchors,
- local coordinate bases for orientation-aware text,
- coordinate-space versus display-space geometry metadata.

Keep-upright polar text is now demonstrated as a test-only adjustment transform
that chooses between the computed display angle and that angle plus 180
degrees. A public transform can be added later if this proves useful as product
API.

## Keep Out Of Scope Until Needed

- Recursive derived graphs.
- Public `.derive(...)` returning `MarkGroup` or compound output.
- Public effects on `MarkGroup`, `Subplot`, or arbitrary compound marks.
- Layout remeasurement driven by effect output.
- Cross-facet label placement or obstacle queries.
- Full-scene obstacle queries as the default.
- External effect-host contracts before the built-in primitive surface has
  another real consumer.
