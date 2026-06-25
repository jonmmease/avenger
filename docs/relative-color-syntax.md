# Relative Color Syntax

Avenger supports CSS-style relative color syntax in the chart theme system, with
the reusable color model and math implemented in `avenger-color`.

## Crate Boundaries

- `avenger-color` owns reusable color functionality:
  - `ColorSpace`
  - `ColorChannel`
  - `AbsoluteColor`
  - CSS color string parsing
  - color-space conversion
  - `color-mix()` math
  - contrast helpers
  - pure color interpolation
  - scene color and gradient model types
- `avenger-chart-core` owns chart theme syntax and evaluation:
  - `ThemeValue`
  - `CssRgba`
  - CSS selector parsing
  - CSS variable resolution
  - `light-dark()`
  - chart theme function resolution
  - relative color parsing and evaluation

The chart theme modules call into `avenger-color` for color primitives and keep
chart-specific CSS language behavior local to chart-core.

## Implementation Map

- `avenger-color/src/types.rs`: `ColorSpace`, `ColorChannel`, and
  `AbsoluteColor`.
- `avenger-color/src/convert.rs`: color-space conversion helpers.
- `avenger-color/src/mix.rs`: CSS-style color mixing.
- `avenger-color/src/contrast.rs`: WCAG contrast helpers.
- `avenger-chart-core/src/theme/parser.rs`: CSS parser support for relative
  color functions.
- `avenger-chart-core/src/theme/calc.rs`: calc expressions with color channel
  leaves.
- `avenger-chart-core/src/theme/color_component.rs`: relative color component
  resolution.
- `avenger-chart-core/src/theme/value.rs`: theme value evaluation into
  `CssRgba`.

## Terminology

CSS calls identifiers such as `l`, `c`, `h`, `r`, `g`, `b`, and `alpha`
channel keywords. In code, these are represented by `avenger_color::ColorChannel`.

## Validation

Useful focused checks:

```sh
cargo test --release -p avenger-color
cargo test --release -p avenger-chart-core theme
cargo test --release -p avenger-chart-core --doc
```
