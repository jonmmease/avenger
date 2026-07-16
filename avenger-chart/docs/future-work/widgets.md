# Chart Widgets

## Status

Initial Rust implementation complete. Work began 2026-07-12, phases W1–W6
completed on 2026-07-14, and this specification was audited against the landed
code on 2026-07-15. The shared composed/native artifact contracts, PixelFrame
host, CSS part machinery, mixed chrome solve, Checkbox, Button, CheckboxList,
RadioButtonList, Slider, WidgetCell content tracks, explicit-frame hosting,
native-widget/TextInput runtime, parameter-change reactions, atomic state
transactions, and Button actions are implemented. Composed and native tiers
are promoted; external-toolkit embedding remains a fallback; text input is a
native widget over the in-repo Typst-based text stack. The completed execution
record is `scratch/2026-07-09/02-widgets/plan.md`.

The Avenger-language compiler is not part of this completed Rust milestone.
Its normative source surface is in `chart-dsl.md`. The Rust–DSL unification
prerequisite has landed: cursor changes are explicit ordered event effects,
and composed widget behavior uses `ToolBehaviorExpansion<PixelFrame>` while
preserving the widget-owned measurement, item, presentation, placement, and
part contracts documented here. The bootstrap `WidgetSchema`/lowerer registry
adapter is available; completing the built-in widget inventory belongs to the
language/compiler plan.

This is the Rust-first specification for interactive input controls built from
the engine's own primitives — marks, params, selections, event bindings, and,
where declarative composition runs out, arbitrary Rust emitting scene marks.

Two tiers are promoted:

- **Composed widgets** (`ChartWidget`): declarative expansions — state +
  event bindings + data-encoded marks. Checkbox, Button, checkbox list,
  radio list, slider.
- **Native widgets** (`NativeWidget`): params in → arbitrary Rust → vector
  scene marks out. Text input is the flagship, built on the engine's own
  text stack while studying how other GUI toolkits implement editing.

Companions: `chart-dsl.md` (both composed and native built-ins surface as the
same opaque, schema-registered `widget <kind> as <instance>` declarations;
the DSL has no widget definitions or widget expansion),
`dashboard-layer.md` (the dashboard layer that
eventually hosts widgets at document scope), and `tools.md` (the tool
system this design is a sibling of).

This document is deliberately independent of both the DSL and the dashboard
layer: everything here is usable from the Rust API against charts and
concats that exist today.

Sequencing receipt (2026-07-09): the Rust authoring-wrapper refactor
(`rust-authoring-wrappers.md`; execution plan in
scratch/2026-07-09/01-rust-chart-refactor/) landed **before** widget
implementation. Examples in this document use the
post-refactor API (`Chart` roots, `Subplot::name`, `configure_coord`);
the widget contracts themselves attach to `Plot` and are untouched by
that refactor.

## The Paradigm

**A widget is a tool with a face.** The engine already has the concept of a
declarative unit that mints interaction state and reacts to events:

- `ToolBehaviorExpansion` (avenger-chart-core/src/tools.rs) carries resolved
  state declarations, ordered event bindings, scale edits, marks, exports,
  instance ancestry, and component/part provenance.
- Tools receive an opaque `ToolInstanceId` before `expand()`, mint typed state
  identities from that instance, and return `ResolvedStateDeclaration` values
  with an explicit `ToolParamSharing` policy where applicable.
- Event bindings already write params (`ChartEventBinding::set_param`, with
  `set_param_at_start_scope` and friends powering drag gestures), update
  selections (`set_selection` + `SelectionUpdate`), and carry event helpers
  (`ev::event_coord`, `ev::start_coord`, `ev::datum`).

A widget reuses all of it and differs in exactly two ways:

1. **It has its own frame.** Tool chrome marks render into the host plot's
   coordinate space; widget marks render into a pixel frame the host
   assigns (a guide slot, a concat cell, or — later — a dashboard layout
   cell).
2. **It has intrinsic size.** Charts accept imposed sizes; a widget reports
   size hints (label text metrics, item counts) that the host's slot or
   track sizing consumes.

The implementation reuses state registration, event routing, scenegraph
rendering, rtree hit-testing, CSS theming, and visual-regression machinery. It
also adds the widget-specific serialized measurement/item contracts, frame
ownership, mixed chrome placement, native lifecycle, and host-service seams
described below.

Two complementary laws bound the design:

- **Item multiplicity is data, not loops.** A checkbox *list* is not N
  checkbox instances; it is one widget whose marks encode an item relation,
  one interactive item per row — the mark model applied to input controls.
  This keeps the composed tier on the engine's evaluation pipeline instead
  of growing a retained-widget-tree runtime.
- **Serialized document state crosses the widget boundary only through declared
  state handles and data reads.** The initial built-ins use params and, for
  CheckboxList, a selection. Native widgets may additionally retain ephemeral
  interaction/presentation state such as TextInput's editing buffer, caret,
  focus, and IME preedit; that state is deliberately recreated rather than
  migrated, snapshotted, or exposed as document state. Undeclared persistent
  state that affects semantic output would break hot-reload state survival,
  baselines, and future dashboard aliasing. This boundary is a contract and
  belongs in the trait docs.

## Landed Foundation

The completed implementation rests on these existing and newly landed pieces:

| Piece | Where | Role here |
| --- | --- | --- |
| `ToolBehaviorExpansion` + resolved-state registration | avenger-chart-core/src/tools.rs | the canonical state/bindings/marks/export bundle reused by tools and composed widgets |
| `ChartEventBinding::set_param` / `set_selection` | avenger-chart-core/src/event_binding.rs | widget interactions |
| start-anchored drag writes (`set_param_at_start_scope`, `ev::start_coord`) | box-select in avenger-chart-tools | the slider gesture |
| `Selection` clause model + `predicate()` | core / used by cross-filter example | checkbox-list membership state |
| `ZeroDCoord` (`NoGuide`, non-spatial mark rendering) | avenger-chart-core/src/zero_d.rs | precedent + sibling for the widget frame |
| guides (axes/legends): Rust logic + resolved state → scene marks | avenger-guides | the native-tier precedent, including CSS-resolved values consumed by Rust |
| custom marks doing arbitrary work (tiles, rasters) | avenger-chart-geo / marks | second native-tier precedent |
| `SceneGraphBuilder<State>` / `EventStreamHandler<State>` | avenger-app | the fully general form a native widget miniaturizes |
| Legend guide slots + legend stacking via `avenger-layout` | legend machinery | chart-chrome widget placement |
| Legend/text measurement services + caches | `PlotSession` measurement phase | widget size hints |
| `Sql` transform | avenger-chart-transforms | item ordering (`row_number()`) in data-encoded widgets |
| `Subplot` marks in concat plots | concat system | the `WidgetCell` sibling |
| explicit cursor event effects plus native dispatch cursor outcomes | event and native-widget runtimes | hover feedback without parameter-name conventions; composed cursor changes commit or roll back with the containing action transaction |
| `SceneKeyPressEvent`, `ClipboardEvent`, and `ImeEvent` | avenger-eventstream | keyboard editing, platform-aware shortcuts, clipboard transfer, and IME preedit/commit routing |
| the custom Typst-based text stack plus `ShapedLine`/`SingleLineEditor` | avenger-text / avenger-typst-label | shaping, line layout, measurement, cursor/selection geometry, grapheme motion, and the editing surface used by TextInput |
| avenger-format-number | formatting | slider value labels |

## Core Contracts

### `PixelFrame` coordinate system

