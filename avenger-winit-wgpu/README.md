# Window and browser host

The host drives an Avenger application, applies interaction commands, and presents its scene. `WinitWgpuAvengerAppOptions` configures window attributes, canvas sizing, resource invalidation, and optional canvas resize handles.

Run `cargo run --release -p avenger-winit-wgpu --example image_resources` to see a placeholder become a generated local image after 1.5 seconds. Keep the pointer still while it loads: resource invalidation requests the redraw. Drag the right or bottom canvas edge to resize the image area independently of the window. This example makes no network request.

The Iris example in `examples/iris-pan-zoom` demonstrates window resizing, panning, and timed interaction updates in native and browser execution.
