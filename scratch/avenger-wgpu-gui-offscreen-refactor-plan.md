# Avenger WGPU GUI Offscreen Refactor Plan

Date: 2026-06-19

## Goal

Refactor `avenger-wgpu` so Avenger can render into offscreen WGPU textures owned by a GUI integration, then let egui display the latest completed texture at 60 FPS while exact Avenger renders run asynchronously.

The intended end state is:

- egui owns the application shell, window, input loop, and WGPU device/queue for the first GUI integration.
- Avenger owns reusable scene/render resources, but not necessarily a winit surface.
- Avenger can render a `SceneGraph` into:
  - a winit swapchain texture,
  - an offscreen texture for GUI sampling,
  - an offscreen texture for PNG/readback,
  - a background-rendered texture published to a GUI frame loop.
- egui routes widget-local events into Avenger's existing `WindowEvent` abstraction.
- Heavy exact renders can complete in the background.
- The first GUI MVP should keep the GUI responsive by sampling the latest completed texture while exact renders run asynchronously. Semantic drag preview is deferred until after the baseline feel is evaluated.

## Read This First

This plan is intentionally egui-first. Do not add Iced, direct rendering into egui's main render pass, semantic drag preview, or an egui-specific text backend while implementing the main phases.

The first successful app should look like this:

```text
eframe app
  left egui side panel:
    normal egui slider
    slider Response::changed() calls plot_handle.set_param("point_size", value)

  central egui panel:
    Plot::new(&plot_handle).show(ui)
    plot widget samples latest completed Avenger offscreen texture
    plot widget routes egui input to Avenger WindowEvent
```

The first MVP is allowed to show the previous completed chart frame while a new exact frame is pending. Do not build special pan/zoom preview behavior until a working egui MVP has been manually tested and found too stale.

Implementation milestones:

- Phases 0-6: refactor `avenger-wgpu` without changing existing window/PNG behavior.
- Phase 7: align WGPU versions for egui integration if needed.
- Phases 8-10: build the egui MVP with latest-frame publishing and low-level `set_param`.
- Phase 11: move full exact GPU rendering off the GUI hot path after the MVP works.
- Phases 12-13: add observability and architecture documentation.
- Deferred follow-up: semantic preview during drag, only if baseline UX requires it.

Non-goals for the main plan:

- [ ] Do not add an Iced integration crate.
- [ ] Do not render Avenger directly into egui's main render pass.
- [ ] Do not add semantic drag preview before the egui MVP is evaluated.
- [ ] Do not add high-level egui param binding helpers such as `param_slider` or `bind_param`.
- [ ] Do not replace Avenger's text measurement/rasterization with egui text. Keep Avenger chart text on the existing Avenger text stack for this plan.
- [ ] Do not use CPU readback to display charts in egui. The chart must remain a GPU texture sampled by egui.

Glossary:

- "Exact render": the authoritative Avenger/chart evaluation and render for the current params/events. The public egui API should not expose this term.
- "Latest completed frame": the newest finished offscreen texture that the egui frame loop can sample without waiting.
- "Preview": a future semantic drag-rendering mode. It is not part of the initial egui MVP.
- "Host wrapper": code that owns a window/surface/presentation loop and delegates rendering to `AvengerWgpuRenderer`.
- "Renderer core": the surface-independent Avenger WGPU renderer that can encode into a caller-provided texture view.

## Implementing Agent Instructions

This document is meant to be edited as implementation progresses.

- [ ] Before starting a task, read the relevant files and confirm the assumptions in that phase still hold.
- [ ] Check off tasks in this document as they are completed.
- [ ] Keep each commit focused. Prefer one commit per phase, or one commit per coherent sub-phase when a phase is large.
- [ ] After each commit, update this document with:
  - completed checkboxes,
  - validation commands run,
  - any changed design decisions,
  - the commit hash.
- [ ] Do not batch many unrelated phases into a single commit.
- [ ] If a task reveals a better design, update the plan before continuing so the next agent can trust the document.
- [ ] Do not revert unrelated dirty work in the repository.
- [ ] Before marking a phase complete, run the validation listed for that phase.

Commit message guidance:

- Use Conventional Commit style.
- Suggested scopes:
  - `refactor(wgpu): ...`
  - `feat(wgpu): ...`
  - `feat(egui): ...`
  - `test(wgpu): ...`
  - `docs(wgpu): ...`

## Current Architecture Summary

Relevant existing files:

- `avenger-wgpu/src/canvas.rs`
  - `Canvas` trait mixes scene-building responsibilities with render target/device access.
  - `WindowCanvas` owns winit `Window`, WGPU `Surface`, `Device`, `Queue`, render state, and presentation.
  - `PngCanvas` owns a separate offscreen texture, readback buffer, `Device`, `Queue`, and mostly duplicates the `WindowCanvas` render path.
- `avenger-wgpu/src/marks/multi.rs`
  - `MultiMarkRenderer::prepare` builds frame resources.
  - `MultiMarkRenderer::encode_multi_ranges` currently creates its own `CommandEncoder` and returns a `CommandBuffer`.
