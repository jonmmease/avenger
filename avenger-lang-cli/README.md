# Avenger CLI

The initial command displays one Avenger chart in a persistent native
winit/wgpu window and reloads it when the root or another compiler-reported
local dependency changes:

```sh
cargo run --release -p avenger-lang-cli -- watch path/to/chart.avenger
```

For an editable repository-local example:

```sh
cargo run --release -p avenger-lang-cli -- watch \
  scratch/watch-playground/chart.avenger
```

Successful reloads retain the native window, GPU canvas, and process-wide
physical-plan evaluation cache. Every compile generation gets a fresh isolated
DataFusion session. Failed reloads leave the last-good chart interactive and
print that generation's compiler diagnostics once to stdout.

The first playable milestone resets chart interaction state after a successful
reload. Typed migration of compatible params, stores, selections, tools, and
widget state is a subsequent milestone.

Run `avenger watch --help` for cache, debounce, project-root, and scale options.
