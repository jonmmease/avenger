# Avenger Chart App Guidelines

The repository-level `AGENTS.md` applies throughout this crate. The guidance
below is specific to running the native examples through Computer Use on macOS.

## Bundle an Example for Computer Use

Raw binaries launched by `cargo run --example ...` are not normal macOS app
bundles and cannot be discovered reliably by Computer Use. Build the example as
an `.app` instead:

```sh
avenger-chart-app/scripts/bundle-example.sh widget_region_cross_filter
```

The script builds the release example with the `winit-wgpu` feature and prints
its absolute `.app` path. It requires `cargo-bundle`; install it once with
`cargo install cargo-bundle --locked` if needed. Do not run `cargo clean` as a
normal part of this loop because the first release rebuild is expensive.

Every example target must have a matching
`[package.metadata.bundle.example.<example_name>]` table in `Cargo.toml` with a
unique human-readable name and reverse-DNS identifier. Update the metadata
inventory whenever an example is added, renamed, or removed.

## Operate the Bundled App

Pass the absolute `.app` path printed by the script to Computer Use
`get_app_state`. Prefer the path over the display name or bundle identifier: a
DMG build or stale target artifact can leave multiple discoverable copies with
the same identity. The development script deliberately uses `--format osx` so
it does not create that duplicate.

Call `get_app_state` before each interaction and again afterward. Avenger's
canvas currently exposes only its native window chrome through the macOS
accessibility tree, so use the returned screenshot for coordinate-based widget
and chart interactions when no semantic accessibility element is available.

After changing example or rendering code, rerun the bundling script before
testing. Cargo reuses the release artifacts, so cached rebuilds are normally
quick.