- `avenger-wgpu/src/marks/instanced_mark.rs`
  - `InstancedMarkRenderer::render` currently creates its own `CommandEncoder` and returns a `CommandBuffer`.
- `avenger-eventstream/src/window/mod.rs`
  - Defines GUI-host-independent logical `WindowEvent`.
- `avenger-eventstream/src/window/winit.rs`
  - winit is already just one translation layer into `WindowEvent`.
- `avenger-app/src/app.rs`
  - `AvengerApp::update_with_status` routes events and rebuilds scenegraphs.
- `avenger-chart/src/plot/compiled/session.rs`
  - `EvaluationRequest::exact()` and `EvaluationRequest::preview()` already model exact vs preview evaluation.
  - `PlotSession::apply_param_patch` already supports param changes.

Main problem:

`WindowCanvas::render` and `PngCanvas::render` contain the reusable Avenger render sequence, but that sequence is not available as a target-agnostic API. GUI integrations need to render into a texture without pretending to own a winit surface.

## Version Strategy

As of this plan:

- Avenger workspace uses `wgpu = 25.0.2`.
- Current egui-wgpu latest uses newer WGPU than Avenger.

Recommended first compatibility target:

- [x] Confirm exact current crate versions before implementation.
- [x] Choose the first egui-compatible WGPU target deliberately.
- [x] Prefer the latest egui/egui-wgpu stack that can be adopted without excessive WGPU migration risk.
- [x] Record the chosen versions here:
  - `wgpu`: `27.0.1`
  - `egui`: `0.33.3`
  - `egui-wgpu`: `0.33.3`
  - `eframe`: `0.33.3`, if used for examples

Phase 0 version decision, 2026-06-19:

Use `egui`/`egui-wgpu`/`eframe` `0.33.3` as the first integration target and update Avenger to `wgpu 27.0.1` in Phase 7. The latest checked egui stack, `0.34.3`, uses `wgpu 29.0.1` but requires Rust `1.92`; the current local toolchain is `rustc 1.91.1`, while the `0.33.3` stack supports Rust `1.88` and matches `wgpu 27.0.1`.

Notes:

- Updating WGPU is acceptable for this project.
- Avoid supporting multiple WGPU major versions inside the same build. That will usually make native texture sharing impossible.
- Keep the renderer core independent enough that other GUI backends can be added later without changing `avenger-wgpu`.

## Design Principles

- [ ] Keep `avenger-wgpu` independent of egui-specific types.
- [ ] Make "render into this WGPU texture view" the core capability.
- [ ] Make "own a winit surface and present" a host wrapper, not the renderer core.
- [ ] Make "render into an offscreen texture and sample it later" a first-class path.
- [ ] Avoid CPU readback for GUI display.
- [ ] Keep the GUI frame loop non-blocking.
- [ ] Make stale background renders cheap to discard.
- [ ] Preserve existing `WindowCanvas` and `PngCanvas` behavior during migration.
- [ ] Validate each refactor step against existing visual baselines.

## Target API Sketch

The final names do not need to match this exactly, but implementation must preserve these responsibilities. If the public shape changes materially, update this sketch and the Phase 13 architecture docs in the same commit.

```rust
pub struct AvengerWgpuRenderer {
    dimensions: CanvasDimensions,
    scene_state: SceneRenderState,
    resources: AvengerRenderResources,
}

pub struct AvengerRendererConfig {
    pub dimensions: CanvasDimensions,
    pub texture_format: wgpu::TextureFormat,
    pub sample_count: u32,
    pub text_builder_ctor: Option<TextBuildCtor>,
}

pub struct AvengerRenderTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub resolve_target: Option<&'a wgpu::TextureView>,
    pub extent: wgpu::Extent3d,
    pub format: wgpu::TextureFormat,
    pub sample_count: u32,
    pub load: wgpu::LoadOp<wgpu::Color>,
}

pub struct PreparedAvengerFrame {
    // Text bind groups, prepared multi resources, per-frame statistics, etc.
}

impl AvengerWgpuRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: AvengerRendererConfig,
    ) -> Result<Self, AvengerWgpuError>;

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        dimensions: CanvasDimensions,
    ) -> Result<(), AvengerWgpuError>;

    pub fn set_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_graph: &SceneGraph,
    ) -> Result<(), AvengerWgpuError>;

    pub fn prepare_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        extent: wgpu::Extent3d,
    ) -> Result<PreparedAvengerFrame, AvengerWgpuError>;

    pub fn encode_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: AvengerRenderTarget<'_>,
        prepared: &PreparedAvengerFrame,
    ) -> Result<(), AvengerWgpuError>;
}
```

Offscreen support:

```rust
pub struct OffscreenTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub extent: wgpu::Extent3d,
    pub format: wgpu::TextureFormat,
    pub generation: u64,
}

pub struct OffscreenTargetPool {
    // double or triple buffered textures
}
```

Texture usages for GUI display:

```rust
wgpu::TextureUsages::RENDER_ATTACHMENT
    | wgpu::TextureUsages::TEXTURE_BINDING
    | wgpu::TextureUsages::COPY_SRC
```

