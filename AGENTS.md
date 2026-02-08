# Repository Guidelines

## Project Structure & Module Organization
The workspace is a Rust visualization stack. `avenger-scenegraph` defines scene state, `avenger-wgpu` handles GPU rendering, `avenger-eventstream` manages interaction, and `avenger-app` coordinates applications. `avenger-scales`, `avenger-geometry`, and `avenger-text` provide reusable utilities. Documentation lives in `docs/` and `avenger-guides/`, while examples and data live in `examples/`, `resources/`, and `avenger-vega-test-data/`. Integration tests sit in the top-level `tests/` directory.

## Avenger Chart Crate
`avenger-chart/` is the primary entry point for chart authors. `src/lib.rs` re-exports mark builders (`src/marks/*`), coordinate systems (`src/cartesian`, `src/polar`), guides (`src/axis`, `src/legend`), and layout logic (`src/layout`). Transforms, data channels, and theme plumbing live in `src/plot`, `src/channel`, and `src/theme`, with serialization helpers in `src/serialization/`. Docs are under `avenger-chart/docs/` and `avenger-chart-mdbook/`. Tests live in `avenger-chart/tests/`, with golden images in `tests/baselines/` and visual scenarios in `tests/visual_tests/` and `tests/data/`. Modify `src/prelude.rs` whenever new user-facing types are added.

## Build, Test, and Development Commands
- `cargo build` / `cargo build --release`: compile the workspace; use release for perf runs.
- `cargo test`, `cargo test -p avenger-wgpu`: execute unit and integration suites.
- `cargo fmt --all`, `cargo clippy --all-targets`: format and lint before review.
- `pixi run dev-py`, `pixi run build-py`: work on the Python bindings.
- `examples/iris-pan-zoom/wasm-pack build --target web --release`: build the WebAssembly demo.
- `cargo test -p avenger-chart -- --nocapture`: run chart suites.
- `RUST_LOG=avenger_chart=debug cargo test -p avenger-chart -- --nocapture`: enable textual diagnostics via tracing.
- `AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart`: enable visual debug overlay marks (no tracing output unless `RUST_LOG` is set).
- `avenger-chart/scripts/check_logging_guardrails.sh`: enforce tracing/overlay logging guardrails.

## Coding Style & Naming Conventions
Follow `rustfmt` defaults (4-space indentation, trailing commas). Use `snake_case` for files and modules, `PascalCase` for public types, and align new APIs with patterns used by `SceneGraphBuilder` and `EventStreamHandler`. Keep shader updates in `avenger-wgpu/src/shaders/` mirrored in host structs, and leave focused comments for GPU or layout edge cases.

## Testing Guidelines
Tests rely on Rust’s harness plus visual baselines. Use descriptive names (`test_handles_zero_width_arc`), run focused suites with `cargo test -p crate_name`, and enable `AVENGER_CHART_DEBUG_LAYOUT=1` when chasing layout bugs. For chart regressions, run `cargo test -p avenger-chart visual_regression -- --nocapture`, review `target/tests/visual_tests/`, and refresh `avenger-chart/tests/baselines/` only for intentional visual updates.

## Commit & Pull Request Guidelines
Commits follow Conventional Commits (`feat(scales):`, `fix(docs):`); match existing scopes. Before pushing, run `cargo fmt`, `cargo clippy`, and required tests. Pull requests should call out impact, validation steps, linked issues, and visuals for chart changes, plus any config or baseline updates reviewers must mirror.
