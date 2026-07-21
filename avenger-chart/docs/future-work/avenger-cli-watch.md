# Avenger CLI: Native `watch` Command

## Status

The playable implementation landed on 2026-07-18 in `avenger-lang-cli`. As of
2026-07-20 it also has dynamic definition/catalog/file-data watching,
context-local content-version providers over one shared physical cache, typed
param/store/selection and built-in-widget state migration, coordinated
watcher/worker/event-loop shutdown, source-aware exactly-once diagnostics, a
checked-in integration project, a fake compiler/host/reporter coordinator
suite, and a 37-root stock-host preparation matrix.

This document remains the requirements and hardening tracker. Native-window
behavior has been exercised manually on macOS, but the complete cross-platform
manual editor matrix remains open. Compiler-owned source/import/discovery,
syntax/SQL/expansion, and local-resource limits are implemented and inherited
by the CLI. The complete local release test matrix passes. Checkboxes below
reflect only behavior backed by current code and tests.

The initial user-facing command is:

```sh
avenger watch path/to/chart.avenger
```

It compiles one Avenger chart, displays it in a native winit/wgpu window, and
hot reloads the chart whenever the chart or any local dependency changes. The
window, GPU canvas, and physical-plan evaluation cache remain alive across
successful reloads. Each reload generation receives an isolated DataFusion
`SessionContext` and chart-runtime resource bundle so preparation cannot mutate
the last-good chart. Compatible typed document state is restored before the
replacement's first evaluation. Native focus/editor state, active gestures,
cursor state, and the narrow snapshot-to-install interaction race follow the
reset rules below.

This milestone intentionally does not add `check`, `render`, `fmt`, editor,
LSP, packaging, project-gallery commands, a live inspection endpoint, session
discovery, or `inspect`/DAP/MCP adapters. The crate and binary should be
structured so those commands can be added later.

The chart DSL reference uses this same `watch` name for the one-chart preview
workflow. Broader project galleries and the integrated `editor` command remain
later work; this milestone neither implements nor reserves `serve` as an alias.

## Naming

Recommended names:

- Cargo package/crate: `avenger-lang-cli`;
- installed binary: `avenger`;
- initial subcommand: `watch`.

`avenger-lang-cli` makes the dependency on the language/project compiler clear
and avoids occupying the broad `avenger-cli` crate name before the CLI surface
settles. The binary should still be `avenger`, since users should not need to
know the internal crate split.

The package should contain both `src/lib.rs` and `src/main.rs`. Argument parsing
and process setup stay thin in `main.rs`; watch orchestration belongs in the
library so it can be tested without starting a real desktop window.

### Deferred inspection architecture

A follow-on milestone may make the watch process the host for a versioned
Avenger inspection protocol over a local Unix socket or Windows named pipe.
Separate `avenger inspect`, `avenger dap`, `avenger mcp`, and `avenger lsp`
processes can then discover and attach to the running chart without sharing
winit or WGPU objects across processes. The protocol should expose opaque
session/object IDs, reload generation, committed revision, stable source/runtime
identity, capability negotiation, snapshots, bounded table previews, and
revision subscriptions; use Arrow IPC for tabular payloads and read-only access
by default.

The future native LSP combines this observed state with the compiler's static
`ProjectAnalysis`. DataFusion-derived planned schemas therefore power column
completion even when no chart is running, while the inspection connection can
add current values, observed schemas, runtime diagnostics, inlay hints, hover,
and an **Open in Inspector** action. The action launches a separate
`avenger inspect --session <id> --object <id>` process directly. This deferred
architecture does not change the playable `watch` requirements or completion
gate.

## Prerequisites

Implementation begins only after the following are available:

1. The Rust semantic-unification work in
   `scratch/avenger-lang/rust-dsl-unification-implementation-plan.md` is
   complete.
2. Phase 6 of
   `scratch/avenger-lang/dsl-language-compiler-implementation-plan.md` is
   complete, including the public single-file compile/evaluate vertical slice.
   Definitions, catalogs, multi-chart support, compiler incremental caches, and
   coverage closure are not prerequisites for the first playable delivery.
3. The compiler exposes the root plus every local dependency and missing-path
   anchor it can discover for the source forms supported by that phase, on both
   successful and failed attempts.
4. The compiler accepts a host-supplied per-generation compile environment
   created through its factory seam, and the physical cache can be shared
   safely. The initial CLI API-seam phase may add the narrow generation-aware
   entry point and existing-cache installation helper needed by the host.
5. The compiled artifact exposes the renderable chart and enough source and
   dependency identity to install and replace it safely.

The CLI should request narrow additions to those APIs rather than duplicating
project loading, import resolution, catalog setup, or dependency tracking.

### Delivery tiers

The **playable milestone** opens one persistent native window, watches the root
and all currently reported local dependencies, recompiles in the background,
atomically installs only the newest successful generation, retains the
last-good chart on failure, prints each failed generation's diagnostics once to
stdout, and reuses one physical cache across isolated generation contexts.
Reload may reset interaction state at this milestone.

The **full v1 milestone** additionally covers definitions and catalog/file-data
dependencies as those language phases land, generalized provider/function
cache identity and invalidation, compatible typed state migration, and the
hardening/acceptance matrix below.

## User Experience

### Command shape

The required invocation is:

```text
avenger watch [OPTIONS] <CHART>
```

