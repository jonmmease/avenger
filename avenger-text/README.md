# avenger-text

Lays out, measures and draws Avenger's text with the Typst label crate, `avenger-typst-label`.

## What it does

- `TextEngine` measures labels, rasterizes them, and lowers them to SVG drawing items and PDF
  glyph runs. Clones share the engine's fonts and its memos of measurements and rasters.
- A label is a `TextConfig`: its source and how it reads, its font, size, weight, style and fill,
  its layout, its parameters, and its formatting providers.
- The syntaxes differ in how they read a newline (`TextSyntaxMode`): plain text reads it as a
  space, `PlainLines` as a line break, and Typst markup as a space, with `\` breaking the line.
  Markup also has `$...$` math, styling and parameters.
- `TextLayout` holds the label crate's line options: a width, wrapping, a line limit, ellipses,
  a line height and line alignment.
- `TextBounds` is a label's box: its lines, with the first line's top and the last line's bottom
  padded to at least the font size. `calculate_origin` places the box: top, middle and bottom
  baselines place the box, line top and line bottom its line box, which adds half the gap between
  lines, and alphabetic the first line's baseline.
- A raster is a whole label. SVG output draws text as native runs where viewers draw it as the
  label does, and as outlines elsewhere, with bitmap glyphs as images. PDF output is glyph runs
  in the label's fonts. Both draw shapes, such as fraction lines, as paths.
- The `_with_plain_fallback` outputs show a label whose source is invalid as plain text. Limit
  and font errors don't fall back.

## Fonts

`TextEngine::new` takes the label crate's `FontOptions`. `default_font_options()` adds the bundled
Lato, DejaVu Sans Mono and Lete Sans Math faces to the system's, and `default_text_engine()` is an
engine with them that every caller shares.

## Formatting

`with_number_formatting` and `with_datetime_formatting` set the providers of `#numfmt` and
`#datetimefmt`. A label's own providers take their place, and caches tell providers apart by
identity. A new engine has none.

## Feature flags

- `serde` (default): serialization for the text types.