## Phase 0 - Baseline, Compatibility, and Guardrails

Purpose: establish the current behavior and pick dependency versions before large refactors.

Tasks:

- [x] Run `git status --short` and record unrelated dirty files in this document.
- [x] Confirm the current workspace `wgpu` version.
- [x] Research current egui-wgpu WGPU version.
- [x] Choose the first target WGPU version.
- [x] Add a short note to this document with the version decision and rationale.
- [x] Run baseline tests for `avenger-wgpu`.
- [x] Run at least one existing visual regression or PNG-render path that exercises `PngCanvas`.
- [x] Capture baseline timing logs for one representative chart, if practical.
- [x] Identify any required WGPU API migration changes before touching renderer architecture.

Phase 0 findings, 2026-06-19:

- Initial `git status --short`: clean worktree; no unrelated dirty files.
- Current workspace `wgpu`: `25.0.2`, confirmed from `Cargo.toml` and `cargo tree -i wgpu --workspace`.
- Current latest checked egui stack:
  - `egui-wgpu 0.34.3` uses `wgpu 29.0.1` and requires Rust `1.92`.
  - `egui-wgpu 0.33.3` uses `wgpu 27.0.1` and requires Rust `1.88`.
  - Local toolchain: `rustc 1.91.1`, `cargo 1.91.1`.
- First target: `wgpu 27.0.1` with `egui`/`egui-wgpu`/`eframe 0.33.3`.
- Baseline `cargo test -p avenger-wgpu` result before code changes:
  - unit tests passed,
  - image baseline suite failed existing cases:
    - `case_090` / `residuals_colorscale`, diff `0.026578`,
    - `case_119` / `geoScale`, diff `0.016531`,
    - `case_120` / `maptile_background`, diff `0.012998`.
- Baseline `cargo test -p avenger-chart visual_regression -- --nocapture` completed successfully but selected zero tests under that filter (`618 filtered out` in `tests/visual_regression.rs`), so it is recorded as a command check rather than meaningful visual coverage.
- Baseline timing logs were not captured; the current baseline has known visual failures and no representative timing harness was selected in this phase.
- Disposable `/tmp/avenger-wgpu27-check` compile audit against `wgpu 27.0.1` found these first-order migration items:
  - `wgpu::DeviceDescriptor` now requires `experimental_features`,
  - `wgpu::RenderPassColorAttachment` now requires `depth_slice`,
  - `wgpu::PollType::Wait` is now a struct-style variant instead of a bare value.

Suggested validation:

```bash
cargo test -p avenger-wgpu
cargo test -p avenger-chart visual_regression -- --nocapture
```

Commit:

- [ ] Commit doc/version baseline updates.
- Commit hash: TBD

## Phase 1 - Extract Renderer Core Without Behavior Change

Purpose: separate reusable Avenger render state from surface ownership while preserving `WindowCanvas` and `PngCanvas` public behavior.

Tasks:

- [ ] Create `avenger-wgpu/src/renderer.rs`.
- [ ] Create `avenger-wgpu/src/target.rs`.
- [ ] Move shared fields out of `WindowCanvas` and `PngCanvas` into a new internal render state:
  - `marks`
  - `shared_multi`
  - `run_start`
  - `current_zindex`
  - `instanced_renderers`
  - `multi_render_resources`
  - `text_atlas_builder`
  - `config`
  - `dimensions`
  - `sample_count`
  - `texture_format`
- [ ] Keep device/queue ownership in existing host canvases for now.
- [ ] Keep surface/window ownership only in `WindowCanvas`.
- [ ] Keep PNG output texture/readback ownership only in `PngCanvas`.
- [ ] Move `commit_all_multi_renderers` to the renderer core.
- [ ] Move `make_frame_overlay_command` or its future equivalent behind the renderer core.
- [ ] Keep the existing `Canvas` trait working, even if implemented by forwarding to the new renderer core.
- [ ] Update module exports.
- [ ] Ensure no behavior changes in `WindowCanvas::new`, `WindowCanvas::render`, `PngCanvas::new`, or `PngCanvas::render`.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
cargo test -p avenger-chart-app --features winit-wgpu
```

Commit:

- [ ] Commit Phase 1.
- Suggested message: `refactor(wgpu): extract reusable renderer state`
- Commit hash: TBD

## Phase 2 - Introduce Explicit Render Targets

Purpose: make render target details explicit instead of implicit in `WindowCanvas` and `PngCanvas`.

Tasks:

- [ ] Add `AvengerRenderTarget<'a>`.
- [ ] Add render target helpers for:
  - swapchain texture view,
  - offscreen texture view,
  - multisampled color target plus resolve target.
