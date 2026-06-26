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
| typst | 19,901,520 | 40,442 |

Typst adds 10,902,672 bytes, about 10.4 MiB, for this low-level PNG render
path. The script also asserts that the Typst probe dependency graph does not
contain `cosmic-text`.

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
