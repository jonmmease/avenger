# avenger-style

The CSS style language of avenger charts. A stylesheet styles the nodes of a chart's style tree: the chart, its plots and marks, its guides and their parts, and its scales. The crate parses stylesheets with `cssparser` and `selectors`, checks them against the vocabulary of the style tree, and reports each problem as a diagnostic at a byte span of the stylesheet's text.

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

`@import 'url'`, with an optional `layer` or `layer(name)`, imports a stylesheet. It comes before every rule except `@layer` statements, and it is valid only in a stylesheet file (`ParseOptions::allow_import`). The caller loads the URLs.

`@layer` declares cascade layers, as a statement that lists one name or several, such as `@layer base, theme;`, or as a block of one name. A block without a name is an anonymous layer. The layers of a stylesheet and of the stylesheets that it imports rank in the order of their first declarations whose conditions hold, so a statement before an `@import` decides the order of the layers that the import declares, and an `@layer` statement inside a false `@media` condition declares nothing. The layer `avenger` and its sublayers are reserved (`ParseOptions::allow_reserved_layers`).

`@media` tests the preferences of the reader and the kind of output. It supports the media types `all`, `screen`, and `print`, `not` and `only`, and the features `prefers-color-scheme`, `prefers-contrast`, `prefers-reduced-motion`, and `forced-colors`, with or without a value. A chart has no viewport, so `@media` has no size features. A size feature such as `width` is an error that suggests `@container`.

`@container` tests the host box: the box that the host gives the chart, which is the only container. Its features are `width`, `height`, `aspect-ratio`, and `orientation`, with `min-` and `max-` prefixes or the range syntax, such as `(400px <= width < 800px)`, and `and`, `or`, and `not`. A size is a length in `px`, `rem`, or `em`. A container name is an error, as are the properties `container`, `container-type`, and `container-name`.

`@media` and `@container` can be nested inside style rules and inside each other. A rule inside several conditions applies when all of them hold. A test of a dimension that the host does not give is unknown, and an unknown condition is false, also under `not`.

`@palette name` defines a palette for the scales of colors, with the descriptors `kind` (`categorical`, `sequential`, or `diverging`, required), `colors` (required: at least one color for a categorical palette, two for a sequential one, and three for a diverging one), `interpolate` (`srgb`, `srgb-linear`, `lab`, `oklab`, `lch`, `oklch`, or `hsl`, by default `oklab`), and `cvd-safe` (`true` or `false`, by default `false`), which states that the colors stay distinguishable for readers with a color-vision deficiency. Palette names are case-insensitive.

Other at-rules, such as `@font-face`, `@keyframes`, `@supports`, `@scope`, and `@namespace`, are errors.

## Values

A declaration is a property, a shorthand, or a custom property such as `--accent`, with an optional `!important`. Property names and keywords are ASCII case-insensitive, and the names of custom properties are case-sensitive. A value with an unmatched `)`, `]`, or `}` is invalid, also in a custom property. A value can be a CSS-wide keyword: `initial`, `inherit`, or `unset`. The language does not support `revert` and `revert-layer`.

The lengths are `px`, `rem`, `em`, and the container units `cqw`, `cqh`, `cqmin`, and `cqmax`, which are fractions of the host box. A number without a unit is a number of pixels where a property takes a length, and a number of degrees where it takes an angle, so `stroke-width: 2` is 2 pixels. Angles also take `deg`, `grad`, `rad`, and `turn`. `1rem` is 10 pixels at a text scale of 1, and the host's text scale multiplies it. `calc()` combines numbers, lengths, angles, and percentages with `+`, `-`, `*`, and `/`. A value of `calc()` outside the range of its property is valid. The language does not support viewport units, `min()`, `max()`, or `clamp()`.

`var(--name, fallback)` reads a custom property, and `env(name, fallback)` reads a value that the caller gives a stylesheet, such as a brand color. Both are substituted when the styles are computed, so a declaration that reads them is checked then. `env()` is not valid in a condition, because conditions read only the host box and the preferences.

