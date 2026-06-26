# Always-On Typst Text Engine Plan

## Goal

Make the owned `avenger-typst` text engine the only Avenger text backend. Text
should always support Typst-style `$...$` math fragments and the owned static
text markup subset. Remove the old optional math wiring, cosmic-text backend,
HTML canvas text path, dynamic measurer/rasterizer traits, and explicit
measurement propagation that existed only to choose between text engines.

## Target Architecture

- [x] `avenger-text` exposes one concrete text engine built on owned
  `avenger-typst`.
- [x] Typst text/math behavior is always available; no runtime
  `TextMathConfig.mode = Plain` switch is needed for normal chart rendering.
- [x] SVG/PDF use the hybrid Typst extraction path by default:
  - native SVG `<text>` for plain text runs;
  - paths for math and decoration shapes.
- [x] WGPU uses Typst rasterization/text-line atlas entries by default.
- [x] Geometry, guides, chart layout, and hit testing use Typst measurement by
  default without accepting a `&dyn TextMeasurer`.
- [x] Cosmic-text and HTML canvas measurement/rasterization code are removed
  from the core workspace.

## Phase 1: Make Typst The Only `avenger-text` Backend

- [x] Update `avenger-text/Cargo.toml`.
  - [x] Remove `cosmic-text` feature.
  - [x] Remove `fontdb` feature if it only exists for the old cosmic resolver.
  - [x] Remove `typst-math`, `typst-math-raster`, `typst-text`, and
    `typst-text-raster` feature gates.
  - [x] Make `avenger-typst` a normal dependency with owned text support.
  - [x] Make raster support a normal dependency if WGPU always needs it, or keep
    a single `raster` feature only if non-rendering builds genuinely benefit.
- [x] Remove cosmic-only modules.
  - [x] Delete `avenger-text/src/measurement/cosmic.rs`.
  - [x] Delete `avenger-text/src/rasterization/cosmic.rs`.
  - [x] Delete `avenger-text/src/font_resolver/cosmic.rs`.
  - [x] Remove cosmic-only helpers/tests from `avenger-text/src/fonts.rs`.
- [x] Remove HTML canvas text modules if no longer part of the selected text
  backend.
  - [x] Delete `avenger-text/src/measurement/html_canvas.rs`.
  - [x] Delete `avenger-text/src/rasterization/html_canvas.rs`.
  - [x] Delete `avenger-text/src/font_resolver/wasm.rs` if it only supports the
    browser text path.
- [x] Rename `avenger-text/src/typst_text.rs` to a backend-neutral name such as
  `engine.rs` or `text_engine.rs`.
- [x] Make `avenger-text/src/math.rs` either:
  - [ ] disappear into the concrete engine config; or
  - [x] become a smaller `TextMarkupConfig` with delimiter and error-policy
    settings only.
- [x] Ensure default behavior treats `$...$` as active math.

## Phase 2: Replace Text Traits With Concrete APIs

- [x] Delete the `TextMeasurer` trait.
- [x] Delete the `TextRasterizer` trait.
- [x] Introduce a concrete `TextEngine` type, or module-level functions if no
  cache/state ownership is needed.
- [x] Provide concrete measurement APIs:
  - [x] `TextEngine::measure_bounds(&TextMeasurementConfig) -> TextBounds`
  - [x] `TextEngine::font_metrics(&FontMetricsConfig) -> FontMetrics`
- [x] Provide concrete raster APIs:
  - [x] `TextEngine::rasterize(&TextRasterizationConfig, scale, cached_entries)`
  - [x] decide the final cache key/value types for whole-line Typst atlas
    entries.
- [x] Provide concrete path extraction APIs:
  - [x] `TextEngine::extract_paths(&TextPathConfig) -> TextPathBuffer`
  - [x] keep plain-run metadata needed for native SVG/PDF text embedding.
- [x] Keep the existing config/result structs where they remain useful:
  - [x] `TextMeasurementConfig`
  - [x] `FontMetricsConfig`
  - [x] `TextRasterizationConfig`
  - [x] `TextPathConfig`
  - [x] `TextBounds`
  - [x] `TextPathBuffer`
- [x] Remove `default_text_measurer()` and `default_rasterizer()`.
- [x] Add compatibility wrappers only if required for downstream crates, and
  mark them as temporary.

## Phase 3: Remove Measurer Propagation From Chart/Layout/Guides