`CHART` must resolve to one local chart root whose source file is named
`*.avenger`. Definition and `.data.avenger` roots are not valid entry points.

Initial options:

```text
--project-root <DIR>       Override the default project/capability root
--debounce-ms <MILLIS>     Filesystem event quiet period (default: 100)
--scale <FACTOR>           Window render scale override
--cache-memory-mb <MB>     Physical cache memory budget
--no-cache                 Disable the physical-plan cache
--log-cache                Log cache metric deltas after each evaluation/reload
```

Avoid adding flags for future commands. Environment and data capabilities use
the language compiler's existing explicit options. `RUST_LOG` and
`AVENGER_PHYSICAL_CACHE=0` retain their existing diagnostic/kill-switch roles;
`--no-cache` is the command-local explicit equivalent of the latter.

### Startup

1. Validate arguments and resolve the chart and project root.
2. Create the long-lived `EvaluationCache`, then create the initial isolated
   `SessionContext` with a planner over that cache before any `DataFrame` is
   constructed.
3. Compile the chart.
4. If compilation fails, print the structured source diagnostics to stdout
   exactly once for that compilation generation and exit nonzero; do not open
   an empty window for the first milestone.
5. Build the initial generation's `ChartAppBundle` and runtime resources.
6. Open one native window whose title includes the chart name or filename.
7. Print a concise ready line containing the chart path and number of watched
   local dependencies.

### Successful reload

When a relevant local change is observed:

1. Coalesce the editor's filesystem burst.
2. Recompile the newest project contents off the winit event-loop thread.
3. Discard a completed result if a newer reload generation already superseded
   it.
4. For the playable milestone, initialize state from the replacement chart's
   declarations. For full v1, best-effort migrate compatible chart state.
5. Replace the chart app and install its scene graph into the existing window
   and GPU canvas.
6. Update the dependency watch set, window title, resize behavior, and scene
   size.
7. Print one concise success line with compile/evaluate/install timings and,
   when requested, physical-cache metric deltas.

The window must not close, flash through an empty scene, or create a second
window during reload.

### Failed reload

If a reload fails to parse, resolve, lower, compile, evaluate, or load a local
resource:

- keep displaying and interacting with the last successfully compiled chart;
- print the new structured compiler diagnostics to stdout exactly once per
  failed compilation generation;
- mark the window title with a short error suffix such as `[compile error]`;
- keep watching the last-good dependency set plus every local path/parent
  discovered by the failed attempt;
- retry after the next relevant filesystem change;
- do not clear the physical cache or destroy the last-good generation's
  resources;
- restore the normal title after the next successful reload.

Failure after startup is therefore recoverable and does not determine the
process exit code unless the window/runtime itself can no longer continue.

### Exit behavior

- Normal window close: exit code `0`.
- Invalid arguments: conventional CLI usage exit code.
- Initial compilation failure: exit code `1`.
- Window/GPU/runtime initialization failure: exit code `1`.
- Reload failure after a successful startup: keep running.
- `Ctrl-C` should request a clean event-loop shutdown when supported by the
  chosen process integration; normal window close remains primary.

## Local Dependency Semantics

“Any local files it references” means the transitive local dependency closure,
not only syntactic `import` statements.

The compiler must classify and return local dependencies such as:

- the root chart source;
- relative mark/tool/transform definition imports;
- locally imported dataset packs and `.data.avenger` files;
- CSV, JSON, Parquet, Arrow/IPC, or other file-backed tables;
- directories and glob roots whose matching file set affects a table;
- local theme CSS files;
- local image, SVG, font, tile, or other declared resources;
- a project `.env` file only when the active compile actually consults it;
- local provider configuration files.

Remote URL imports and remote object-store/catalog data are compiled normally
but are not polled by `watch` v1. Their locally cached implementation files are
not treated as authored project dependencies.

Each dependency record should contain:

```rust
pub struct LocalDependency {
    pub canonical_path: PathBuf,
    pub role: LocalDependencyRole,
    pub watch_anchor: PathBuf,
    pub version: Option<DependencyVersion>,
}
```

The compiler-facing operation returns an attempt report rather than losing
dependency discovery inside `Err`:

```rust
pub struct CompileAttempt<T> {
    pub result: Result<T, CompileFailure>,
    pub dependencies: LocalDependencySet,
}
```

`result` is successful only when there are no error diagnostics. The report accumulates
root/import/data/resource paths as they are discovered, including missing-path
parent anchors, even when parsing or resolution later fails. The CLI assigns
the reload generation outside this structure and is solely responsible for
printing its diagnostics once.

`watch_anchor` is often the containing directory rather than the file itself.
Watching parent directories is necessary for atomic-save workflows, where an
editor writes a temporary file and renames it over the original, and for
missing dependencies that may be created later.

After every compile attempt, the effective watch set is:

```text
root chart
union last successful dependency closure
union local dependencies and missing-path parents discovered by latest attempt
```

After a successful compile, obsolete last-good dependencies may be removed.
The watcher must support replacing its filtered dependency set without
restarting the process.

## Filesystem Watching Requirements

Use `notify` on native targets behind a small CLI-owned abstraction. The current
`avenger-winit-wgpu::FileWatcher` is not sufficient as-is: it captures a fixed
startup file set, watches files rather than robust parent anchors, and handles
only data-modification events.

