# Avenger CLI

`avenger watch` compiles one Avenger chart into a persistent native
winit/wgpu window and hot reloads it when the chart or a compiler-reported
local dependency changes.

## Build and run

From the workspace:

```sh
cargo build --release -p avenger-lang-cli
target/release/avenger watch path/to/chart.avenger
```

During development, Cargo can launch the binary directly:

```sh
cargo run --release -p avenger-lang-cli -- watch \
  scratch/watch-playground/chart.avenger
```

The argument must be a local `*.avenger` chart root. Its containing directory
is the default project and capability root.

```text
avenger watch [OPTIONS] <CHART>

--project-root <DIR>       Override the project/capability root
--debounce-ms <MILLIS>     Filesystem quiet period (default: 100)
--scale <FACTOR>           Positive finite native render scale (default: 1)
--cache-memory-mb <MB>     Physical-result cache budget (default: 256)
--no-cache                 Disable physical-result caching
--log-cache                Include process-wide cache deltas after reload
```

`AVENGER_PHYSICAL_CACHE=0` is the environment-wide cache kill switch;
`--no-cache` is the explicit command-local equivalent. `RUST_LOG` controls
detailed tracing. Run `avenger watch --help` for the installed command help.

## Reload behavior

The compiler supplies the dependency closure. The watcher follows the chart
root, relative definitions/imports, catalog and schema files, local table
files, and reported directory/glob roots. Failed attempts contribute their
newly discovered and missing-path anchors while the last-good closure remains
watched; a successful install replaces that union with its exact closure.
Atomic editor saves, remove/recreate, and descendant changes under reported
directory/glob roots are supported.

Compilation, evaluation, and replacement-app construction run on the reload
worker, not the winit event-loop thread. A successful generation atomically
replaces the app, scene, interaction handlers, invalidation subscription,
title, and sizing policy in the existing window. The winit window, WGPU
canvas/device/surface, and process-long evaluation cache are retained. Every
generation receives a fresh isolated DataFusion `SessionContext` and runtime
resource bundle. If another change arrives, stale prepared work is discarded
and only the newest requested generation may install.

Compilation or preparation failures leave the last-good chart interactive.
The title receives a concise error suffix and the next successful install
restores it. One source-aware compiler diagnostic batch is written to stdout
for each failed generation; a later generation with the same error receives
its own single batch. CLI usage errors and watcher/window/GPU/runtime failures
go to stderr. The canonical project root is rewritten to `.` in process output;
success lines show project-relative affected paths and state migration totals;
cache metrics appear only with `--log-cache`.

## Cache and state guarantees

One bounded `EvaluationCache` is shared by isolated generation contexts. Each
context captures its own immutable local-resource content versions, so a file
replacement cannot reinterpret an older displayed generation. This includes
same-size replacements whose modification time is restored. Style-only
reloads can reuse eligible unchanged data results. Cache-enabled and disabled
evaluation are required to produce equivalent scenes.

Immediately before constructing a replacement app, watch captures typed
document state. Params, stores, and selections—including generated tool and
built-in widget document state—migrate only when compiler-derived migration
identity and the complete kind-specific compatibility contract match:

- params require the same Arrow physical type and sharing scope;
- stores require the same ordered field types/nullability, primary key, and
  sharing scope;
- selections require the same selection contract.

Root and facet-scoped values retain their owner paths. Removed, newly added,
unkeyed, or incompatible state uses the replacement source defaults; state is
never matched by source name or opaque runtime ID. Native editor/focus state,
active gestures and their start/previous snapshots, cursor state, runtime
resources, and cache entries are intentionally not part of the snapshot. An
interaction committed after snapshot capture but before installation may miss
that reload; this narrow race avoids blocking the displayed chart.

## Host and capability boundary

The stock binary installs the stock Avenger language registry and built-in
widget runtime factories. Projects requiring downstream custom marks,
coordinates, widgets, catalog providers, or table providers need a downstream
binary that composes those registrations; the stock CLI does not claim to host
extension fixtures. The automated stock-host matrix currently compiles and
prepares all 37 visual fixture roots whose manifest host is `stock`.

The command exposes no flags for HTTP imports, environment reads, object-store
credentials, or custom providers. Compiler defaults keep those capabilities
disabled; local file access is constrained to the canonical project root.
Default output does not print table contents, environment values, credentials,
or cache keys. Authored source excerpts remain part of compiler diagnostics,
so secrets should never be embedded directly in chart source.

Window close and `Ctrl-C` request clean event-loop termination. Shutdown stops
new generations, drops the native watcher, cancels an in-flight compile or app
preparation, and joins the worker. Watcher startup is limited to ten seconds and
worker shutdown to five seconds; exceeding either bound returns an operational
error instead of waiting indefinitely. Filesystem changes use a bounded
latest-work queue and bounded affected-path bursts.

The production source loader currently limits each Avenger source to 2 MiB and
HTTP redirect chains to five (HTTP imports remain disabled by this stock
command). Broader compiler hardening limits for import/declaration depth,
expansion and SQL complexity, and local resource trees belong to language
Phase 11 and are not yet a release claim of this CLI.

## Tests and fixture

[`tests/fixtures/watch_project`](tests/fixtures/watch_project) is the checked-in
integration project. It contains a chart, relative mark definition, catalog,
local CSV table, and typed state. The release-mode suite covers argument/exit
semantics, dependency filtering, fake-watcher replacement, a fake compiler and
host coordinator, diagnostics/recovery, cache reuse and invalidation, typed
state migration, the stock fixture matrix, and an ignored manual native-window
smoke test:

```sh
cargo test --release -p avenger-lang-cli -- --nocapture
cargo test --release -p avenger-lang-cli headful_watch_smoke \
  -- --ignored --nocapture
```

Formatting, checking, rendering, inspection protocols, LSP/DAP/MCP adapters,
multi-chart galleries, and additional CLI subcommands are separate milestones;
they are not partially scaffolded in this crate.
