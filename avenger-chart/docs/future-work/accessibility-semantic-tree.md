# Accessibility Semantic Rendering

## Status

Direction sketch, 2026-07-11. **Deferred until the widget implementation is
complete.** This is not an active widget gate or an implementation plan.

The architectural direction is settled enough to preserve: accessibility is a
parallel semantic render target, with AccessKit on native hosts and a real
spatial semantic DOM over the canvas on browser/Wasm hosts. Detailed APIs,
crate placement, data-exploration policy, and the execution plan remain a
post-widgets design spike.

## Goal

Expose the meaning and operation of Avenger charts and controls to screen
readers and other assistive technology without replacing vector/GPU rendering:

- chart title and subtitle;
- chart description and encoding/domain summary;
- axis titles;
- legend title and items, including interactive legend items;
- widget title/group label and every widget item;
- widget role, value, checked/selected/disabled state, and actions;
- keyboard and assistive-technology focus synchronized with visible focus;
- equivalent native and browser behavior.

This note focuses on semantic rendering and screen-reader operation. A complete
accessibility pass must also cover keyboard-only operation, contrast, forced
colors, text scaling, reduced motion, target size, non-drag alternatives, and
manual assistive-technology verification.

## Core Model: A Sibling Render Target

Do not infer accessibility from the finished scene graph. Once a title,
gridline, legend swatch, checkbox check, or data point has become a generic
`Text`, `Rule`, `Rect`, or `Symbol`, much of its meaning is gone.

Instead, produce coordinated sibling projections from the same evaluated chart:

```text
Compiled chart
    |
    +-- evaluate visuals -----> SceneGraph
    +-- evaluate semantics ---> AccessibilityTree
    +-- evaluate interaction -> InteractionIndex
```

Conceptually:

```rust,ignore
struct EvaluatedChart {
    scene: SceneGraph,
    accessibility: AccessibilityTree,
    interaction: InteractionIndex,
}
```

The projections share stable identities, resolved values, visibility, layout
frames, and bounds. None is reconstructed from another.

An evaluated semantic node needs at least:

- stable semantic id and parent/child order;
- role;
- accessible name, description, and current value/value text;
- checked, selected, disabled, expanded, or invalid state where applicable;
- logical bounds and visibility;
- supported actions;
- `labelled_by` / `described_by` relationships;
- focusability and current focus.

Compiled artifacts retain the semantic source and expressions. Runtime-only
evaluated nodes contain resolved text, data-backed items, bounds, and state.
Stable ids derive from chart/guide/widget ids and canonical item identities,
not scene-array positions or formatted labels.

## Semantic Shape

The initial vocabulary should stay small and map cleanly to both AccessKit and
HTML/ARIA.

| Avenger concept | Native role direction | Browser direction |
| --- | --- | --- |
| Chart | `Figure` or `GraphicsDocument` | `<figure>` |
| Title | `Heading` | heading within `<figcaption>` |
| Subtitle/summary | `Paragraph` | paragraph / `aria-describedby` |
| Axis | labelled `Group` | labelled section or description-list entry |
| Axis title | `Label` | text naming the axis |
| Legend | `Legend` | labelled section/list |
| Static legend item | `ListItem` | `<li>` |
| Interactive legend item | `CheckBox`/`Button` | native checkbox/button |
| Widget group/title | labelled `Group` | `<fieldset><legend>` where applicable |
| Checkbox/List item | `CheckBox` | `<input type="checkbox">` |
| Radio list | `RadioGroup`/`RadioButton` | fieldset + native radios |
| Slider | `Slider` | `<input type="range">` |
| Button | `Button` | `<button>` |
| TextInput | `TextInput` | `<input type="text">` |

Axis semantics should include title and a concise domain/encoding summary, not
blindly repeat every visual tick. Static legends are lists; interactive legends
are controls with state and actions.

Do not expose every data mark by default. A chart with hundreds of thousands of
marks needs a chart summary and an optional navigable data-table/exploration
mode, not hundreds of thousands of accessibility nodes. That mode is a separate
design decision.

## Actions, Focus, And Ordering

Accessibility actions are another input origin, not a new mutation system:

