# avenger-widgets

Reusable scenegraph controls with application-owned values and layout. The crate
builds on Avenger text, geometry, and eventstream primitives. It does not depend
on a window host, chart model, query engine, or expression language.

Describe controls with stable IDs and current values. Prepare descriptions with
the application's `TextEngine`, read their measurements, and allocate rectangles
in root-canvas logical pixels. Finish the frame, put its scene fragment in the
application scene, and install the frame as part of that successful scene build.
Deliver the returned host commands after installing the scene.

Preparation does not change live focus or dispatch host effects. A frame becomes
stale if the runtime handles another event before installation. Applications that
use `avenger-app` can keep the runtime in their cloned state and return effects
through `SceneGraphBuilder::build_with_effects`.

```rust
use avenger_widgets::prelude::*;

let mut widgets = WidgetRuntime::new();
let engine = avenger_text::default_text_engine();
let controls = vec![Checkbox::new("grid", "Show grid", true).into()];
let mut prepared = widgets.prepare(&controls, &WidgetTheme::light(), &engine)?;
let size = prepared.metrics("grid").unwrap().preferred;
prepared.place("grid", Rect::new(12.0, 12.0, size.width, size.height), None)?;
let frame = prepared.finish()?;
let scene_fragment = frame.scene.clone();
// Assemble the full scene here, then install matching widget state.
let effects = widgets.install(frame)?;
# Ok::<(), WidgetError>(())
```

Route events through `WidgetRuntime::handle` using the complete installed scene's
R-tree. Apply `WidgetAction` values synchronously before the next frame. The
runtime previews accepted changes immediately. On the next build, the supplied
value is authoritative. Echoing the event value preserves the interaction.
Supplying a different value restores or replaces it.

Merge the returned `status` into the application's `UpdateStatus` and deliver its
host commands. Keep `consume` and `suppress_click` so a control gesture does not
also activate a plot handler. Place the widget handler before plot handlers.

A button activates on an inside release, Enter press, or Space release. A
checkbox toggles on an inside release or Space release. Held activation keys do
not repeat. Tab order follows declaration order and excludes disabled or fully
clipped controls. Browser embeddings use the default `FocusBoundary::Handoff`.
Standalone native windows can opt into `FocusBoundary::Cycle`.

`semantics()` provides names, roles, values, focus, and visible bounds for adapter
code. It does not install an operating-system accessibility bridge.

Checkbox groups bind to `BTreeSet<ChoiceItemId>` and contribute one Tab stop per
enabled item. Radio groups bind to `Option<ChoiceItemId>` and contribute one Tab
stop. Arrow keys move the radio selection, wrapping across enabled items.
Both groups share `ChoiceItem` and vertical/horizontal row layout. Duplicate IDs
and unknown checked/selected items fail preparation. Disabled selections remain
visible. Item reordering preserves focus and values by ID.

`SliderDomain::stepped(0.0, 10.0, 3.0)` admits 0, 3, 6, 9, and 10. Normalization
chooses the nearest allowed value, with ties upward. Continuous domains accept
any finite value between their bounds. Their default keyboard increment is one
hundredth of the span. Invalid spans and increments that cannot advance distinct
values fail construction.

Sliders emit immediate changes and one commit on release after a change. Escape
restores the transaction's starting value. Focus or capture loss retains its last
value and emits cancellation without a commit. A different application value
cancels the transaction during frame installation. Optional readouts reserve a
fixed width, so changing digit counts do not move the track.

Render the state gallery with `cargo run --release -p avenger-widgets --example
gallery -- widgets-gallery.png`. The example uses the bundled Lato font and one
shared text engine for layout and rendering.

![Light and dark widget states](docs/images/gallery.png)

`TextInput` edits one plain-text draft. Its uniform font can be styled, and its
contents can contain Typst source. The application validates and typesets that
source elsewhere. Keep the last accepted annotation separately from the draft so
invalid markup remains available for correction.

Each edit emits `TextChanged` immediately. `TextCommitPolicy` controls only
`TextCommitted`: immediate, after a debounce interval, or on Enter/blur. Enter
flushes a pending commit, emits `TextSubmitted`, and retains focus. Ordinary blur
flushes before the focus-loss notification. Removal and disabling cancel pending
work. A commit expresses user intent, not successful application validation.

Escape cancels composition first and restores its selection. A later Escape
restores the last committed or programmatically supplied draft. Both retain field
focus. Applications can use `reset_text` for an explicit reset that immediately
cancels composition and retires queued input. During ordinary external replacement,
a composing field defers the new draft until composition ends. The latest supplied
value wins. Neither kind of programmatic replacement emits a user change event.