- [ ] Replace hard-coded background clear with target-provided `load`.
- [ ] Preserve default white clear for existing window/png behavior.
- [ ] Move background clear encoding into renderer core.
- [ ] Add a renderer method that builds commands for an arbitrary `AvengerRenderTarget`.
- [ ] Keep existing command-buffer-returning API available until Phase 3 completes.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
```

Commit:

- [ ] Commit Phase 2.
- Suggested message: `refactor(wgpu): make render targets explicit`
- Commit hash: TBD

## Phase 3 - Encode Into Caller-Provided Command Encoders

Purpose: allow egui callbacks and offscreen workers to compose Avenger rendering into their own frame lifecycle.

Tasks:

- [ ] Add `MultiMarkRenderer::encode_multi_ranges_into`.
- [ ] Keep `MultiMarkRenderer::encode_multi_ranges` as a wrapper that creates an encoder and returns a `CommandBuffer`.
- [ ] Add `InstancedMarkRenderer::encode_into`.
- [ ] Keep `InstancedMarkRenderer::render` as a wrapper that creates an encoder and returns a `CommandBuffer`.
- [ ] Ensure all render passes use `LoadOp::Load` after the explicit background pass unless the target requests otherwise.
- [ ] Ensure scissor state remains correct after moving to shared encoders.
- [ ] Preserve path/stencil clipping behavior.
- [ ] Add regression tests for at least:
  - basic rect/symbol/text chart,
  - path-clipped mark,
  - instanced symbol mark,
  - multi-mark z-order.
- [ ] Add debug labels to new encoders/passes so GPU captures remain readable.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
cargo test -p avenger-chart visual_regression -- --nocapture
```

Commit:

- [ ] Commit Phase 3.
- Suggested message: `refactor(wgpu): encode marks into provided command encoders`
- Commit hash: TBD

## Phase 4 - Add Offscreen Render Targets and Pooling

Purpose: make the GUI texture route first-class.

Tasks:

- [ ] Create `avenger-wgpu/src/offscreen.rs`.
- [ ] Add `OffscreenTarget`.
- [ ] Add `OffscreenTargetDescriptor`.
- [ ] Add `OffscreenTarget::new`.
- [ ] Add `OffscreenTarget::resize_or_recreate`.
- [ ] Add `OffscreenTargetPool`.
- [ ] Support double buffering.
- [ ] Support optional triple buffering for background render plus GUI sampling.
- [ ] Track texture generation IDs.
- [ ] Track size, scale, format, and sample count.
- [ ] Use texture usages:
  - `RENDER_ATTACHMENT`
  - `TEXTURE_BINDING`
  - `COPY_SRC`
- [ ] Add a renderer convenience method:

```rust
pub fn render_to_offscreen(
    &mut self,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &mut OffscreenTarget,
) -> Result<RenderedOffscreenFrame, AvengerWgpuError>;
```

- [ ] Ensure offscreen render does not submit implicitly unless the method name says it does.
- [ ] Provide both low-level encode and high-level submit helpers if needed.
- [ ] Implement `OffscreenTargetPool` in `avenger-wgpu`.
- [ ] Let `avenger-egui` own an `OffscreenTargetPool` instance for each plot handle/widget because egui owns the WGPU device/queue and texture registration lifecycle.
- [ ] Document this ownership decision in this file and in Phase 13 architecture docs.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
```

Commit:

- [ ] Commit Phase 4.
- Suggested message: `feat(wgpu): add offscreen render targets`
- Commit hash: TBD

## Phase 5 - Rebuild PngCanvas on the Renderer Core

Purpose: prove the extracted renderer core and offscreen path produce the same pixels before introducing GUI frameworks.

Tasks:

- [ ] Replace `PngCanvas`'s duplicated render logic with:
  - `AvengerWgpuRenderer`,
  - an `OffscreenTarget`,
  - a readback buffer.
- [ ] Move readback-specific code into `readback.rs`.
- [ ] Preserve `PngCanvas::render` public behavior.
- [ ] Preserve existing image dimensions and padding behavior.
- [ ] Ensure MSAA resolve behavior matches the old implementation.
- [ ] Verify text atlas behavior is unchanged.
- [ ] Verify `PngCanvas` does not require `TEXTURE_BINDING` unless using shared `OffscreenTarget` directly.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
cargo test -p avenger-chart visual_regression -- --nocapture
```

Commit:

- [ ] Commit Phase 5.
- Suggested message: `refactor(wgpu): render png output through renderer core`
- Commit hash: TBD

## Phase 6 - Rebuild WindowCanvas as a Host Wrapper

Purpose: keep the existing winit integration working while proving the renderer core can target a swapchain texture.

Tasks:

- [ ] Reduce `WindowCanvas` to:
  - winit `Window`,
  - WGPU `Surface`,
  - `SurfaceConfiguration`,
  - `Device`,
  - `Queue`,
  - `AvengerWgpuRenderer`.
- [ ] Make `WindowCanvas::render`:
  - acquire surface texture,
  - create surface texture view,
  - create target descriptor,
  - call renderer prepare/encode,
  - submit,
  - present.
