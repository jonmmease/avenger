Avenger project development guidelines (for contributors)

This document captures project-specific practices for building, testing, and contributing to the Avenger Rust workspace. It is written for experienced Rust developers and consolidates the most relevant information from the repository and CLAUDE.md.

Workspace overview
- Avenger is a multi-crate Rust workspace focused on GPU-accelerated information visualization.
- Key crates include:
  - avenger-scenegraph (scene graph representation)
  - avenger-wgpu (GPU rendering over wgpu)
  - avenger-scales (visualization scales)
  - avenger-guides, avenger-text, avenger-image
  - avenger-eventstream, avenger-app
  - avenger-vega-scenegraph (Vega compatibility)
  - avenger-chart (plotting/visual testing harness) and examples/*
- The workspace is configured in Cargo.toml with resolver = "2" and shared workspace dependencies.

Build and configuration
- Standard native builds
  - Build whole workspace (debug): cargo build
  - Release: cargo build --release
  - Build a single crate:
    - Example: cargo build -p avenger-scenegraph
    - Or: cd avenger-scenegraph && cargo build
- WebAssembly example
  - Examples contain a WASM target (e.g., examples/iris-pan-zoom).
  - Build with wasm-pack: cd examples/iris-pan-zoom && wasm-pack build --target web --release
  - Serve via your preferred static server; ensure the browser supports WebGPU/WebGL2.
- Python developer workflow (optional)
  - The repo includes Pixi tasks for Python-related flows:
    - pixi run dev-py — develop Python bindings
    - pixi run build-py — build Python package
    - pixi run bump-version — versioning utility
  - See pixi.toml for full task definitions.
- Linting/formatting/strict checks
  - Format: cargo fmt --all
  - Lint: cargo clippy --all-targets
  - Strict check (deny warnings): RUSTFLAGS="-D warnings" cargo check --tests

Testing
The project uses both standard Rust unit/integration tests and image-based visual regression tests (primarily under avenger-chart). Some tests require GPU context creation and may be platform/driver sensitive.

Core commands
- Run all workspace tests: cargo test
- Run tests for a specific crate: cargo test -p avenger-wgpu
- Show test stdout: cargo test -- --nocapture
- avenger-chart specific debug mode for layout tests:
  - AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart
  - To run a specific visual regression test with layout overlays and captured output:
    - AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart --test visual_regression <test_name> -- --nocapture

Notes about CI and GPU
- Some CI environments (notably Linux without proper GPU/driver support) can hit MakeWgpuAdapterError during adapter/device creation. Expect to skip GPU-dependent tests or use headless backends where possible on CI.
- When debugging adapter selection locally, prefer recent drivers and ensure the OS/browser/tooling support the wgpu backend in use.

How to add and run new tests
- Unit tests (within a crate):
  - Add #[cfg(test)] mod tests blocks alongside code, or place files under <crate>/tests/ for integration tests.
  - Example minimal integration test (we verified this flow):
    - Create a file avenger-common/tests/smoke.rs with:
      
      #[test]
      fn smoke_addition() {
          assert_eq!(2 + 2, 4);
      }
      
    - Run just that crate’s tests: cargo test -p avenger-common --tests
    - We confirmed this compiles and runs successfully locally.
  - Remove temporary test files after experimentation to keep the tree clean, especially when tests are only instructional.
- Visual regression tests (avenger-chart):
  - Tests live under avenger-chart/tests, with PNG baselines under avenger-chart/tests/baselines.
  - Tests typically render to an image, then compare to the baseline using pixelmatch (configured in workspace dependencies) with tolerance thresholds.
  - To add a new visual test:
    - Create a new test in avenger-chart/tests/visual_tests/ that renders your scene/mark configuration.
    - Generate a baseline image and place it in the matching baselines subfolder. Ensure the test references the correct path.
    - Use AVENGER_CHART_DEBUG_LAYOUT=1 during development to debug layout boxes.
  - Run: cargo test -p avenger-chart --test visual_regression -- --nocapture

Project conventions and tips
- Architectural patterns (from CLAUDE.md):
  - SceneGraphBuilder<State> and EventStreamHandler<State> are the primary extension points for reactive visualization logic.
  - Scales implement pan/zoom-aware transformations; add new scales by implementing the ScaleImpl trait in avenger-scales.
  - For rendering new mark types: extend SceneMark and add GPU pipeline/shader support in avenger-wgpu.
  - Performance: favor instanced rendering for large mark counts; minimize per-frame allocations.
- Resource management
  - GPU resources are initialized and owned by the Canvas; ensure proper lifecycle when adding features that require buffers, textures, or pipelines.
- Coordinate systems
  - Be mindful of Scene vs. viewport coordinates and any transforms applied by the scene graph and renderer; layout debugging is invaluable here.
- Style and hygiene
  - Keep the workspace consistent via cargo fmt and clippy. New warnings should be addressed or explicitly justified; run strict checks as above.

Example end-to-end test development flow (validated during preparation of this guide)
1) Add a minimal integration test in a small crate to avoid heavy dependencies:
   - File: avenger-common/tests/smoke_temp.rs
   - Content: trivial assert as shown above.
2) Run: cargo test -p avenger-common --tests
3) Confirm output shows the test executed and passed.
4) Remove the temporary test file to keep the repo clean.

WASM and examples
- examples/iris-pan-zoom includes both native and WASM paths:
  - Native run: cd examples/iris-pan-zoom && cargo run --release
  - WASM build: cd examples/iris-pan-zoom && wasm-pack build --target web --release
- examples/wgpu* demonstrate renderer usage with winit/wgpu; use these to validate GPU changes across platforms.

Troubleshooting
- If visual tests fail due to minor anti-aliasing or driver differences, consider threshold tuning in pixel comparison, but ensure substantive diffs are investigated.
- On platforms lacking a functional adapter, skip GPU-dependent tests or gate them with cfg flags if appropriate. Keep an eye on CI errors mentioning adapter creation.

Provenance of this guidance
- Consolidated from CLAUDE.md (build/dev commands, architecture summaries, testing notes) and verified against the current workspace configuration.

Appendix: quick commands
- Build all: cargo build
- Test all: cargo test
- Single crate: cargo test -p avenger-chart
- Layout debug: AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart
- Format/lint: cargo fmt --all && cargo clippy --all-targets
- Strict check: RUSTFLAGS="-D warnings" cargo check --tests
