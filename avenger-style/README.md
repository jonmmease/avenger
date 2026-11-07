# avenger-style

The CSS style language of avenger charts. A stylesheet styles the nodes of a chart's style tree: the chart, its plots and marks, its guides and their parts, and its scales. The crate parses stylesheets with `cssparser` and `selectors`, checks them against the vocabulary of the style tree, reports each problem as a diagnostic at a byte span of the stylesheet's text, and computes the style of every node of a style tree by the CSS cascade.

The language is CSS: the syntax of rules, selectors, at-rules, and values is the syntax of the [CSS reference](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference). This README lists the parts of CSS that the language supports, the parts that it does not, and where it behaves differently from CSS in a web page.

```rust
use avenger_style::{ParseOptions, SourceId, Stylesheet};

let (sheet, diagnostics) = Stylesheet::parse(
    "mark[type=bar] { fill: #9e9e9e; corner-radius: 2; }",
    SourceId(1),
    ParseOptions::default(),
);
assert!(diagnostics.is_empty());
assert_eq!(sheet.rules.len(), 1);
assert_eq!(sheet.rules[0].declarations.len(), 2);
```

## The style tree

A renderer builds a style tree for each chart. Each node has an element, attributes, classes, and an id, and a selector matches nodes as a CSS selector matches the elements of an HTML document. The classes and the ids come from the chart. Ids can repeat: the plots of the facets of one table can share an id, and a selector matches each node with the id.

| Element | Node | Attributes |
|---|---|---|
| `chart` | The root, and each chart inside a composition. It has one `plot` child. | |
| `title`, `subtitle` | The title and the subtitle of a chart, or the title of a guide | |
| `footer`, `description` | Texts of a chart | |
| `plot` | The coordinate system of a chart, or the layout of a composition | `type`, `projection`, `discrete` |
| `mark` | The marks of a layer, or the box or the lasso that a selection tool draws | `type`, `store` |
| `axis` | An axis | `channel`, `scale`, `kind` |
| `legend` | A legend or a colorbar | `type`, `channel`, `scale`, `kind` |
| `header` | The header of a facet | `type` |
| `scale` | A scale, which is not drawn | `type`, `channel`, `kind`, `marks` |
| `attribution` | The credits of a plot's sources | |
| `graticule` | The graticule of a map | |
| `level` | A level of the axis of a nested band scale | |
| `label`, `tick`, `rule`, `grid` | The parts of an axis, a header, a colorbar, or a graticule, and the labels of marks | |
| `swatch`, `ramp` | The symbols of a legend and the color ramp of a colorbar | |
| `item`, `group`, `entry` | Virtual nodes: a mark item, a group of items, and a legend entry | |
| `widget` | A widget that writes a selection: a slider, a range slider, a radio group, a check box list, or a check box | `type`, `key` |
| `track`, `thumb`, `readout` | The parts of a slider: its track, its thumbs, and the readout of its value | |

The attributes have these values:

| Attribute | Value |
|---|---|
| `plot[type]` | `cartesian`, `polar`, or `geo` for a coordinate system, and `hconcat`, `vconcat`, `wrap`, `facet_row`, `facet_col`, or `facet_wrap` for a composition |
| `plot[projection]` | The projection of a map, such as `albers_usa` |
| `plot[discrete]` | The position channels whose scale is a band, point, or nested band scale, separated by spaces |
| `mark[type]` | Two forms: the kind of the mark, such as `bar`, `symbol`, or `shape`, and its name with the prefix of its coordinate system, such as `polar_bar` or `geo_shape`. A Cartesian mark has the same name in both forms. |
| `mark[store]` | The name of the selection whose box or lasso the mark draws |
| `legend[type]` | `colorbar` for the legend of a continuous color scale, and `symbol` for every other legend |
| `header[type]` | `row`, `col`, or `by`, the facet that the header labels |
| `widget[type]` | `slider`, `range_slider`, `radio`, `checkboxes`, or `checkbox` |
| `widget[key]` | The key of the widget, which its copies share |
| `channel` | The channel of the scale, such as `x`, `theta`, `fill`, or `stroke_dash` |
| `axis[scale]`, `legend[scale]`, `scale[type]` | The type of the scale, such as `linear`, `log`, `band`, or `ordinal` |
| `kind` | The kind of the scale: `categorical`, `ordinal`, `sequential`, or `diverging` |
| `scale[marks]` | The types of the marks that use the scale, separated by spaces, in the two forms of `mark[type]` |

