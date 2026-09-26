# Window and browser host

The host drives an Avenger application, applies interaction commands, and presents its scene. `WinitWgpuAvengerAppOptions` configures window attributes, canvas sizing, resource invalidation, and optional canvas resize handles.

Run `cargo run --release -p avenger-winit-wgpu --example image_resources` to see a placeholder become a generated local image after 1.5 seconds. Keep the pointer still while it loads: resource invalidation requests the redraw. Drag the right or bottom canvas edge to resize the image area independently of the window. This example makes no network request.

The Iris example in `examples/iris-pan-zoom` demonstrates window resizing, panning, and timed interaction updates in native and browser execution.

`RuntimeHostCommand::UpdateTooltip` accepts formatted rows in logical canvas coordinates. Use owner-scoped show, move, and hide updates for application tooltips. The host clears transient overlays when focus or viewport size changes.

The [annotation editor](../examples/winit-annotation-editor/README.md) demonstrates native and browser text editing. The browser text agent owns a hidden input for text, composition, focus, and clipboard events. Keyed timers remain independent of that input. A synchronous `ClipboardPayloadProvider` supplies the installed editor's selected text during browser copy and cut events.