- [ ] Preserve surface resize behavior.
- [ ] Preserve frame overlay behavior.
- [ ] Preserve `WindowCanvas::set_scene`.
- [ ] Preserve `Canvas` trait compatibility for downstream crates.
- [ ] Run an existing winit example manually if practical.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
cargo test -p avenger-winit-wgpu
cargo test -p avenger-chart-app --features winit-wgpu
```

Commit:

- [ ] Commit Phase 6.
- Suggested message: `refactor(wgpu): make window canvas a renderer host`
- Commit hash: TBD

## Phase 7 - WGPU Upgrade

Purpose: align Avenger with the first target GUI backend dependency set.

Do this before backend crates if version mismatch prevents texture/resource sharing.

Tasks:

- [ ] Update workspace `wgpu`.
- [ ] Update WGPU API calls across `avenger-wgpu`.
- [ ] Update WASM-specific WGPU features.
- [ ] Update examples that directly depend on WGPU.
- [ ] Re-check texture format feature queries.
- [ ] Re-check surface configuration fields.
- [ ] Re-check `Device::poll` usage.
- [ ] Re-check command encoder, copy, and texture descriptor APIs.
- [ ] Re-check validation errors around stencil/depth attachments.
- [ ] Confirm native builds.
- [ ] Confirm WASM builds if in scope for this phase.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
cargo test -p avenger-winit-wgpu
cargo test -p avenger-chart visual_regression -- --nocapture
```

Optional WASM validation:

```bash
examples/iris-pan-zoom/wasm-pack build --target web --release
```

Commit:

- [ ] Commit Phase 7.
- Suggested message: `refactor(wgpu): update wgpu dependency for gui integration`
- Commit hash: TBD

## Phase 8 - Async Frame Publishing Core

Purpose: provide the background exact render and latest-frame handoff primitives needed by the egui integration.

Tasks:

- [ ] Create a small frame-publishing module, either in `avenger-wgpu` or a GUI-support crate.
- [ ] Define `RenderedFrame`.
- [ ] Define `FrameGeneration`.
- [ ] Define `LatestFrame`.
- [ ] Define `FramePublisher`.
- [ ] Ensure GUI readers can grab the latest completed frame without blocking.
- [ ] Ensure background writers never overwrite a texture currently sampled by the GUI.
- [ ] Use generation IDs to discard stale renders.
- [ ] Add a cancellation/drop-stale policy.
- [ ] Add a "render in progress" flag.
- [ ] Add metrics:
  - scene evaluation time,
  - `set_scene` time,
  - prepare time,
  - command encode time,
  - submit time,
  - texture publish time.
- [ ] In Phase 8, implement CPU scene evaluation / scene publication as the background work.
- [ ] In Phase 8, keep GPU upload/render in the egui render preparation path for simpler WGPU ownership.
- [ ] Do not implement full background GPU submission until Phase 11.
- [ ] Document this staged decision here and in Phase 13 architecture docs.

Recommended initial policy:

- CPU scene evaluation may run in the background.
- GPU upload/render should initially happen in the GUI callback `prepare` phase for simpler WGPU ownership and easier validation.
- Full background GPU render can be enabled after the offscreen path is stable.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
```

Commit:

- [ ] Commit Phase 8.
- Suggested message: `feat(wgpu): add latest-frame publishing primitives`
- Commit hash: TBD

## Deferred Follow-Up - Semantic Fast Preview and Drag Rendering

Status: intentionally out of the first egui MVP.

Purpose: revisit drag rendering only after testing the baseline offscreen/latest-frame model. The initial implementation must not build a separate semantic preview scheduler, texture-transform fallback, or mark-adjustment path. First measure how it feels when drag/param changes enqueue asynchronous exact renders and the GUI keeps painting the latest completed frame.

If the baseline feels too stale during pan/zoom, the follow-up requirement is semantic preview, not a simple texture shift. For pan/zoom, axes, guides, ticks, selections, and marks must update according to the interaction. A texture transform can be used only as a temporary continuity fallback because axes and guides would otherwise be stale.

This is not an implementation phase. Do not add tasks or commits for this follow-up until the egui MVP has been manually evaluated and the plan has been updated with a concrete new phase.

## Phase 9 - Chart/App Param APIs for Native Widgets

Purpose: let native egui controls drive Avenger chart params without pretending to be pointer events.

Tasks:

- [ ] Add public `ChartAppState` or wrapper APIs for param patching.
- [ ] Add a public `set_param` API for the first GUI MVP.
- [ ] Make `set_param` enqueue an exact/settled param update internally without exposing "exact" in the egui app-author API.
- [ ] Make `set_param` update an optimistic local param mirror immediately so egui controls do not snap back while an exact frame is pending.
- [ ] Defer preview param patch API unless the baseline latest-frame model feels too stale.
- [ ] Add param snapshot API.
- [ ] Add cheap typed param getters needed by the example:
  - `param_f64(name) -> Option<f64>`,
  - `param_bool(name) -> Option<bool>` if the checkbox example uses a bool param.
- [ ] Add param change records with revision IDs so `PlotOutput` can report which params changed during a widget frame.
- [ ] Add selection snapshot API if selection observation is in scope.
- [ ] Add revision counters for:
  - params,
  - selections,
  - stores if needed.
- [ ] Prefer implementing GUI-neutral param/change APIs in `avenger-chart-app` or a small GUI-neutral adapter; keep egui types out of chart/app crates.
- [ ] Ensure `set_param` can enqueue exact renders without blocking the GUI frame.
- [ ] Add tests for slider-like workflows:
  - many rapid `set_param` calls,
  - optimistic getter returns latest requested value before exact frame publishes,
  - param change record is emitted for a changed param,
  - stale exact result discarded.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-chart-app
```