The watch layer must:

- watch a minimal deduplicated set of directories non-recursively where
  possible;
- filter events back to dependency files, glob roots, and missing paths;
- recognize create, modify, remove, rename, and relevant metadata events;
- tolerate duplicate/out-of-order platform events;
- support atomic replacement and delete-then-create saves;
- debounce a burst using a quiet period, defaulting to 100 ms;
- include all affected canonical paths in the reload request;
- trigger at most one active compile at a time;
- set a dirty/latest-generation flag when changes arrive during compilation;
- immediately schedule the newest generation when the current compile ends;
- expose a fake backend and deterministic clock for tests;
- report watcher setup/runtime errors without panicking.

Do not run the language compiler inside a `notify` callback. The callback only
records paths and wakes the reload coordinator.

## Reload Architecture

The CLI owns five long-lived objects:

```text
WatchCommand
  ├─ avenger_lang::Compiler
  ├─ Arc<EvaluationCache>
  ├─ ReloadSessionFactory
  │    └─ isolated SessionContext + context-local version providers/generation
  └─ ReloadCoordinator
       ├─ DependencyWatcher
       ├─ background compile worker
       ├─ generation/dirty state
       └─ winit wake-up bridge
```

The compilation worker produces a prepared reload result:

```rust
pub enum ReloadResult {
    Success(PreparedChartReload),
    Failure(PreparedCompileFailure),
}

pub struct PreparedChartReload {
    pub generation: u64,
    pub artifact: CompiledChartArtifact,
    pub context: Arc<SessionContext>,
    pub bundle: ChartAppBundle,
    pub dependencies: LocalDependencySet,
    pub timings: ReloadTimings,
    pub cache_metrics_before: CacheMetricsSnapshot,
    pub cache_metrics_after: CacheMetricsSnapshot,
}
```

The generation context stays alive in its `PlotSession`, but retaining it
explicitly in the prepared payload makes the ownership boundary reviewable.
Compilation and initial chart evaluation happen away from the winit event-loop
callback. Only final bundle/scene installation and window mutation happen on
the event-loop thread.

### Winit hot-swap seam

Add a supported host-update seam to `avenger-winit-wgpu`; do not have the CLI
reach into its public `Rc<RefCell<AvengerApp<_>>>` field and replace internals
ad hoc.

The seam should let a native host:

- enqueue a prepared replacement and send a lightweight host wake event;
- atomically replace `AvengerApp<ChartAppState>` on the event-loop thread;
- install the replacement's already built scene graph into the existing
  `WindowCanvas`;
- update `WindowSceneSizing`, title, and requested window size;
- reset stale render-pending and resize-coalescing bookkeeping;
- retain the winit `Window`, `WindowCanvas`, device/queue/surface, scale, and
  frame configuration;
- replace the old render-invalidation subscription with one bound to the new
  generation's hub and replay its latest pending invalidation;
- request a redraw;
- surface installation errors to the coordinator rather than panic.

A `HostWake` event plus a shared prepared-update queue is sufficient; the full
compiled chart need not be embedded directly in `WinitWgpuEvent`. Keep the API
generic enough for future hosts, but do not build a general plugin system.

### Static thread-boundary findings

Static inspection supports preparing a complete replacement bundle on a
background worker:

- `AvengerApp<State>` itself contains no `Rc`, `RefCell`, window, canvas, or
  WGPU object. Its scene builder and event handlers require `Send + Sync`, and
  its state generic requires `Send + Sync`.
- `ChartAppState` is composed from `Arc` plus synchronized state, and its
  `PlotSession` owns an `Arc<SessionContext>` and compiled trait objects whose
  core traits require `Send + Sync`.
- `ChartRuntimeResources` contains an `Arc<dyn ImageResourceResolver>` whose
  trait is `Send + Sync`, plus a mutex-backed `RenderInvalidationHub`.
- `WinitWgpuAvengerApp` is deliberately event-thread-bound: it owns
  `Rc<RefCell<...>>`, the winit window/canvas state, and the native runtime used
  for interactive event dispatch. It must never cross the worker boundary.
- `EventLoopProxy` is already the supported cross-thread wake mechanism.

These bounds strongly indicate that `ChartAppBundle` can be sent from the
worker, but Phase 0 must add non-executing compile-time assertions such as
`assert_send::<ChartAppBundle>()` after the prerequisite work lands. If a
landed field breaks the bound, keep that field worker-local and send the
largest `Send` prepared payload; do not move chart compilation/evaluation back
onto the event loop.

The winit app currently owns a current-thread Tokio runtime and blocks on it for
interactive updates. The reload worker therefore owns a separate runtime or
dedicated runtime thread. Do not attempt to drive background compilation from
the runtime after it has been moved into `WinitWgpuAvengerApp`.

### Runtime resources and invalidation generations

Use one `ChartRuntimeResources` bundle per reload generation, not one bundle
shared across old and preparing apps. The current render invalidations do not
carry a chart generation, and `ImageResourceCache` owns one mutable
invalidation sink. Sharing the bundle while the replacement is prepared could
deliver a new app's invalidation to the still-installed old app, or redirect an
old app's pending image completion to the new app.

The worker constructs the replacement with a new generation-local
`RenderInvalidationHub` and resource cache. The winit hot-swap seam atomically:

1. drops the subscription to the old hub;
2. installs the replacement app;
3. subscribes its event proxy to the new hub;
4. installs the replacement scene;
5. replays the new hub's latest pending evaluation invalidation;
6. drops the old app and old runtime resources.

This is consistent with the winit runner's existing startup replay mechanism
and prevents cross-generation invalidation races. Cross-generation image/tile
payload reuse may be added later by separating payload storage from
generation-local invalidation routing; it is not required for the initial
watch milestone. A fresh resource cache also guarantees that a changed local
resource is not served from the previous generation under the same key.

## Physical-Plan Cache Requirements

Hot reload must opt into the existing `avenger-datafusion-cache` integration.
This cache accelerates DataFusion execution of unchanged physical subtrees; it
does not skip parsing, name resolution, lowering, logical optimization, or
physical planning. Reuse the language compiler's own parsed/resolved project
caches for those stages and report their timings separately. Physical-cache
hits still pay fingerprint/planner overhead, so performance assertions must
measure the complete reload as well as cache counters.

### Lifetime

- Construct one `Arc<EvaluationCache>` for the watch process.
- Construct a fresh isolated `SessionContext` for each generation and install
  an `EvaluationCachePlanner` over that shared cache before any
  compiler-created `DataFrame` exists.
- Keep the last-good generation context alive inside its displayed
  `PlotSession` until the replacement is atomically installed.
- Never clear the cache merely because chart/definition/theme source changed.
- Never serialize the cache into compiled artifacts.
- Retain the environment kill switch and implement `--no-cache` without
  installing a dormant optimizer rule.

Static inspection confirms that `EvaluationCachePlanner::new` already accepts
an `Arc<EvaluationCache>`, and `EvaluationCache` protects entries,
observations, metrics, the content-hash memo, and providers with thread-safe
interior state. The missing chart-level API is a composable helper such as:

```rust
pub fn install_shared_physical_cache(
    builder: SessionStateBuilder,
    cache: Arc<EvaluationCache>,
    version_providers: Vec<Arc<dyn CacheVersionProvider>>,
) -> SessionStateBuilder;
```

This helper installs the existing cache as the session-config extension and
appends a planner over it as the final physical optimizer rule. The current
`install_physical_cache` remains the convenience that creates a new cache.

Cross-context reuse is compatible with the current fingerprint design: keys
include DataFusion/fingerprint format versions, a result-affecting execution
config subset, local plan proto bytes, child/source fingerprints, output
schema, partitioning, ordering, and boundedness. The shared memory-source hash
memo is guarded by weak `Arc` identity and content hashing, so it remains safe
across contexts.

At the playable gate, every generation installs the same stock Avenger
UDF/UDAF semantics. When language phases permit custom function semantics, full
v1 makes the process registry immutable: the first registration fixes a
canonical semantic fingerprint for each name/signature, every generation
installs that same snapshot, and a conflicting same-name/signature redefinition
is rejected before compilation rather than sharing the cache. The overall
function-registry fingerprint also participates in the compile/cache
environment fingerprint. Versioned redefinition is a future extension, not an
alternative left to the implementation.

For full v1 catalog support, the current cache-global append-only provider list
is not sufficient for generation-specific catalog providers: providers are
consulted in registration order and the first match wins. Extend
`EvaluationCachePlanner` to own a context-local provider snapshot, optionally
following stable process-global providers. A generation's planner must
fingerprint its plan with that generation's provider/snapshot versions; do not
mutate a global “current version” map that could reinterpret an old displayed
context's plan.

The cache's observe-only flag and metric counters remain process-global. An
interaction in the displayed chart may temporarily prevent the preparing
generation from admitting new writes, but it cannot cause an incorrect hit.
Metric deltas recorded during concurrent interaction are process-wide and may
include work from the displayed chart; logs must describe them that way.

### Watch-oriented policy

Use a named watch profile based on `EvaluationCacheConfig`:

- `min_seen_count: 1`, so the initial evaluation can populate reusable
  subtrees for the first reload;
- bounded memory, configurable with `--cache-memory-mb`;
- the existing per-entry limit and execution-time admission floor unless
  measurement shows a better watch-specific value;
- normal lookups/admission during exact reload evaluation.

Do not globally enter observe-only mode during compilation or reload: hot
reload is precisely when useful new stable subtrees should be admitted.

### Correctness and invalidation

The cache may reuse a subtree only when its data inputs and execution semantics
are unchanged. The playable milestone proves this for the Phase 6 input forms;
the remaining bullets become active as definitions, catalogs, files, and custom
providers land.

- A style, layout, mark, or definition edit should preserve hits for unchanged
  data subplans.
- A changed query/transform naturally changes the plan fingerprint.
- File scans currently include paths, sizes, and mtimes in physical proto
  identity; test this behavior rather than assuming it is sufficient forever.
- Memory sources use content hashes and must be rebuilt from changed data.
- External/provider-backed sources must register a `CacheVersionProvider` when
  the physical plan does not encode a snapshot/version identity.
- A local dependency generation/version provider should cover any file-backed
  source shape whose native DataFusion fingerprint could otherwise false-hit,
  including same-size replacements with coarse or preserved mtimes.
- Failed compile attempts mutate only their isolated generation context and
  context-local provider set. Dropping that context cannot alter the displayed
  chart's catalog.
- A successful replacement publishes the new context only by installing its
  app. No catalog/table registration is copied into the old context.

