# WGPU GUI Offscreen Rendering

This document describes the current egui-first GUI integration path for
Avenger charts rendered through `avenger-wgpu`.

The integration goal is to let a native GUI toolkit own the app shell,
widgets, input loop, and WGPU device while Avenger owns chart evaluation,
event handling, and scene rendering. The first GUI backend is `avenger-egui`.

## Ownership Model

```mermaid
flowchart TD
    Egui["egui / eframe\nwindow, widgets, input frame"]
    Handle["avenger-egui\nAvengerPlotHandle, Plot"]
    App["avenger-app\nAvengerApp"]
    ChartApp["avenger-chart-app\nChartAppState, ChartSceneGraphBuilder"]
    Events["avenger-eventstream\nWindowEvent routing"]
    Renderer["avenger-wgpu\nAvengerWgpuRenderer"]
    Pool["OffscreenTargetPool\nsampled WGPU textures"]
    Painter["egui painter\nTextureId image"]

    Egui --> Handle
    Handle --> App
    App --> Events
    App --> ChartApp
    ChartApp --> App
    Handle --> Renderer
    Renderer --> Pool
    Pool --> Painter
    Painter --> Egui
```

`egui` owns the frame loop and WGPU `Device`/`Queue` exposed by
`egui_wgpu::RenderState`. `avenger-egui` borrows those objects when it needs
to upload renderer resources, render a scene into an offscreen texture, or
register/update that texture with `egui-wgpu`.

`avenger-wgpu` stays independent of egui-specific types. Its reusable public
entry point is `AvengerWgpuRenderer`, configured by `AvengerRendererConfig`.
The renderer can set a `SceneGraph`, resize logical dimensions, encode frame
commands for a supplied render target, and render into an `OffscreenTarget`.

`WindowCanvas` and `PngCanvas` are host wrappers around the same renderer
capability. `WindowCanvas` owns a Winit surface. `PngCanvas` owns readback
resources. `avenger-egui` owns an `OffscreenTargetPool` and a stable egui
`TextureId`.

## egui Widget Shape

The public egui API intentionally looks like a normal egui widget:

```rust
let changed = ui
    .add(egui::Slider::new(&mut point_size, 32.0..=240.0).text("Point size"))
    .changed();

if changed {
    if plot_handle.set_param("point_size", point_size).changed {
        plot_handle.request_scene_rebuild_with_repaint(runtime.handle(), ctx, true);
    }
}

let output = avenger_egui::Plot::new(&plot_handle)
    .desired_size(ui.available_size_before_wrap())
    .show(ui);

if !output.events.is_empty() {
    plot_handle.request_event_dispatch_with_repaint(runtime.handle(), ctx);
}
```

`Plot::show(ui)` allocates a widget rectangle, paints the latest completed
texture, translates egui input into Avenger `WindowEvent` values, queues those
events on the handle, and returns `PlotOutput`.

`PlotOutput` contains the underlying `egui::Response`, param changes observed
since the previous show call, reserved future selection changes, the frame
status, and the Avenger events routed during that egui frame.

The initial API deliberately keeps param binding low level. Normal egui
controls use their own `.changed()` methods. Application code calls
`set_param(name, value)` and schedules a scene rebuild explicitly.

## Event Routing

egui events are the host input source for this integration. `avenger-egui`
does not use Winit directly.

`EguiEventTranslator` converts widget-local input into
`avenger-eventstream::window::WindowEvent`:

- pointer positions subtract the plot `Rect::min` and stay in egui logical
  point units;
- pointer enter, leave, move, click, release, and capture state become cursor
  and mouse events;
- wheel input is routed only while the plot is hovered;
- keyboard input is routed only while the plot has focus;
- plot size changes emit `CanvasResize`.

`Plot::show(ui)` snapshots `EguiResponseState` before entering
`ui.input(...)`. This avoids calling `Response` methods that lock egui context
while egui's input lock is held.

Queued events are dispatched through the owned `AvengerApp` by
`request_event_dispatch` or `request_event_dispatch_with_repaint`. That path
uses the same `avenger-eventstream` handlers as Winit-hosted chart apps.

## Offscreen Texture Lifecycle

`avenger-egui` renders charts into WGPU textures, not CPU images.

```mermaid
sequenceDiagram
    participant UI as egui frame
    participant Handle as AvengerPlotHandle
    participant Worker as native render worker
    participant Renderer as worker AvengerWgpuRenderer
    participant Pool as worker OffscreenTargetPool
    participant EguiWgpu as egui-wgpu Renderer
    participant Painter as egui Painter

    UI->>Handle: request_background_scene_texture(render_state, scene, dimensions)
    Handle->>Worker: enqueue latest SceneGraph + dimensions
    Worker->>Renderer: resize(dimensions)
    Worker->>Renderer: set_scene(device, queue, scene)
    Worker->>Pool: resize_or_recreate(dimensions, Rgba8Unorm)
    Worker->>Pool: acquire target excluding front texture generation
    Worker->>Renderer: encode_to_offscreen_commands(target)
    Worker->>Worker: queue.submit(command buffers)
    Worker->>Handle: publish TextureView + render generation
    Handle->>EguiWgpu: register/update native texture view
    UI->>Painter: image(TextureId, widget rect)
```