An attribute selector on `mark[type]` or `scale[marks]` matches a node when it matches either form. `mark[type=bar]` selects the bars of Cartesian and polar plots, `mark[type=polar_bar]` selects the polar ones, and `mark[type^=polar_]` selects every mark of a polar plot. Either form gives the selector the same specificity.

## Selectors

The language supports these selectors:

- Element selectors, `*`, class selectors, id selectors, and attribute selectors with the operators `=`, `~=`, `|=`, `^=`, `$=`, and `*=`, and the flags `i` and `s`
- The combinators ` `, `>`, `+`, and `~`
- `:is()`, `:where()`, `:not()`, and `:has()`, with selector lists that are not forgiving: an invalid selector in the list makes the rule invalid
- `:root`, `:first-child`, `:last-child`, `:only-child`, `:nth-child()`, `:nth-last-child()`, the `of S` argument of both, and the `-of-type` forms of each
- `:focus-visible`, the only interaction pseudo-class
- Nested rules, with `&` or without it

A selector that names an element outside the style tree is an error, as is an attribute that the element does not have, or an attribute that no element has when the compound selector names no element. Element names and attribute names are case-sensitive. A class or an id that is not a CSS identifier needs CSS escapes, such as `#\31 st` for the id `1st`.

`:focus-visible` is valid only in the last compound selector of a rule, outside `:not()` and `:has()`, and only on the elements that keyboard focus reaches: `chart`, `axis`, `legend`, `mark`, `entry`, `group`, and `item`. A rule with `:focus-visible` sets only the outline properties, and the outline properties apply only through `:focus-visible`. Every selector of a rule, and every alternative of an `:is()` or a `:where()` in its last compound, ends with `:focus-visible`, or none does, so a list such as `mark:focus-visible, axis` is an error. The checks of elements, attributes, and focus apply to each alternative of `:is()` and `:where()` with the rest of its compound selector, and to a nested rule with its parent in place of `&`, so `title { &:focus-visible { ... } }` is an error, as `title:focus-visible` is. A virtual node has no attributes, classes, or id, so a selector matches it only as `item:focus-visible`, `group:focus-visible`, or `entry:focus-visible`.

The language does not support pseudo-elements, `:empty`, interaction pseudo-classes such as `:hover`, `:active`, and `:focus`, other pseudo-classes such as `:link` or `:checked`, or namespaces. An unsupported pseudo-class is an error with a hint: `:focus` suggests `:focus-visible`, and `:hover` suggests styling the marks of a selection with a selection condition.

## At-rules

`@import 'url'`, with an optional `layer` or `layer(name)`, imports a stylesheet. It comes before every rule except `@layer` statements, and it is valid only in a stylesheet file (`ParseOptions::allow_import`). `Sheet::load` parses a stylesheet and loads its imports, and theirs in turn, through a `Loader` that the caller gives, such as a closure that reads files. The loader receives each URL as the rule writes it, with the URL of the importing stylesheet, against which it resolves a relative URL. The standard stylesheets `std:base.css`, `std:reset.css`, and `std:vegalite.css` load without the loader. An import that does not load, of another `std:` URL, or that imports itself through other stylesheets is an error at the span of its URL, and the import loads nothing. The diagnostics of an imported stylesheet are at the spans of its own source, whose id the loader gives.

`@layer` declares cascade layers, as a statement that lists one name or several, such as `@layer base, theme;`, or as a block of one name. A block without a name is an anonymous layer. The layers of a stylesheet and of the stylesheets that it imports rank in the order of their first declarations whose conditions hold, so a statement before an `@import` decides the order of the layers that the import declares, and an `@layer` statement inside a false `@media` condition declares nothing. The layer `avenger` and its sublayers are reserved for the standard stylesheets (`ParseOptions::allow_reserved_layers`).