- [x] Remove `TextMeasurementRuntime` from `avenger-chart/src/render/context.rs`.
- [x] Remove `text_measurer` from evaluation/render contexts.
- [x] Remove `text_measurement_cache_tag` unless a concrete Typst engine cache
  still needs an explicit tag.
- [x] Replace calls to `eval_ctx.text_measurer().measure_text_bounds(...)` with
  one concrete Typst measurement path.
- [x] Keep `TextMeasurementService` only as the chart adjustment cache hook, not
  as a backend-selection trait; adjustment transforms should call
  `AdjustmentTransformContext::measure_text_bounds(...)`.
- [x] Collapse chart helper methods that only pass a measurer through call
  layers.
- [x] Remove `*_with_text_measurer` APIs from `avenger-guides`.
  - [x] Axis builders.
  - [x] Numeric/band/nested-band guide sizing.
  - [x] Colorbar guide sizing.
  - [x] Line/symbol legend builders.
- [x] Remove `*_with_text_measurer` APIs from chart legend/render construction.
- [x] Remove `text_measurer` arguments in facet and container guide code.
- [x] Remove pointer-based text-measurer cache keys from container band guide
  height caches.
- [x] Replace those cache keys with either:
  - [x] no text-engine identity because there is only one engine; or
  - [ ] an explicit markup/config version if delimiter/error policy remains
    configurable.

## Phase 4: Simplify Geometry And Hit Testing

- [x] Remove `geometry_iter_with_text_measurer`.
- [x] Remove `bounding_box_with_text_measurer`.
- [x] Make `geometry_iter` and `bounding_box` use Typst measurement internally.
- [x] Remove `SceneGraphRTree::from_scene_graph_with_text_measurer`.
- [x] Make `SceneGraphRTree::from_scene_graph` Typst-aware by default.
- [x] Update chart evaluation to call the default geometry/R-tree constructors.
- [x] Update direct scenegraph tests that expected plain/cosmic geometry.

## Phase 5: Simplify Renderers

### WGPU

- [x] Remove cosmic glyph atlas paths from `avenger-wgpu`.
- [x] Remove `CanvasConfig.text_math`.
- [x] Make text atlas entries line-based Typst entries by default.
- [x] Confirm cache keys include source, font family, font size, weight, style,
  fill, scale, and remaining markup settings.
- [x] Remove `typst-text-raster` / `typst-math-raster` feature gates from WGPU.
- [ ] Verify emoji, bidi, complex scripts, and math render through the Typst
  path.

### SVG

- [x] Remove `SvgRenderOptions.text_math`.
- [x] Always use Typst path extraction for text marks.
- [x] Emit native `<text>` for plain runs.
- [x] Emit paths for math/decorations.
- [x] Keep font subset collection for native text runs.
- [x] Remove non-Typst native text fallback code paths if they only exist for
  cosmic/plain text.
- [ ] Keep existing unsupported gradient behavior for labels that contain math
  paths, or implement a deliberate replacement.

### PDF

- [x] Remove `PdfRenderOptions.text_math`.
- [x] Continue feeding hybrid SVG into `svg2pdf` with text embedding.
- [x] Keep math as paths for this stage.
- [x] Preserve future PDF glyph metadata in Typst path outputs for later direct
  math font embedding.

## Phase 6: Cargo Features And Public API Cleanup

- [x] Remove chart features:
  - [x] `typst-text`
  - [x] `typst-text-layout`
  - [x] `typst-text-raster`
  - [x] `typst-math-layout`
  - [x] `typst-math-raster`
  - [x] `typst-math-svg-pdf`
- [x] Remove renderer crate feature flags that only selected the text backend.
- [x] Remove public `TextMathConfig` from renderer options and chart evaluation
  options.
- [x] Decide whether any public delimiter configuration remains.
  - [ ] If no, hard-code default Typst-style delimiters.
  - [x] If yes, expose a small `TextMarkupConfig` but do not allow disabling the
    Typst engine.
- [x] Update `avenger-chart/src/prelude.rs` if public types are removed or
  renamed; no text-engine public type removal remains after retaining the
  adjustment cache hook.
- [x] Update docs/future-work notes to say Typst text is the default path.
- [x] Update probes:
  - [x] Keep `tools/text-render-probe` Typst path as the default current probe.
  - [x] Remove cosmic comparison or mark it historical.
  - [x] Keep `tools/text-size-probe` only if it still answers a useful question.

## Phase 7: Tests And Baselines

