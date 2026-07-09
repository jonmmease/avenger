# Chart Widgets

## Status

Draft design, 2026-07-09 (revised same day: composed + native tiers
promoted; external-toolkit embedding demoted to a recorded fallback; text
input planned as a native widget over the in-repo Typst-based text stack).
Rust-first implementation plan for the widget paradigm: interactive input
controls built from the engine's own primitives — marks, params,
selections, event bindings, and, where declarative composition runs out,
arbitrary Rust emitting scene marks.

Two tiers are promoted:

- **Composed widgets** (`ChartWidget`): declarative expansions — state +
  event bindings + data-encoded marks. Checkbox, checkbox list, radio
  list, slider.
- **Native widgets** (`NativeWidget`): params in → arbitrary Rust → vector
  scene marks out. Text input is the flagship, built on the engine's own
  text stack while studying how other GUI toolkits implement editing.

Companions: `chart-dsl.md` (the future `define widget` language surface
lowers onto the composed tier the way compound marks and tools lower onto
theirs; native widgets surface as registered kinds the way primitive marks
do), `dashboard-layer.md` (the dashboard layer that
eventually hosts widgets at document scope), and `tools.md` (the tool
system this design is a sibling of).

This document is deliberately independent of both the DSL and the dashboard
layer: everything here is usable from the Rust API against charts and
concats that exist today.

## The Paradigm

**A widget is a tool with a face.** The engine already has the concept of a
declarative unit that mints interaction state and reacts to events:

- `ToolExpansion` (avenger-chart-core/src/tools.rs) carries `params`,
  `stores`, `selections`, `event_bindings`, `scale_edits`, and `marks`.
- Tools auto-mint their params in `expand()`
  (`Param::raw_domain(generated_tool_name(&self.id, ...))`, plus an
  unconditional `enabled` param), return them in the expansion, and the
  compiler registers them automatically with a `ToolParamSharing` policy.
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

Everything else — state minting and registration, event routing, scenegraph
rendering, rtree hit-testing, CSS theming, visual-regression baselines — is
the existing machinery, unchanged.

Two complementary laws bound the design:

- **Item multiplicity is data, not loops.** A checkbox *list* is not N
  checkbox instances; it is one widget whose marks encode an item relation,
  one interactive item per row — the mark model applied to input controls.
  This keeps the composed tier on the engine's evaluation pipeline instead
  of growing a retained-widget-tree runtime.
- **Document state crosses the widget boundary only as params** (and
  declared data reads). Whatever a widget does inside — declarative
  encoding or arbitrary Rust — hidden state that affects output breaks
  hot-reload state survival, baselines, and future dashboard aliasing.
  This is a contract, not a hope, and belongs in the trait docs.

## Current Foundation

Everything the design builds on already exists:

| Piece | Where | Role here |
| --- | --- | --- |
| `ToolExpansion` + auto-registration | avenger-chart-core/src/tools.rs | the state/bindings/marks bundle widgets reuse |
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
| `param kind: cursor` | param system | hover cursor feedback (I-beam over text input) |
| the custom Typst-based text stack: `avenger-text` (`TextEngine`, font resolver, measurement, rasterization, path/PDF output) over `avenger-typst-label`'s adapted Typst layout/eval/render modules, rustybuzz shaping | avenger-text / avenger-typst-label | shaping, line layout, and measurement for text input; **no editing surface exists yet** — the editing layer is new work over this stack |
| avenger-format-number | formatting | slider value labels |

## Core Contracts

### `PixelFrame` coordinate system

A new coordinate system in avenger-chart-core (sibling of `zero_d.rs`):
position channels (`x`, `y`, `x2`, `y2`) interpreted directly as pixels in
the widget's frame, identity coordinate transform, `NoGuide`. Its type-level
job is "positions are pixels; no data scales, no axes, no legends here."

