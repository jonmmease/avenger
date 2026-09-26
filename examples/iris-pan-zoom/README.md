# Iris interaction example

Run `cargo run --release -p iris` from the workspace root. Drag the plot to pan or use the wheel to zoom. The cursor changes while dragging. The visible-domain readout updates 350 ms after release or the last wheel event, while the plot responds immediately. Moving focus away cancels the drag.

For a browser, run `wasm-pack build examples/iris-pan-zoom --target web --release --no-opt`, then serve that directory with `python3 -m http.server 8768 --directory examples/iris-pan-zoom`. Open http://localhost:8768 in a browser with WebGPU support.

The readout uses an event-stream debounce and host wake-ups. It needs no text-input element or clipboard integration.
