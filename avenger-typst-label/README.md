# avenger-typst-label

Typesets labels written in [Typst](https://typst.app) markup, with inline math, for Avenger's
charts. A label is one paragraph: a single line, or several lines that end at explicit breaks or
wrap at a width. It compiles to a positioned frame of glyphs and shapes, which lowers to SVG
drawing items, PDF text runs, or a raster image.

The typesetting is Typst's own: the crate uses upstream's parser and ports Typst 0.15.1's
evaluator, realization, text and math libraries, and inline and math layout, reduced to one
paragraph. [UPSTREAM.md](UPSTREAM.md) maps the ported files, lists where labels deliberately differ,
and says how to follow upstream.

![Markup and math labels](../docs/images/typst-labels.png)

## Usage

In this example, `lato` and `math` hold the bytes of Lato and Lete Sans Math font files.

```rust
use std::sync::Arc;

use avenger_typst_label::{
    EngineOptions, LabelEngine, LabelOptions, LabelValue, LabelValues, RegisteredFont,
    SvgOptions, bind, svg_items,
};

let mut engine_options = EngineOptions::default();
engine_options.fonts.registered_fonts = vec![RegisteredFont::new(lato), RegisteredFont::new(math)];
engine_options.fonts.default_sans_serif_family = Some("Lato".into());
engine_options.fonts.default_math_family = Some("Lete Sans Math".into());
let engine = LabelEngine::new(engine_options)
    .with_number_formatting(Arc::new(avenger_format_number_d3::D3NumberFormatProvider::new()));

let mut options = LabelOptions::default();
options.text.font_size = 14.0;

let values = LabelValues::from([("r2".to_string(), LabelValue::Float(0.9412))]);
let markup = bind("*Fit* $R^2 = #numfmt(r2, \".2f\")$", &values)?;
let label = engine.compile(&markup, &options)?;
println!("{} × {} pt", label.metrics.width, label.metrics.height);
let svg = svg_items(&label, &SvgOptions::default());
```

- `LabelEngine` owns the fonts and caches and is cheap to clone. Fonts come from registered
  data, from `extra_font_dirs` and, unless `load_system_fonts` is off, from the system.
- `compile` takes markup. `compile_text` takes literal text and lays it out as
  `compile(&escape_text(text))` would. `measure` and `measure_text` return only the metrics.
- `CompiledLabel` has the `frame`, its `metrics`, the `semantic_text` for text extraction,
  `flags` and `warnings`. The metrics give the width, the height, the pitch of plain lines, and
  each line's left, right, top, baseline and bottom. A label aligns by its first line's
  baseline.
- `svg_items` lowers a label to paths (glyph outlines and shapes) and images (bitmap glyphs, such
  as color emoji). With `native_text`, text that an SVG viewer would draw exactly as the label
  does lowers to selectable text runs instead. `pdf_items` lowers a label to glyph runs in their
  fonts, including bitmap glyphs, and to paths. With the `raster` feature, `rasterize` renders it
  to an RGBA image.

### Options

`LabelOptions` sets one label's style:

- `text`: the font families (a CSS-style list such as `"Lato, sans-serif"`), size, fill, weight,
  style, language, region and direction. The language also picks smart quotes and, by default,
  the direction.
- `math`: the math font family. Math takes the text's size, fill and weight unless `math` sets
  them.
- `width`: how wide the label is. `Auto`, the default, ends lines only at explicit breaks.
  `Max(w)` wraps them at `w` points, as Typst wraps them, and the label is as wide as its widest
  line. `Fixed(w)` makes the label exactly `w` wide.
- `wrap`: whether lines wrap at the width, on by default. When it is off, lines end only at
  explicit breaks, even with a width, and a line wider than the width overflows it unless
  `ellipsis` cuts it.
- `align`: where lines sit within the label's width: `Start`, the default, `Left`, `Center`,
  `Right` or `End`. Start and end follow the text direction.
- `line_height`: the distance between baselines. `Auto`, the default, is Typst's spacing, 0.65em
  between one line's bottom and the next line's top. `Fixed(d)` spaces baselines `d` points
  apart, and `Relative(m)` spaces them a multiple of plain lines' spacing, so that a line with
  math stays on the grid of plain text. Lines can overlap.
- `max_lines`: the most lines the label keeps. Lines past the limit are dropped.
- `ellipsis`: whether "…" marks cut text. Each line wider than the width, and the last line when
  lines are dropped, ends in "…" and is shortened to fit. `flags.truncated` says whether text
  was cut.
- `hanging_signs`: whether a sign that starts a line, such as the `−` of `−1,234.5`, hangs out of
  the line by its full width, so that numbers align by their digits whatever their sign.
- `newline_breaks`: whether each newline in `compile_text`'s literal text ends a line, as `\`
  does in markup.
- `limits`: bounds on the source's size, its number of equations and how deep its math nests.

![Multi-line labels: widths, wrapping, alignment, line limits and ellipses, line heights, hanging signs and newline breaks](../docs/images/typst-multiline-labels.png)

The engine falls back to its sans-serif family and then to emoji fonts for text that the label's
fonts don't cover, and to its math family for math. `missing_font` chooses whether a family
missing from a label's font lists is silent, a warning or an error.

### Errors

A label that doesn't compile returns `LabelError::Source`, with Typst's message, the byte range
of the source it is about, and Typst's hints, or one of the variants for the limits and fonts.
Typst's warnings, such as for an unknown family, arrive in `CompiledLabel::warnings`.

## Boxes, rasters and fallbacks

For charts, the engine memoizes whole labels. A `Label` is a source, literal text or markup,
with its options. When a label's markup is invalid, `bounds` and `raster` lay out its source as
literal text, so a chart still shows it. They return limit and font errors. `compile` reports
invalid markup, for callers that validate it.

- `bounds` returns a label's box, a `TextBounds`. The box spans the label's lines, with the
  first line's top and the last line's bottom padded so that those lines are at least the font
  size tall. It also holds the gap that plain lines leave between such boxes, and a line box
  adds half of that gap above and below.
- `raster`, with the `raster` feature, rasterizes a whole label at a scale.
- `svg` returns a label's box and its drawing items in the box, as `svg_items` lowers them with
  native text runs. It isn't memoized, since exports draw each label once.

Clones share the memos, which tell labels apart by source and options. An engine with another
formatting provider starts new memos.

With the `bundled-fonts` feature, `bundled_font_options` registers the Lato, DejaVu Sans Mono
and Lete Sans Math faces that Avenger bundles as the default families, and it also loads the
system's fonts. `bundled_label_engine` returns a shared engine with these fonts.

## What labels support

Labels take Typst's markup syntax ([reference](https://typst.app/docs/reference/)) in one
paragraph. [docs/typst-support.md](docs/typst-support.md) goes through Typst's reference item by
item. In brief:

- Text, with Typst's whitespace rules, escapes, smart quotes and symbol shorthands.
- Line breaks: `\` and `#linebreak()`. Line breaks in data, such as in strings, are spaces.
- `*strong*` and `_emphasis_`, and `#strong`, `#emph`, `#underline`, `#overline`, `#strike`,
  `#highlight`, `#sub`, `#super`, `#smallcaps`, `#upper` and `#lower`.
- Inline raw text (`` `code` `` and `#raw`), on one line and without a language.
- `#text` with `fill`, `size`, `weight`, `style`, `font`, `lang`, `region`, `dir`, `baseline`,
  `tracking` and `features`.
- `#sym` and `#emoji` names, and the color constructors `rgb` and `luma`. Named colors are CSS
  colors, so `red` is `#ff0000`, and `rgb("…")` takes any CSS color string.
- Inline equations (`$…$`) with Typst's math: attachments and limits, fractions, `binom`, roots,
  accents, delimiters with `lr`, `mid` and the shorthands such as `abs` and `norm`, `cancel`,
  under- and over-braces, operators, `stretch`, `class`, alphabets such as `bold` and `cal`,
  and sizes such as `display`.
- The `calc` module's functions and constants, such as `calc.round(x, digits: 1)`,
  `calc.max(a, b)` and `calc.pi`.
- `#numfmt` and `#datetimefmt`, and `datetime` to build dates.

Labels can't use what needs more than a paragraph or a program: `#let`, `#set`, `#show`, loops
and imports, paragraph breaks, block equations, line breaks, matrices, vectors, case
distinctions and alignment points in math, and layout elements such as boxes, grids and images.
These are errors in Typst's style ("… are not supported in labels").

[UPSTREAM.md](UPSTREAM.md#deliberate-divergences) lists where labels deliberately differ from
Typst.

## Number and date formatting

`#numfmt(value, pattern)` and `#datetimefmt(value, pattern)` format values with the engine's
providers, which `with_number_formatting` and `with_datetime_formatting` set. A provider's
settings, such as its locale and timezone, apply to every pattern it prepares, and prepared
patterns are reused. Labels that don't format values need no provider. A label that formats a
value fails without one.

Exponent notation becomes math: `#numfmt(1234.5, ".1e")` lays out as 1.2 × 10³.

`datetime(year: 2024, month: 1, day: 5)` builds a date, and with `hour`, `minute` and `second`
a datetime, as in Typst. A label's `datetime` also takes `nanosecond`, for the time within a
second, and `utc: true`, which makes the datetime an instant that formatters show in their
timezone.

## Values

`bind` writes values into a label's markup, for the libraries that compute what labels show.
The markup typesets as the source would with each value bound to its name, as `#let` would bind
it: each reference to a name, `#name` in markup and code or `name` in math, becomes the value
written as code.

```rust
let values = LabelValues::from([("r2".to_string(), LabelValue::Float(0.9412))]);
let markup = bind("*Fit* $R^2 = #numfmt(r2, \".2f\")$", &values)?;
assert_eq!(markup, "*Fit* $R^2 = #numfmt((0.9412), \".2f\")$");
```

Values shadow the library's names. Names without a value are left for compilation to report. As
in Typst, a single letter in math displays as itself, so `$n$` shows the letter n even when `n`
has a value, and `$#n$` shows the value. Math that calls a value's name, as in `$rate(x)$`, is
an error, and `$rate (x)$` sets the value before the parentheses. `escape_text` writes a string
as markup that shows it literally.

## Size

The `typst-label-math-svg-probe` binary (feature `size-probe`) lays out one math label and
writes its drawing items as SVG. Built with the workspace's `release-size` profile on macOS
(Rust 1.96), it measures 1,862,240 bytes (1.78 MiB). A program that does the same through
upstream Typst's `typst`, `typst-layout` and `typst-svg` measured 17,002,560 bytes (16.21 MiB).

```bash
cargo build --profile release-size -p avenger-typst-label --features size-probe \
  --bin typst-label-math-svg-probe
```

## Tests

```bash
cargo test --release -p avenger-typst-label
cargo test --release -p avenger-typst-label --features raster,upstream-png-parity \
  --test upstream_png_parity
```

The default suite runs the oracles, which compare every case in `tests/fixtures` with upstream
Typst's frames, math IR and evaluation. The PNG suite compares rasterized labels with upstream's
renders. Both read checked-in references, and [UPSTREAM.md](UPSTREAM.md#tools) has the tools
that regenerate them.

`examples/label_bench.rs` times compilation, and `examples/gallery.rs` renders the image above:

```bash
cargo run --release -p avenger-typst-label --features raster --example gallery -- \
  docs/images/typst-labels.png
```

`examples/multiline.rs` renders the image of multi-line labels above, with their boxes outlined:

```bash
cargo run --release -p avenger-typst-label --features raster --example multiline -- \
  docs/images/typst-multiline-labels.png
```