The landed coordinate system in avenger-chart-core (sibling of `zero_d.rs`):
position channels (`x`, `y`, `x2`, `y2`) interpreted directly as pixels in
the widget's frame, identity coordinate transform, `NoGuide`. Its type-level
job is "positions are pixels; no data scales, no axes, no legends here."

`ZeroDCoord` is the spiritual precedent ("non-spatial mark rendering",
already used by legends) but has no position channels, which widget marks
need. The dedicated `PixelFrame` type avoids the rejected interim of plain
`Cartesian` with identity scales, which would drag axis/scale machinery along
and let a widget author accidentally bind a positional data scale.
(Cost note, corrected 2026-07-10: the coordinate *type* is
`zero_d.rs`-sized, but marks are implemented per coordinate system —
`Rect` exists only as `Mark<Cartesian>` — so `PixelFrame` also brings
a `Rect`/`Symbol`/`Rule`/`Text` mark matrix and an explicit
unscaled-channel mechanism; expression channels default to scaled.)

`PixelFrameTransform::channel_uses_scale` is false only for
`x`/`y`/`x2`/`y2`; generic mark preparation evaluates those expressions
directly, including conditionals. Color, opacity, size, and other channels keep
the ordinary scale path, so data-encoded widgets can use real scales without
axes/guides. Compilation records widget visual scales in a two-level registry
keyed first by widget id. Parts within one widget share its local scale names,
while host marks and other widgets never couple domains accidentally. One
prepared item relation feeds one widget scale builder; its cached domain data
is reused for provisional measurement and final-frame ranges. Widget scales
do not enter automatic axis or legend generation.

View-local transforms use `View::pixel_frame()`, not `View::cartesian()`.
That scale-free view exposes x/y domain and range parameters as
`0..frame_width` and `0..frame_height` in logical pixels, so existing async
materialization and stale-result policy work for a widget part without
creating positional scales or guides. The compiled view kind is serializable;
direct and bincode evaluation must emit equivalent requests.

### Shared types

Sizing is serializable and numeric. Each axis of `WidgetMeasureSpec` is
`Fixed { px }`, `Content { expr, min_px, max_px }`, or
`Fill { expr, min_px, max_px, stretch }`. The landed enum also has
`StyledFill { preferred, min, max_px, stretch }` for axes such as Slider width
whose minimum and preferred sizes both come from the resolved style snapshot;
its one-pixel provisional frame is replaced before final rendering.
`WidgetMeasureExpr` contains pixel constants, typed resolved-style lengths,
measured part text width/height, item-count extents, `Add`, and `Max`.
Evaluation returns finite, nonnegative
`{ min_px, preferred_px, stretch }`; content tracks consume the preferred size
and flexible hosts honor all three values. Runtime sizing closures and implicit
400×300 plot defaults are forbidden.

`WidgetItems` is also explicit and serializable:

- `Static(Vec<WidgetItemRow>)` requires identical ordered fields in every row
  and preserves declaration order with a compiler-owned monotonic `__order`.
- `DataFrame { data, order_key: Vec<Expr> }` requires a nonempty total key;
  every materialized revision rejects NULL/duplicate key tuples and a
  preexisting reserved ordering/index column, then derives `__order` and
  `__idx` before sizing or marks consume the relation.
- `Configured` is the public canonicalization wrapper produced by
  `project`, `derive_identity`, and `validate`; it records canonical value and
  label expressions, type-preserving item-id derivation, and generic
  validation without adding a widget-kind switch to the chart compiler.

Both base forms reject author columns named `__order` or `__idx` and the
reserved `__avenger_widget_` runtime-input prefix. These names, along with the
configured pipeline's `__value`, `__label`, and item-identity columns, are
implementation-only: authored expressions, schema inspection, and completion
continue to see the source relation's public columns. The generic compiler adds
only `__order` and `__idx`; each data-encoded built-in's W2 expansion projects
its source-specific fields into canonical `__value` and `__label` columns
before constructing `WidgetItems`. Evaluation builds one shared
`WidgetPreparedBaseData` per result/revision. Validation, measurement, part
mark preparation, scale domains, and event datums all consume that same object;
no part independently rematerializes the item query.

### State wiring: the three tiers (identical to tools)

1. **Auto-mint by default.** The widget mints its state at construction,
   named by widget id and role (`{id}__checked`, `{id}__value`, or
   `{id}__selection`). Expansion/state-spec returns it; the
   compiler registers it. No `.add_param()` ceremony.
2. **Consumer accessors.** Each widget exposes the expressions consumers
   need, paralleling `Selection::predicate()`:
   `checkbox.checked() -> Expr`, `radio.value() -> Expr`,
   `slider.value() -> Expr`, `text_input.value() -> Expr`,
   `checkbox_list.selected() -> Expr` (a membership predicate).
3. **Sharing override.** Widget-specific builders replace minted state with a
   caller-provided handle: `Checkbox::checked_param(Param)`,
   `Button::with_activation_param(Param)`, `CheckboxList::selection(&Selection)`,
   `Slider::value_param(Param)`, and `TextInput::value_param(Param)`. These
   support linked widgets, widget-drives-tool, and the seam the dashboard
   layer's param aliasing later plugs into. The initial Rust
   `RadioButtonList` always mints its selected-value param; the
   `WidgetSchema` registry work must add the corresponding typed
   `value_param` lowering seam before claiming uniform DSL existing-state
   binding for that kind.

Sharing scope: `ToolParamSharing::Explicit(CoordinationScope::Shared)` by
default (a chart-chrome widget is chart-global state); the mirror-the-scale
policy tools use is irrelevant in a pixel frame. Per-facet widgets are out
of scope for v1.

### Sizing pass

Size hints resolve during the existing measurement phase, alongside legend
measurement (same caches, same async context):

- Text metrics via the shared text measurement service.
- Data-encoded widgets need their item relation materialized first; item
  relations are expected small (distinct-value lists), materialized once
  and reused by sizing and mark evaluation.
- The host consumes hints: guide-slot stacking (chart chrome), content
  track sizing (concat), document layout (dashboard, later).

Widget frames never depend on widget *content* beyond these hints, so
sizing stays a single pre-solve pass — no measure/re-solve fixed point.

## The Composed Tier: `ChartWidget`

```rust
pub struct WidgetExpansion {
    /// Resolved state, event bindings, marks, and exports — the behavior
    /// bundle, fixed to the widget frame's coordinate system.
    pub behavior: ToolBehaviorExpansion<PixelFrame>,
    /// Ordered/validated item provenance for data-encoded widgets.
    pub items: Option<WidgetItems>,
    /// Serializable intrinsic measurement program.
    pub measure: WidgetMeasureSpec,
    /// Serializable persistent host-state expressions used by CSS matching.
    pub presentation: WidgetPresentationBindings,
}

pub trait ChartWidget: Send + Sync {
    fn id(&self) -> &str;
    /// Stable kind name — the CSS element (`checkbox`, `slider`, …).
    fn kind(&self) -> &'static str;

    /// Same shape as ChartTool::expand. Marks author in the widget's
    /// pixel frame; event bindings default-target the widget's own marks.
    /// The expansion carries a declarative `WidgetMeasureSpec` (per-axis
    /// SizePolicy + measurable inputs: label text exprs, item-count
    /// source, padding) — sizing is a SPEC the measurement phase
    /// evaluates each pass (the legend precedent), never a runtime
    /// closure: the compiled artifact is serializable, and a one-shot
    /// sizing call would freeze theme/param/data-dependent measurement.
    /// Runtime sizing callbacks are deliberately absent.
    fn expand(&self, ctx: WidgetExpansionContext<'_>)
        -> Result<WidgetExpansion, AvengerChartError>;
}
```

