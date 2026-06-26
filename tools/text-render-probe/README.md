# Text Render Probe

This probe measures the release binary size of a minimal WGPU PNG renderer that
draws a small text-only scene graph.

Run from the workspace root:

```bash
tools/text-render-probe/measure.sh
```

The script prints current macOS/Linux release-profile sizes for the `cosmic`
and owned `typst` paths, and asserts that the Typst probe dependency graph does
not contain `cosmic-text`.

The historical vendor-backed Typst path has been removed. Rerun this probe when
recording size changes; do not compare against the old `typst-vendor` numbers.

Current macOS arm64 results with the workspace `release` profile:

| Backend | Binary bytes | PNG bytes |
| --- | ---: | ---: |
| cosmic | 8,998,848 | 56,378 |
| typst | 8,333,472 | 42,692 |

Host-platform dependency counts from `cargo tree -p text-render-probe`:

| Backend | Total crates | Unique to backend |
| --- | ---: | ---: |
| cosmic | 166 | 13 |
| typst | 163 | 10 |

The cosmic-only crates include `cosmic-text`, `swash`, `harfrust`, `skrifa`,
`read-fonts`, `font-types`, `linebender_resource_handle`, `rangemap`,
`self_cell`, `sys-locale`, `unicode-linebreak`, `yazi`, and `zeno`.

The Typst-only side should be checked after major text-engine changes. The owned
path is expected to include shaping, font fallback, bidi/segmentation,
math-glyph path extraction, and optional `tiny-skia` rasterization, but not the
old vendored `avenger-typst-*` crates.

## Cargo Bloat

Install `cargo-bloat` once:

```bash
cargo install cargo-bloat
```

Then run:

```bash
tools/text-render-probe/bloat.sh
```

This writes crate-level bloat JSON for the cosmic and Typst builds, plus a full
ranking of Typst-only packages, to `target/text-render-probe-results/`.

Previous Typst bloat runs are no longer representative because they included
the vendored Typst crates. Regenerate this section after running:

```bash
tools/text-render-probe/bloat.sh
```

`cargo-bloat` reports symbol/code-section attribution, so it is best read as
"where compiled code is coming from." It does not fully explain stripped binary
file-size differences, data sections, proc-macro build costs, or code that is
inlined and attributed to a caller.