`@media` tests the preferences of the reader and the kind of output. It supports the media types `all`, `screen`, and `print`, `not` and `only`, and the features `prefers-color-scheme`, `prefers-contrast`, `prefers-reduced-motion`, and `forced-colors`, with or without a value. A chart has no viewport, so `@media` has no size features. A size feature such as `width` is an error that suggests `@container`.

`@container` tests the host box: the box that the host gives the chart, which is the only container. Its features are `width`, `height`, `aspect-ratio`, and `orientation`, with `min-` and `max-` prefixes or the range syntax, such as `(400px <= width < 800px)`, and `and`, `or`, and `not`. A size is a length in `px`, `rem`, or `em`. A container name is an error, as are the properties `container`, `container-type`, and `container-name`.

`@media` and `@container` can be nested inside style rules and inside each other. A rule inside several conditions applies when all of them hold. A test of a dimension that the host does not give is unknown, and an unknown condition is false, also under `not`.

`@palette name` defines a palette for the scales of colors, with the descriptors `kind` (`categorical`, `sequential`, or `diverging`, required), `colors` (required: at least one color for a categorical palette, two for a sequential one, and three for a diverging one), `interpolate` (`srgb`, `srgb-linear`, `lab`, `oklab`, `lch`, `oklch`, or `hsl`, by default `oklab`), and `cvd-safe` (`true` or `false`, by default `false`), which states that the colors stay distinguishable for readers with a color-vision deficiency. Palette names are case-insensitive.

Other at-rules, such as `@font-face`, `@keyframes`, `@supports`, `@scope`, and `@namespace`, are errors.

## Values

A declaration is a property, a shorthand, or a custom property such as `--accent`, with an optional `!important`. Property names and keywords are ASCII case-insensitive, and the names of custom properties are case-sensitive. A value with an unmatched `)`, `]`, or `}` is invalid, also in a custom property. A value can be a CSS-wide keyword: `initial`, `inherit`, or `unset`. The language does not support `revert` and `revert-layer`.

The lengths are `px`, `rem`, and `em`. A number without a unit is a number of pixels where a property takes a length, and a number of degrees where it takes an angle, so `stroke-width: 2` is 2 pixels. Angles also take `deg`, `grad`, `rad`, and `turn`. `1rem` is 10 pixels at a text scale of 1, and the host's text scale multiplies it. `calc()` combines numbers, lengths, angles, and percentages with `+`, `-`, `*`, and `/`. A value of `calc()` outside the range of its property is valid, and the cascade clamps it to the range. The language does not support viewport units, container units such as `cqw`, `min()`, `max()`, or `clamp()`. No value reads the size of the host box: a style changes with the size only at the breakpoints of `@container` conditions, and the layout of the chart fits its parts to the size. A container unit is an error with a hint to use `@container`.

`var(--name, fallback)` reads a custom property, and `env(name, fallback)` reads a value that the caller gives a stylesheet, such as a brand color. `Sheet::env_names` lists the names that a stylesheet and the stylesheets that it imports read, which are the parameters of its entry of a style table. Both are substituted when the styles are computed, so a declaration that reads them is checked then: a value that is invalid after substitution is a warning, and the declaration is `unset`. A reference without a value and without a fallback makes the value invalid. `env()` is not valid in a condition, because conditions read only the host box and the preferences.