`ZeroDCoord` is the spiritual precedent ("non-spatial mark rendering",
already used by legends) but has no position channels, which widget marks
need. Plain `Cartesian` with identity scales works as an interim at the
cost of dragging axis/scale machinery along and letting a widget author
accidentally bind a data scale. Target state is the dedicated type; the
implementation is expected to be `zero_d.rs`-sized.

### Shared types

```rust
/// Sizing policy resolved before the host's slot/track solve.
pub enum SizePolicy {
    Fixed(f32),        // exact pixels
    FillAvailable,     // take what the host grants
    Content(f32),      // content-derived pixels
}

pub struct WidgetSizeHints {
    pub width: SizePolicy,
    pub height: SizePolicy,
}
```

### State wiring: the three tiers (identical to tools)

1. **Auto-mint by default.** The widget mints its state at construction,
   named by widget id (`generated_widget_name(id, "value")` following the
   `generated_tool_name` convention; for single-value widgets the param
   name is simply the widget id). Expansion/state-spec returns it; the
   compiler registers it. No `.add_param()` ceremony.
2. **Consumer accessors.** Each widget exposes the expressions consumers
   need, paralleling `Selection::predicate()`:
   `checkbox.checked() -> Expr`, `radio.value() -> Expr`,
   `slider.value() -> Expr`, `text_input.value() -> Expr`,
   `checkbox_list.selected() -> Expr` (a membership predicate).
3. **Sharing override.** `.param(&existing)` / `.selection(&existing)`
   replaces the minted state with a caller-provided handle — linked
   widgets, widget-drives-tool (`Checkbox::new("lock_zoom")
   .param(&zoom.enabled_param())`), and the seam the dashboard layer's
   param aliasing later plugs into.

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
    /// Params, stores, selections, event bindings, marks — the tool
    /// bundle, fixed to the widget frame's coordinate system.
    pub expansion: ToolExpansion<PixelFrame>,
    /// Item relation for data-encoded widgets. Marks inherit it the way
    /// group marks inherit a group's data context. None for scalar
    /// widgets (checkbox, slider).
    pub data: Option<DataFrame>,
}

pub trait ChartWidget: Send + Sync {
    fn id(&self) -> &str;