Text input requires the host's input-session protocol. Forward tagged keyboard,
IME, clipboard, and keyed wakeup events through the runtime. Publish every returned
host effect, including initial-build and successful-rebuild effects. The winit
host supplies this protocol on native and WASM targets. Use `with_text_shortcuts`
to select Mac conventions in a browser running on macOS.

History retains 100 groups. Typing or deletion coalesces within one second until
navigation, focus, or edit class changes. Paste, cut, and IME commits are separate
groups. External replacements clear history. Line feeds, carriage returns, tabs,
other control characters, and Unicode line/paragraph separators are removed from
single-line values and edits. For example, pasting `one\ntwo` produces `onetwo`.
Debounce intervals use millisecond wakeups, rounding a positive fraction upward.

Read-only fields allow caret navigation, selection, and copying. Caret blinking
runs only for a focused editable field with a visible caret. Selection dragging
can scroll horizontally, including while the pointer remains beyond a field edge.
Selection queries expose byte offsets at grapheme boundaries.

Allocate controls with `WidgetMetrics::preferred` or choose a smaller rectangle
and let the content clip. `minimum` describes the control's compact geometry.
`baseline` is the first text baseline relative to its preferred rectangle, and
`paint_overflow` reserves space for the focus outline. Hover, press, and focus
change paint without changing intrinsic measurements. Place the fragment at the
root origin because allocations and IME rectangles use root coordinates.

The optional placement clip constrains drawing and picking. The content also
clips to its allocation, while a focus outline may extend by `paint_overflow`.
A fully clipped control cannot receive focus. The runtime checks the complete
scene's topmost interactive mark, so an overlay can block a widget. Decorative
widget marks do not participate in picking. If another part of the application
changes interactive geometry, set `rebuild_geometry` on its update.

Use one runtime per canvas. Keep IDs stable across builds and distinct within
that runtime. Group-item IDs need to be distinct only within their group. A live
ID cannot switch control kinds. Removing and later reusing an ID starts a new
attachment, so delayed input and wakeups cannot target its replacement.

The application processes two kinds of output. `handle` returns user actions and
input effects. `install` returns lifecycle notifications and effects caused by
reconciliation, such as removal or disabling. Apply user values before the next
build. Publish install effects only with the corresponding successful scene.
Keep application work such as validation, data loading, and typesetting separate
from the widget's commit schedule.

The keyboard map follows each control's value model:

| Control | Keys |
|---|---|
| Button | Enter activates on press. Space activates on release. Escape cancels a pending press. |
| Checkbox or checkbox-group item | Space toggles on release. Escape cancels a pending press. |
| Radio group | Arrow keys move and select. Space selects the focused item. |
| Slider | Arrow keys change by one increment. Page Up/Down change by ten. Home/End select endpoints. Key repeat previews, and release commits. |
| Text input | Arrows, Home/End, Shift selection, word navigation/deletion, select all, clipboard, undo, and redo. Enter submits. Escape cancels. |

Mac text shortcuts use Command for select all, clipboard, history, and line
start/end, and Option for word navigation. Other platforms use Control for
shortcuts and words. Command+Shift+Z and Control+Y redo. `TextInput` remains a
plain source editor when another part of the application typesets its draft.
`text_is_composing` lets a caller display composition feedback without reading or
modifying the editor internals.

Concrete theme fields control fonts, spacing, paint states, and focus outlines.
Use the same `TextEngine` to prepare controls and render the full scene, including
its font configuration. The runtime emits ordinary scenegraph marks, so an idle
frame can also be exported through the SVG and PDF renderers.

```sh
cargo run --release -p avenger-widgets --example gallery -- widgets-light.png light
cargo run --release -p avenger-widgets --example gallery -- widgets-dark.png dark
cargo run --release -p avenger-widgets --example gallery -- widgets.svg
cargo run --release -p avenger-widgets --example gallery -- widgets.pdf
```

The first version supports desktop mouse and keyboard input, single-line text,
and horizontal sliders. It does not provide touch gestures, multiline or rich
text editing, container scrolling, automatic form validation, or native
accessibility nodes. Semantic snapshots are an input to a future accessibility
adapter, not a screen-reader integration.

See [Plot Style Studio](../examples/winit-widgets/README.md) for a complete native
and WASM application. The [annotation editor](../examples/winit-annotation-editor)
and [panels explorer](../examples/winit-panels) show reuse in other applications.
A higher-level chart API can map widget IDs and typed actions to its own
parameters, lay out the measured controls, and schedule work on commit. Those
bindings belong in that API. The widget crate has no chart, language, expression,
query-engine, or theme-selector dependency.