Cache correctness always wins over reuse. When a changed provider cannot
supply a trustworthy version, exclude or invalidate that source rather than
risk a stale hit.

### Observability

Record metric snapshots around each successful reload and log deltas when
`--log-cache` or suitable tracing is enabled:

- hits and misses;
- admitted/committed/discarded writes;
- entries and bytes;
- evictions;
- exclusions by reason;
- fingerprint/planner time.

The normal default output should report only a compact process-wide hit/miss
summary after reload, not every cache node.

## State Migration

The first playable hot-reload delivery may reset params, stores, selections,
tools, widgets, gestures, and cursor to the replacement generation's declared
initial state. It must not guess compatibility from source names or opaque
runtime IDs. This keeps a safe native watch loop available before the complete
snapshot/restore API lands.

Full v1 should preserve compatible interaction state rather than resetting
every pan, selection, and tool whenever an unrelated style changes.

Before replacing the app, obtain a state snapshot from the last-good
`PlotSession`. Migrate by the compiler's stable state migration key/typed
identity metadata, never by generated string-prefix conventions.

Opaque per-compilation runtime IDs are not by themselves a migration contract.
The unified compiler/runtime must expose a separate deterministic
`StateMigrationKey` derived from the source declaration identity and component/
tool/widget instance ancestry, excluding irrelevant style/property content. It must
also expose a complete `ChartSessionSnapshot` containing all root and scoped
param values, stores, and selections. Current Rust exposes root param snapshots
and an internal scoped-param snapshot, but does not yet expose complete
store/selection session snapshots. Ownership is split deliberately: Rust
unification adds the compiled migration-key metadata hook, the DSL compiler
derives source-stable keys, and the runtime-owning chart/app crates expose the
complete typed snapshot/restore API during the full-v1 migration phase. The CLI
composes those surfaces and never inspects private session fields.

`ChartAppState` is a cloneable synchronized handle, so the reload worker can
request this snapshot after compilation succeeds and immediately before it
constructs/evaluates the replacement app. This keeps snapshot work off the
event loop and captures state near installation time. Interaction committed
after that snapshot but before installation may not migrate; the initial
milestone accepts this narrow race and records the snapshot timestamp and
generation in tracing rather than pausing the displayed chart during compile.

- Param: preserve when declared Arrow type and sharing contract are compatible.
- Store: preserve when ordered schema, nullability, primary key, and sharing
  contract are compatible.
- Selection: preserve when the selection schema/combine contract remains
  compatible; discard clauses that cannot resolve under the new definition.
- Private native/defined tool state: preserve when its deterministic instance
  identity and state contract match.
- Widget-exported params/stores/selections follow the same typed rules using
  widget instance ancestry. Native widget ephemeral editor/focus/gesture state
  is not document state and is not migrated in this milestone.
- Raw view/tool domain params follow the same typed-param rule.
- Active gesture/start/previous snapshots are not migrated; reload cancels the
  in-progress gesture.
- Cursor state is reset/re-derived by the replacement chart.

Incompatible or removed state resets to the new declaration's default/initial
value. This is not a compile error. Emit debug-level migration details and a
compact count in reload tracing.

State migration is implemented after the playable app-replacement gate and is
part of the full v1 `watch` completion gate.

## Concurrency and Generation Rules

Use monotonically increasing reload generations.

- Every debounced change batch allocates a generation.
- At most one compile/evaluate job runs at a time initially; this avoids
  overlapping mutation of compiler project caches while generation contexts
  and catalogs remain isolated.
- A change arriving during a job records a newer pending generation.
- A job result whose generation is not the latest requested generation is not
  installed.
- Safe reusable compiler/cache work from a stale result may remain cached, but
  no state, dependency watch set, title, diagnostics status, or chart artifact
  from it is published.
- Dropping/cancelling the process must stop the watcher and worker without
  blocking indefinitely.
- The event-loop thread never waits for filesystem debounce or compilation.

## Window and Rendering Behavior

- Use `chart_avenger_app_with_runtime_resources` to build each chart app.
- Use `WinitWgpuAvengerApp` and the existing window sizing helper derived from
  the compiled chart resize policy.
- Default window title: `avenger — <chart-name-or-file>`.
- Preserve user-moved window position across reload.
- Preserve user-resized dimensions when the new chart remains
  window-constrained.
- Recompute/request dimensions when chart-controlled canvas sizing changes.
- Reinstall the scene graph and rebuild interaction geometry atomically.
- Replace chart event bindings and tools with the new app; no old handlers may
  remain registered.
- Preserve the existing WGPU surface/device rather than reconstructing them.
- Keep rendering and interaction responsive while the background compile runs.
- On reload failure, the last-good chart remains fully interactive.

The initial command opens exactly one chart in exactly one window. Project
galleries, multiple chart tabs, and multiple windows are later work.

## Diagnostics and Logging

Default terminal output should be concise and stable enough to understand the
watch lifecycle:

```text
compiled sales.avenger in 184 ms; watching 6 local dependencies
reloaded after chart.avenger, theme.css in 71 ms; cache 14 hit / 3 miss
reload failed; keeping previous chart
<structured source diagnostics>
```

Requirements:

- preserve compiler diagnostic formatting, codes, source excerpts, and import/
  expansion traces;