Commit:

- [ ] Commit Phase 9.
- Suggested message: `feat(chart-app): expose param update APIs for gui widgets`
- Commit hash: TBD

## Phase 10 - egui Integration Crate

Purpose: implement the first GUI backend using the offscreen texture route.

Crate:

- `avenger-egui`

Tasks:

- [ ] Add `avenger-egui` crate to the workspace.
- [ ] Depend on compatible `egui`, `egui-wgpu`, and optionally `eframe`.
- [ ] Define `AvengerEguiHandle`.
- [ ] Define `Plot` as the primary egui widget builder: `avenger_egui::Plot::new(&handle).show(ui)`.
- [ ] Optionally define `AvengerPlotWidget` internally if useful, but the example and public docs should use `Plot`.
- [ ] Define an egui-style `PlotOutput` returned by the plot widget:
  - contains the underlying `egui::Response`,
  - exposes `changed()`,
  - exposes `params_changed()`,
  - exposes `param_changed(name)`,
  - exposes `param_changes()`,
  - reserves space for future `selection_changes()`,
  - includes lightweight frame/render status for debug UI.
- [ ] Call `response.mark_changed()` when routed plot input causes observable Avenger param or selection changes.
- [ ] Implement widget allocation with `Sense::click_and_drag()` for the first MVP.
- [ ] Keep builder options low-level and minimal:
  - `desired_size(Vec2)` if needed,
  - `sense(Sense)` if needed,
  - no high-level param binding helpers.
- [ ] Add an explicit `EguiEventTranslator` or equivalent module for routing egui input into Avenger events.
- [ ] Make the translator convert egui pointer positions from screen-space points to widget-local logical coordinates:
  - subtract `rect.min`,
  - preserve egui logical point units,
  - output Avenger positions as `[f32; 2]`.
- [ ] Convert egui pointer movement and drag/click state to Avenger cursor and mouse events:
  - `WindowEvent::CursorMoved`,
  - `WindowEvent::MouseInput`,
  - `WindowEvent::CursorEntered`,
  - `WindowEvent::CursorLeft`.
- [ ] Convert egui wheel events to `WindowEvent::MouseWheel`.
- [ ] Convert egui keyboard events to `WindowEvent::KeyboardInput`.
- [ ] Route pointer events only when the pointer is hovered, dragging, or otherwise captured by the plot widget.
- [ ] Route wheel events only when the pointer is hovered over the plot widget.
- [ ] Request plot focus on click/drag start.
- [ ] Route keyboard events only when the plot widget has focus.
- [ ] Route widget resize to `CanvasResize`.
- [ ] Add debounced or settled resize routing to `CanvasResizeSettled` if needed by chart resize behavior.
- [ ] Ensure the translated Avenger events are dispatched through the Avenger app/eventstream path, not through winit.
- [ ] Add translator tests for:
  - coordinate conversion,
  - hover filtering,
  - active-drag routing after the pointer leaves the rect,
  - wheel filtering,
  - keyboard focus filtering,
  - resize event generation.
- [ ] Register or update Avenger offscreen texture with egui-wgpu.
- [ ] Prefer egui-wgpu native texture registration for the first MVP.
- [ ] Keep and reuse a stable egui `TextureId` while the underlying offscreen texture remains compatible.
- [ ] Re-register/update the texture when the offscreen texture is recreated because of size/format changes.
- [ ] Paint latest frame texture into widget rect.
- [ ] Keep `show(ui)` non-blocking: it may allocate the rect, route input, enqueue work, update texture registration, and paint the latest completed frame, but it must not wait for exact evaluation or rendering.
- [ ] During drag, request repaint every frame.
- [ ] On background frame publish, request repaint.
- [ ] Add an example app with:
  - chart widget,
  - param slider,
  - checkbox/toggle param,
  - pan/zoom interaction,
  - visible metrics/debug panel.
- [ ] Make the example app demonstrate a real egui control driving an Avenger param:
  - use an `eframe` app shell,
  - place controls in an egui side panel,
  - place the Avenger chart widget in the central panel,
  - bind an egui slider to a numeric chart param,
  - call `set_param` when the slider value changes,
  - keep painting the latest completed Avenger frame while the new exact render is pending,
  - show the latest frame generation/render status in a small debug readout.

Example shape:

```rust
struct BasicChartApp {
    plot: avenger_egui::AvengerPlotHandle,
    point_size: f64,
}

impl eframe::App for BasicChartApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::SidePanel::left("controls").show(ctx, |ui| {
            let changed = ui
                .add(egui::Slider::new(&mut self.point_size, 1.0..=20.0).text("Point size"))
                .changed();

            if changed {
                self.plot.set_param("point_size", self.point_size);
            }

            self.plot.show_metrics(ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let output = avenger_egui::Plot::new(&self.plot).show(ui);

            if output.param_changed("point_size") {
                if let Some(value) = self.plot.param_f64("point_size") {
                    self.point_size = value;
                }
            }
        });
    }
}
```