Colors are the named colors, `transparent`, hexadecimal colors, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`, `color(srgb ...)`, `currentColor`, the system colors `Canvas`, `CanvasText`, `Highlight`, `HighlightText`, `GrayText`, and `LinkText`, `color-mix()` in `srgb`, `hsl`, `hwb`, `lab`, `lch`, `oklab`, or `oklch` with a hue interpolation method, and `contrast-color()`. The components of a color can be `calc()` expressions. A component or an alpha written as `none` is missing, and it draws as zero. Colors are normalized as CSS Color 4 specifies: the chroma of `lch()` and `oklch()` and the saturation of `hsl()` are at least zero, the lightness of `lab()`, `lch()`, `oklab()`, and `oklch()` is within its range, the channels of `rgb()` are from 0 to 255, and in its comma syntax they are all numbers or all percentages, and the alpha is from 0 to 1.

The shorthands are `padding` (one to four lengths, and with one number the paddings of a band or point scale), `border`, `outline`, `gap` (one length, or two for rows and then columns), and `all`, which takes only a CSS-wide keyword.

## Properties

The language has the properties of this table. A declaration on a node whose element does not accept the property has no effect, but an inherited property still passes through it to its children. A property that the table does not list is a warning, not an error, and the rule keeps its other declarations.

<!-- properties -->
| Property | Values | Initial | Inherited | Elements |
|---|---|---|---|---|
| `width` | `<length [0,∞]> \| auto` | `auto` | no | `plot` |
| `height` | `<length [0,∞]> \| auto` | `auto` | no | `plot` |
| `aspect-ratio` | `<ratio> \| auto` | `auto` | no | `plot` |
| `step` | `<length [0,∞]> \| none` | `none` | no | `scale` |
| `padding-top` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution` |
| `padding-right` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution` |
| `padding-bottom` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution` |
| `padding-left` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution` |
| `columns` | `<integer [1,∞]> \| auto \| none` | `auto` | no | `plot`, `legend` |
| `max-columns` | `<integer [1,∞]> \| none` | `none` | no | `plot` |
| `min-column-width` | `<length [0,∞]> \| auto` | `auto` | no | `plot` |
| `align` | `true \| false \| auto \| left \| center \| right` | `auto` | no | `plot`, `mark` |
| `axes` | `outer \| each` | `each` | no | `plot` |
| `row-gap` | `<length [0,∞]>` | `0` | no | `plot`, `legend` |
| `column-gap` | `<length [0,∞]>` | `0` | no | `plot`, `legend` |
| `color` | `<color>` | `black` | yes | every node |
| `fill` | `auto \| none \| <color>` | `auto` | yes | `mark`, `title`, `subtitle`, `footer`, `description`, `attribution`, `label`, `tick`, `rule`, `grid`, `swatch`, `ramp` |
| `stroke` | `auto \| none \| <color>` | `auto` | yes | `mark`, `title`, `subtitle`, `footer`, `description`, `attribution`, `label`, `tick`, `rule`, `grid`, `swatch`, `ramp` |
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
| `max-width` | `<length [0,∞]> \| none` | `none` | no | `title`, `subtitle`, `footer`, `description`, `label`, `mark`, `attribution` |
| `max-lines` | `<integer [1,∞]> \| none` | `1` | no | `title`, `subtitle`, `footer`, `description`, `label`, `mark`, `attribution` |
| `text-align` | `start \| center \| end \| auto` | `auto` | no | `title`, `subtitle`, `footer`, `description`, `label`, `mark`, `attribution` |
| `vertical-align` | `top \| middle \| bottom \| alphabetic \| auto` | `auto` | no | `title`, `subtitle`, `footer`, `description`, `label`, `mark`, `attribution` |
| `title-frame` | `plot \| chart` | `chart` | no | `chart` |
| `label-angle` | `<angle>` | `0` | no | `axis`, `header`, `legend`, `level` |
| `orient` | `top \| bottom \| left \| right \| top-left \| top-right \| bottom-left \| bottom-right` | `bottom` | no | `axis`, `legend`, `header`, `title`, `attribution` |
| `label-orient` | `top \| bottom \| left \| right \| radial \| tangential` | `top` | no | `header`, `axis` |
| `title-orient` | `top \| bottom \| left \| right` | `top` | no | `header` |
| `direction` | `horizontal \| vertical` | `vertical` | no | `legend` |
| `wrap` | `flow \| grid` | `flow` | no | `legend` |
| `tick-count` | `<number [0,∞]>` | `10` | no | `axis`, `legend` |
| `tick-size` | `<length [0,∞]>` | `5` | no | `tick` |
| `label-padding` | `<length>` | `2` | no | `axis`, `legend`, `header`, `graticule`, `mark` |
| `title-padding` | `<length>` | `4` | no | `axis`, `legend`, `header` |
| `offset` | `<length>` | `0` | no | `axis`, `legend`, `header` |
| `label-offset` | `<length>` | `0` | no | `axis`, `mark` |
| `min-extent` | `<length [0,∞]> \| none` | `none` | no | `axis` |
| `max-extent` | `<length [0,∞]> \| none` | `none` | no | `axis` |
| `thickness` | `<length [0,∞]>` | `16` | no | `ramp` |
| `length` | `<length [0,∞]> \| auto` | `auto` | no | `ramp` |
| `max-length` | `<length [0,∞]> \| none` | `none` | no | `ramp` |
| `axis-gap` | `<length [0,∞]>` | `0` | no | `legend` |
| `label-overlap` | `none \| parity \| greedy` | `none` | no | `axis`, `mark` |
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
| `background` | `<color>` | `transparent` | no | `chart`, `plot`, `legend`, `attribution` |
| `border-width` | `<length [0,∞]>` | `1` | no | `chart`, `plot`, `legend`, `attribution` |
| `border-style` | `none \| solid \| dashed \| dotted` | `none` | no | `chart`, `plot`, `legend`, `attribution` |
| `border-color` | `<color>` | `currentColor` | no | `chart`, `plot`, `legend`, `attribution` |
| `border-radius` | `<length [0,∞]>` | `0` | no | `chart`, `plot`, `legend`, `attribution` |
| `display` | `none \| block \| inline \| inline-block \| flex \| inline-flex \| grid \| inline-grid \| contents \| flow-root \| list-item \| table` | `block` | no | every node |
| `outline-width` | `<length [0,∞]>` | `3` | no | `chart`, `axis`, `legend`, `mark`, `entry`, `group`, `item` |
| `outline-style` | `none \| solid \| dashed \| dotted` | `none` | no | `chart`, `axis`, `legend`, `mark`, `entry`, `group`, `item` |
| `outline-color` | `<color>` | `currentColor` | no | `chart`, `axis`, `legend`, `mark`, `entry`, `group`, `item` |
| `outline-offset` | `<length>` | `0` | no | `chart`, `axis`, `legend`, `mark`, `entry`, `group`, `item` |
| `forced-color-adjust` | `auto \| none` | `auto` | yes | every node |
| `scheme` | `auto \| <palette-name> <integer>? reverse?` | `auto` | no | `scale` |
| `range` | `auto \| <output>#` | `auto` | no | `scale` |
| `unknown` | `auto \| <output>` | `auto` | no | `scale` |
| `other` | `auto \| <output>` | `auto` | no | `scale` |
| `padding-inner` | `<number [0,1]>` | `0.1` | no | `scale` |
| `padding-outer` | `<number [0,∞]>` | `0.05` | no | `scale` |
<!-- /properties -->

## Diagnostics

`Stylesheet::parse` never fails. It returns the rules that parse and a `Diagnostic` for each problem, with its severity, its message, a span of the stylesheet's text, and sometimes a hint. An invalid declaration is dropped from its rule, an invalid selector or condition drops its rule, and parsing continues after each problem, as CSS parsing does. An unknown property is a warning, and every other problem is an error.