- emit one complete compiler-diagnostic batch to stdout per failed compilation
  generation, guarded by generation in the coordinator so the worker, event
  loop, and generic logger cannot each print it;
- use one stdout lock/write for the batch when practical so diagnostics are not
  interleaved with reload status lines;
- permit a later explicit filesystem generation that produces the same error
  to print one new batch, but never repeat a batch without a new compilation;
- reserve stderr for CLI usage failures and watcher/window/GPU/runtime failures,
  not chart compilation diagnostics;
- normalize displayed dependency paths relative to the project root;
- never print credentials, environment values, URL query secrets, or table
  contents;
- use tracing spans for watch debounce, compile, catalog update, state
  migration, app construction, scene install, and redraw;
- suppress duplicate delivery of one failed generation even if multiple host
  wake events are coalesced;
- allow `RUST_LOG` to expose detailed compiler, app, WGPU, and cache timings.

No in-window diagnostic overlay is required initially; the title error suffix
and terminal diagnostics are sufficient.

## Crate Dependencies

Expected direct dependencies for `avenger-lang-cli`:

- `avenger-lang`;
- `avenger-chart` for physical-cache configuration/types;
- `avenger-chart-app` with `winit-wgpu` support;
- `avenger-winit-wgpu` for the host-update seam;
- `avenger-resource` / `avenger-image` only if targeted resource invalidation
  is not exposed through chart-app resources;
- `clap` with derive support;
- `notify`;
- `tokio` native runtime features;
- `tracing` and `tracing-subscriber`;
- `thiserror`;
- `indexmap` or standard collections as required.

Prefer re-exported chart/cache/app types over reaching through implementation
crates. Do not make the language compiler depend on the CLI or winit.

## Test Strategy

### Unit tests

- argument validation and option/default mapping;
- dependency-set normalization and anchor minimization;
- event filtering for create/modify/remove/rename/atomic replace;
- debounce with deterministic fake time;
- generation and dirty-state transitions;
- stale-result rejection;
- dependency-set replacement after success and failure;
- state compatibility/migration decisions;
- cache-profile option construction;
- diagnostic generation guarding, exactly-once stdout capture, and title state.

### Compiler/reload integration tests without a window

Use temporary project directories, the real strict compiler, isolated
generation contexts sharing one cache, and a fake host installer.

Required scenarios:

1. Edit the root chart and install a new scene.
2. Edit a relative definition and reload.
3. Edit a local theme/resource and reload/invalidate the resource.
4. Edit CSV/Parquet data and produce new values without stale cache hits.
5. Add a new import, then confirm it enters the watch set.
6. Remove an import, then confirm it leaves after a successful compile.
7. Introduce a syntax error, retain last-good chart, then repair it.
8. Delete a dependency and recreate it through an atomic save.
9. Generate multiple changes during one compile and install only the newest.
10. Change a param type/store schema and verify selective state reset.
11. Verify one failed generation emits one stdout diagnostic batch even when
    worker completion and host wake-up are coalesced/replayed.

### Physical-cache integration tests

- Two isolated contexts with equivalent plans/data and the same shared cache
  reuse a committed entry.
- Two isolated contexts with the same logical table name but different data or
  provider snapshot versions never cross-hit.
- Context-local version providers take precedence only for their own planner
  and do not reinterpret an older context's plan.
- With `min_seen_count: 1`, initial evaluation admits eligible writes and an
  unchanged data subtree hits after a style-only reload.
- Cache-enabled and cache-disabled reloads render equivalent scene graphs.
- A transform/query change does not hit the old result.
- A same-size local data replacement with controlled timestamp behavior cannot
  produce a stale hit.
- Inline memory data changes invalidate through content identity.
- Provider snapshot/version changes invalidate through the registered version
  provider.
- A failed reload leaves cache entries usable by the last-good chart.
- Metrics demonstrate reuse without asserting fragile exact node counts except
  in focused cache-unit fixtures.

### Winit host-swap tests

Factor the host-update application logic so most behavior is testable with a
fake canvas/window adapter. Test:

- app replacement installs one scene and requests one redraw;
- title/error suffix transitions;
- resize-policy changes;
- stale render invalidations from the old app cannot overwrite the new scene;
- the shared invalidation hub still wakes the replacement app;
- old event handlers no longer receive input.

A real native window smoke test may be ignored/headful in CI, but it should run
on macOS, Linux, and Windows release workflows when display infrastructure is
available.

### Visual/reload fixtures

Add a small project under:

```text
avenger-lang-cli/tests/fixtures/watch_project/
  chart.avenger
  marks/error_bar.mark.avenger
  data/data.csv
  theme.css
```

Maintain reviewed headless render baselines for:

- startup chart;
- style-only edit;
- definition edit;
- data edit;
- repaired chart after a failed generation.

Prefer existing `avenger-chart` baselines when the expected scene is already
covered. Generated output belongs under `target/tests/avenger-lang-cli/`.

### Manual acceptance matrix

Before declaring the milestone complete, verify on at least macOS plus one of
Linux/Windows, and add CI/manual coverage for the third platform:

- common editor in-place save;
- atomic rename save;
- rapid repeated save;
- dependency add/remove;
- syntax error and recovery;
- window resize before and after reload;
- active interaction followed by reload;
- close during compilation;
- cache enabled, `--no-cache`, and environment kill switch.

## Implementation Phases

