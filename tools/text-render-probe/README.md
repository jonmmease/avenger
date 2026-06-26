# Text Render Probe

This probe measures the release binary size of a minimal WGPU PNG renderer that
draws a small text-only scene graph.

Run from the workspace root:

```bash
tools/text-render-probe/measure.sh
```

Current macOS arm64 results with the workspace `release` profile:

| Backend | Binary bytes | PNG bytes |
| --- | ---: | ---: |
| cosmic | 8,998,848 | 56,378 |
| typst-vendor | 19,901,648 | 40,442 |
| typst-owned | 19,901,664 | 40,442 |

The current vendor Typst path adds 10,902,800 bytes, about 10.4 MiB, for this
low-level PNG render path. The current owned Typst path adds 10,902,816 bytes,
just 16 bytes more than vendor while it delegates to the vendor backend. The
script also asserts that neither Typst probe dependency graph contains
`cosmic-text`.

After aligning direct workspace dependency versions with the vendored Typst path
where practical, this probe uses a single `png`, `svgtypes`, `kurbo`, and `phf`
version. Remaining duplicate-version families are shared WGPU/platform
transitives (`bitflags`, `core-foundation`, `core-graphics-types`, `foldhash`,
`hashbrown`, `thiserror`) plus `rustc-hash` 1.x from `wgpu`/`naga` and 2.x from
Typst. The cosmic path also has its own internal split across `font-types`,
`read-fonts`, and `skrifa`.

Host-platform dependency counts from `cargo tree -p text-render-probe`:

| Backend | Total crates | Unique to backend |
| --- | ---: | ---: |
| cosmic | 177 | 17 |
| typst | 258 | 98 |

The cosmic-only crates are `cosmic-text`, `swash`, `harfrust`, `skrifa`,
`read-fonts`, `font-types`, `linebender_resource_handle`, `memmap2`, `rangemap`,
`self_cell`, `sys-locale`, `unicode-linebreak`, `yazi`, and `zeno`, including
two versions of the `skrifa`/`read-fonts`/`font-types` family.

The Typst-only side is dominated by the vendored `avenger-typst-*` crates, ICU
segmentation/properties data, Typst's Unicode/math helpers, `rustybuzz`,
`tiny-skia`, parser/cache utilities, and serialization/numeric support.

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

Current Typst bloat run:

| Metric | Value |
| --- | ---: |
| bloat binary file size | 25,562,256 bytes |
| `.text` section | 10,514,392 bytes |
| Typst-only attributed `.text` | 6,164,204 bytes |
| Typst-only packages with direct attribution | 56 / 98 |

Top Typst-only package attributions:

| Rank | Package | `.text` bytes |
| ---: | --- | ---: |
| 1 | `avenger-typst-library` | 3,863,880 |
| 2 | `avenger-typst-layout` | 553,680 |
| 3 | `regex-automata` | 318,192 |
| 4 | `rustybuzz` | 251,276 |
| 5 | `tiny-skia` | 246,936 |
| 6 | `regex-syntax` | 199,320 |
| 7 | `avenger-typst-syntax` | 144,292 |
| 8 | `aho-corasick` | 117,708 |
| 9 | `avenger-typst` | 68,480 |
| 10 | `time` | 66,748 |

`cargo-bloat` reports symbol/code-section attribution, so it is best read as
"where compiled code is coming from." It does not fully explain stripped binary
file-size differences, data sections, proc-macro build costs, or code that is
inlined and attributed to a caller.