The outer widget owns and preserves `items`, intrinsic measurement,
presentation, placement, and public part behavior around the resolved
`ToolBehaviorExpansion<PixelFrame>`. This internal Rust representation does
not make a composed widget source-expandable in the DSL.

**The compiled form is serializable.** `WidgetExpansion` lowers to
`CompiledWidget::Composed(CompiledComposedWidget { id, kind, marks,
relative_target_paths, measure, items, presentation })`. Widget-local scale
specs live in the owning `CompiledPlot`'s widget-id namespace rather than in
the coordinate-scale map. The compiler owns a group named
`{id}` with structurally named child parts (`box`, `check`, `label`, and so
on); periods remain invalid inside structural ids, while public targeting uses
the existing dot-separated `{id}.{part}` path. An omitted widget mark target
expands to all interactive parts, never empty chrome whitespace or decorative
parts. The relative target registry is rebased under the group's stable scene
slot and merged before event-binding target resolution.

`CompiledWidget` is always positionless. `CompiledPlot.widgets` stores
`CompiledWidgetAttachment { widget, placement, declaration_order }`, where
placement is either a guide side or an explicit frame. `WidgetCell` embeds the
same positionless artifact. Direct evaluation and bincode round trips must
produce equivalent sizing, validation, target registries, and scenes.

`CompiledWidgetItemPlan` serializes the lowered relation, compiler-owned order
column, and generic `NonNullUnique`, `ContainsScalar`, and `ContainsParam`
validations. The latter two are available for W2 built-ins to encode radio
default/current-value requirements without a widget-kind switch in
`avenger-chart`. Validations run once per newly materialized revision before
measurement or parts. Dynamic invalidation is an evaluation diagnostic and
performs no implicit state repair.

Deliberately **not generic over a coordinate system**. `ToolBehaviorExpansion<C>`'s
generic is load-bearing for tools (a tool expands *into* a host plot and
must match it); a widget never renders into a host space — the own-frame is
its definitional difference — so its marks live in one fixed coordinate
system. Monomorphism is also what lets hosts hold
`Vec<Arc<dyn ChartWidget>>` and lets one future chrome plot host every
widget. A widget wanting exotic internal geometry embeds a plot via
subplot machinery rather than parameterizing the trait.

`scale_edits` is dead weight in a pixel frame (nothing to scale-edit); it
stays empty. If that reads too loosely in practice, `WidgetExpansion` grows
its own field set sharing types with `ToolBehaviorExpansion` — siblings, not
parent/child.

The composed tier is fully declarative internally: expansions are stateless and
the engine evaluates the marks (data-encoding, conditional channels,
`datum()`). **Prefer this tier whenever the logic fits** because it reuses the
ordinary chart engine, but that implementation shape is not a DSL authoring or
source-expansion surface. Composed built-ins serialize through their registered
kind and remain opaque in Avenger source, just like native widgets.

## The Native Tier: `NativeWidget`

Some components need logic that is miserable as SQL but trivial as Rust: a
text input's editing model, a date-picker's calendar month grid, a color
picker's gradient plane and hit math, gauges with procedural tick
geometry, gesture state machines beyond declarative bindings. The engine
already builds things this way in three places — guides (axes/legends are
Rust logic reading resolved state and emitting scene marks, including
CSS-resolved theme values), custom marks (tiles, rasters), and
`SceneGraphBuilder<State>` at app scope. A `NativeWidget` is that pattern
miniaturized to a widget frame. Three contracts remain deliberately distinct:

1. `NativeWidget` is the authoring trait: `id`, `kind`, `schema_version`,
   canonicalizable JSON payload, `NativeWidgetMeasureSpec`, and ordered
   param-only `NativeWidgetStateSpec`. It compiles without a registry.
2. `NativeWidgetFactory` is an injected, kind-keyed runtime capability. It
   validates the schema version, parses canonical JSON, evaluates registry
   sizing when requested, and creates an instance.
3. `NativeWidgetInstance` alone owns `on_state_sync`, `on_event`, dirty state,
   `scene`, `on_environment_sync`, `on_deactivate`, `on_session_detach`, and
   `on_unmount`.

The serialized variant is `CompiledNativeWidgetSpec { id, kind,
schema_version, payload: CanonicalJson, measure, state }`. Canonical JSON is a
UTF-8 compact string with recursively sorted object keys, not
`serde_json::Value`, so bincode and direct evaluation agree. Unknown
kind/version, malformed payload, duplicate state names, or a native attachment
evaluated without runtime resources are structured errors—not
deserializer lookups, empty scenes, or panics. Factories, instances, prepared
batches, and caches never serialize.

Live instances are host-owned in `NativeWidgetInstanceStore`, keyed by stable
document id, plot/member id, and widget id. A session receives immutable
registry/store handles and an explicit namespace through
`NativeWidgetRuntimeResources`; attachment epochs make stale cleanup, IME
commands, and wakeups inert after replacement. Session detach preserves the
instance/editor for rebuild, deactivation handles retained-but-hidden owners,
and final unmount runs exactly once only on store eviction.

`NativeWidgetCtx` exposes the one resolved style snapshot through typed
`part_theme(part, mark_kind)` queries and accumulates a host-neutral dispatch outcome—resolved scoped
param assignments, evaluation intent, scene/index dirty flags, focus, cursor,
dynamic consume, and `RuntimeHostCommand`s. Commands include keyed exact
wakeups, IME enable/cursor area, and clipboard writes. Rectangles are
root-canvas logical pixels; native hosts apply device scale and Wasm hosts
apply the full canvas-to-CSS-client affine transform. Instances never mutate a
`PlotSession` directly or read wall-clock time directly.

Properties of the tier:

- **Vector all the way down.** Scene marks, not rasters: SVG/PDF export
  keeps text as text and rects as rects; baselines are ordinary scenegraph
  baselines.
- **Themed like guides.** Native widgets receive the same resolved style
  snapshot used by measurement and hit testing through
  `NativeWidgetCtx::part_theme`; paint-only changes rebuild the scene while
  geometry/text changes remeasure and rebuild the sub-index.
- **Lifecycle-full, unlike the composed tier.** The instance owns exactly
  the state the design classifies as ephemeral (cursor, selection,
  scroll-within-widget, gesture progress). Document state still crosses
  only as params.
- **A Rust extension point, not a definition.** Like composed widgets and
  primitive marks, native widgets are DSL *kinds* (schema-registered and
  instantiable), but widget kinds are not definable in the language and
  `avenger expand` treats every widget declaration as opaque. The composed tier
  remains preferred where it suffices for Rust implementation reasons, not
  because it has a different language contract. **Serialization rides a kind registry**
  (2026-07-10): the compiled artifact carries a kind-keyed spec; a
  registered factory constructs instances after artifact deserialization at
  evaluation/hosting time, and
  headless/export paths construct an instance and call `scene()` without an
  app loop. `WgpuRenderer::with_native_widget_runtime` and
  `PdfRenderer::with_native_widget_runtime` use an isolated export namespace
  and detach/final-evict on success or failure, which keeps native widgets
  inside the PNG and vector-PDF stories. Measurement for a reconstructed native
  widget comes from the registered instance (or a registered
  measurement evaluator), never from a serialized closure.

The two-tier split is Rust-only. The language does not mirror it: composed and
native built-ins are both opaque registered widget kinds, and neither tier is a
DSL definition or expansion target.

**External toolkits, in passing.** A `NativeWidget` whose `scene()`
returns a single `Image` mark could wrap an external GUI component (an
egui `TextEdit` rendered offscreen, published through the image-resource /
`RenderInvalidationHub` path the tile system already uses). This is a
viable fallback — recorded here so the option is not lost — but it is
**not a promoted path**: it splits theming (foreign toolkit styles vs
CSS), rasterizes in vector exports, and imports a foreign dependency for
problems the native tier can own. Revisit only if a concrete component
proves uneconomical to build natively.

