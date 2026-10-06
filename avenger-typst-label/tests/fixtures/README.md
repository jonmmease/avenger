# Upstream fixtures

The label pipeline is tested against upstream Typst's own output. Every generated fixture comes
from the release pinned in [`typst-pin.toml`](typst-pin.toml), currently Typst 0.15.1. Both
generators refuse to run against any other release.

| Directory | Holds | Written by | Checked by |
|---|---|---|---|
| `upstream_frames/` | single-line text labels: frames | `tools/upstream-typst-probe` | `tests/upstream_oracle.rs` |
| `upstream_math/` | inline equations: frames and resolved math IR | `tools/upstream-typst-probe` | `tests/upstream_oracle.rs` |
| `upstream_png/` | PNG renders | `generate_upstream_png_refs` | `tests/upstream_png_parity.rs` |
| `fonts/` | fixture fonts (see its README) | `fonts/rebuild.py` | — |

The tests read only checked-in files; they never run Typst. Regenerating a reference is always
an explicit command.

## Frame and math references

Each `cases.toml` holds one-line label sources with their fonts and size. The probe wraps a case
in a box on an auto-sized page:

```typst
#set page(width: auto, height: auto, margin: 0pt)
#set text(font: "Lato", size: 12pt, weight: 500)
#show math.equation: set text(font: "Lete Sans Math", weight: 500)
#show raw: set text(font: "DejaVu Sans Mono")
#box[<source>]
```

It compiles the page with the fixture fonts only and writes `ref/{id}.json`:

- `frame`: the box's frame, the label's frame. It has width, height, baseline and items. Text
  items reference `fonts` by index. Each glyph is
  `[id, x_advance, x_offset, y_advance, y_offset, range, span, span_offset]`, with advances and
  offsets in em, `range` into the item's text, and `span` the byte range of the glyph's source
  node in the label source.
- `equations`: the resolved math IR of each inline equation, as upstream's
  `resolve_equation` returns it. It holds item kinds, slots, classes, sizes, crampedness,
  spacing, stretch targets and source spans. Properties are omitted when they have their
  default value, and `style` lists only the properties that differ from the parent component.
- `errors` and `warnings`: upstream's diagnostics, with label-relative byte ranges.

Floats are rounded to nine decimals. Regenerate after editing a manifest or moving the pin
(`../typst` must be checked out at the pinned commit):

```sh
cargo run --release --locked --manifest-path tools/upstream-typst-probe/Cargo.toml -- \
    avenger-typst-label/tests/fixtures/upstream_frames \
    avenger-typst-label/tests/fixtures/upstream_math
```

Pass `--check` to compare against the checked-in references without writing, and `--only <id>`
to run one case.

## Comparing

`tests/upstream_oracle.rs` flattens the reference frame and the label's frame to the same form:
metrics, positioned glyphs (font, glyph id, size, fill, origin) and the ink bounds of shapes. It
compares them within 0.001 pt. Four checks can fail:

| Check | Fails when |
|---|---|
| `metrics` | width, height or baseline differ |
| `glyphs` | a glyph is missing, extra, moved, resized or recolored |
| `rules` | a shape's ink bounds or paint differ |
| `source` | a glyph's source range differs; checked only where the cluster is a verbatim copy of its source |

Upstream's glyph spans are only exact for verbatim text, so case transforms, symbols, escapes,
smart quotes and parameters are not compared on `source`.

## Known failures

Each suite's `expected_failures.toml` lists the cases the current pipeline fails, with the
checks that fail, the reason, and the plan phase expected to fix them. The tests fail when a
case's failing checks change, including when a listed case starts passing.

Each run writes the outcomes in the same format to the gitignored
`tests/output/{suite}/census.toml`, along with the flattened expected and actual frames of every
failing case. To accept a change, copy the census over `expected_failures.toml` and review the
reasons:

```sh
cp avenger-typst-label/tests/output/upstream_frames/census.toml \
   avenger-typst-label/tests/fixtures/upstream_frames/expected_failures.toml
```