Use the checkboxes as the progress tracker. Check a task only after its focused
tests pass; check a phase gate only after all tasks in that phase are complete.

### Phase 0 — API seams and CLI scaffold

- [x] Confirm language/compiler Phase 6, including its
  host-supplied isolated generation-environment factory and `CompileAttempt`
  dependency metadata for success/failure on currently supported source forms.
- [x] Add missing compiler APIs rather than reconstructing dependency graphs in
  the CLI.
- [x] Add a generation-aware single-chart compile entry point, or an equivalent
  one-shot environment contract, that retains the exact isolated context used
  for compilation and evaluation.
- [x] Add installation of an existing shared cache plus the minimum correct
  cache identity for Phase 6 inputs, with static thread-safety assertions.
- [x] Design and implement the supported `avenger-winit-wgpu` host-update seam.
- [x] Add `avenger-lang-cli` with library/binary targets and `clap` command
  parsing.
- [x] Implement `avenger watch --help`, argument validation, tracing setup, and
  exit-code mapping.
- [x] Add unit-test scaffolding, fake host, fake watcher, and fixture project.
- [x] **Phase 0 gate:** CLI help works and the required compiler/cache/winit
  seams have focused tests independent of a real window.

### Phase 1 — Compile and display one static chart

- [x] Create the cached context before compilation with the watch cache profile.
- [x] Compile one chart through `avenger_lang::Compiler`.
- [x] Build the initial generation's `ChartRuntimeResources` and
  `ChartAppBundle`.
- [x] Open the native winit window with correct title, scene sizing, and scale.
- [x] Render and interact with the chart until close.
- [x] Implement initial compile/runtime failure diagnostics and exits.
- [x] Capture initial compiler diagnostics to stdout exactly once.
- [x] Add a headful smoke example/test and headless startup baseline.
- [x] **Phase 1 gate:** `avenger watch chart.avenger` opens and displays the
  compiled chart, with physical cache installed and no file watching yet.

### Phase 2 — Dynamic dependency watching

- [x] Implement compiler dependency-set conversion to watch filters/anchors.
- [x] Implement the `notify` backend and fake backend.
- [x] Handle create/modify/remove/rename/atomic-save event forms.
- [x] Implement debounce, path accumulation, generation allocation, and one-job
  scheduling.
- [x] Dynamically replace watch sets after compile attempts.
- [x] Cover the root and every local import/theme/resource/missing-path form the
  Phase 6 compiler currently reports; keep dependency roles extensible.
- [x] Add deterministic unit/integration tests for the required event matrix.
- [x] **Phase 2 gate:** every relevant local dependency change produces exactly
  one debounced reload request and new dependencies become watchable.

### Phase 3 — Playable background reload and last-good app replacement

- [x] Add the background compile/evaluate worker.
- [x] Add generation supersession and dirty-while-compiling behavior.
- [x] Prepare replacement chart apps with generation-local runtime resources.
- [x] Install successful replacements through the winit host-update seam.
- [x] Update title, window sizing, dependency set, and redraw atomically.
- [x] Keep the last-good chart interactive on failure and deduplicate
  diagnostics.
- [x] Route each failed generation's compiler diagnostics through the
  coordinator's exactly-once stdout batch.
- [x] Recover automatically after the source is repaired.
- [x] Initialize replacement interaction state from its declarations; never
  migrate by name or opaque runtime ID.
- [x] Add root/current-local-dependency edit integration tests and visual
  baselines.
- [x] **Phase 3 playable gate:** a chart and every currently supported local
  dependency hot reload without blocking or recreating the window, failed
  generations never replace the last-good chart, and one physical cache is
  shared safely across isolated contexts.

### Phase 4 — Full dependency/cache correctness and state migration

- [x] Verify one cache is shared across isolated reload contexts.
- [x] As language definition and catalog phases land, cover definitions, local
  data/catalog inputs, globs/directories, consulted environment, and provider
  configuration in dependency discovery and dynamic watching.
- [x] Record the current function policy: the stock language exposes no custom
  UDF/UDAF registration, and every generation uses the same DataFusion/Avenger
  built-ins. Add semantic snapshots and conflict rejection when custom
  function registration becomes a language capability.
- [x] Add watch-specific cache configuration and CLI overrides.
- [x] Implement/verify trustworthy source versions for every local data source
  shape.
- [x] Verify generation-local resource caches reload changed local resources and
  that hub subscription swap prevents cross-generation invalidations.
- [x] Implement typed param/store/selection/tool-state migration.
- [x] Cancel active gestures and reset/rederive cursor on install.
- [x] Add cache metric delta logging.
- [x] Add cache correctness and state compatibility integration tests across
  the CLI, chart runtime, and physical-cache crates.
- [x] **Phase 4 gate:** style-only reloads demonstrably reuse eligible physical
  results, data changes never return stale results, and compatible interaction
  state survives reload.

Implementation checkpoint (2026-07-20):

- `avenger-lang-cli` has a fake compiler/host/reporter coordinator test that
  covers successful install, two explicit generations with identical errors,
  repair, typed migration, and a delayed stale generation discarded in favor
  of the newest request.
- The checked-in watch fixture and cache integration test cover a relative mark
  definition, catalog/schema source, CSV table, style-only reuse,
  cache-enabled/disabled scene equivalence, and a same-size CSV replacement
  with its original modification time restored.
