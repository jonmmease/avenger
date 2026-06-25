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
| cosmic | 8,967,264 | 56,415 |
| typst | 19,754,208 | 40,472 |

Typst adds 10,786,944 bytes, about 10.3 MiB, for this low-level PNG render
path. The script also asserts that the Typst probe dependency graph does not
contain `cosmic-text`.

