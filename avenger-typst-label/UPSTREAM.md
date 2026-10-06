# Upstream Typst

This crate typesets labels with Typst's own pipeline. It depends on upstream's `typst-syntax`
and `typst-utils` crates for the parser and utilities, and ports the evaluator, realization, the
text and math libraries, and inline and math layout, reduced to what a single line needs. The
ported files mirror upstream's, so that upstream's fixes can be followed by diffing. The rest of
the crate is Avenger's: the label engine and its options around the pipeline, the formatting
functions, and the SVG, PDF and raster lowerers.

## Revision

Everything is ported from Typst 0.15.1
([`9dfd3a08`](https://github.com/typst/typst/tree/9dfd3a08500b7896045f907433cf7b4b02434fad)).
The crate depends on `typst-syntax` and `typst-utils` at exactly that version, and
[`tests/fixtures/typst-pin.toml`](tests/fixtures/typst-pin.toml) pins it for the diff tooling and
the reference fixtures, whose generators refuse any other release. Upstream paths starting with `crates/` are in the
Typst repository; Avenger paths are relative to this crate's `src/`.

## Provenance

- A ported file starts with `//! Ported from crates/<path> @ v0.15.1, modified for Avenger.`
  The rest is upstream's code and doc comments, verbatim except where an `avenger:` comment
  says otherwise: at the top of the module for what the port leaves out, and at each changed
  item for how it changed.
- A function that ports an upstream function into an Avenger file is marked
  `// upstream: crates/<path>::<function> @ v0.15.1`.
- Upstream's procedural macros have `macro_rules!` stand-ins: `elem!` and `func!` take
  upstream's `#[elem]` and `#[func]` items inside a block, and `cast!` and `derive_cast!` take
  the input of upstream's `cast!` and `#[derive(Cast)]`.
- Files without a header are Avenger's.

Ported code is licensed under the Apache License 2.0, as upstream is
([LICENSE-APACHE](LICENSE-APACHE)), and Avenger's code under the BSD 3-Clause License
([LICENSE](LICENSE)). [NOTICE](NOTICE) has the attributions.

## Mirror

| Status | Meaning |
|---|---|
| V | Every upstream item, verbatim. |
| S | A subset: upstream items removed, the rest verbatim. |
| A | Adapted: upstream items changed, or Avenger items added. |

`python3 tools/typst-upstream/upstream_diff.py status` computes the statuses below, and
`upstream_diff.py diff [FILE ...]` shows the changes. Both compare item by item after undoing
what the port changes mechanically: headers and module notes, imports, crate paths, the macro
stand-ins' wrappers, and doc-fence markers.

| Avenger file | Upstream file | Status |
|---|---|---|
| `typst_eval/call.rs` | `crates/typst-eval/src/call.rs` | A |
| `typst_eval/code.rs` | `crates/typst-eval/src/code.rs` | A |
| `typst_eval/markup.rs` | `crates/typst-eval/src/markup.rs` | A |
| `typst_eval/math.rs` | `crates/typst-eval/src/math.rs` | A |
| `typst_eval/mod.rs` | `crates/typst-eval/src/lib.rs` | A |
| `typst_eval/ops.rs` | `crates/typst-eval/src/ops.rs` | A |
| `typst_eval/vm.rs` | `crates/typst-eval/src/vm.rs` | A |
| `typst_layout/inline/collect.rs` | `crates/typst-layout/src/inline/collect.rs` | A |
| `typst_layout/inline/deco.rs` | `crates/typst-layout/src/inline/deco.rs` | V |
| `typst_layout/inline/line.rs` | `crates/typst-layout/src/inline/line.rs` | A |
| `typst_layout/inline/linebreak.rs` | `crates/typst-layout/src/inline/linebreak.rs` | A |
| `typst_layout/inline/mod.rs` | `crates/typst-layout/src/inline/mod.rs` | A |
| `typst_layout/inline/prepare.rs` | `crates/typst-layout/src/inline/prepare.rs` | A |
| `typst_layout/inline/shaping.rs` | `crates/typst-layout/src/inline/shaping.rs` | A |
| `typst_layout/math/accent.rs` | `crates/typst-layout/src/math/accent.rs` | A |
| `typst_layout/math/cancel.rs` | `crates/typst-layout/src/math/cancel.rs` | A |
| `typst_layout/math/fenced.rs` | `crates/typst-layout/src/math/fenced.rs` | A |
| `typst_layout/math/fraction.rs` | `crates/typst-layout/src/math/fraction.rs` | A |
| `typst_layout/math/fragment/glyph.rs` | `crates/typst-layout/src/math/fragment/glyph.rs` | A |
| `typst_layout/math/fragment/mod.rs` | `crates/typst-layout/src/math/fragment/mod.rs` | A |
| `typst_layout/math/line.rs` | `crates/typst-layout/src/math/line.rs` | A |
| `typst_layout/math/mod.rs` | `crates/typst-layout/src/math/mod.rs` | A |
| `typst_layout/math/radical.rs` | `crates/typst-layout/src/math/radical.rs` | A |
| `typst_layout/math/run.rs` | `crates/typst-layout/src/math/run.rs` | A |
| `typst_layout/math/scripts.rs` | `crates/typst-layout/src/math/scripts.rs` | A |
| `typst_layout/math/shaping.rs` | `crates/typst-layout/src/math/shaping.rs` | A |
| `typst_layout/math/text.rs` | `crates/typst-layout/src/math/text.rs` | A |
| `typst_layout/mod.rs` | `crates/typst-layout/src/lib.rs` | A |
| `typst_layout/rules.rs` | `crates/typst-layout/src/rules.rs` | A |
| `typst_layout/shapes.rs` | `crates/typst-layout/src/shapes.rs` | S |
| `typst_library/diag.rs` | `crates/typst-library/src/diag.rs` | A |
| `typst_library/engine.rs` | `crates/typst-library/src/engine.rs` | A |
| `typst_library/foundations/args.rs` | `crates/typst-library/src/foundations/args.rs` | A |
| `typst_library/foundations/array.rs` | `crates/typst-library/src/foundations/array.rs` | A |
| `typst_library/foundations/auto.rs` | `crates/typst-library/src/foundations/auto.rs` | A |
| `typst_library/foundations/bool.rs` | `crates/typst-library/src/foundations/bool.rs` | A |
| `typst_library/foundations/cast.rs` | `crates/typst-library/src/foundations/cast.rs` | A |
| `typst_library/foundations/content/element.rs` | `crates/typst-library/src/foundations/content/element.rs` | A |
| `typst_library/foundations/content/field.rs` | `crates/typst-library/src/foundations/content/field.rs` | A |
| `typst_library/foundations/content/mod.rs` | `crates/typst-library/src/foundations/content/mod.rs` | A |
| `typst_library/foundations/content/packed.rs` | `crates/typst-library/src/foundations/content/packed.rs` | A |
| `typst_library/foundations/dict.rs` | `crates/typst-library/src/foundations/dict.rs` | A |
| `typst_library/foundations/fields.rs` | `crates/typst-library/src/foundations/fields.rs` | A |
| `typst_library/foundations/float.rs` | `crates/typst-library/src/foundations/float.rs` | A |
| `typst_library/foundations/func.rs` | `crates/typst-library/src/foundations/func.rs` | A |
| `typst_library/foundations/int.rs` | `crates/typst-library/src/foundations/int.rs` | A |
| `typst_library/foundations/mod.rs` | `crates/typst-library/src/foundations/mod.rs` | A |
| `typst_library/foundations/module.rs` | `crates/typst-library/src/foundations/module.rs` | A |
| `typst_library/foundations/none.rs` | `crates/typst-library/src/foundations/none.rs` | A |
| `typst_library/foundations/ops.rs` | `crates/typst-library/src/foundations/ops.rs` | A |
| `typst_library/foundations/repr.rs` | `crates/typst-library/src/foundations/repr.rs` | S |
| `typst_library/foundations/scope.rs` | `crates/typst-library/src/foundations/scope.rs` | A |
| `typst_library/foundations/str.rs` | `crates/typst-library/src/foundations/str.rs` | A |
| `typst_library/foundations/styles.rs` | `crates/typst-library/src/foundations/styles.rs` | A |
| `typst_library/foundations/symbol.rs` | `crates/typst-library/src/foundations/symbol.rs` | A |
| `typst_library/foundations/ty.rs` | `crates/typst-library/src/foundations/ty.rs` | A |
| `typst_library/foundations/value.rs` | `crates/typst-library/src/foundations/value.rs` | A |
| `typst_library/layout/abs.rs` | `crates/typst-library/src/layout/abs.rs` | V |
| `typst_library/layout/align.rs` | `crates/typst-library/src/layout/align.rs` | A |
| `typst_library/layout/angle.rs` | `crates/typst-library/src/layout/angle.rs` | A |
| `typst_library/layout/axes.rs` | `crates/typst-library/src/layout/axes.rs` | V |
| `typst_library/layout/container.rs` | `crates/typst-library/src/layout/container.rs` | A |
| `typst_library/layout/corners.rs` | `crates/typst-library/src/layout/corners.rs` | S |
| `typst_library/layout/dir.rs` | `crates/typst-library/src/layout/dir.rs` | A |
| `typst_library/layout/em.rs` | `crates/typst-library/src/layout/em.rs` | V |
| `typst_library/layout/fr.rs` | `crates/typst-library/src/layout/fr.rs` | A |
| `typst_library/layout/frame.rs` | `crates/typst-library/src/layout/frame.rs` | A |
| `typst_library/layout/length.rs` | `crates/typst-library/src/layout/length.rs` | A |
| `typst_library/layout/mod.rs` | `crates/typst-library/src/layout/mod.rs` | S |
| `typst_library/layout/point.rs` | `crates/typst-library/src/layout/point.rs` | V |
| `typst_library/layout/ratio.rs` | `crates/typst-library/src/layout/ratio.rs` | A |
| `typst_library/layout/rel.rs` | `crates/typst-library/src/layout/rel.rs` | A |
| `typst_library/layout/sides.rs` | `crates/typst-library/src/layout/sides.rs` | A |
| `typst_library/layout/size.rs` | `crates/typst-library/src/layout/size.rs` | V |
| `typst_library/layout/spacing.rs` | `crates/typst-library/src/layout/spacing.rs` | A |
| `typst_library/layout/transform.rs` | `crates/typst-library/src/layout/transform.rs` | S |
| `typst_library/math/accent.rs` | `crates/typst-library/src/math/accent.rs` | A |
| `typst_library/math/attach.rs` | `crates/typst-library/src/math/attach.rs` | S |
| `typst_library/math/cancel.rs` | `crates/typst-library/src/math/cancel.rs` | A |
| `typst_library/math/equation.rs` | `crates/typst-library/src/math/equation.rs` | A |
| `typst_library/math/frac.rs` | `crates/typst-library/src/math/frac.rs` | A |
| `typst_library/math/ir/item.rs` | `crates/typst-library/src/math/ir/item.rs` | A |
| `typst_library/math/ir/mod.rs` | `crates/typst-library/src/math/ir/mod.rs` | A |
| `typst_library/math/ir/process.rs` | `crates/typst-library/src/math/ir/process.rs` | A |
| `typst_library/math/ir/resolve.rs` | `crates/typst-library/src/math/ir/resolve.rs` | A |
| `typst_library/math/lr.rs` | `crates/typst-library/src/math/lr.rs` | A |
| `typst_library/math/mod.rs` | `crates/typst-library/src/math/mod.rs` | A |
| `typst_library/math/op.rs` | `crates/typst-library/src/math/op.rs` | V |
| `typst_library/math/root.rs` | `crates/typst-library/src/math/root.rs` | V |
| `typst_library/math/style.rs` | `crates/typst-library/src/math/style.rs` | A |
| `typst_library/math/underover.rs` | `crates/typst-library/src/math/underover.rs` | V |
| `typst_library/mod.rs` | `crates/typst-library/src/lib.rs` | A |
| `typst_library/model/emph.rs` | `crates/typst-library/src/model/emph.rs` | V |
| `typst_library/model/mod.rs` | `crates/typst-library/src/model/mod.rs` | A |
| `typst_library/model/par.rs` | `crates/typst-library/src/model/par.rs` | A |
| `typst_library/model/strong.rs` | `crates/typst-library/src/model/strong.rs` | V |
| `typst_library/routines.rs` | `crates/typst-library/src/routines.rs` | A |
| `typst_library/symbols.rs` | `crates/typst-library/src/symbols.rs` | A |
| `typst_library/text/case.rs` | `crates/typst-library/src/text/case.rs` | A |
| `typst_library/text/deco.rs` | `crates/typst-library/src/text/deco.rs` | A |
| `typst_library/text/font/book.rs` | `crates/typst-library/src/text/font/book.rs` | V |
| `typst_library/text/font/exceptions.rs` | `crates/typst-library/src/text/font/exceptions.rs` | A |
| `typst_library/text/font/info.rs` | `crates/typst-library/src/text/font/info.rs` | A |
| `typst_library/text/font/metrics.rs` | `crates/typst-library/src/text/font/metrics.rs` | A |
| `typst_library/text/font/mod.rs` | `crates/typst-library/src/text/font/mod.rs` | A |
| `typst_library/text/font/tag.rs` | `crates/typst-library/src/text/font/tag.rs` | A |
| `typst_library/text/font/variant.rs` | `crates/typst-library/src/text/font/variant.rs` | A |
| `typst_library/text/font/variations.rs` | `crates/typst-library/src/text/font/variations.rs` | A |
| `typst_library/text/item.rs` | `crates/typst-library/src/text/item.rs` | A |
| `typst_library/text/lang.rs` | `crates/typst-library/src/text/lang.rs` | A |
| `typst_library/text/linebreak.rs` | `crates/typst-library/src/text/linebreak.rs` | V |
| `typst_library/text/mod.rs` | `crates/typst-library/src/text/mod.rs` | A |
| `typst_library/text/raw.rs` | `crates/typst-library/src/text/raw.rs` | A |
| `typst_library/text/shift.rs` | `crates/typst-library/src/text/shift.rs` | V |
| `typst_library/text/smallcaps.rs` | `crates/typst-library/src/text/smallcaps.rs` | V |
| `typst_library/text/smartquote.rs` | `crates/typst-library/src/text/smartquote.rs` | A |
| `typst_library/text/space.rs` | `crates/typst-library/src/text/space.rs` | S |
| `typst_library/visualize/color.rs` | `crates/typst-library/src/visualize/color.rs` | A |
| `typst_library/visualize/curve.rs` | `crates/typst-library/src/visualize/curve.rs` | S |
| `typst_library/visualize/mod.rs` | `crates/typst-library/src/visualize/mod.rs` | S |
| `typst_library/visualize/paint.rs` | `crates/typst-library/src/visualize/paint.rs` | A |
| `typst_library/visualize/shape.rs` | `crates/typst-library/src/visualize/shape.rs` | A |
| `typst_library/visualize/stroke.rs` | `crates/typst-library/src/visualize/stroke.rs` | A |
| `typst_realize/mod.rs` | `crates/typst-realize/src/lib.rs` | A |
| `typst_realize/spaces.rs` | `crates/typst-realize/src/spaces.rs` | A |

Avenger's files are `label/*` (the engine, options, parameters, errors, the public frame and its
lowering, the font world, `#numfmt` and `#datetimefmt`, and the test oracle), `typst_svg`,
`typst_pdf` and `typst_render` (lowerers after upstream's `typst-svg`, PDF and `typst-render`),
`typst_library/foundations/{elem,datetime}.rs` and `typst_library/text/font/outline.rs`.

## Deliberate divergences

Labels behave as upstream Typst does, except as listed here. The numbers are those of the design
decisions behind the divergences, which code comments cite, as in `(D22)`.

| | Divergence |
|---|---|
| D3 | Values without a text form are errors. Upstream displays booleans, dates, arrays and dictionaries as their code; a label rejects them with a hint, such as to format a date with `#datetimefmt`. |
| D4 | Integers and floats are content where content is expected, as in `frac(#n, 2)`, which upstream rejects. |
| D5 | A label is one line, so line breaks in data become spaces: in strings, parameters, formatted values and escaped line breaks, each run of line-break characters is one space. Explicit line breaks are errors. |
| D12 | An equation lays out at most 50,000 items, and is an error beyond that. Without upstream's memoization, nested `lr` groups with `mid` delimiters relayout exponentially. |
| D14 | `#text` takes fill, size, weight, style, font, lang, region, dir, baseline, tracking and features. Its other arguments are unexpected. |
| D22 | Colors are CSS colors: named colors are CSS's, so `red` is `#ff0000` where upstream's is `#ff4136`, and `rgb("…")` takes any CSS color string. Labels have no other color spaces, gradients or tilings, so errors that list the types a stroke takes don't mention them. |
| D25 | `compile_text` maps each run of line breaks in its text to one space, as data does (D5). |

Other differences:

- **Fallback.** Text falls back to the engine's sans-serif family, then to the emoji families;
  math to the engine's math family, then sans-serif, then emoji; raw text uses the engine's
  monospace family. Upstream falls back to Libertinus Serif. This is the frame oracle's one
  divergence, `text-unknown-family`.
- **Unsupported constructs** are errors in upstream's style ("… are not supported in labels"):
  statements and imports; matrices, vectors and case distinctions; block equations; alignment
  points and line breaks in math; raw text with a language or more than one line. Labels have
  no `#let`, `#set` or `#show`.
- **Parameters** are a scope above the library, so they shadow library names.
- **Raw blocks** of one line lay out inline. Upstream puts them in a block, which lays out the
  same (the `raw-fenced-one-line` frame case).
- **Empty text in math** (`$""$`) resolves to an empty group, where upstream's is a multiline
  item without rows. The frames agree; the IR oracle lists this case.
- **Limits.** A label's source, its number of equations and its math nesting are bounded
  (`LabelLimits`). The nesting depth counts nested math constructs in the parse tree, and is
  checked before any recursive pass; the default of 32 keeps the deepest label within a 1 MiB
  stack. Upstream's parser caps nesting at 256 levels, which chains of `/` bypass, and its
  evaluator grows the stack with `stacker`.

## Additions

- **D15. Label math style.** `EquationElem` has a ghost property, `label_style`, that its
  show-set rule applies as a document's `show math.equation: set text(..)` would: the math
  font and weight, and optionally a size relative to the text and a fill. Otherwise math
  inherits the text's size and fill.
- **Fallback lists.** `EquationElem::fallbacks` and `RawElem::label_font` carry the engine's
  fallback families (see above).
- **`#numfmt` and `#datetimefmt`** format numbers and dates with the engine's formatting
  providers (`label/format.rs`).

## Following upstream

1. List the mirrored files that a new release changes:
   `git -C ../typst diff --stat vOLD..vNEW -- $(python3 tools/typst-upstream/upstream_diff.py paths)`.
2. Dry-run upstream's changes on the ported files:
   `python3 tools/typst-upstream/upstream_diff.py bump vNEW`. It ports the changes' crate paths
   and reports, per file, the hunks that apply, that the file already has, and that fail
   because the port changed the code around them. Apply the changes, then check with
   `upstream_diff.py diff` that only Avenger's changes remain.
3. Move the pins: the headers, the `typst-syntax` and `typst-utils` versions in `Cargo.toml`,
   `tests/fixtures/typst-pin.toml` and the probe's checkout. Regenerate the reference fixtures (`tests/fixtures/README.md`), and run the
   suites.