- [ ] Unit tests:
  - [x] `avenger-text` measurement for plain text.
  - [x] `avenger-text` measurement for mixed plain/math text.
  - [x] `avenger-text` rasterization for whole-line Typst atlas entries.
  - [x] `avenger-text` path extraction for native plain runs plus math paths.
  - [x] emoji fallback and named `#emoji.face` syntax.
  - [x] bidi and complex-script shaping.
  - [x] escaped dollars and unmatched delimiter policy.
  - [x] unsupported syntax errors for evaluator/document features.
  - [ ] geometry/R-tree text bounds use Typst by default.
- [x] Renderer tests:
  - [x] WGPU text renders without cosmic.
  - [x] SVG contains native `<text>` for regular runs and paths for math.
  - [x] PDF regular text remains extractable/selectable through `svg2pdf`.
  - [x] math remains path-only in PDF for this stage.
- [ ] Visual tests:
  - [ ] Run all chart baselines, not just `typst_math`, because all labels now
    use Typst.
  - [ ] Review title/subtitle spacing.
  - [ ] Review legends.
  - [ ] Review axis labels and rotated labels.
  - [ ] Review Vega-derived baselines for font/line-height changes.
  - [ ] Read every generated failure/baseline image before accepting.
  - [ ] Accept baselines only after judging that changes are correct.
  - [x] Reviewed and accepted treemap baselines after moving treemap guide/label
    measurement onto `TextEngine`.

## Phase 8: Validation Commands

Run release mode throughout.

- [x] `cargo fmt --all`
- [x] `cargo test --release -p avenger-typst --all-features`
- [x] `cargo test --release -p avenger-text`
- [x] `cargo test --release -p avenger-geometry`
- [x] `cargo test --release -p avenger-guides`
- [x] `cargo test --release -p avenger-svg`
- [x] `cargo test --release -p avenger-pdf`
- [ ] `cargo test --release -p avenger-wgpu`
  - [x] `cargo test --release -p avenger-wgpu --lib`
  - [ ] Full WGPU package run currently fails image baseline tests, including
    broad non-text baseline drift and one SVG-resource feature case.
- [x] `cargo test --release -p avenger-chart -- --nocapture`
- [x] `cargo check --release --workspace`
- [x] `cargo test --release -p avenger-chart-webmercator -- --nocapture`
- [x] `cargo test --release -p avenger-chart-treemap --test visual_regression -- --nocapture`
- [ ] `cargo test --release -p avenger-chart --features visual-tests --test visual_regression -- --nocapture`
- [ ] Run SVG/PDF sidecar validation used by chart visual tests.
- [ ] Run wasm build checks for browser targets that previously relied on HTML
  canvas text measurement.
- [ ] Run text-size and text-render probes and record current numbers.
  - [x] `cargo build --release --manifest-path tools/text-size-probe/Cargo.toml --features typst`

## Migration Notes

- [ ] Commit in small slices:
  - [x] avenger-text backend removal.
  - [x] trait removal and concrete API.
  - [x] geometry/guides/chart propagation removal.
  - [x] renderer feature cleanup.
  - [ ] baseline updates.
- [x] Keep each slice compiling in release mode before moving on.
- [x] Prefer deleting compatibility layers quickly once all workspace call sites
  are updated.
- [x] Do not preserve cosmic behavior as a hidden fallback.
- [x] Do not preserve HTML canvas behavior as a hidden fallback.
- [ ] If a platform issue appears, fix the owned Typst path for that platform
  instead of reintroducing backend selection.

## Main Risks

- [ ] Wasm/browser builds may need owned Typst font fallback adjustments after
  removing HTML canvas text measurement.
- [ ] All chart baselines can shift because regular text now always goes through
  Typst shaping/rasterization.
- [ ] Whole-line atlas entries may affect cache pressure compared with
  glyph-level cosmic entries.
- [ ] Removing `TextMeasurer` propagation simplifies correctness but may expose
  places that relied on custom measurers in tests.
- [ ] Some downstream code may depend on old `TextMeasurer`/`TextRasterizer`
  public traits.

## Success Criteria

- [x] Workspace builds without `cosmic-text`.
- [x] Workspace builds without HTML canvas text measurement/raster modules.
- [x] `avenger-text` has one concrete text engine.
- [x] Chart layout, guides, geometry, WGPU, SVG, and PDF all use the Typst path
  by default.
- [x] No `&dyn TextMeasurer` propagation remains in chart/guides/geometry.
- [x] No `typst-*` text feature flag is required to get math-capable text.
- [ ] All release tests pass.
- [ ] Visual baselines are reviewed and updated intentionally.