## Placement Modes

One widget type, three hosts. The contract is identical in all three; only
the frame provider differs.

Authoring remains type-disjoint. `ChartWidgetPlacementExt::position` produces
`PositionedChartWidget<W>` consumed by `.widget`; the native equivalent is
consumed by `.native_widget`. `Plot<PixelFrame>::host_widget` and
`host_native_widget` accept bare sources and select explicit-frame placement.
Internally the sealed `WidgetSource::{Composed, Native}` uses explicit
constructors—there are no overlapping blanket `From`/`Into` implementations,
so a downstream type may implement both authoring traits without ambiguity.

### 1. Chart chrome (guide slots) — the v1 target

```rust
Chart::<Cartesian>::new()                 // post-wrapper-refactor API; .widget()
    .mark(...)                            // lives on Plot and forwards
    .widget(trend_toggle.position(ChromePosition::Right))
```

Widgets occupy positioned guide slots exactly as legends do, joining the
legend stacking `Layout`. `ChromePosition` is a widget-facing alias for the
existing side-only `LegendPosition`; corners and inside overlays are not v1.
Legends and widgets form one declaration-ordered `ChromeOccupant` list and
each side gets exactly one layout solve, including reprojection. Updating a
keyed declaration preserves its first position. Widget measurements feed that
mixed solve the way legend measurements already do. This is the cheapest placement to implement
(the slot machinery is mature) and the most immediately useful: toggles,
filters, and parameter controls that belong to one chart.

### 2. Concat cells (`WidgetCell`)

```rust
Chart::<HConcat>::new()
    .selection(regions)
    .configure_coord(|c| c.widths([TrackSizing::Auto, TrackSizing::Flex(1.0)]))
    .mark(WidgetCell::widget(region_filter).name("filters"))
    .mark(Subplot::new(scatter).name("scatter"));
```

`WidgetCell` is `Subplot`'s widget sibling and does **not** create an implicit
default-sized `Plot<PixelFrame>`. It embeds a positionless compiled widget,
forwards its numeric preferred size to `GridCell.content_size`, rebases its
targets under the cell path, and registers its state in the enclosing compile
scope. Because a concat is one `CompiledPlot`, shared
params/selections and cross-filtering run through the existing scoped
state machinery — **control-panel layouts ship on today's runtime, before
any dashboard layer exists.** Requires content-sized concat tracks
(`TrackSizing::Auto` fed by widget measurements).

### 3. Dashboard chrome plot (future)

The dashboard layer (`dashboard-layer.md`) hosts every
widget as a hygienically-named group inside one document-spanning
`Plot<PixelFrame>`, frames assigned by the document layout — via the
**explicit-frame hosting API** (2026-07-10): per-evaluation frame
assignments (`widget_id → frame rect`) supplied on session/evaluation
state, with the evaluated frame state retained for event provenance,
so re-assignment moves a widget without recompilation. Nothing in the
widget contract changes; it is listed here only to show the contract was
designed against all three hosts.

The explicit-frame API is already part of the widget contract even though the
dashboard host is deferred. `WidgetFrame` rejects non-finite coordinates and
negative/non-finite extents; `WidgetFrameAssignments` rejects duplicates;
evaluation requires exactly one assignment for every explicit attachment and
rejects unknown ids or guide-widget assignments. `EvaluatedWidgetFrameState`
retains both widget-id and final numeric mark-path maps for event provenance.
An x/y-only move reuses child geometry and rebuilds placement/index state;
width/height changes invalidate measurement, frame inputs, geometry, and hit
data without recompilation. V1 rejects facet/repeat-local multi-instantiation.

## Built-In Widgets

The initial implementation ships six widgets: five composed and one native.
All live in `avenger-chart-widgets`; there is no per-widget engine code.

### `Checkbox` — composed, scalar boolean

```rust
let trend_toggle = Checkbox::new(
    "trend_toggle",
    "Show 3-month trend",
    true,
);

chart.mark(trend_line().visible(trend_toggle.checked()))
    .widget(trend_toggle.position(ChromePosition::Right));
```

- **State**: one shared boolean param (default name
  `{widget_id}__checked`). `param() -> &Param` and `checked() -> Expr` expose
  it; `checked_param(existing)` supplies an external boolean parameter.
- **Marks**: box `Rect` (named `box`), two-segment check-glyph `Rule` (named
  `check`) with `.visible(param.expr())` — the widget's own glyph runs on
  the same mechanism it controls — and a `Text` label.
- **Bindings**: `Click` on any interactive widget part →
  `set_param(checked_param, not(checked_param.expr()))`;
  `mark_mouse_enter`/`leave` currently set a generated cursor-kind param for
  pointer feedback. Rust–DSL unification replaces those writes with the
  explicit cursor effect before DSL lowering; the cursor is never a widget
  state export.
- **Sizing**: `Content(choice-control-size + control-label-gap + measured
  label)` × `Content(resolved host height)`.

### `Button` — composed, momentary activation

```rust
let clear = Button::new("clear").label("Clear selection");
let clear_selection = ChartParamChangeBinding::on(clear.activation_param())
    .set_selection(&regions, SelectionUpdate::Clear);

chart
    .param_change_binding(clear_selection)
    .widget(clear.position(ChromePosition::Right));
```

- **State**: one shared `UInt64` activation parameter (default name
  `{widget_id}__activations`), initially zero. A
  pointer activation increments exactly once; overflow is a structured event
  assignment error. A boolean pulse is forbidden because coalescing can lose
  occurrences. `activation_param() -> Param` identifies the reaction source;
  `activations() -> Expr` reads its count. An external-param override follows
  the same sharing convention as other widgets.
- **Marks**: bounded background/outline, label, and a decorative focus-ring
  part. The
  whole interactive surface activates; decorative parts do not enter the hit
  registry.
- **Actions**: W1 shipped the counter. W6 added
  `Button::action(ChartAction)` as sugar for a parameter-change binding while
  retaining the independent plot-level reaction API. The exit example clears
  a selection.
- **Sizing**: width is the greater of themed minimum width and measured label
  plus twice the themed inline padding; height is the resolved host height.

### `CheckboxList` — composed, data-encoded multi-select

```rust
let regions = Selection::new("regions").empty_selects_all();
let region_filter = CheckboxList::new("region_filter", region_items)
    .value(col("region"))
    .label(col("label"))
    .selection(&regions);

scatter_mark.transform_no_output(Filter::new(regions.predicate()), |m| m);
```

- **State**: selection membership — checked ⇔ the item's typed equality
  predicate is present. Toggling emits
  `SelectionUpdate::toggle_equality_value(field_expr,
  ev::datum("__value"), ev::datum("__item_id"))`, i.e. *semantically identical to
  clicking marks under point selection*; the widget is an alternate
  rendering of an existing interaction. `empty_selects_all()` gives
  filter-style semantics (nothing checked = no filter). Default tier mints
  the selection; `.selection(&external)` is the common override because
  cross-filtering wants the handle in other charts.
- **Item pipeline**: project the author expressions to `__value` and
  `__label` once, while retaining the original field expression for selection
  clauses. Static rows are stamped with one-based declaration ordinals before
  their single materialization; `__idx = __order - 1` is therefore stable and
  zero-based. An arbitrary DataFrame must declare a nonempty total-order key;
  `row_number()` derives the ordinal from that key before collection. Every
  revision rejects NULL or duplicate key tuples, NULL or duplicate typed
  values, and NULL or duplicate type-preserving item identities. There is no
  implicit `ORDER BY __label`: labels may repeat or change without reordering
  navigation. Positions are `__idx * item_height` arithmetic—no positional
  scales.