```text
AccessKit action request --+
                          +--> Avenger action/transaction coordinator
DOM input/click/focus -----+
```

Button activation, Checkbox/Radio changes, Slider increment/decrement/set-value,
legend toggles, and TextInput edits must enter the same validated FIFO
coordinator used by pointer, keyboard, event-binding, and host parameter writes.
They must not bypass reaction ordering or write directly to a session.

One focus owner is shared by pointer, keyboard, AccessKit, and DOM focus. A
focus change invalidates the semantic state and the scene-graph focus-ring part.
The visible focus ring remains scene-graph-rendered in both hosts.

Semantic/Tab order follows the logical chart and composition order, not scene
z-order or absolute coordinates. Static labels are virtual-cursor content, not
Tab stops. Interactive items alone join sequential keyboard focus unless a
future chart-exploration mode defines its own composite-widget navigation.

## Native Adapter: AccessKit

[AccessKit](https://accesskit.dev/) is designed for toolkits that draw their own
interfaces. The toolkit pushes an initial tree and subsequent updates; platform
adapters expose it through macOS NSAccessibility, Windows UI Automation, and
Unix AT-SPI, and return assistive-technology action requests.

Two native host paths consume the same `AccessibilityTree`:

1. **Direct winit hosts:** translate to AccessKit nodes and use
   `accesskit_winit::Adapter` in the owning event loop.
2. **egui hosts:** graft the Avenger chart subtree beneath the chart's egui
   accessibility node. Do not create a second AccessKit adapter for the same
   window.

Egui's model is the direct precedent: widgets contribute stable-id AccessKit
nodes with roles, labels, values, states, bounds, and actions; the pass emits a
tree update; `egui-winit` sends it to AccessKit; returned action requests become
egui input events. Avenger's custom-painted chart is opaque to egui unless we
add its nodes explicitly.

At the 2026-07-11 audit, Avenger used egui/eframe 0.33.3 and selected eframe
with `default-features = false`, enabling only `default_fonts` and `wgpu` in
`avenger-egui` and `avenger-chart-egui`. Native AccessKit support was therefore
not enabled. The post-widgets spike must re-audit current versions and make
native accessibility an explicit host feature rather than assuming an eframe
default.

Bounds are in evaluated Avenger logical pixels, then transformed through the
host frame, scale, and DPI into the coordinate system required by the adapter.
The transformation must reuse the same frame provenance as rendering and hit
testing.

## Browser/Wasm Adapter: Spatial Semantic DOM

AccessKit's web adapter for canvas-rendered toolkits remains planned. Eframe's
current web fallback is an experimental speech-synthesis path over recent egui
output events, not a browser accessibility tree. Avenger should therefore emit
real HTML semantics.

The browser host owns one relative container with two sibling projections:

```text
relative chart host
    +-- canvas / WebGPU surface
    +-- transparent semantic DOM layer
```

The DOM layer mirrors semantic concepts, not visual primitives. Meaningful
elements are spatially aligned with their visible counterparts:

- title/subtitle and axis titles use their text bounds;
- legend items use their complete item rows;
- Button/TextInput use their complete control frames;
- Checkbox/Radio items use the full labelled hit row, not only the glyph;
- Slider uses the complete operable track/control region;
- the plot gets one chart/exploration region.

Spatial alignment supports touch exploration, magnification, voice control,
accessibility inspection, and synchronization of DOM focus with the canvas
focus ring. DOM source order remains the meaningful reading and focus order;
absolute positioning never defines semantics.

Interactive browser nodes are real native elements, visually transparent but
not `display:none`, `visibility:hidden`, or `aria-hidden`. The layer itself may
use `pointer-events: none`; operable controls may use `pointer-events: auto` and
become the canonical browser input surface for those widget regions. Canvas
pointer dispatch remains active elsewhere. The two paths must never process one
gesture twice.

DOM `focus` updates Avenger focus and redraws the scene focus ring. Canvas
pointer focus focuses the matching DOM control with `preventScroll`. DOM
`click`, `input`, `change`, keyboard, and assistive-technology actions translate
to the same Avenger actions as native requests.

The planned Wasm text/IME agent for `TextInput` should become this real
positioned accessible input rather than creating a second, competing hidden
control.

Logical bounds convert to CSS pixels from the canvas client size and evaluated
scene size, not framebuffer pixels. The overlay shares the canvas parent so page
scroll moves both together. Resize, responsive layout, page zoom, widget
measurement, and visibility changes update bounds; paint-only changes do not.
A `ResizeObserver`/host layout invalidation path is preferable to polling.

HTML canvas fallback descendants remain a possible non-spatial fallback, but
the sibling overlay is the chosen direction because it produces real CSS boxes
matching the rendered interface. There must be only one active semantic tree;
do not emit both an offscreen duplicate and a spatial overlay.

## Post-Widgets Design And Implementation Sequence

1. **Semantic contract spike:** audit current AccessKit/egui/browser behavior;
   choose crate placement and whether the core vocabulary wraps or maps to
   AccessKit; freeze node ids, roles, state, actions, bounds, focus, compiled vs
   evaluated forms, and serialization policy.
2. **Headless semantic renderer:** produce deterministic semantic trees beside
   scene and interaction outputs; add tree validation, stable-id/diff tests, and
   action routing into the transaction coordinator.
3. **Native AccessKit adapter:** direct winit adapter plus egui subtree graft;
   bounds/DPI/focus/action tests; no duplicate window adapter.
4. **Browser semantic-DOM adapter:** spatial overlay, native widget proxies,
   ResizeObserver/layout synchronization, focus/action bridge, and TextInput
   host reuse.
5. **Chart semantics:** titles, subtitles, axis titles/domain summaries,
   legends, chart description, and optional author-provided alternative text.
6. **Widget semantics:** Button, Checkbox, lists, Radio, Slider, and TextInput;
   stable data-backed item identity and live value/state updates.
7. **Capstone and hardening:** one chart with every semantic layer plus
   interactive legend and widgets, native and Wasm parity, export/static-HTML
   considerations, documentation, and manual assistive-technology matrix.

## Verification

- Headless semantic-tree snapshots verify hierarchy, roles, names, states,
  values, actions, focus, bounds, stable ids, and update minimality.
- Direct-vs-bincode chart evaluation produces equivalent semantics.
- Accessibility updates occur for semantic state changes, not hover-only or
  paint-only frames; rapid chart evaluation does not spam live announcements.
- AccessKit consumer tests exercise the native tree without requiring an OS
  screen reader for every unit test.
- Browser tests inspect real DOM roles/names/states/order/bounds, run automated
  accessibility checks, and cover keyboard-only operation and focus-ring sync.
- Manual matrix includes at least VoiceOver on macOS and Safari/Wasm, NVDA on
  Windows and a Chromium/Firefox Wasm host, plus a Unix AT-SPI reader when that
  host is supported.
- Zoom, responsive reflow, DPR changes, nested CSS transforms, scrolling,
  hidden/retained views, and overlapping controls preserve semantic geometry
  and focus.

## Open Decisions For The Post-Widgets Spike

- New `avenger-accessibility` crate vs contracts in an existing host-neutral
  crate.
- Small Avenger-owned role vocabulary vs AccessKit schema types in core
  artifacts.
- Author API for chart summary/alternative text and automatic fallback quality.
- Default data-summary and opt-in point/table exploration model.
- Virtualization policy for large data-backed widget lists.
- Exact transparent-control CSS and pointer-ownership policy across browser and
  assistive-technology combinations.
- Static HTML/PDF accessibility output beyond the live native/Wasm runtimes.

## References

- [AccessKit: how it works](https://accesskit.dev/how-it-works/)
- [AccessKit role vocabulary](https://docs.rs/accesskit/latest/accesskit/enum.Role.html)
- [egui accessibility discussion and current support](https://github.com/emilk/egui)
- [HTML canvas accessibility](https://html.spec.whatwg.org/multipage/canvas.html)
- [WCAG 2.2 Meaningful Sequence](https://www.w3.org/WAI/WCAG22/Understanding/meaningful-sequence)
- [WCAG 2.2 Focus Order](https://www.w3.org/WAI/WCAG22/Understanding/focus-order.html)
- [WAI custom controls guidance](https://www.w3.org/WAI/tutorials/forms/custom-controls/)