Initial API shape:

```rust
pub struct PlotOutput {
    pub response: egui::Response,
    pub param_changes: Vec<ParamChange>,
    pub selection_changes: Vec<SelectionChange>,
    pub frame_status: FrameStatus,
}

impl PlotOutput {
    pub fn changed(&self) -> bool;
    pub fn params_changed(&self) -> bool;
    pub fn param_changed(&self, name: &str) -> bool;
    pub fn param_changes(&self) -> &[ParamChange];
    pub fn selection_changes(&self) -> &[SelectionChange];
}

pub struct ParamChange {
    pub name: String,
    pub value: ScalarValue,
    pub previous: Option<ScalarValue>,
    pub revision: u64,
}

pub struct SelectionChange {
    pub name: String,
    pub revision: u64,
}
```

If selection observation is not wired in the first egui crate commit, keep `selection_changes` present but always empty, and document that it is reserved for the next selection-observation pass.

Implementation note:

The first version should prefer egui's native texture registration and `ui.painter().image(...)`. Only introduce `egui_wgpu::CallbackTrait` if same-frame GPU preparation or custom blit behavior is needed.

API scope note:

Keep the initial egui-facing param-setting API low-level and explicit. Provide `set_param(name, value)` and use normal egui widgets plus `Response::changed()` in examples. Observation should feel widget-like: the plot widget returns `PlotOutput`, and app code can inspect `output.changed()`, `output.param_changed(name)`, or `output.param_changes()`. Do not add `bind_param`, `param_slider`, `slider_f64`, or similar higher-level binding helpers in this plan. Those can be considered later after the low-level API and async render behavior feel right.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-egui
cargo run -p avenger-egui --example basic_chart
```

Manual checks:

- [ ] Slider changes enqueue non-blocking exact renders.
- [ ] Rapid slider changes keep the GUI responsive while exact renders are pending.
- [ ] Drag/pan interactions keep the GUI responsive while exact renders are pending, even if the displayed chart remains on the latest completed frame.
- [ ] Record subjective staleness during drag/pan so semantic preview can be evaluated later.
- [ ] Window resize does not stall.
- [ ] Latest completed frame is reused when no new frame is ready.

Commit:

- [ ] Commit Phase 10.
- Suggested message: `feat(egui): add offscreen avenger plot widget`
- Commit hash: TBD

## Phase 11 - Full Background GPU Rendering

Purpose: move exact rendering fully off the GUI hot path where native WGPU permits it.

This phase should happen only after the egui example works with UI-thread GPU upload/render.

Tasks:

- [ ] Confirm WGPU `Device`/`Queue` sharing requirements for the chosen WGPU version.
- [ ] Decide thread ownership model:
  - cloned `Device`/`Queue`,
  - one worker per widget,
  - shared worker pool.
- [ ] Ensure all renderer resources used by a background worker are not mutated by the GUI thread at the same time.
- [ ] Give background worker its own `AvengerWgpuRenderer` or carefully synchronized renderer state.
- [ ] Render exact frame into a back `OffscreenTarget`.
- [ ] Submit background render commands.
- [ ] Publish only after submission is complete enough for safe sampling.
- [ ] Add optional fences/polling if required by WGPU semantics.
- [ ] Drop stale generations before expensive GPU work when possible.
- [ ] Add fallback for wasm, where true background GPU rendering may not be available.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
cargo test -p avenger-egui
```

Manual checks:

- [ ] Artificially slow exact render does not stall egui frame loop.
- [ ] Rapid param changes discard stale generations.
- [ ] No WGPU validation errors under repeated resize.

Commit:

- [ ] Commit Phase 11.
- Suggested message: `feat(wgpu): support background offscreen exact rendering`
- Commit hash: TBD

## Phase 12 - Observability, Metrics, and Debugging

Purpose: make performance and correctness visible enough to tune.

Tasks:

- [ ] Add tracing spans for:
  - GUI event routing,
  - param patch enqueue,
  - exact enqueue,
  - exact evaluation,
  - `set_scene`,
  - offscreen render encode,
  - offscreen render submit,
  - frame publish,
  - GUI texture paint.
- [ ] Add counters for:
  - frames painted,
  - exact frames published,
  - stale exact frames dropped,
  - reused latest-frame paints,
  - frames painted while an exact render is pending.
- [ ] Add an example debug overlay for egui.
- [ ] Document recommended env vars for tracing.
- [ ] Add a small benchmark or profiling example.

Validation:

```bash
cargo fmt --all
cargo test -p avenger-wgpu
```

Commit:

- [ ] Commit Phase 12.
- Suggested message: `feat(wgpu): add gui render timing instrumentation`
- Commit hash: TBD

## Phase 13 - Documentation and Final Cleanup

Purpose: make the architecture maintainable.

Tasks:

- [ ] Add or update architecture docs under `avenger-chart/docs/architecture/`.
- [ ] Prefer a focused document such as `avenger-chart/docs/architecture/wgpu-gui-offscreen.md`.
- [ ] Link the new architecture document from the existing architecture docs index or nearest relevant architecture overview.
- [ ] Document renderer core ownership.
- [ ] Document host wrappers.
- [ ] Document offscreen texture lifecycle.
- [ ] Document async render lifecycle.
- [ ] Document egui integration.
- [ ] Document limitations:
  - texture format constraints,
  - WGPU version coupling,
  - wasm limitations,
  - background GPU render caveats,
  - direct rendering status.
- [ ] Document explicit non-goals:
  - no Iced crate in this plan,
  - no direct egui render-pass integration in this plan,
  - no semantic drag preview until baseline staleness is evaluated,
  - no egui-backed Avenger text backend in this plan.
- [ ] Remove temporary compatibility wrappers if no longer needed.
- [ ] Remove dead code from the old render path.
- [ ] Update examples list.

Validation:

```bash
cargo fmt --all
cargo clippy --all-targets
cargo test
```

Commit:

- [ ] Commit Phase 13.
- Suggested message: `docs(wgpu): document gui offscreen rendering architecture`
- Commit hash: TBD

## Milestone Completion Definitions

Use these definitions when reporting progress.

Renderer refactor complete:

- [ ] Phases 0-6 are complete.
- [ ] `WindowCanvas` and `PngCanvas` still pass their existing tests.
- [ ] `avenger-wgpu` can encode a scene into a caller-provided texture view.

egui MVP complete:

- [ ] Phases 0-10 are complete.
- [ ] The egui example displays an Avenger chart through an offscreen texture.
- [ ] `set_param` from an egui slider enqueues a non-blocking chart update.
- [ ] The plot widget returns `PlotOutput`.
- [ ] egui input routes through Avenger `WindowEvent`s.
- [ ] The GUI keeps repainting/latest-frame sampling while a new exact frame is pending.

Full plan complete:

- [ ] Phases 0-13 are complete.
- [ ] Phase 11 full background GPU rendering has either been implemented or explicitly documented as not needed after measurement.
- [ ] Architecture docs under `avenger-chart/docs/architecture/` are updated.

## Acceptance Criteria

The refactor is complete when:

- [ ] `WindowCanvas` still works.
- [ ] `PngCanvas` still works and visual baselines pass.
- [ ] `avenger-wgpu` exposes a renderer core that can render into a caller-provided texture view.
- [ ] `avenger-wgpu` exposes offscreen targets usable as sampled GUI textures.
- [ ] egui can display an Avenger chart as a native widget.
- [ ] Native GUI controls can patch Avenger params.
- [ ] Widget-local GUI events route into Avenger `WindowEvent`s.
- [ ] Exact renders can be scheduled asynchronously.
- [ ] The GUI frame loop can sample the latest completed frame without waiting.
- [ ] Drag/pan/zoom does not block the GUI frame while an exact render is pending. The first MVP may display the latest completed frame until a new exact frame publishes.
- [ ] Baseline drag/pan/zoom staleness has been manually evaluated before deciding whether semantic preview is needed.
- [ ] Stale background results are discarded safely.
- [ ] The design is documented.

## Risks and Mitigations

### WGPU Version Coupling

Risk:

egui-wgpu may require a newer WGPU version than Avenger currently uses.

Mitigation:

- Pick one egui-compatible WGPU version for the first integration.
- Keep egui-specific types out of `avenger-wgpu`.
- Keep the renderer core backend-neutral so future GUI crates can be added later.

### Texture Format Constraints

Risk:

GUI backends may require specific sampleable texture formats.

Mitigation:

- Use `Rgba8Unorm` for offscreen GUI textures initially.
- Convert or blit from Avenger's preferred format if needed.
- Document any backend-specific constraints.

### GUI Frame Blocking

Risk:

GPU resource upload or accidental synchronous exact work may still happen on the GUI hot path.

Mitigation:

- Keep paint/draw paths limited to sampling an existing texture.
- Move exact work to background lanes in phases.
- Keep semantic preview out of the first MVP until baseline feel has been measured.

### Resource Lifetime Bugs

Risk:

The GUI may sample a texture while a worker writes to it.

Mitigation:

- Use double or triple buffering.
- Publish immutable `Arc<RenderedFrame>` handles.
- Never mutate the published front frame.

### Stale Render Results

Risk:

Slow exact results can overwrite newer interactions.

Mitigation:

- Use generation IDs.
- Drop stale generations before publishing.
- Prefer latest-wins semantics.

### Direct Render Temptation

Risk:

Trying to render directly into GUI render passes too early may complicate backend integration.

Mitigation:

- Make offscreen/blit the first-class path.
- Treat direct rendering as a later optimization only if profiling shows the extra composite pass matters.

## References

- egui-wgpu callback lifecycle: https://docs.rs/egui-wgpu/latest/egui_wgpu/trait.CallbackTrait.html
- egui-wgpu render state: https://docs.rs/egui-wgpu/latest/egui_wgpu/struct.RenderState.html
- egui-wgpu native texture registration: https://docs.rs/egui-wgpu/latest/egui_wgpu/struct.Renderer.html