Colors are the named colors, `transparent`, hexadecimal colors, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`, `color(srgb ...)`, `currentColor`, the system colors `Canvas`, `CanvasText`, `Highlight`, `HighlightText`, `GrayText`, and `LinkText`, `color-mix()` in `srgb`, `hsl`, `hwb`, `lab`, `lch`, `oklab`, or `oklch` with a hue interpolation method, and `contrast-color()`. The components of a color can be `calc()` expressions. A component or an alpha written as `none` is missing, and it draws as zero. `color-mix()` takes the other color's value for a missing component, also when the mix converts the color to another space, for the components of the same kind, such as the hues of `hsl()` and `oklch()`. Colors are normalized as CSS Color 4 specifies: the chroma of `lch()` and `oklch()` and the saturation of `hsl()` are at least zero, the lightness of `lab()`, `lch()`, `oklab()`, and `oklch()` is within its range, the channels of `rgb()` are from 0 to 255, and in its comma syntax they are all numbers or all percentages, and the alpha is from 0 to 1.

The shorthands are `padding` (one to four lengths, and with one number the paddings of a band or point scale), `border`, `outline`, `gap` (one length, or two for rows and then columns), `contain-intrinsic-size` (`none`, or one length, or two for the width and then the height), and `all`, which takes only a CSS-wide keyword.

## Properties

The language has the properties of this table. A declaration on a node whose element does not accept the property has no effect, but an inherited property still passes through it to its children. A property that the table does not list is a warning, not an error, and the rule keeps its other declarations.

The initial values of the table hold on every node except these, where they depend on the node:

- `orient`: `left` on an axis of `y` or `radius`, `right` on a legend, and `top` on a header and a title
- `direction`: `horizontal` on a legend at the top or the bottom
- `axes`: `outer` on a facet
- `columns`: `none` on an `hconcat`
- `max-lines`: `none` on a mark and a description, and `display`: `none` on a description
- `label-position`: `top` on a symbol mark
- `size`: `11` on a text mark, and `stroke-width`: `2` on a line mark
- `label-overlap`: `greedy` on the axis of a log scale, and `parity` on the axis of a sequential or a diverging scale
- `label-flush`: `true` on an x axis of a sequential or a diverging scale
- `padding-outer`: `0.5` on a point scale

<!-- properties -->
| Property | Values | Initial | Inherited | Elements |
|---|---|---|---|---|
| `width` | `<length [0,∞]> \| auto` | `auto` | no | `plot`, `mark` |
| `height` | `<length [0,∞]> \| auto` | `auto` | no | `plot`, `mark` |
| `contain-intrinsic-width` | `<length [0,∞]> \| none` | `none` | no | `plot` |
| `contain-intrinsic-height` | `<length [0,∞]> \| none` | `none` | no | `plot` |
| `aspect-ratio` | `<ratio> \| auto` | `auto` | no | `plot` |
| `step` | `<length [0,∞]> \| none` | `none` | no | `scale` |
| `padding-top` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `padding-right` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `padding-bottom` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `padding-left` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `columns` | `<integer [1,∞]> \| auto \| none` | `auto` | no | `plot`, `legend` |
| `max-columns` | `<integer [1,∞]> \| none` | `none` | no | `plot` |
| `min-column-width` | `<length [0,∞]> \| auto` | `auto` | no | `plot` |
| `align` | `true \| false \| auto \| left \| center \| right` | `auto` | no | `plot`, `mark` |
| `axes` | `outer \| each` | `each` | no | `plot` |
| `row-gap` | `<length [0,∞]>` | `0` | no | `plot`, `legend` |
| `column-gap` | `<length [0,∞]>` | `0` | no | `plot`, `legend` |
| `color` | `<color>` | `black` | yes | every node |
| `accent-color` | `auto \| <color>` | `auto` | yes | `widget`, `track`, `thumb` |
| `fill` | `auto \| none \| <color>` | `auto` | yes | `mark`, `title`, `subtitle`, `footer`, `description`, `attribution`, `label`, `tick`, `rule`, `grid`, `swatch`, `ramp`, `track`, `thumb`, `readout` |
| `stroke` | `auto \| none \| <color>` | `auto` | yes | `mark`, `title`, `subtitle`, `footer`, `description`, `attribution`, `label`, `tick`, `rule`, `grid`, `swatch`, `ramp`, `track`, `thumb`, `readout` |
| `opacity` | `<number [0,1]>` | `1` | no | `mark` |
| `fill-opacity` | `<number [0,1]>` | `1` | no | `mark` |
| `stroke-opacity` | `<number [0,1]>` | `1` | no | `mark` |
| `stroke-width` | `<length [0,∞]>` | `1` | no | `mark`, `tick`, `rule`, `grid` |
| `stroke-dash` | `none \| <length>+` | `none` | no | `mark`, `tick`, `rule`, `grid` |
| `stroke-dashoffset` | `<length>` | `0` | no | `mark` |
| `stroke-linecap` | `butt \| round \| square` | `butt` | no | `mark` |
| `stroke-linejoin` | `miter \| round \| bevel` | `miter` | no | `mark` |
| `stroke-miterlimit` | `<number [1,∞]>` | `4` | no | `mark` |
| `shape` | `<shape>` | `circle` | no | `mark`, `swatch` |
| `angle` | `<angle>` | `0` | no | `mark`, `title`, `axis` |
| `size` | `<number [0,∞]> \| <percentage>` | `30` | no | `mark` |
| `continuous-size` | `<length [0,∞]>` | `5` | no | `mark` |
| `bin-spacing` | `<length [0,∞]>` | `0` | no | `mark` |
| `band-position` | `<number [0,1]>` | `0.5` | no | `mark`, `axis` |
| `band-padding` | `<number [0,1]>` | `0` | no | `mark` |
| `corner-radius` | `<length [0,∞]>` | `0` | no | `mark` |
| `corner-radius-end` | `<length [0,∞]> \| auto` | `auto` | no | `mark` |
| `corner-radius-top-left` | `<length [0,∞]> \| auto` | `auto` | no | `mark` |
| `corner-radius-top-right` | `<length [0,∞]> \| auto` | `auto` | no | `mark` |
| `corner-radius-bottom-left` | `<length [0,∞]> \| auto` | `auto` | no | `mark` |
| `corner-radius-bottom-right` | `<length [0,∞]> \| auto` | `auto` | no | `mark` |
| `pad-angle` | `<angle>` | `0` | no | `mark` |
| `dx` | `<length>` | `0` | no | `mark` |
| `dy` | `<length>` | `0` | no | `mark` |
| `dx2` | `<length>` | `0` | no | `mark` |
| `dy2` | `<length>` | `0` | no | `mark` |
| `d-radius` | `<length>` | `0` | no | `mark` |
| `d-theta` | `<angle>` | `0` | no | `mark` |
| `inset` | `<length [0,∞]>` | `0` | no | `mark` |
| `baseline` | `top \| middle \| bottom \| alphabetic \| line-top \| line-bottom` | `middle` | no | `mark` |
| `width-mode` | `max \| fixed` | `max` | no | `mark` |
| `white-space` | `normal \| nowrap` | `normal` | no | `mark` |
| `text-overflow` | `clip \| ellipsis` | `clip` | no | `mark` |
| `radius-side` | `outward \| middle \| inward` | `middle` | no | `mark` |
| `theta-side` | `clockwise \| middle \| counterclockwise` | `middle` | no | `mark` |
| `text-orient` | `horizontal \| radial \| tangential` | `horizontal` | no | `mark` |
| `object-fit` | `contain \| fill` | `contain` | no | `mark` |
| `image-rendering` | `auto \| smooth \| pixelated` | `auto` | no | `mark` |
| `pattern` | `none \| <pattern>` | `none` | no | `mark`, `swatch` |
| `font-size` | `<length [0,∞]>` | `1rem` | yes | every node |
| `font-weight` | `normal \| bold \| <number [1,1000]>` | `normal` | yes | every node |
| `font-family` | `<family-name>#` | `sans-serif` | yes | every node |
| `font-style` | `normal \| italic` | `normal` | yes | every node |
| `line-height` | `normal \| <number [0,∞]> \| <length [0,∞]>` | `normal` | yes | every node |
| `max-width` | `<length [0,∞]> \| none` | `none` | no | `title`, `subtitle`, `footer`, `description`, `label`, `mark`, `attribution`, `readout` |
| `max-lines` | `<integer [1,∞]> \| none` | `1` | no | `title`, `subtitle`, `footer`, `description`, `label`, `mark`, `attribution`, `readout` |
| `text-align` | `start \| center \| end \| auto` | `auto` | no | `title`, `subtitle`, `footer`, `description`, `label`, `mark`, `attribution`, `readout` |
| `vertical-align` | `top \| middle \| bottom \| alphabetic \| auto` | `auto` | no | `title`, `subtitle`, `footer`, `description`, `label`, `mark`, `attribution`, `readout` |
| `title-frame` | `plot \| chart` | `chart` | no | `chart` |
| `label-angle` | `<angle>` | `0` | no | `axis`, `header`, `legend`, `level` |
| `orient` | `top \| bottom \| left \| right \| top-left \| top-right \| bottom-left \| bottom-right` | `bottom` | no | `axis`, `legend`, `header`, `title`, `attribution` |
| `label-orient` | `top \| bottom \| left \| right \| radial \| tangential` | `top` | no | `header`, `axis` |
| `title-orient` | `top \| bottom \| left \| right` | `top` | no | `header` |
| `direction` | `horizontal \| vertical` | `vertical` | no | `legend` |
| `wrap` | `flow \| grid` | `flow` | no | `legend` |
| `tick-count` | `<number [0,∞]> \| auto` | `auto` | no | `axis`, `legend` |
| `tick-spacing` | `<length [0,∞]> \| auto` | `auto` | no | `axis` |
| `max-entries` | `<integer [2,∞]> \| none` | `none` | no | `legend` |
| `tick-size` | `<length [0,∞]>` | `5` | no | `tick` |
| `label-padding` | `<length>` | `2` | no | `axis`, `legend`, `header`, `graticule`, `mark` |
| `title-padding` | `<length>` | `4` | no | `axis`, `legend`, `header` |
| `offset` | `<length>` | `0` | no | `axis`, `legend`, `header`, `title`, `attribution` |
| `label-offset` | `<length>` | `0` | no | `axis`, `mark` |
| `min-extent` | `<length [0,∞]> \| none` | `none` | no | `axis` |
| `max-extent` | `<length [0,∞]> \| none` | `none` | no | `axis` |
| `thickness` | `<length [0,∞]>` | `16` | no | `ramp` |
| `length` | `<length [0,∞]> \| auto` | `auto` | no | `ramp` |
| `max-length` | `<length [0,∞]> \| none` | `none` | no | `ramp` |
| `axis-gap` | `<length [0,∞]>` | `0` | no | `legend` |
| `label-overlap` | `none \| parity \| greedy` | `none` | no | `axis`, `legend`, `mark` |
| `label-separation` | `<length [0,∞]>` | `0` | no | `axis` |
| `label-flush` | `true \| false \| <length [0,∞]>` | `false` | no | `axis` |
| `tick-band` | `center \| extent` | `center` | no | `axis` |
| `z-index` | `<integer [0,∞]>` | `0` | no | `axis` |
| `separator` | `<string>` | `" · "` | no | `attribution` |
| `separator-space` | `<length [0,∞]>` | `4` | no | `attribution` |
| `line-gap` | `<length [0,∞]>` | `2` | no | `attribution` |
| `label-position` | `[inside \| inside-start \| inside-end \| outside \| middle \| top \| bottom \| left \| right \| top-left \| top-right \| bottom-left \| bottom-right]+` | `inside` | no | `mark` |
| `label-overflow` | `hide \| outside \| shrink \| show` | `show` | no | `mark` |
| `label-avoid` | `labels \| marks` | `labels` | no | `mark` |
| `label-min-font-size` | `<length [0,∞]>` | `0.7rem` | no | `mark` |
| `background` | `<color>` | `transparent` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `border-width` | `<length [0,∞]>` | `1` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `border-style` | `none \| solid \| dashed \| dotted` | `none` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `border-color` | `<color>` | `currentColor` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `border-radius` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution`, `widget` |
| `display` | `none \| block \| inline \| inline-block \| flex \| inline-flex \| grid \| inline-grid \| contents \| flow-root \| list-item \| table` | `block` | no | every node |
| `outline-width` | `<length [0,∞]>` | `3` | no | `chart`, `axis`, `legend`, `mark`, `entry`, `group`, `item`, `widget` |
| `outline-style` | `none \| solid \| dashed \| dotted` | `none` | no | `chart`, `axis`, `legend`, `mark`, `entry`, `group`, `item`, `widget` |
| `outline-color` | `<color>` | `currentColor` | no | `chart`, `axis`, `legend`, `mark`, `entry`, `group`, `item`, `widget` |
| `outline-offset` | `<length>` | `0` | no | `chart`, `axis`, `legend`, `mark`, `entry`, `group`, `item`, `widget` |
| `forced-color-adjust` | `auto \| none` | `auto` | yes | every node |
| `scheme` | `auto \| <palette-name> <integer>? reverse?` | `auto` | no | `scale` |
| `range` | `auto \| <output>#` | `auto` | no | `scale` |
| `unknown` | `auto \| <output>` | `auto` | no | `scale` |
| `other` | `auto \| <output>` | `auto` | no | `scale` |
| `padding-inner` | `<number [0,∞]> \| <number [0,∞]>{2,8}` | `0.1` | no | `scale` |
| `padding-outer` | `<number [0,∞]> \| <number [0,∞]>{2,8}` | `0.05` | no | `scale` |
<!-- /properties -->