On native targets, `request_background_scene_texture(...)` starts a per-widget
render worker after the first `egui_wgpu::RenderState` is available. The worker
owns an independent `AvengerWgpuRenderer`, cloned `wgpu::Device` and
`wgpu::Queue` handles, and its own `OffscreenTargetPool`. It performs
`set_scene`, command encoding, and `queue.submit(...)` off the egui frame. The
egui frame still owns native texture registration/update because that mutates
`egui-wgpu` renderer state.

The older synchronous `render_scene_to_texture(...)` path remains available as
a fallback and is used by `request_background_scene_texture(...)` on wasm.

`OffscreenTargetPool` is triple buffered. The native background path tracks the
current front target generation, an in-progress render target, and the latest
published-but-not-yet-consumed texture. The worker waits rather than rendering
over a published texture that egui has not consumed, and it excludes the current
front target from the next worker render.

The texture usage is:

```rust
wgpu::TextureUsages::RENDER_ATTACHMENT
    | wgpu::TextureUsages::TEXTURE_BINDING
    | wgpu::TextureUsages::COPY_SRC
```

The first egui path uses `TextureFormat::Rgba8Unorm` because
`egui-wgpu::Renderer::register_native_texture` expects that format for native
texture registration.

When the offscreen target stays compatible, `avenger-egui` keeps the same egui
`TextureId` and calls
`update_egui_texture_from_wgpu_texture`. When size or format changes recreate
the offscreen target, the same high-level path updates the registered native
texture view.

## Async Scene Lifecycle

The egui integration moves chart scene evaluation off the egui hot path. Native
builds can also move Avenger's WGPU scene upload and offscreen command
submission to the background render worker. The egui frame remains responsible
for consuming the latest rendered texture, registering/updating the stable egui
`TextureId`, painting that texture, and routing fresh input.

```mermaid
sequenceDiagram
    participant Widget as egui widgets
    participant Handle as AvengerPlotHandle
    participant Publisher as FramePublisher
    participant Worker as Tokio worker
    participant App as AvengerApp
    participant UI as egui frame

    Widget->>Handle: set_param(name, value)
    Widget->>Handle: request_scene_rebuild(runtime)
    Handle->>Publisher: request_frame()
    Handle->>Worker: spawn if idle
    Worker->>App: rebuild_scene_graph(rebuild_geometry)
    Worker->>Publisher: publish latest SceneGraph
    Worker->>UI: request_repaint()
    UI->>Handle: latest_scene_frame()
    UI->>Handle: request_background_scene_texture(...)
    UI->>Handle: Plot::show(ui)
```

`FramePublisher<Arc<SceneGraph>>` owns requested, in-progress, and latest
published scene generations. If a newer generation is requested while older
work is running, stale results are dropped. The worker schedules another pass
for the latest request when needed.

The egui frame can keep painting the latest completed texture while scene
evaluation or background GPU rendering for a newer request is pending. Render
requests are latest-wins: if a newer scene/dimension request arrives before an
older request starts or publishes, stale work is dropped where possible. The
worker publishes only after queue submission, and egui consumes that texture on
a later frame.

The 100k-point `basic_chart` validation still showed scene evaluation as the
dominant pan/zoom cost. Background GPU submission removes `set_scene`, command
encoding, and submit from egui frames, but it does not by itself make semantic
pan/zoom realtime when chart scene evaluation dominates.

## Metrics And Tracing

`AvengerPlotHandle::metrics()` returns a `PlotMetrics` snapshot with counters
and latest timing values for:

- param set calls and changed param enqueues;
- routed event batches and event counts;
- scene rebuild and event-dispatch requests;
- scene frames published and stale scene frames dropped;
- background GPU render requests, coalesced requests, submissions,
  publications, and egui consumptions;
- offscreen texture renders, registrations, and updates;
- frames painted, reused latest-frame paints, and paints while a scene render
  is pending;
- latest scene evaluation, `set_scene`, command encode, queue submit, and
  texture publication timings;
- scene-to-texture publication timing and queue-wait timing for the background
  render worker;
- current texture render mode (`uninitialized`, `ui-thread-gpu`, or
  `background-gpu`).

`AvengerPlotHandle::show_metrics(ui)` provides a small egui debug readout for
examples and manual testing. `reset_metrics()` clears the counters.

The egui integration emits `tracing::debug!` events and spans around param
patching, event queueing, scene rebuild/event-dispatch requests, scene worker
publication, background render enqueue/submit/publish, offscreen render timing,
and texture paint. Use `RUST_LOG` with a subscriber in the host app to inspect
these diagnostics.

## Limitations

- The egui integration currently uses `egui-wgpu` native texture registration
  rather than a custom `egui_wgpu::CallbackTrait` render pass.
- Native background GPU submission uses one render worker per plot widget. A
  shared render pool may be worth revisiting if many plot widgets are active in
  one app.
- WebAssembly uses the UI-thread render fallback because it may not support the
  same worker and WGPU sharing model as native apps.
- `selection_changes()` is reserved but not wired to chart selection snapshots
  yet.
- `CanvasResizeSettled` routing is not emitted by the egui widget yet. The
  first MVP uses `CanvasResize`; settled routing should be added only if
  manual resize behavior needs it.
- The current chart text path remains Avenger's text stack. egui font
  measurement and rendering are not used for chart labels.

## Explicit Non-Goals

- No Iced integration crate is part of this plan.
- No direct rendering into egui's main swapchain pass is part of this plan.
- No semantic drag preview should be added until the latest-frame MVP has been
  manually evaluated.
- No high-level egui param binding helpers such as `param_slider` or
  `bind_param` are part of the initial API.
- No CPU readback is used to display charts in egui.