- The compiler propagates resolver-owned migration keys onto final compiled
  param/store/selection registries, including generated built-in widget state.
  Runtime snapshot tests cover root and scoped values; native editor/focus,
  gestures, cursor, resources, and cache state are excluded by construction.
- The stock host compiles and prepares 37 visual fixture roots. The remaining
  visual roots require fixture-specific downstream extension/provider hosts;
  the two diagnostic roots are intentionally not runnable charts.

### Phase 5 — Hardening and milestone completion

- [x] Add cancellation and clean shutdown of watcher/worker/event loop.
- [x] Audit reload paths for panics, deadlocks, RefCell borrow overlap, and stale
  invalidation races.
- [x] Add file/import/resource size and event-burst limits inherited from the
  compiler.
- [x] Complete structured tracing and secret-redaction audit.
- [x] Run release-mode unit, integration, visual, app, and cache tests.
- [ ] Complete the manual OS/editor acceptance matrix.
- [x] Document installation and `watch` usage with the fixture project.
- [x] Record deferred CLI subcommands/features without scaffolding them.
- [ ] **Phase 5 gate:** all completion criteria below are satisfied.

Implementation checkpoint (2026-07-20):

- Watcher events use a bounded 64-entry latest-work queue and each burst retains
  at most 1,024 affected paths. Prepared host updates are latest-only, and
  completion acknowledgements cannot block the event loop.
- Window close and `Ctrl-C` stop new generations, cancel cooperative async work,
  drop the watcher, and join the worker. Watcher startup is bounded at ten
  seconds and shutdown at five seconds; timeout becomes an operational error
  rather than an unbounded wait.
- The CLI uses the fallible native-host constructor. Window/canvas/initial-scene
  setup and GPU out-of-memory failures return through the CLI instead of
  panicking or exiting successfully. Native wake-scheduler lock poisoning drops
  work with an error instead of panicking.
- Reload tracing records generation, request epoch, affected-path count,
  duration, migration totals, and optional cache deltas without table values,
  environment values, credentials, or cache keys. Default process output
  rewrites the canonical project root to project-relative paths. Compiler
  diagnostics intentionally retain authored source excerpts, so source must not
  contain secrets.
- Commits `23b79d68c` and `311249d27` add configurable compiler-owned bounds for
  project discovery, source and import closures, syntax/declaration nesting,
  SQL token/recursion complexity, definition expansion, and aggregate local
  file/directory/glob fingerprinting. The CLI inherits their safe defaults;
  its event queue and affected-path bursts are separately bounded.
- `cargo fmt --all -- --check`, strict no-dependency CLI Clippy, and the complete
  release suites for the CLI, language facade/compiler (including all fixture
  visual baselines), chart app, winit host, physical cache, and `avenger-chart`
  passed on macOS. Cross-platform manual acceptance remains open.

## Completion Criteria

The playable CLI milestone is complete when:

- [x] `avenger watch <chart.avenger>` displays a Phase 6 chart in one persistent
  native winit/wgpu window.
- [x] The root and every currently reported local dependency trigger background
  hot reload and dynamic watch-set updates.
- [x] Only the latest requested generation installs; failed reloads retain the
  last-good interactive chart, print diagnostics once, and recover after edit.
- [x] Window/GPU/cache identity persists while generation contexts and runtime
  resources remain isolated.
- [x] Enabled and disabled cache evaluation are scene-equivalent, and eligible
  unchanged work can reuse the shared cache.
- [x] Reload-state migration/reset behavior is documented and tested.
- [x] The focused unit, fake-host, integration, visual, and native smoke checks
  for Phases 0–3 pass.

The full v1 milestone is complete when:

- [x] `avenger watch <chart.avenger>` displays the compiled chart in one native
  winit/wgpu window.
- [x] Root chart, definition/import, local catalog/data, theme, and local
  resource changes trigger hot reload.
- [x] Atomic editor saves and dependency add/remove work.
- [x] Compilation/evaluation occurs off the event-loop thread.
- [x] Only the latest requested generation can be installed.
- [x] Failed reloads retain the last-good interactive chart and recover after a
  later edit.
- [x] The window, GPU canvas, and evaluation cache survive reload; the
  `SessionContext`, chart app, and runtime resources swap atomically by
  generation.
- [x] Physical-cache tests prove reuse for unchanged subplans and invalidation
  for changed data.
- [x] Compatible param/store/selection/tool state migrates; incompatible state
  resets safely.
- [x] The dependency watch set updates from compiler output after every attempt.
- [x] Default diagnostics are concise; detailed tracing/cache metrics are
  available without exposing secrets.
- [x] Every failed compilation generation prints its compiler diagnostics to
  stdout exactly once.
- [ ] Required tests, visual baselines, and native acceptance checks pass.

## Explicitly Deferred

- `avenger check`, `render`, `fmt`, `schema`, `info`, `tables`, `expand`, and
  project-gallery commands;
- watching directories as a gallery or accepting multiple chart arguments;
- multiple windows/tabs;
- remote dependency polling;
- browser/Wasm watch mode;
- in-window diagnostics/editor UI;
- automatic import pinning or source mutation;
- file watching as a chart-authored event feature;
- persistent on-disk physical cache;
- general process daemon/client architecture.

The architecture should leave room for these, but none should enlarge the
first `watch` implementation.
