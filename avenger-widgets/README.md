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