- **Marks**: box rects for every row; checked overlay behind
  `Selection::contains_equality_value(field_expr, col("__value"))`, a
  **membership-display predicate** (is a clause for this item's value
  present in the selection?) — corrected 2026-07-10: under
  `empty_selects_all` the *filter* predicate is true-for-all when
  nothing is checked, so `Filter::new(selection.predicate())` would
  draw every box checked; display state and filter state are distinct
  serializable membership expression independent of clause ids; text labels.
- **Sizing**: `Content(box + gap + max measured label)` ×
  `Content(n_items * item_height)`.
- **Extensions** (recorded, not v1): store-backed variant (a store with a
  `checked` column as both row source and write target, consumed by SQL
  semijoin), `max_items_visible` viewport, search-filter param, per-item
  `color` channel riding a shared ordinal scale (the interactive-legend
  configuration).

### `RadioButtonList` — composed, data-encoded single-select

```rust
let measure = RadioButtonList::new("measure", measure_items)
    .item_value(col("key"))
    .label(col("title"))
    .default("revenue");

// consumer: CASE-switch an encoding on measure.value()
```

- **State**: one scalar param holding the selected item's value. A nonempty
  static list defaults to its first declaration-order value when the value is
  a bare column. Computed static values and arbitrary DataFrames require
  `.default(...)`. Serialized validation requires both the default and live
  parameter value to remain present after every item or parameter revision;
  invalid values are errors, never implicit resets. Accessor `value() -> Expr`.
  Radio semantics—always exactly one selected—fall out of scalar-param
  semantics.
- **Marks**: outer circle `Symbol` per row; selected-ring and surface-center
  overlays behind
  `Filter::new(col("__value").eq(param.expr()))`; text labels. Same
  `__idx` pipeline as `CheckboxList`.
- **Bindings**: `Click` → `set_param(id, ev::datum("__value"))`.
- **Sizing**: as `CheckboxList`.

### `Slider` — composed, scalar numeric

```rust
let min_fare = Slider::new("min_fare", 0.0, 100.0)
    .step(1.0)
    .default(10.0)
    .title("Min fare")        // widget caption = title (heading law); label is the item channel
    .format(".0f");

mark.transform_no_output(Filter::new(col("fare").gt_eq(min_fare.value())), |m| m);
```

- **State**: one numeric param. Accessor `value() -> Expr`.
- **Marks**: track `Rect`; filled-portion `Rect` whose `x2` is
  `track_x0 + (param - min) / (max - min) * track_width`; circular handle
  `Symbol` at the same position; title and value `Text`, with the value
  formatted through avenger-format-number. Mark geometry reads typed,
  evaluation-local frame dimensions and resolved CSS style inputs; these are
  not document params.
- **Bindings**: the box-select drag pattern —
  `mouse_down` on thumb or track anchors the gesture, `cursor_moved`
  writes the value using frame-local coordinates, and a surface-global
  `mouse_up` ends it even after the pointer leaves the widget. Track clicks
  jump. Mapping and min-anchored quantization are pure expression arithmetic:
  `raw = min + ((frame_x - track_x0) / track_width) * (max - min)` and
  `value = clamp(min + round((raw - min) / step) * step, min, max)`.
  A nonpositive inner track collapses at frame center and ignores pointer
  writes without evaluating the division.
- **Frame-local foundation (implemented 2026-07-13)**: frame-local event coordinates
  (`ev::frame_x()`, `frame_y()`, `frame_width()`, `frame_height()`) over
  reserved `__frame_*` fields — the widget-frame analogs of the existing
  event/canvas helpers. A between gesture snapshots widget id + frame at
  gesture start and retains it through mouse-up, even if layout changes.
  Evaluation retains exact final mark-path-to-frame ownership for every
  widget descendant, and the app captures that ownership before authored
  streams can consume the mouse-down. Non-widget events receive NULLs.
- **Update cadence**: bindings reuse existing `throttle_ms`; a
  `commit: on_release` option (write a preview param during drag, commit
  on `mouse_up`) is recorded as an open question shared with the dashboard
  layer's deferred-commit forms.
- **Sizing**: CSS-derived `StyledFill` minimum/preferred width with positive
  stretch × content height. Default geometry is a 32 px control, 2 px track,
  16 px handle, 12 px labels, and 80 px minimum width.

### `TextInput` — native, single-line text

