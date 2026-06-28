# Text Render Probe

This probe measures the release binary size of a minimal WGPU PNG renderer that
draws a small text-only scene graph.

Run from the workspace root:

```bash
tools/text-render-probe/measure.sh
```

The script prints current macOS/Linux release-profile sizes for the default
Typst text path, and asserts that the probe dependency graph does not contain
`cosmic-text`.

The historical vendor-backed Typst path has been removed. Rerun this probe when
recording size changes; do not compare against the old `typst-vendor` numbers.

Current macOS arm64 results with the workspace `release` profile:

| Backend | Binary bytes | PNG bytes |
| --- | ---: | ---: |
| typst | 8,333,472 | 42,692 |

Host-platform dependency counts from `cargo tree -p text-render-probe`:

| Backend | Total crates |
| --- | ---: |
| typst | 163 |

The Typst path should be checked after major text-engine changes. It is
expected to include shaping, font fallback, bidi/segmentation, math-glyph path
extraction, and optional `tiny-skia` rasterization, but not the old vendored
`avenger-typst-label-*` crates.

## Cargo Bloat

Install `cargo-bloat` once:

```bash
cargo install cargo-bloat
```

Then run:

```bash
tools/text-render-probe/bloat.sh
```

This writes crate-level bloat JSON for the default Typst build to
`target/text-render-probe-results/`.

Previous Typst bloat runs are no longer representative because they included
the vendored Typst crates. Regenerate this section after running:

```bash
tools/text-render-probe/bloat.sh
```

`cargo-bloat` reports symbol/code-section attribution, so it is best read as
"where compiled code is coming from." It does not fully explain stripped binary
file-size differences, data sections, proc-macro build costs, or code that is
inlined and attributed to a caller.