## Diagnostics

`Stylesheet::parse` never fails. It returns the rules that parse and a `Diagnostic` for each problem, with its severity, its message, a span of the stylesheet's text, and sometimes a hint. An invalid declaration is dropped from its rule, an invalid selector or condition drops its rule, and parsing continues after each problem, as CSS parsing does. An unknown property is a warning, and every other problem is an error.

## Computing styles

```rust
use avenger_style::{
    Cascade, Element, Env, HostInputs, Paint, ParseOptions, Property, SourceId, StyleTable,
    StyleTree, Stylesheet,
};

let (sheet, _) = Stylesheet::parse(
    ".highlight { fill: crimson; }",
    SourceId(1),
    ParseOptions::default(),
);
let table = StyleTable::new().with_author(sheet, Env::new());

let mut tree = StyleTree::new(Element::Chart);
let plot = tree.add(tree.root(), Element::Plot);
tree.set_attribute(plot, "type", "cartesian");
let bars = tree.add(plot, Element::Mark);
tree.set_attribute_forms(bars, "type", "bar", "bar");
tree.add_class(bars, "highlight");

let styles = Cascade::new(&table).compute(&tree, &HostInputs::sized(400.0, 300.0));
assert_eq!(
    styles[bars].paint(Property::Fill),
    Paint::Color([220.0 / 255.0, 20.0 / 255.0, 60.0 / 255.0, 1.0])
);
```