The flagship `NativeWidget`, and the reason the tier exists: full text
editing (cursor, selection, undo, clipboard, IME) is arbitrary-logic
territory. It builds on the engine's own text stack — `avenger-text`
(`TextEngine`, font resolution, measurement, rasterization, vector/PDF
output) over `avenger-typst-label`'s adapted Typst layout modules, with
rustybuzz shaping. W5 added the widget-neutral `ShapedLine` and
`SingleLineEditor` surface—cursor/selection state, x↔cursor hit testing,
selection geometry, IME splicing, and grapheme/word motion—so `TextInput` is
an *editing layer over avenger-text*, designed after
studying how other toolkits structure theirs (see
[Prior Art](#prior-art-to-study-before-building-textinput)).

```rust
let search = TextInput::new("search")
    .placeholder("Filter by name…")
    .commit(TextCommit::OnChange)     // or OnEnterOrBlur
    .debounce(150);

plot.mark(
        Symbol::new().transform_no_output(
            Filter::new(col("name").ilike(pattern_contains(search.value()))),
            |m| m,
        )
        .x(col("horsepower")).y(col("mpg")),
    )
    .native_widget(search.position(ChromePosition::Top));
```

- **Document state**: one Utf8 param holding *committed* text, with
  `TextCommit::OnChange` (param tracks keystrokes, debounced) or
  `OnEnterOrBlur` (form-style; the half-typed buffer is exactly the
  "staged form input" ephemeral state from the dashboard exploration).
  Accessor `value() -> Expr`.
- **Opt-in editing-state params** (added 2026-07-10): cursor position
  (grapheme index, integer) and selected text (Utf8) surface as
  additional minted params — `generated_widget_name(id, "cursor")` /
  `(id, "selected_text")`, accessors `cursor_position() -> Expr` and
  `selected_text() -> Expr`. Minted **only when consumed** (the
  accessor marks them live): today any param write triggers
  re-evaluation (no dependency graph), so an unused cursor param must
  not write on every arrow key. When minted they are ordinary document
  state; when not, cursor and selection keep the ephemeral
  classification below.
- **Ephemeral state** (instance fields, never serialized): the editing
  buffer with cursor and selection (unless their opt-in params are
  minted, above), horizontal scroll offset, undo stack,
  IME composition, focus flag.
- **`on_event`**: pointer down/drag → cursor placement and drag selection
  via shaped-line hit-testing; double/triple-click → word/all selection;
  keys → motion and editing operations through a per-platform binding
  table (arrows, Home/End, word-wise with modifiers, shift-selection,
  cut/copy/paste, undo/redo); text and IME preedit/commit → insertions;
  Enter → commit (and blur, per policy). `on_state_sync` implements the
  controlled-input contract: external param writes replace the buffer
  unless composing.
- **`scene()`**: background + border rects (theme tokens; focus ring when
  focused), selection highlight rects from shaped-run geometry drawn
  *behind* the text run, the text
  as a `Text` scene mark shaped by avenger-text (placeholder dimmed when
  empty), preedit underline, caret rule when focused — all clipped to the
  frame with the scroll offset applied. Caret blink is disabled for
  determinism (a static caret in baselines; blink can arrive later as
  runtime presentation policy, exempt from semantics).
- **Sizing**: `Fill` width (Fixed override) ×
  `Content(line height + padding)`.
- **avenger-text editing-support extensions** (the landed engine surface this
  widget drove): single-line shaping with per-cluster metrics exposed,
  x-offset → cursor position and cursor position → caret x mapping on a
  shaped line, selection rect geometry across runs (bidi-correct, since
  the Typst-derived layout and rustybuzz shaping are already
  bidi-capable), and grapheme-aware cursor arithmetic.
- **Shared infrastructure it added** (reused by everything after it): the
  minimal focus service (click-to-focus; keyboard and IME route to the focused
  widget until focus is lost), IME event surfacing through
  avenger-eventstream (winit `Ime::{Enabled, Preedit, Commit, Disabled}`)
  plus `set_ime_allowed` / `set_ime_cursor_area` window plumbing driven by
  the widget's reported IME rect, a clipboard service in `NativeWidgetCtx`
  plus semantic `ClipboardEvent::{Cut, Copy, Paste(text)}` events in
  avenger-eventstream,
  synthesized natively from key chords (`SceneKeyPressEvent` already
  carries `key` + `ModifiersState`, scene.rs:156) + arboard, but
  sourced from DOM `cut`/`copy`/`paste` events on wasm, where the
  permission-gated async Clipboard API cannot be read synchronously on
  a key-down — and the I-beam cursor through the native dispatch outcome.
- **Deliberate v1 limits**: single line only (no wrapping; Enter commits),
  no password masking yet, no drag-and-drop text, LTR-biased keybinding
  table first (shaping and motion are bidi-correct from the stack; the
  binding table grows).

## TextInput Prior-Art Study

> **Study complete (2026-07-10).** The answers to every question below,
> the editing-layer architecture, and the exact avenger-text exposure
> API live in `text-editing-layer.md` — the normative design for the
> widget plan's W5 phases. The list below is preserved as the study's
> original charter.

The implementation study examined how existing toolkits implement single-line
editing before the Avenger layer was written. Because the editing layer was
built over avenger-text, these were **reference designs, not substrates**. What
each contributed:

- **parley `PlainEditor` (Linebender/masonry)** — the closest structural
  match to our situation: a deliberately clean editor-state layer over a
  separate layout engine. Study its state/layout/rendering separation and
  its API boundary as the primary design template for the
  editing-over-avenger-text layer.
- **cosmic-text `Editor`** — the most complete standalone Rust editing
  machine. Study its `Action` vocabulary (motions, insert/delete,
  click/drag), cursor/selection model, and how it handles grapheme
  clusters and affinity — as a semantics checklist for our layer.
- **iced `TextInput`** — a wgpu-rendered, retained-scene toolkit's text
  input end to end. Study its grapheme-aware `Value` wrapper, cursor
  state machine, click-to-cursor hit-testing against shaped text, and IME
  handling.
- **egui `TextEdit`** — a complete, battle-tested editing model in a
  different style. Study its `Undoer` (time-batched undo — settle +
  auto-save intervals, no edit-distance component (verified
  2026-07-10) — worth copying outright), CCursor/PCursor duality, and
  IME composition handling.
- **Slint `TextInput`** — the exact architectural precedent for this
  design: a native runtime primitive beneath a language-composed widget
  library (`LineEdit` wraps it). Study its property surface (text,
  read-only, input-type, cursor callbacks) as a checklist for ours, and
  how it reports IME cursor areas.
- **winit IME** — the platform input contract we must surface through
  avenger-eventstream: `Ime::Preedit`/`Commit` sequencing,
  `set_ime_allowed`, `set_ime_cursor_area` timing.

The study answered what editing-support API avenger-text must expose
(per-cluster metrics,
x↔cursor mapping, selection run rects — and whether any of it already
falls out of the Typst-derived layout structures); grapheme-vs-byte cursor
arithmetic and who owns it; undo batching policy; the per-platform
keybinding table (macOS word-motion and Home/End conventions differ);
double/triple-click selection conventions; scroll-to-keep-caret-visible
behavior; selection rendering across shaped/bidi runs; what IME preedit
rendering requires from the scene (underline segments, candidate window
positioning via the IME rect); the write cadence for the opt-in
cursor/selected-text params (per keystroke vs settled — study how egui
and cosmic-text expose cursor/selection state to hosts).

## Interaction And Evaluation

- **Hit-testing and routing**: widget marks are scene marks; the existing
  rtree and event-stream routing apply unchanged. Composed-tier bindings
  default-target the widget's own instance-scoped mark names
  (`{widget_id}.box`, ...); native-tier instances receive frame-local
  `WidgetEvent`s.
- **Update loop**: a widget interaction is a param/selection write →
  `EvaluationMode::Exact` on the next build, the same path as every tool.
  Visibility toggles don't perturb layout (the plot area is unchanged), so
  the common checkbox case is a cheap exact pass with no measure churn.
  Slider drags ride binding throttles exactly as pan/zoom does; text
  inputs debounce their `OnChange` commits.
- **Native-tier dirty loop**: `on_event`/`on_state_sync` return dirty;
  `scene()` runs only then. Idle cost is a retained scene fragment.
- **Cursor feedback**: composed widgets currently write a generated cursor-kind
  param on enter/leave, while native widgets report cursor intent directly in
  `NativeWidgetDispatchOutcome`. Rust–DSL unification converts composed widgets
  to the same explicit transactional cursor-effect model before the DSL is
  implemented; no cursor param is exposed in `WidgetSchema`. Hover *styling*
  (pressed/hover states) is deferred; when it arrives it should be runtime
  presentation policy, not document semantics.

## Parameter-Change Reactions And Actions

W6 added a second serializable trigger surface parallel to event bindings.
`ChartAction` is the shared payload for parameter assignments, store
assignments, selection assignments, and evaluation intent. Existing
`ChartEventBinding` fluent methods remain source-compatible and delegate to
that payload; serde accepts the old flattened representation and emits one
canonical form. Arbitrary Rust callbacks do not enter compiled artifacts.

The landed W6 representation still stores one ordered vector per mutation kind
and `ChartAction::then` concatenates those vectors by kind. It therefore does
not preserve authored order *across* param, store, and selection mutations.
This is sufficient for the initial Button convenience and its validated
single-writer reaction graph, but it is not the DSL action contract. The
Rust–DSL prerequisite replaces these arrays with one ordered action vector and
adds explicit cursor effects before any Button `action:` block or event body is
lowered from Avenger source.

`ChartParamChangeBinding::on(param)` names one registered shared source and
owns filters plus a `ChartAction`. Its immutable typed row exposes
`param_change::value()` and `previous_value()` with the source parameter's
exact Arrow type. During evaluation, `source.expr()` is also the newly staged
value and other parameter placeholders see the current staged snapshot.
Initialization and equal assignments do not fire.

All mutation origins—host `set_param`, chart events, resize, composed widgets,
and native outcomes—enter one FIFO transaction coordinator. Direct changed
parameters form wave zero; matching reactions evaluate once per initiating
transaction; their outputs form successive immutable waves; then store/
selection effects commit and one evaluation is requested. The compiled
source→written-param graph rejects unknown names, non-shared scopes,
self/transitive cycles, and multiple reactive writers for one parameter.
Evaluation failure or same-transaction store/selection collision aborts the
whole staged transaction with no published change or sink write.

Observation reports one transaction id and ordered direct+derived
`ParamChange`s through `ParamSetResult`, `param_changes_since`, and app
outputs. The observation log never recursively triggers reactions. A batch API
lets callers intentionally create one simultaneous initiating patch.

Button's monotonic activation count is the initial consumer. This supports
both `ChartParamChangeBinding::on(clear.activation_param())` and
`clear.action(ChartAction::new().clear_selection(&selection))`; changed-value
expressions can also copy or transform one parameter into another. A separate
runtime-only host subscription may be added later, but no serialized
`on_param_change` Rust closure is part of this contract.

Execution receipt (2026-07-14): W6 landed in `2ed226e5c` through `73b263c6e`.
`ChartAction` is shared by event and parameter-change triggers; legacy flattened
event JSON migrates to the canonical action form. The compiled reaction graph,
FIFO coordinator, rollback/collision rules, transaction-aware observation, and
`Button::action` are implemented. The `button_clear_selection` example and its
headless gate prove a point selection changes the scene and one activation
clears the selection and restores the scene. A populated selection and store
are also cleared in the same transaction that resets and copies typed params.

## Theming And Testing

The initial language is the approved **Avenger hybrid**: Spectrum's compact
32 px density and separate focus-ring clarity, Carbon's neutral layer
structure, and Avenger's flat square-ish geometry and Okabe–Ito dark blue
`#0072B2`. It does not ship Adobe/IBM fonts, icons, CSS, or code.

| Role | Light | Dark |
| --- | --- | --- |
| main surface | `#FFFFFF` | `#1D1D1D` |
| layer/control-group | `#F4F4F4` | `#2B2B2B` |
| field surface | `#FFFFFF` | `#323232` |
| text / strong / muted | `#202020` / `#101010` / `#5F5F5F` | `#EDEDED` / `#FFFFFF` / `#B8B8B8` |
| border / grid | `#D7D7D7` / `#E5E5E5` | `#505050` / `#484848` |
| unselected control | `#525252` | `#D1D1D1` |
| accent / selection / focus | `#0072B2` | `#0072B2` |

Standard geometry is 32 px control/row height, 14 px primary text, 12 px
auxiliary text, 1 px default borders, 3 px field radius, 4 px Button radius,
2 px choice radius, 8 px control-label/item gaps, and a separate 2 px focus
ring with a 2 px gap. Button is at least 72 px wide with 14 px inline padding;
choice controls are 14 px; slider is at least 80 px with a 2 px track and
16 px handle; TextInput has an 11 px inline inset and 1 px caret. There are no
shadows, decorative gradients, top accent stripes, or Spectrum pill Buttons.
A bounded control-group has the same 1 px border on all four sides or none; a
one-sided stroke is only a true divider. The Checkbox check is centered at
50% x / 45% y and visibly inset on every edge.

### Public CSS contract

The default `:root` defines real semantic custom properties. The exhaustive v1
families are:

- foundation: `--widget-control-height`, `--widget-font-size`,
  `--widget-aux-font-size`, `--widget-line-height`;
- strokes/focus: `--widget-border-width`, `--widget-strong-border-width`,
  `--widget-focus-width`, `--widget-focus-gap`, `--widget-focus-color`;
- radii/spacing: `--widget-radius-small`, `--widget-radius-field`,
  `--widget-radius-large`, `--widget-radius-pill`, `--widget-edge-padding`,
  `--widget-control-label-gap`, `--widget-visual-label-gap`,
  `--widget-item-gap`;
- colors: `--widget-surface`, `--widget-app-surface`,
  `--widget-field-surface`, `--widget-text`, `--widget-text-strong`,
  `--widget-text-muted`, `--widget-border`, `--widget-grid`,
  `--widget-control`, `--widget-track`, `--widget-accent`,
  `--widget-on-accent`, `--widget-accent-hover`, `--widget-accent-down`,
  `--widget-negative`, `--widget-disabled-surface`,
  `--widget-disabled-border`, `--widget-disabled-text`, and
  `--widget-selection`;
- component geometry: `--widget-button-min-width`,
  `--widget-button-inline-padding`, `--widget-button-line-height`,
  `--widget-button-radius`, `--widget-button-border-width`,
  `--widget-choice-control-size`, `--widget-radio-selected-border-width`,
  `--widget-radio-center-size`, `--widget-slider-min-width`,
  `--widget-slider-track-height`, `--widget-slider-handle-size`,
  `--widget-slider-handle-border-width`,
  `--widget-slider-handle-pressed-border-width`,
  `--widget-slider-value-padding`, `--widget-input-inline-inset`,
  `--widget-input-caret-width`, `--widget-input-placeholder-color`, and
  `--widget-input-selection-opacity`.

`WidgetStyleProperty` owns every advertised property name and expected typed
`ThemeValue`. The closed table is: `fill`/`stroke` (color), `opacity`
(finite number in 0…1), `font-family` (string), `font-size` (definite length),
`font-weight` (number or supported keyword), `stroke-width`,
`focus-ring-width`, `corner-radius`, `width`, `height`, `min-width`,
`min-height`, `padding-inline`, `padding-block`, `control-label-gap`,
`visual-label-gap`, `item-gap`, `focus-gap`, `control-height`, `border-width`,
and each component extent above (definite length), plus `cursor` (supported
cursor keyword). Unknown CSS
declarations may parse for forward compatibility, but a custom property has no
effect until a widget rule maps it to this closed consumer schema. Intrinsic
geometry accepts only finite nonnegative definite lengths (`px`, `rem`, or
reducible `calc()`); percentages, unresolved variables, cycles, and unsupported
effects are structured widget-style errors.

`WidgetPartManifest` publishes each stable part, scene-mark kind, supported
properties/states, and hit participation. Decorative parts such as focus ring,
selection, caret, and preedit are styleable but not targetable/hittable.
Composed marks retain ordinary `mark[type=…]` identity and carry dedicated
widget kind/id/part provenance. The selector parser and `CssElement` implement
real `::part()` plus shadow-host traversal, enabling
`checkbox#warning::part(box)` under normal specificity; attribute/class
emulation is forbidden. Native parts use the same resolver, not a separate
token lookup.

Custom properties use selector-aware importance/specificity/source-order
cascade and inherit through the widget shadow host and ordinary parent chain.
`:root` is the default, while exact `--name` runtime params remain intentional
environment-wide overrides. A variable in a nonmatching selector cannot leak
to another widget instance. V1 state styling uses explicit host attributes:
`variant`, `disabled`, `checked`/`selected`, `orientation`, `focus-visible`,
`hover`, and `pressed`. Persistent attributes derive from staged params;
ephemeral presentation state never mints public params. A state remains
inactive until its runtime phase can know it; interactive pseudo-classes are
not falsely claimed.

### One style snapshot

Each evaluation resolves one nonserialized `ResolvedWidgetStyleSet` from the
theme, host/part provenance, CSS-variable params, color scheme, available
frame, and presentation state. Its deterministic digest and typed host/part
styles are shared by intrinsic measurement, text metrics, mark generation,
focus construction, scene output, and hit geometry—none may re-query CSS.
Measurement/native-scene keys include that digest, relevant dimensions/state,
text config, data/revisions, and referenced variable params. CSS mutation,
media/base-font changes, runtime overrides, or geometric state changes
invalidate all affected consumers together; an x/y-only frame move remains
reusable. `Theme` therefore gains a content fingerprint/revision as part of
the W1 theming work.

Both tiers render through the scenegraph. Baselines pair light/dark output and
numeric geometry checks; interaction tests drive synthetic events through
binding/state writes to scene deltas. Tests pin custom-property scope,
id-isolation, direct `::part()` overrides, measurement/paint/hit consistency,
direct-vs-bincode equivalence, vector PDF output, symmetric container borders,
the inset check, and absence of a top stripe or superseded Spectrum constants.

## Crate Boundary

- `avenger-text`: the editing-support extensions (single-line shaping
  with per-cluster metrics, x↔cursor mapping, selection rect geometry,
  grapheme cursor arithmetic) — text-stack concerns, widget-agnostic.
- `avenger-chart-core`: `PixelFrame` (sibling of `zero_d.rs`),
  `ChartWidget`, `WidgetExpansion`, serializable measure/item/style contracts,
  `NativeWidget` authoring specs, `ChartAction` and parameter-change binding
  artifacts, plus frame-local event helpers.
- `avenger-chart`: compile-pipeline registration of widget state
  (params/stores/selections/bindings, mark hosting, frame assignment),
  `Plot::widget()`/`Chart::widget()` + `ChromePosition` (guide-slot
  integration), one mixed chrome solve, `WidgetCell`, explicit-frame
  evaluation, native registry/store injection, and parameter transactions.
- `avenger-eventstream` / `avenger-winit-wgpu` / app hosts: focus,
  typed-text/IME/clipboard events, exact wake scheduling, host-command
  coordinate conversion, and thin application of native outcomes.
- `avenger-chart-widgets`: `Checkbox`, `Button`, `CheckboxList`,
  `RadioButtonList`, `Slider`, `TextInput`. Prelude re-exports.

When the DSL arrives, every built-in here surfaces through one registered
`WidgetSchema` plus lowerer. The schema declares placement, properties, typed
state exports, parts, measurement/presentation metadata, and any property that
binds an existing compatible param or selection. The composed/native trait split
remains below that registry boundary. There is no `define widget`, stdlib widget
definition, or widget form emitted by `avenger expand`.

## Implementation Receipt

1. **Contracts + Checkbox + Button + chrome placement — complete (2026-07-13).** `PixelFrame` and
   its mark matrix; frozen composed/native serde schema; typed CSS,
   context-aware variables, real parts, state registration and target
   rebasing; one mixed guide-slot solve; `Checkbox` and activation-counter
   `Button`; baselines.
   Exit criterion: the trend-line toggle example renders and round-trips
   interaction in `chart_avenger_app`.
2. **Data-encoded widgets — complete (2026-07-13).** Widget data contexts (materialize + share
   with sizing); `CheckboxList` (new predicate-aware equality membership and
   toggle operations that preserve `empty_selects_all` filter semantics);
   `RadioButtonList`. Exit criterion: the
   region cross-filter example, checkbox list in a chrome slot. Implemented in
   `avenger-chart-app/examples/widget_region_cross_filter.rs`; the phase landed
   in `32090604a`, `4b3bf9857`, `06314afc2`, `c3fd116bb`, `d991252b6`,
   `7ee4bd823`, `e9c94f54c`, and `9c4145b64`.
3. **Slider — complete (2026-07-13).** Frame-local event coordinates and
   stable gesture-frame capture; typed mark-evaluation frame/style inputs;
   CSS-driven track/fill/handle/label marks; min-anchored normalization;
   click/drag bindings; formatting; throttling; visible widget overflow and
   light/dark baselines. The exit example is
   `avenger-chart-app/examples/widget_slider_live_filter.rs`, which live
   range-filters a scatter at interactive cadence.
4. **`WidgetCell` + content tracks — complete (2026-07-13).** Numeric hint
   plumbing, target/state rebasing, data-encoded event-datum inference,
   Auto/Flex/Px content sizing, and typed explicit-frame evaluation are
   implemented. `widget_cell_sidebar` demonstrates an intrinsic filter panel
   sharing a selection with a flexible scatter plot; its light/dark baselines
   pin the neutral four-sided container and forbid an accent top stripe. The
   dashboard host remains deferred, but phases 1–4 now provide its published
   widget input and frame contracts.
5. **`NativeWidget` + `TextInput` — complete (2026-07-14).** The prior-art study pass
   (deliverable: notes answering the questions above, plus a design for
   the editing layer and the avenger-text editing-support API); then the
   shared infrastructure — focus service, IME through avenger-eventstream,
   the clipboard service + `Cut`/`Copy`/`Paste` events (DOM-sourced on
   wasm), native-tier dirty loop; then the avenger-text
   extensions and `TextInput` itself. The exit example is
   `avenger-chart-app/examples/widget_text_input_filter.rs`; paired light/dark
   baselines cover placeholder, focused caret, and selection states. A
   bincode-round-tripped third-party native kind is exercised through PNG and
   vector-true PDF export, with structured unknown-kind errors and lifecycle
   cleanup on success/failure. Native typing, selection replacement, debounce,
   filtering, and blur were exercised against the same runnable macOS example;
   a hands-on non-synthetic IME preedit pass remains tracked separately from
   the automated `Ime::Preedit`/`Commit` coverage.
6. **Parameter-change reactions + Button actions — complete (2026-07-14).** Extract `ChartAction`,
   compile the acyclic shared-scope reaction graph, route every origin through
   one FIFO transaction coordinator, publish direct+derived changes, add
   `Button::action`, and ship the Button-clears-selection example. Implemented
   in `2ed226e5c`, `3bc9e4f5f`, `6c6857c0e`, `093141741`, `1cf2eb1f1`,
   `70518d28a`, and `73b263c6e`; the runnable exit example is
   `avenger-chart-app/examples/button_clear_selection.rs`.

The native authoring trait and frozen artifact schema landed in phase 1; the
factory, live-instance runtime, and TextInput landed in phase 5. Phase 5's
weight was the text-editing layer and cross-host services, not artifact design.

## Deliberately Out Of Scope

- DSL-authored widget definitions, widget slots, and source expansion of widget
  implementations. The DSL may instantiate and reference registered built-ins
  only; custom widget kinds are Rust registry extensions.
- External-toolkit embedding (an egui component rendered offscreen and
  composited as an `Image` mark via the tile-machinery resource path): a
  viable fallback recorded in
  [The Native Tier](#the-native-tier-nativewidget), not a promoted path —
  it splits theming, rasterizes vector exports, and imports a foreign
  dependency for problems the native tier can own.
- Popup/overlay surfaces (combo menus, tooltips escaping the frame) — no
  overlay story yet; select-from-list covers dropdown needs interim.
- Multi-line text editing; password masking; drag-and-drop text.
- General keyboard traversal (Tab order) and scenegraph-native AccessKit
  semantic nodes — the minimal focus service ships with `TextInput`; full
  traversal and accessibility are deferred to
  [accessibility-semantic-tree.md](accessibility-semantic-tree.md), explicitly
  sequenced after widgets.
- Per-facet widget instances and facet-scoped widget params.
- Hover/pressed styling states and animated transitions (runtime
  presentation policy when they arrive, never document semantics).
- Data-encoded *layout* repetition (a panel per row) — `mark subplot` is
  the designated precedent when demand appears.

## Deferred Extension Questions

The v1 choices above are settled: real `PixelFrame`; embedded resolved
`ToolBehaviorExpansion<PixelFrame>`; disjoint explicit composed/native constructors;
`avenger-text::text_edit`; side-only chrome; host-owned native instances; and
params as the only guaranteed serialized native state. These are extensions,
not unfinished initial-implementation choices:

- slider release-only commit policy and generalized form transactions;
- widget param access qualifiers for a future dashboard DSL;
- virtualization/scrolling for very long item lists;
- inside/overlay chrome positions;
- native widgets with prepared data beyond param-only document state;
- user-remappable keybinding tables;
- responsive data-query defaults versus one-time initialization. A query
  changes only when its declared dependencies change, but explicit initial-only
  evaluation policy is deferred until real use cases require it.