    /// Intrinsic sizing. `ctx` provides the text measurement service and
    /// the materialized item relation (for row counts / max label width).
    fn size_hints(&self, ctx: &WidgetSizeContext<'_>) -> WidgetSizeHints;

    /// Same shape as ChartTool::expand. Marks author in the widget's
    /// pixel frame; event bindings default-target the widget's own marks.
    fn expand(&self, ctx: WidgetExpansionContext<'_>)
        -> Result<WidgetExpansion, AvengerChartError>;
}
```

Deliberately **not generic over a coordinate system**. `ToolExpansion<C>`'s
generic is load-bearing for tools (a tool expands *into* a host plot and
must match it); a widget never renders into a host space — the own-frame is
its definitional difference — so its marks live in one fixed coordinate
system. Monomorphism is also what lets hosts hold
`Vec<Arc<dyn ChartWidget>>` and lets one future chrome plot host every
widget. A widget wanting exotic internal geometry embeds a plot via
subplot machinery rather than parameterizing the trait.

`scale_edits` is dead weight in a pixel frame (nothing to scale-edit); it
stays empty. If that reads too loosely in practice, `WidgetExpansion` grows
its own field set sharing types with `ToolExpansion` — siblings, not
parent/child.

The composed tier is fully declarative: expansions are stateless, the
engine evaluates the marks (data-encoding, conditional channels,
`datum()`), and the whole tier is expressible later as `define widget`
definition files. **Prefer this tier whenever the logic fits** — it is the
tier that serializes, expands, and vendors.

## The Native Tier: `NativeWidget`

Some components need logic that is miserable as SQL but trivial as Rust: a
text input's editing model, a date-picker's calendar month grid, a color
picker's gradient plane and hit math, gauges with procedural tick
geometry, gesture state machines beyond declarative bindings. The engine
already builds things this way in three places — guides (axes/legends are
Rust logic reading resolved state and emitting scene marks, including
CSS-resolved theme values), custom marks (tiles, rasters), and
`SceneGraphBuilder<State>` at app scope. A `NativeWidget` is that pattern
miniaturized to a widget frame, with `State` = its declared params plus
private ephemeral fields:

```rust
pub trait NativeWidget: Send {
    fn id(&self) -> &str;
    fn size_hints(&self, ctx: &WidgetSizeContext<'_>) -> WidgetSizeHints;

    /// Params it mints/reads/writes, plus optional small data
    /// requirements (a relation materialized and handed to `scene`).
    /// Registered by the compiler exactly like tool/widget params.
    fn state_spec(&self) -> WidgetStateSpec;

    /// External param writes flow in (controlled-input contract): a
    /// reset button, dashboard aliasing, or hot-reload restore must
    /// update the widget's internal state; the external write wins
    /// unless an IME composition is in flight. Returns dirty.
    fn on_state_sync(&mut self, params: &WidgetParamView) -> bool;

    /// Arbitrary Rust event logic; param writes + dirty + cursor + IME
    /// rect out. Events arrive frame-local, carrying any rtree hit on
    /// the widget's own named marks; the widget may also do its own
    /// geometry hit-testing (it computed the layout).
    fn on_event(&mut self, event: &WidgetEvent, ctx: &mut NativeWidgetCtx)
        -> WidgetEventResponse;

    /// Arbitrary Rust scene construction — called only when dirty.
    /// Vector marks: full export fidelity, native baselines, theme
    /// tokens via `ctx` (the guide-style CSS-to-Rust path).
    fn scene(&mut self, frame: &FrameSpec, ctx: &SceneCtx) -> Vec<SceneMark>;
}
```

Properties of the tier:

- **Vector all the way down.** Scene marks, not rasters: SVG/PDF export
  keeps text as text and rects as rects; baselines are ordinary scenegraph
  baselines.
- **Themed like guides.** Native widgets receive resolved theme tokens
  through `SceneCtx` — the same CSS-resolved-values-consumed-by-Rust path
  axes and legends use today.
- **Lifecycle-full, unlike the composed tier.** The instance owns exactly
  the state the design classifies as ephemeral (cursor, selection,
  scroll-within-widget, gesture progress). Document state still crosses
  only as params.
- **A Rust extension point, not a definition.** Like primitive marks,
  native widgets are DSL *kinds* (schema-registered, instantiable) but
  not definable in the language, and `avenger expand` treats them as
  opaque primitives. This is the cost that keeps the composed tier
  preferred where it suffices.

The two-tier split mirrors the language's own law for marks: compounds
live in the language, primitives live in Rust.

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

### 1. Chart chrome (guide slots) — the v1 target

```rust
Plot::<Cartesian>::new()
    .mark(...)
    .widget(trend_toggle.position(ChromePosition::TopRight))
```

Widgets occupy positioned guide slots exactly as legends do, joining the
legend stacking `Layout`. `ChromePosition` reuses the legend position
vocabulary. Widget size hints feed guide measurement the way legend
measurement already does. This is the cheapest placement to implement
(the slot machinery is mature) and the most immediately useful: toggles,
filters, and parameter controls that belong to one chart.

### 2. Concat cells (`WidgetCell`)

```rust
Plot::<HConcat>::new()
    .add_selection(regions)
    .widths([TrackSize::content(), TrackSize::fr(1.0)])
    .mark(WidgetCell::new(region_filter).key("filters"))
    .mark(Subplot::new(scatter).key("scatter"));
```

`WidgetCell` is `Subplot`'s widget sibling: a concat mark wrapping the
widget in an implicit `Plot<PixelFrame>` so concat sizing and placement
treat it like any cell. Because a concat is one `CompiledPlot`, shared
params/selections and cross-filtering run through the existing scoped
state machinery — **control-panel layouts ship on today's runtime, before
any dashboard layer exists.** Requires content-sized concat tracks
(`TrackSize::content()` fed by widget hints).

### 3. Dashboard chrome plot (future)

The dashboard layer (`dashboard-layer.md`) hosts every
widget as a hygienically-named group inside one document-spanning
`Plot<PixelFrame>`, frames assigned by the document layout. Nothing in the
widget contract changes; it is listed here only to show the contract was
designed against all three hosts.

## Built-In Widgets

Five ship first: four composed, one native. All live in a new
`avenger-chart-widgets` crate — no per-widget engine code.

### `Checkbox` — composed, scalar boolean

```rust
let trend_toggle = Checkbox::new("trend_toggle")
    .label("Show 3-month trend")
    .default(true);

plot.mark(trend_line().visible(trend_toggle.checked()))
    .widget(trend_toggle.position(ChromePosition::TopRight));
```

- **State**: one boolean param (name = widget id). Accessor
  `checked() -> Expr`.
- **Marks**: box `Rect` (named `box`), check-glyph `Rect`/`Path` (named
  `check`) with `.visible(param.expr())` — the widget's own glyph runs on
  the same mechanism it controls — and a `Text` label.
- **Bindings**: `Click` on the widget's marks →
  `set_param(id, not(param.expr()))`; `mark_mouse_enter`/`leave` set the
  existing cursor-kind param for pointer feedback.
- **Sizing**: `Content(box + gap + measured label)` × `Content(line
  height)`.

### `CheckboxList` — composed, data-encoded multi-select

```rust
let regions = Selection::new("regions").empty_selects_all();
let region_filter = CheckboxList::new("region_filter", region_items, &regions)
    .value(col("origin"))
    .label(col("origin"));

scatter_mark.transform_no_output(Filter::new(regions.predicate()), |m| m);
```

- **State**: selection membership — checked ⇔ the item's clause is in the
  selection. Toggling emits `SelectionUpdate::toggle_clause` with the item
  identity (`ev::datum("__value")`), i.e. *semantically identical to
  clicking marks under point selection*; the widget is an alternate
  rendering of an existing interaction. `empty_selects_all()` gives
  filter-style semantics (nothing checked = no filter). Default tier mints
  the selection; `.selection(&external)` is the common override because
  cross-filtering wants the handle in other charts.
- **Item pipeline**: project `value AS __value, label AS __label` once;
  a `Sql` stage assigns presentation order
  (`row_number() OVER (ORDER BY __label) - 1 AS __idx`); positions are
  `__idx * item_height` arithmetic — no scales.
- **Marks**: box rects for every row; checked overlay behind
  `Filter::new(selection.predicate())` — the same filtered-layer idiom the
  cross-filter example uses; text labels.
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
    .value(col("key"))
    .label(col("title"))
    .default(lit("revenue"));

// consumer: CASE-switch an encoding on measure.value()
```

- **State**: one scalar param holding the selected item's value (default
  from `.default(...)`). Accessor `value() -> Expr`. Radio semantics —
  always exactly one selected — fall out of scalar-param semantics.
- **Marks**: outer circle `Symbol` per row; selected-dot overlay behind
  `Filter::new(col("__value").eq(param.expr()))`; text labels. Same
  `__idx` pipeline as `CheckboxList`.
- **Bindings**: `Click` → `set_param(id, ev::datum("__value"))`.
- **Sizing**: as `CheckboxList`.

### `Slider` — composed, scalar numeric

```rust
let min_fare = Slider::new("min_fare", 0.0, 100.0)
    .step(1.0)
    .default(10.0)
    .label("Min fare")
    .format(".0f");

mark.transform_no_output(Filter::new(col("fare").gt_eq(min_fare.value())), |m| m);
```

- **State**: one numeric param. Accessor `value() -> Expr`.
- **Marks**: track `Rect`; filled-portion `Rect` whose `x2` is
  `(param - min) / (max - min) * frame_width()`; thumb `Symbol` at the
  same position; optional value `Text` formatted via avenger-format-number.
- **Bindings**: the box-select drag pattern —
  `mouse_down` on thumb or track anchors the gesture, `cursor_moved`
  writes `set_param(id, clamp(round_to_step(px_to_value(...))))` using
  frame-local coordinates, `mouse_up` ends it. Track clicks jump. Value
  mapping is pure SQL arithmetic; `step` is a `round(x / step) * step`
  wrapper.
- **New helper requirement**: frame-local event coordinates
  (`ev::frame_coord(x)`) and `frame_width()` — the widget-frame analogs of
  the existing `ev::event_coord(channel)` / `canvas_width()` helpers.
  These are the slider's only genuinely new engine surface.
- **Update cadence**: bindings reuse existing `throttle_ms`; a
  `commit: on_release` option (write a preview param during drag, commit
  on `mouse_up`) is recorded as an open question shared with the dashboard
  layer's deferred-commit forms.
- **Sizing**: `FillAvailable` width (with a `Fixed` override) ×
  `Content(thumb + label)`.

### `TextInput` — native, single-line text

The flagship `NativeWidget`, and the reason the tier exists: full text
editing (cursor, selection, undo, clipboard, IME) is arbitrary-logic
territory. It builds on the engine's own text stack — `avenger-text`
(`TextEngine`, font resolution, measurement, rasterization, vector/PDF
output) over `avenger-typst-label`'s adapted Typst layout modules, with
rustybuzz shaping. **The stack has no editing surface today** — no cursor
model, no x↔cursor hit-testing, no selection geometry — so `TextInput`'s
real substance is an *editing layer over avenger-text*, designed after
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
    .widget(search.position(ChromePosition::Top));
```

- **Document state**: one Utf8 param holding *committed* text, with
  `TextCommit::OnChange` (param tracks keystrokes, debounced) or
  `OnEnterOrBlur` (form-style; the half-typed buffer is exactly the
  "staged form input" ephemeral state from the dashboard exploration).
  Accessor `value() -> Expr`.
- **Ephemeral state** (instance fields, never serialized): the editing
  buffer with cursor and selection, horizontal scroll offset, undo stack,
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
  focused), selection highlight rects from shaped-run geometry, the text
  as a `Text` scene mark shaped by avenger-text (placeholder dimmed when
  empty), preedit underline, caret rule when focused — all clipped to the
  frame with the scroll offset applied. Caret blink is disabled for
  determinism (a static caret in baselines; blink can arrive later as
  runtime presentation policy, exempt from semantics).
- **Sizing**: `FillAvailable` width (Fixed override) ×
  `Content(line height + padding)`.
- **avenger-text editing-support extensions** (the new engine surface this
  widget drives): single-line shaping with per-cluster metrics exposed,
  x-offset → cursor position and cursor position → caret x mapping on a
  shaped line, selection rect geometry across runs (bidi-correct, since
  the Typst-derived layout and rustybuzz shaping are already
  bidi-capable), and grapheme-aware cursor arithmetic.
- **Shared infrastructure it pulls in** (reused by everything after it):
  the minimal focus service (click-to-focus; keyboard and IME route to
  the focused widget until focus is lost), IME event surfacing through
  avenger-eventstream (winit `Ime::{Enabled, Preedit, Commit, Disabled}`)
  plus `set_ime_allowed` / `set_ime_cursor_area` window plumbing driven by
  the widget's reported IME rect, a clipboard service in
  `NativeWidgetCtx` (arboard-style natively; the async, permission-gated
  web Clipboard API on wasm), and the I-beam cursor via the existing
  cursor-kind param.
- **Deliberate v1 limits**: single line only (no wrapping; Enter commits),
  no password masking yet, no drag-and-drop text, LTR-biased keybinding
  table first (shaping and motion are bidi-correct from the stack; the
  binding table grows).

## Prior Art To Study Before Building `TextInput`

The plan is explicitly to study how existing toolkits implement single-line
editing before writing ours. Because the editing layer will be built fresh
over avenger-text, these are **reference designs, not substrates**. What
each is for:

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
  different style. Study its `Undoer` (time/edit-distance batched undo is
  worth copying outright), CCursor/PCursor duality, and IME composition
  handling.
- **Slint `TextInput`** — the exact architectural precedent for this
  design: a native runtime primitive beneath a language-composed widget
  library (`LineEdit` wraps it). Study its property surface (text,
  read-only, input-type, cursor callbacks) as a checklist for ours, and
  how it reports IME cursor areas.
- **winit IME** — the platform input contract we must surface through
  avenger-eventstream: `Ime::Preedit`/`Commit` sequencing,
  `set_ime_allowed`, `set_ime_cursor_area` timing.

Questions the study should answer before implementation: exactly what
editing-support API avenger-text must expose (per-cluster metrics,
x↔cursor mapping, selection run rects — and whether any of it already
falls out of the Typst-derived layout structures); grapheme-vs-byte cursor
arithmetic and who owns it; undo batching policy; the per-platform
keybinding table (macOS word-motion and Home/End conventions differ);
double/triple-click selection conventions; scroll-to-keep-caret-visible
behavior; selection rendering across shaped/bidi runs; what IME preedit
rendering requires from the scene (underline segments, candidate window
positioning via the IME rect).

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
- **Cursor feedback**: enter/leave write the existing cursor-kind param
  (pointer for clickables, I-beam for text). Hover *styling*
  (pressed/hover states) is deferred; when it arrives it should be runtime
  presentation policy, not document semantics.

## Theming And Testing

- **Composed tier**: widget marks are marks — mark CSS applies. Public
  part names (`box`, `check`, `dot`, `track`, `thumb`, `label`) get
  `theme_part` provenance stamps so part selectors work
  (`checkbox::part(box)`, `slider::part(thumb)`), reusing the
  compound-mark part machinery.
- **Native tier**: widgets receive resolved theme tokens through
  `SceneCtx` — the guide path (axes and legends already consume
  CSS-resolved values from Rust). Part-selector support for native
  widgets means stamping their emitted scene marks with the same
  `theme_part` provenance.
- **Baselines**: both tiers render through the scenegraph, so
  visual-regression baselines cover them with zero new harness — vector
  everywhere, including the text input. Interaction tests drive synthetic
  events through the existing event stream (set param → assert scene
  delta), as tool tests do today.

## Crate Boundary

- `avenger-text`: the editing-support extensions (single-line shaping
  with per-cluster metrics, x↔cursor mapping, selection rect geometry,
  grapheme cursor arithmetic) — text-stack concerns, widget-agnostic.
- `avenger-chart-core`: `PixelFrame` (sibling of `zero_d.rs`),
  `ChartWidget` / `WidgetExpansion` / `WidgetSizeHints` /
  `WidgetSizeContext`, `NativeWidget` + `WidgetEvent` /
  `WidgetEventResponse` (sibling of `tools.rs`), frame-local event
  helpers.
- `avenger-chart`: compile-pipeline registration of widget state
  (params/stores/selections/bindings, mark hosting, frame assignment),
  `Plot::widget()` + `ChromePosition` (guide-slot integration),
  `WidgetCell` in the concat system, content-sized tracks, the focus
  service and IME/clipboard plumbing at the app layer.
- `avenger-chart-widgets` (new): `Checkbox`, `CheckboxList`,
  `RadioButtonList`, `Slider`, `TextInput`. Prelude re-exports.

When the DSL arrives, `define widget` lowers onto the composed tier, and
native widgets surface as registered kinds exactly as primitive marks do.
Whether Rust built-ins remain parallel natives or lower through stdlib
definitions is the same open question chart-dsl.md already records for
compound marks.

## Implementation Phases

1. **Contracts + Checkbox + chrome placement.** `PixelFrame`;
   `ChartWidget`/`WidgetExpansion`; state registration in the compile
   pipeline; guide-slot placement and stacking; `Checkbox`; baselines.
   Exit criterion: the trend-line toggle example renders and round-trips
   interaction in `chart_avenger_app`.
2. **Data-encoded widgets.** Widget data contexts (materialize + share
   with sizing); `CheckboxList` (+ `SelectionUpdate::toggle_clause` and
   `empty_selects_all` if missing); `RadioButtonList`. Exit criterion: the
   region cross-filter example, checkbox list in a chrome slot.
3. **Slider.** Frame-local coordinate helpers; drag bindings; step/format;
   throttle. Exit criterion: live range filtering of a scatter at
   interactive frame rates.
4. **`WidgetCell` + content tracks.** Concat placement; the sidebar
   control-panel example. This is also the integration point the dashboard
   layer later builds on.
5. **`NativeWidget` + `TextInput`.** First the prior-art study pass
   (deliverable: notes answering the questions above, plus a design for
   the editing layer and the avenger-text editing-support API); then the
   shared infrastructure — focus service, IME through avenger-eventstream,
   clipboard service, native-tier dirty loop; then the avenger-text
   extensions and `TextInput` itself. Exit criterion: a search box
   filtering a chart, fully vector in PNG baselines and PDF export,
   IME-verified on macOS.

The `NativeWidget` trait itself is small and may land earlier
opportunistically; phase 5's weight is the text-editing layer and its
avenger-text extensions.

## Deliberately Out Of Scope

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
  traversal and a11y remain future.
- Per-facet widget instances and facet-scoped widget params.
- Hover/pressed styling states and animated transitions (runtime
  presentation policy when they arrive, never document semantics).
- Data-encoded *layout* repetition (a panel per row) — `mark subplot` is
  the designated precedent when demand appears.

## Open Questions

- `PixelFrame` as a new coordinate system vs extending `ZeroDCoord` with
  position channels vs interim `Cartesian`-with-identity-scales. (Leaning:
  new type; the interim is acceptable scaffolding.)
- Should `WidgetExpansion` embed `ToolExpansion<PixelFrame>` (with dead
  `scale_edits`) or own its field set? (Leaning: embed first, split if it
  chafes.)
- Trait layering: do `ChartWidget` and `NativeWidget` want a shared
  supertrait (id + size hints + state spec) so hosts hold one collection,
  or does the host adapt composed widgets into the native lifecycle
  internally?
- Where in avenger-text does the editing layer live: inside the crate (a
  `text_edit` module beside `text_line`) or a small sibling crate — and
  how much of the needed metric surface already exists in the
  Typst-derived layout structures vs needs exposing?
- Slider drag ergonomics: is `commit: on_release` a per-widget property, a
  binding-level policy, or deferred to the dashboard layer's form
  semantics? (Same question for `TextInput`'s `OnChange` debounce
  defaults.)
- Do widget params want an access qualifier story (Slint's `in`/`in-out`)
  when the dashboard layer starts aliasing them, or is whole-program write
  analysis enough?
- Item virtualization for long lists (`max_items_visible` as widget-chrome
  scrolling) vs document growth — interacts with the dashboard layer's
  no-panel-scroll law.
- Where exactly does ephemeral native-widget state live relative to
  `PlotSession` recreation (hot reload should preserve the text buffer?
  or only the committed param)? Candidate: instances live at the app
  layer keyed by widget id, surviving session rebuilds; params remain the
  only *guaranteed* survivors.
- Does `ChromePosition` support overlay positions inside the plot area
  (Plotly-modebar style) in addition to guide slots?
- `NativeWidget::state_spec` data requirements: share the composed tier's
  widget data-context machinery (materialize once, hand a batch to
  `scene()`), and what size ceiling keeps that honest?
- Keybinding tables: hardcode per-platform defaults or expose a remapping
  surface from day one?