A `StyleTable` holds the author stylesheets of an output, each with the values of its `env()` names. A table without stylesheets gives every node its initial values, and a stylesheet adds the base stylesheet with `@import 'std:base.css'`. `HostInputs` holds what the host gives a render: the width and the height of the host box, the preferences of the reader (color scheme, contrast, reduced motion, forced colors, print, the text scale, and the system colors), and a host theme, a stylesheet of the user origin. `Cascade::compute` gives the computed style of every node, with typed accessors such as `number`, `color`, `paint`, and `keyword`, and the warnings of the computation. Siblings with the same element, id, classes, and attributes and without inline styles share one computed style, unless a selector reads the positions of nodes among their siblings or their descendants, as `:nth-child()`, `+`, `~`, and `:has()` do. `examples/cascade_bench.rs` times the computation on a chart-sized tree.

The cascade orders the declarations that apply to a node as CSS does, from the lowest precedence to the highest: the normal declarations of the base stylesheet, of the host theme, and of the author stylesheets, then the inline styles, then the important declarations of the author stylesheets, of the host theme, and of the base stylesheet. The inline styles are the style fields of the chart, so a field wins over every normal declaration, and an important declaration of a stylesheet wins over the field. Within an origin, layers decide first: the layer `avenger` comes first, the other layers come in the order of their first declaration, the declarations of a layer come after those of its sublayers, and declarations outside layers come last. Important declarations reverse the order of the layers. Then specificity decides, and then the order of the declarations. A rule that matches a node through several of its selectors counts at the highest of their specificities.

A property without a declaration takes the value of the parent when it is inherited, and its initial value otherwise.

A custom property computes at the node that declares it, with the custom properties of that node and the `env()` values of its own stylesheet, and the children inherit the computed value. So `--b: var(--a)` on a plot keeps the plot's `--a` in the marks below it, also when a mark declares its own `--a`. Custom properties that refer to each other in a cycle, also through a fallback, are invalid, and an invalid custom property has no value, so a `var()` of it takes its fallback. `initial` removes an inherited custom property, and `inherit` and `unset` keep it. `1rem` is 10 pixels times the text scale, and `em` is the font size of the node, or of its parent for `font-size`. `currentColor` is the `color` of the node, or of its parent for `color`. A node that inherits a color that reads `currentColor`, such as `fill: currentColor` or a `color-mix()` of it, resolves it against its own `color`, as CSS Color 4 specifies.

`auto` paints by the kind of node: `fill: auto` is the `color` of a filled mark, a text, a swatch, or a ramp, `stroke: auto` is the `color` of a line, a trail, a rule, a tick, or a grid line, and the `fill` of the label of a mark contrasts with the mark that it labels. Other nodes paint nothing with `auto`. When the reader prefers forced colors and a stylesheet imports the base stylesheet, its important declarations give the system colors, and `forced-color-adjust: none` on a node keeps its own colors.

`ComputedStyles::needs_update` tells whether a render at new host inputs needs new styles: when the preferences or the host theme change, or when a condition changes its outcome. A resize within a breakpoint keeps the styles, so a renderer computes them again only when the host box crosses a breakpoint. Two `StyleTree`s are equal when their nodes have the same elements, ids, classes, attributes, inline styles, and places, so a renderer that builds the tree of an updated chart keeps the old computed styles when the trees are equal and the cascade and the host inputs are the same. On the tree of `examples/cascade_bench.rs`, the comparison takes about 1% of the time of `Cascade::compute`.

## Palettes

A palette is a named list of colors for the scales of colors, which the `scheme` property names, with an optional count and `reverse`. The built-in palettes are the color schemes of Vega 6 and `okabe-ito`, and they interpolate in sRGB:

- Categorical: `accent`, `category10`, `category20`, `category20b`, `category20c`, `dark2`, `observable10`, `okabe-ito`, `paired`, `pastel1`, `pastel2`, `set1`, `set2`, `set3`, `tableau10`, and `tableau20`
- Sequential: `blues`, `greens`, `greys`, `oranges`, `purples`, `reds`, `bluegreen`, `bluepurple`, `greenblue`, `orangered`, `purpleblue`, `purplebluegreen`, `purplered`, `redpurple`, `yellowgreen`, `yelloworangebrown`, `yelloworangered`, `yellowgreenblue`, `viridis`, `magma`, `inferno`, `plasma`, `cividis`, `rainbow`, `sinebow`, `turbo`, `browns`, `tealblues`, `teals`, `warmgreys`, `goldgreen`, `goldorange`, `goldred`, `lightgreyred`, `lightgreyteal`, `lightmulti`, `lightorange`, `lighttealblue`, `darkblue`, `darkgold`, `darkgreen`, `darkmulti`, and `darkred`
- Diverging: `blueorange`, `brownbluegreen`, `purplegreen`, `purpleorange`, `redblue`, `redgrey`, `redyellowblue`, `redyellowgreen`, `pinkyellowgreen`, and `spectral`

An `@palette` rule defines a palette, and of two rules with one name, the one with the higher cascade precedence wins. The colors of a palette resolve with the host's preferences, so a palette of system colors follows the color scheme. A palette whose colors read `env()` is checked after substitution, as a literal one is when it parses, and an invalid one is a warning and defines no palette. A rule can redefine a built-in name. `ComputedStyles::palette` looks a palette up by its name. A categorical palette fits categorical and ordinal scales, and a sequential or a diverging palette fits ordinal scales and scales of its own kind. An ordinal scale of `n` values samples a sequential or a diverging palette at `k / (n + 1)`, so no value gets an end color.

## Standard stylesheets

`std:base.css` is the base stylesheet. It gives the look of Vega-Lite 6 on a white background with the font Lato, and its `env()` names are `background`, `foreground`, `accent`, and `font-family`. It is in the user-agent origin whichever stylesheet imports it, so the host theme and the author stylesheets win over it, and it reads the `env()` values of the stylesheet that imports it. Its import takes no layer. `std:reset.css` sets every property of every node to its inherited or initial value in the layer `avenger.reset`, so a document that imports it removes the normal declarations of the base stylesheet and the host theme. `std:vegalite.css` gives the looks of Vega-Lite 6 that the base stylesheet does not, such as hollow symbols and the turned labels of a discrete x axis.
